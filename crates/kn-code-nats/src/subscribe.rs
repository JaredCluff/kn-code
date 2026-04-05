use crate::connection::NatsConnection;
use async_nats::Subscriber;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NatsMessage {
    pub subject: String,
    pub payload: Vec<u8>,
    pub reply_to: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

pub struct SubscriptionManager {
    connection: NatsConnection,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionHandle>>>,
}

struct SubscriptionHandle {
    #[allow(dead_code)]
    pub id: String,
    pub subject: String,
    #[allow(dead_code)]
    pub tx: mpsc::Sender<NatsMessage>,
    pub task: tokio::task::JoinHandle<()>,
}

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        if let Ok(mut subs) = self.subscriptions.try_write() {
            for (_, handle) in subs.drain() {
                handle.task.abort();
            }
        }
    }
}

impl SubscriptionManager {
    pub fn new(connection: NatsConnection) -> Self {
        Self {
            connection,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn subscribe(
        &self,
        subject: &str,
        buffer_size: usize,
    ) -> anyhow::Result<(String, mpsc::Receiver<NatsMessage>)> {
        let client = self.connection.client().await?;
        let subject_owned = subject.to_string();
        let subscriber = client
            .subscribe(subject_owned)
            .await
            .map_err(|e| anyhow::anyhow!("Subscribe failed: {}", e))?;

        let (tx, rx) = mpsc::channel(buffer_size);
        let id = Uuid::new_v4().to_string();
        let subject_str = subject.to_string();

        let task = tokio::spawn(Self::message_loop(
            subscriber,
            tx.clone(),
            subject_str.clone(),
        ));

        let handle = SubscriptionHandle {
            id: id.clone(),
            subject: subject.to_string(),
            tx,
            task,
        };

        self.subscriptions.write().await.insert(id.clone(), handle);
        tracing::info!("Subscribed to '{}' (id: {})", subject, id);

        Ok((id, rx))
    }

    pub async fn subscribe_with_queue_group(
        &self,
        subject: &str,
        queue_group: &str,
        buffer_size: usize,
    ) -> anyhow::Result<(String, mpsc::Receiver<NatsMessage>)> {
        let client = self.connection.client().await?;
        let subject_owned = subject.to_string();
        let subscriber = client
            .queue_subscribe(subject_owned.clone(), queue_group.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("Queue subscribe failed: {}", e))?;

        let (tx, rx) = mpsc::channel(buffer_size);
        let id = Uuid::new_v4().to_string();
        let subject_str = subject.to_string();

        let task = tokio::spawn(Self::message_loop(
            subscriber,
            tx.clone(),
            subject_str.clone(),
        ));

        let handle = SubscriptionHandle {
            id: id.clone(),
            subject: subject.to_string(),
            tx,
            task,
        };

        self.subscriptions.write().await.insert(id.clone(), handle);
        tracing::info!(
            "Queue subscribed to '{}' group '{}' (id: {})",
            subject,
            queue_group,
            id
        );

        Ok((id, rx))
    }

    pub async fn unsubscribe(&self, id: &str) -> anyhow::Result<()> {
        if let Some(handle) = self.subscriptions.write().await.remove(id) {
            handle.task.abort();
            tracing::info!("Unsubscribed from '{}' (id: {})", handle.subject, id);
        }
        Ok(())
    }

    pub async fn subscriptions(&self) -> Vec<(String, String)> {
        self.subscriptions
            .read()
            .await
            .iter()
            .map(|(id, handle)| (id.clone(), handle.subject.clone()))
            .collect()
    }

    async fn message_loop(
        mut subscriber: Subscriber,
        tx: mpsc::Sender<NatsMessage>,
        subject: String,
    ) {
        use futures::StreamExt;
        while let Some(msg) = subscriber.next().await {
            let headers = if let Some(ref hdrs) = msg.headers {
                if !hdrs.is_empty() {
                    let mut h = std::collections::HashMap::new();
                    for (k, v) in hdrs.iter() {
                        if let Some(val) = v.first() {
                            h.insert(k.to_string(), val.as_str().to_string());
                        }
                    }
                    Some(h)
                } else {
                    None
                }
            } else {
                None
            };

            let nats_msg = NatsMessage {
                subject: subject.clone(),
                payload: msg.payload.to_vec(),
                reply_to: msg.reply.map(|r| r.to_string()),
                headers,
            };

            if tx.send(nats_msg).await.is_err() {
                break;
            }
        }
    }
}
