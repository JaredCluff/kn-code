use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const CLIENT_TTL_SECS: u64 = 600;

#[derive(Debug, Clone)]
struct ClientInfo {
    tokens: f64,
    last_refill: Instant,
    last_access: Instant,
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    pub requests_per_minute: usize,
    pub max_concurrent: usize,
    clients: Arc<Mutex<HashMap<String, ClientInfo>>>,
}

impl RateLimiter {
    pub fn new(requests_per_minute: usize, max_concurrent: usize) -> Self {
        Self {
            requests_per_minute,
            max_concurrent,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn allow_request(&self, client_info: &mut ClientInfo) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(client_info.last_refill).as_secs_f64();
        let refill_rate = self.requests_per_minute as f64 / 60.0;
        client_info.tokens =
            (client_info.tokens + elapsed * refill_rate).min(self.requests_per_minute as f64);
        client_info.last_refill = now;
        client_info.last_access = now;

        if client_info.tokens >= 1.0 {
            client_info.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub async fn check_rate_limit(&self, client_id: &str) -> bool {
        let mut clients = self.clients.lock().await;

        let ttl = Duration::from_secs(CLIENT_TTL_SECS);
        clients.retain(|_, info| info.last_access.elapsed() < ttl);

        let client_info = clients
            .entry(client_id.to_string())
            .or_insert_with(|| ClientInfo {
                tokens: 1.0,
                last_refill: Instant::now(),
                last_access: Instant::now(),
            });
        self.allow_request(client_info)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60, 10)
    }
}

pub async fn rate_limit_middleware(
    limiter: Arc<RateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    rate_limit_middleware_with_limiter(limiter, request, next).await
}

pub async fn rate_limit_middleware_with_limiter(
    limiter: Arc<RateLimiter>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let client_ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if !limiter.check_rate_limit(&client_ip).await {
        tracing::warn!(
            client_ip = %client_ip,
            "Rate limit exceeded"
        );
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(request).await)
}
