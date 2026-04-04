use crate::connection::NatsConnection;
use async_nats::HeaderMap;

#[derive(Clone)]
pub struct Publisher {
    connection: NatsConnection,
}

impl Publisher {
    pub fn new(connection: NatsConnection) -> Self {
        Self { connection }
    }

    pub async fn publish(&self, subject: &str, payload: &[u8]) -> anyhow::Result<()> {
        let client = self.connection.client().await?;
        let subject = subject.to_string();
        let payload = payload.to_vec();
        client
            .publish(subject, payload.into())
            .await
            .map_err(|e| anyhow::anyhow!("Publish failed: {}", e))
    }

    pub async fn publish_with_headers(
        &self,
        subject: &str,
        payload: &[u8],
        headers: HeaderMap,
    ) -> anyhow::Result<()> {
        let client = self.connection.client().await?;
        let subject = subject.to_string();
        let payload = payload.to_vec();
        client
            .publish_with_headers(subject, headers, payload.into())
            .await
            .map_err(|e| anyhow::anyhow!("Publish with headers failed: {}", e))
    }

    pub async fn request(
        &self,
        subject: &str,
        payload: &[u8],
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<Vec<u8>> {
        let client = self.connection.client().await?;
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
        let subject = subject.to_string();
        let payload = payload.to_vec();

        let response = tokio::time::timeout(timeout, client.request(subject, payload.into()))
            .await
            .map_err(|_| {
                anyhow::anyhow!("Request timed out after {}ms", timeout_ms.unwrap_or(5000))
            })?
            .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

        Ok(response.payload.to_vec())
    }

    pub async fn publish_json<T: serde::Serialize>(
        &self,
        subject: &str,
        value: &T,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(value)?;
        self.publish(subject, &payload).await
    }
}
