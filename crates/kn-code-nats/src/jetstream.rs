use crate::connection::NatsConnection;
use async_nats::jetstream;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct JetStreamManager {
    connection: NatsConnection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    pub name: String,
    pub subjects: Vec<String>,
    pub max_bytes: Option<i64>,
    pub max_age_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub name: String,
    pub subjects: Vec<String>,
    pub messages: u64,
    pub bytes: u64,
    pub max_age_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsMessage {
    pub subject: String,
    pub payload: Vec<u8>,
    pub seq: u64,
}

impl JetStreamManager {
    pub fn new(connection: NatsConnection) -> Self {
        Self { connection }
    }

    pub async fn create_stream(&self, config: &StreamConfig) -> anyhow::Result<()> {
        let js = self.connection.jetstream().await?;

        let mut stream_config = jetstream::stream::Config {
            name: config.name.clone(),
            subjects: config.subjects.clone(),
            ..Default::default()
        };

        if let Some(max_bytes) = config.max_bytes {
            stream_config.max_bytes = max_bytes;
        }
        if let Some(max_age_secs) = config.max_age_secs {
            stream_config.max_age = std::time::Duration::from_secs(max_age_secs);
        }

        match js.create_stream(stream_config).await {
            Ok(_) => {
                tracing::info!("Created JetStream stream: {}", config.name);
                Ok(())
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("stream name already in use") || err_str.contains("10058") {
                    tracing::debug!("JetStream stream already exists: {}", config.name);
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Failed to create stream: {}", e))
                }
            }
        }
    }

    pub async fn stream_info(&self, name: &str) -> anyhow::Result<StreamInfo> {
        let js = self.connection.jetstream().await?;
        let mut stream = js
            .get_stream(name)
            .await
            .map_err(|e| anyhow::anyhow!("Stream not found: {}", e))?;

        let info = stream
            .info()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get stream info: {}", e))?;

        Ok(StreamInfo {
            name: info.config.name.clone(),
            subjects: info.config.subjects.clone(),
            messages: info.state.messages,
            bytes: info.state.bytes,
            max_age_secs: if info.config.max_age.as_secs() > 0 {
                Some(info.config.max_age.as_secs())
            } else {
                None
            },
        })
    }

    pub async fn delete_stream(&self, name: &str) -> anyhow::Result<()> {
        let js = self.connection.jetstream().await?;
        js.delete_stream(name)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete stream: {}", e))?;
        tracing::info!("Deleted JetStream stream: {}", name);
        Ok(())
    }

    pub async fn publish(
        &self,
        subject: &str,
        payload: &[u8],
        _msg_id: Option<&str>,
    ) -> anyhow::Result<u64> {
        let js = self.connection.jetstream().await?;
        let subject = subject.to_string();
        let payload = payload.to_vec();

        let ack = js
            .publish(subject, payload.into())
            .await
            .map_err(|e| anyhow::anyhow!("JetStream publish failed: {}", e))?;

        let ack = ack
            .await
            .map_err(|e| anyhow::anyhow!("JetStream publish ack failed: {}", e))?;

        Ok(ack.sequence)
    }

    pub async fn consume(
        &self,
        stream: &str,
        consumer_name: Option<&str>,
        batch: usize,
        timeout_ms: u64,
    ) -> anyhow::Result<Vec<JsMessage>> {
        let js = self.connection.jetstream().await?;
        let stream = js
            .get_stream(stream)
            .await
            .map_err(|e| anyhow::anyhow!("Stream not found: {}", e))?;

        let consumer = match consumer_name {
            Some(name) => stream
                .get_or_create_consumer(
                    name,
                    jetstream::consumer::pull::Config {
                        durable_name: Some(name.to_string()),
                        ack_policy: jetstream::consumer::AckPolicy::Explicit,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get/create consumer: {}", e))?,
            None => {
                return Err(anyhow::anyhow!(
                    "consumer_name is required — ephemeral consumers leak resources"
                ));
            }
        };

        let mut messages = Vec::new();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        use futures::StreamExt;
        let mut iter = consumer
            .messages()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get message iterator: {}", e))?;

        for _ in 0..batch {
            match tokio::time::timeout(timeout, iter.next()).await {
                Ok(Some(Ok(msg))) => {
                    let meta = msg
                        .info()
                        .map_err(|e| anyhow::anyhow!("Failed to get message info: {}", e))?;
                    messages.push(JsMessage {
                        subject: msg.subject.to_string(),
                        payload: msg.payload.to_vec(),
                        seq: meta.stream_sequence,
                    });
                    let _ = msg.ack().await;
                }
                _ => break,
            }
        }

        Ok(messages)
    }
}
