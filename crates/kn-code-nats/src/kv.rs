use crate::connection::NatsConnection;
use async_nats::jetstream;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct KvStore {
    connection: NatsConnection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub revision: u64,
}

impl KvStore {
    pub fn new(connection: NatsConnection) -> Self {
        Self { connection }
    }

    async fn get_bucket(&self, bucket: &str) -> anyhow::Result<jetstream::kv::Store> {
        let js = self.connection.jetstream().await?;
        js.create_key_value(jetstream::kv::Config {
            bucket: bucket.to_string(),
            ..Default::default()
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get KV bucket '{}': {}", bucket, e))
    }

    pub async fn put(&self, bucket: &str, key: &str, value: &[u8]) -> anyhow::Result<u64> {
        let store = self.get_bucket(bucket).await?;
        let value = value.to_vec();
        let key = key.to_string();
        store
            .put(key, value.into())
            .await
            .map_err(|e| anyhow::anyhow!("KV put failed: {}", e))
    }

    pub async fn put_string(&self, bucket: &str, key: &str, value: &str) -> anyhow::Result<u64> {
        self.put(bucket, key, value.as_bytes()).await
    }

    pub async fn get(&self, bucket: &str, key: &str) -> anyhow::Result<Option<KvEntry>> {
        let store = self.get_bucket(bucket).await?;
        match store.get(key).await {
            Ok(Some(entry)) => {
                let value = entry.to_vec();
                Ok(Some(KvEntry {
                    key: key.to_string(),
                    value,
                    revision: 0,
                }))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("KV get failed: {}", e)),
        }
    }

    pub async fn get_string(&self, bucket: &str, key: &str) -> anyhow::Result<Option<String>> {
        match self.get(bucket, key).await? {
            Some(entry) => Ok(Some(String::from_utf8_lossy(&entry.value).to_string())),
            None => Ok(None),
        }
    }

    pub async fn delete(&self, bucket: &str, key: &str) -> anyhow::Result<()> {
        let store = self.get_bucket(bucket).await?;
        store
            .delete(key)
            .await
            .map_err(|e| anyhow::anyhow!("KV delete failed: {}", e))
    }

    pub async fn keys(&self, bucket: &str, prefix: Option<&str>) -> anyhow::Result<Vec<String>> {
        let store = self.get_bucket(bucket).await?;
        let mut key_stream = store
            .keys()
            .await
            .map_err(|e| anyhow::anyhow!("KV keys failed: {}", e))?;

        use futures::StreamExt;
        let mut keys = Vec::new();
        while let Some(key_result) = key_stream.next().await {
            if let Ok(k) = key_result
                && prefix.map(|p| k.starts_with(p)).unwrap_or(true)
            {
                keys.push(k);
            }
        }

        Ok(keys)
    }
}
