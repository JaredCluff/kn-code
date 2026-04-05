use axum::{
    Router,
    http::HeaderValue,
    middleware,
    routing::{get, post},
};
use kn_code_auth::FileTokenStore;
use kn_code_config::Settings;
use kn_code_session::SessionStore;
use kn_code_tools::bash::BashTool;
use kn_code_tools::file_edit::FileEditTool;
use kn_code_tools::file_read::FileReadTool;
use kn_code_tools::file_write::FileWriteTool;
use kn_code_tools::glob::GlobTool;
use kn_code_tools::grep::GrepTool;
use kn_code_tools::traits::Tool;
use kn_code_tools::web_fetch::WebFetchTool;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::middleware::auth::{self, JwtAuth};
use crate::middleware::rate_limit::RateLimiter;
use crate::routes::{health, models, providers, run, sessions};
use crate::ws::{self, WsState};

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_concurrent_sessions: usize,
    pub request_timeout: Duration,
    pub jwt_secret: Option<String>,
    pub jwt_issuer: Option<String>,
    pub allowed_origins: Option<Vec<String>>,
    pub max_request_body_size: usize,
    pub session_store_dir: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3200,
            max_concurrent_sessions: 10,
            request_timeout: Duration::from_secs(600),
            jwt_secret: None,
            jwt_issuer: None,
            allowed_origins: None,
            max_request_body_size: 10 * 1024 * 1024,
            session_store_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp/.kn-code-fallback"))
                .join(".kn-code")
                .join("sessions"),
        }
    }
}

pub struct Server {
    config: ServerConfig,
}

impl Server {
    pub fn new(config: ServerConfig) -> Self {
        crate::middleware::logging::setup_logging();
        health::init_start_time();
        Self { config }
    }

    fn cors_layer(&self) -> CorsLayer {
        match &self.config.allowed_origins {
            Some(origins) => {
                let allowed: Vec<HeaderValue> =
                    origins.iter().filter_map(|o| o.parse().ok()).collect();
                CorsLayer::new()
                    .allow_origin(allowed)
                    .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                    .allow_headers([
                        axum::http::header::CONTENT_TYPE,
                        axum::http::header::AUTHORIZATION,
                    ])
                    .max_age(Duration::from_secs(3600))
            }
            None => CorsLayer::new()
                .allow_origin(axum::http::HeaderValue::from_static(
                    "http://localhost:3000",
                ))
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .max_age(Duration::from_secs(3600)),
        }
    }

    pub fn app(&self) -> Router {
        let cors = self.cors_layer();
        let rate_limiter = Arc::new(RateLimiter::default());

        let session_store = Arc::new(SessionStore::new(self.config.session_store_dir.clone()));

        let jwt_auth = if let (Some(secret), Some(issuer)) =
            (&self.config.jwt_secret, &self.config.jwt_issuer)
        {
            Some(Arc::new(JwtAuth::new(secret.clone(), issuer.clone())))
        } else {
            None
        };

        let ws_state = Arc::new(WsState {
            session_store: session_store.clone(),
            jwt_auth: jwt_auth.clone(),
        });

        let session_state = Arc::new(sessions::SessionState {
            session_store: session_store.clone(),
        });

        let token_store = Arc::new(FileTokenStore::new(
            Settings::config_dir().join("tokens.enc"),
        ));

        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(BashTool::default()),
            Arc::new(FileReadTool),
            Arc::new(FileWriteTool),
            Arc::new(FileEditTool),
            Arc::new(GlobTool),
            Arc::new(GrepTool),
            Arc::new(WebFetchTool),
        ];

        let run_state = Arc::new(run::RunState {
            session_store: session_store.clone(),
            token_store,
            tools,
        });

        let public_routes = Router::new()
            .route("/health", get(health::health))
            .route("/v1/providers", get(providers::list_providers))
            .route("/v1/models", get(models::list_models))
            .layer(cors.clone());

        let session_routes = Router::new()
            .route("/v1/sessions", get(sessions::list_sessions))
            .route("/v1/sessions/{session_id}", get(sessions::get_session))
            .route(
                "/v1/sessions/{session_id}/cancel",
                post(sessions::cancel_session),
            )
            .route(
                "/v1/sessions/{session_id}/transcript",
                get(sessions::get_transcript),
            )
            .with_state(session_state.clone());

        let jwt_auth_ref = jwt_auth.clone();
        let protected_routes = if let Some(jwt_auth) = jwt_auth_ref {
            Router::new()
                .route(
                    "/v1/run",
                    post(run::run_agent).with_state(run_state.clone()),
                )
                .merge(session_routes)
                .route("/v1/ws", get(ws::ws_handler).with_state(ws_state.clone()))
                .layer(middleware::from_fn(move |req, next| {
                    let auth = jwt_auth.clone();
                    auth::auth_middleware(auth, req, next)
                }))
                .layer(cors)
        } else {
            tracing::warn!(
                "No JWT secret configured — server running without authentication. Set KN_CODE_JWT_SECRET to enable auth."
            );
            Router::new()
                .route(
                    "/v1/run",
                    post(run::run_agent).with_state(run_state.clone()),
                )
                .merge(session_routes)
                .route("/v1/ws", get(ws::ws_handler).with_state(ws_state.clone()))
                .layer(cors)
        };

        public_routes
            .merge(protected_routes)
            .layer(middleware::from_fn(move |req, next| {
                let limiter = rate_limiter.clone();
                crate::middleware::rate_limit::rate_limit_middleware_with_limiter(
                    limiter, req, next,
                )
            }))
            .layer(TimeoutLayer::with_status_code(
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                self.config.request_timeout,
            ))
            .layer(RequestBodyLimitLayer::new(
                self.config.max_request_body_size,
            ))
            .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()))
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let app = self.app();
        let addr_str = format!("{}:{}", self.config.host, self.config.port);
        let addr: SocketAddr = addr_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid address {}: {}", addr_str, e))?;

        tracing::info!("Starting server on {}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        tracing::info!("Server shut down gracefully");
        Ok(())
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Received shutdown signal, shutting down...");
}
