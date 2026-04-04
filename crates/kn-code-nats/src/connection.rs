use crate::config::NatsConfig;
use async_nats::jetstream;
use secrecy::ExposeSecret;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct NatsConnection {
    pub config: NatsConfig,
    client: Arc<RwLock<Option<async_nats::Client>>>,
    jetstream: Arc<RwLock<Option<jetstream::Context>>>,
}

impl std::fmt::Debug for NatsConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsConnection")
            .field("config", &self.config)
            .finish()
    }
}

fn redact_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let mut redacted = parsed.clone();
        let _ = redacted.set_password(None);
        if parsed.username() != "" {
            let _ = redacted.set_username("***");
        }
        redacted.to_string()
    } else {
        url.to_string()
    }
}

impl NatsConnection {
    pub fn new(config: NatsConfig) -> Self {
        Self {
            config,
            client: Arc::new(RwLock::new(None)),
            jetstream: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn connect(&self) -> anyhow::Result<()> {
        let mut options = async_nats::ConnectOptions::new();

        match &self.config.auth {
            crate::config::NatsAuth::None => {}
            crate::config::NatsAuth::Token(token) => {
                options = options.token(token.expose_secret().clone());
            }
            crate::config::NatsAuth::UserPass { user, pass } => {
                options = options.user_and_password(user.clone(), pass.expose_secret().clone());
            }
            crate::config::NatsAuth::NKey(nkey) => {
                options = async_nats::ConnectOptions::with_nkey(nkey.expose_secret().clone());
            }
        }

        let client = options.connect(&self.config.url).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to NATS at {}: {}",
                redact_url(&self.config.url),
                e
            )
        })?;

        let js = jetstream::new(client.clone());

        *self.client.write().await = Some(client);
        *self.jetstream.write().await = Some(js);

        tracing::info!("Connected to NATS at {}", redact_url(&self.config.url));
        Ok(())
    }

    pub async fn client(&self) -> anyhow::Result<async_nats::Client> {
        let guard = self.client.read().await;
        guard
            .clone()
            .ok_or_else(|| anyhow::anyhow!("NATS not connected"))
    }

    pub async fn jetstream(&self) -> anyhow::Result<jetstream::Context> {
        let guard = self.jetstream.read().await;
        guard
            .clone()
            .ok_or_else(|| anyhow::anyhow!("JetStream not initialized"))
    }

    pub async fn is_connected(&self) -> bool {
        self.client.read().await.is_some()
    }

    pub async fn disconnect(&self) {
        if let Some(client) = self.client.write().await.take() {
            let _ = client.flush().await;
        }
        *self.jetstream.write().await = None;
        tracing::info!("Disconnected from NATS");
    }
}
