use crate::connection::NatsConnection;
use crate::kv::KvStore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

type AgentCapabilities = (Vec<String>, HashMap<String, String>);

const AGENTS_REGISTRY_BUCKET: &str = "agents-registry";
const AGENTS_ANNOUNCE_SUBJECT: &str = "agents.registry.announce";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub agent_id: String,
    pub capabilities: Vec<String>,
    pub metadata: HashMap<String, String>,
    pub last_seen: DateTime<Utc>,
}

pub struct AgentRegistry {
    connection: NatsConnection,
    kv: KvStore,
    instance_id: String,
    heartbeat_task: Option<tokio::task::JoinHandle<()>>,
    heartbeat_cancel: Option<CancellationToken>,
    announced_capabilities: Arc<RwLock<AgentCapabilities>>,
}

impl std::fmt::Debug for AgentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRegistry")
            .field("instance_id", &self.instance_id)
            .finish()
    }
}

impl AgentRegistry {
    pub fn new(connection: NatsConnection, instance_id: String) -> Self {
        Self {
            kv: KvStore::new(connection.clone()),
            connection,
            instance_id,
            heartbeat_task: None,
            heartbeat_cancel: None,
            announced_capabilities: Arc::new(RwLock::new((Vec::new(), HashMap::new()))),
        }
    }

    pub async fn announce(
        &self,
        capabilities: Vec<String>,
        metadata: HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let record = AgentRecord {
            agent_id: self.instance_id.clone(),
            capabilities: capabilities.clone(),
            metadata: metadata.clone(),
            last_seen: Utc::now(),
        };

        *self.announced_capabilities.write().await = (capabilities, metadata);

        let value = serde_json::to_vec(&record)?;
        self.kv
            .put(AGENTS_REGISTRY_BUCKET, &self.instance_id, &value)
            .await?;

        let client = self.connection.client().await?;
        if let Err(e) = client.publish(AGENTS_ANNOUNCE_SUBJECT, value.into()).await {
            tracing::warn!(
                "Failed to publish agent announcement for {}: {}",
                self.instance_id,
                e
            );
        }

        tracing::info!(
            "Agent announced: {} with capabilities: {:?}",
            self.instance_id,
            record.capabilities
        );
        Ok(())
    }

    pub async fn discover(&self, capability: Option<&str>) -> anyhow::Result<Vec<AgentRecord>> {
        let keys = self.kv.keys(AGENTS_REGISTRY_BUCKET, None).await?;
        let mut agents = Vec::new();

        for key in keys {
            if let Some(entry) = self.kv.get(AGENTS_REGISTRY_BUCKET, &key).await?
                && let Ok(record) = serde_json::from_slice::<AgentRecord>(&entry.value)
                && capability.is_none_or(|cap| record.capabilities.iter().any(|c| c == cap))
            {
                agents.push(record);
            }
        }

        Ok(agents)
    }

    pub fn start_heartbeat(&mut self, interval_secs: u64) {
        let instance_id = self.instance_id.clone();
        let kv = KvStore::new(self.connection.clone());
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        let capabilities = self.announced_capabilities.clone();

        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                tokio::select! {
                    _ = child.cancelled() => {
                        tracing::info!("Heartbeat task cancelled for {}", instance_id);
                        break;
                    }
                    _ = interval.tick() => {
                        let guard = capabilities.read().await;
                        let record = AgentRecord {
                            agent_id: instance_id.clone(),
                            capabilities: guard.0.clone(),
                            metadata: guard.1.clone(),
                            last_seen: Utc::now(),
                        };
                        if let Ok(value) = serde_json::to_vec(&record)
                            && let Err(e) = kv.put(AGENTS_REGISTRY_BUCKET, &instance_id, &value).await {
                                tracing::warn!("Heartbeat KV write failed for {}: {}", instance_id, e);
                        }
                    }
                }
            }
        });

        self.heartbeat_task = Some(task);
        self.heartbeat_cancel = Some(cancel);
    }

    pub fn stop_heartbeat(&mut self) {
        if let Some(cancel) = self.heartbeat_cancel.take() {
            cancel.cancel();
        }
        if let Some(task) = self.heartbeat_task.take() {
            task.abort();
        }
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

impl Drop for AgentRegistry {
    fn drop(&mut self) {
        self.stop_heartbeat();
    }
}
