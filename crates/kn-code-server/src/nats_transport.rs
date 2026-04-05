use crate::headless::events::SdkEvent;
use futures::StreamExt;
use kn_code_nats::{NatsConfig, NatsConnection, Publisher};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct NatsTransport {
    pub config: NatsConfig,
    pub connection: NatsConnection,
    pub publisher: Publisher,
    pub subject_prefix: String,
    pub instance_id: String,
    pub tasks: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
}

impl NatsTransport {
    pub fn new(config: NatsConfig) -> Self {
        let instance_id = config.instance_id();
        let connection = NatsConnection::new(config.clone());
        let publisher = Publisher::new(connection.clone());
        let subject_prefix = "kn-code".to_string();

        Self {
            config,
            connection,
            publisher,
            subject_prefix,
            instance_id,
            tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn connect(&self) -> anyhow::Result<()> {
        self.connection.connect().await?;

        let cmd_subject = format!("{}.{}.in.command", self.subject_prefix, self.instance_id);
        self.subscribe(&cmd_subject).await?;

        let task_subject = format!("{}.{}.in.task", self.subject_prefix, self.instance_id);
        self.subscribe(&task_subject).await?;

        for sub in &self.config.startup_subs {
            self.subscribe(sub).await?;
        }

        tracing::info!(
            "NATS transport connected — instance: {}, prefix: {}",
            self.instance_id,
            self.subject_prefix
        );

        Ok(())
    }

    pub async fn emit(&self, event: &SdkEvent) -> anyhow::Result<()> {
        let subject = format!("{}.{}.out.events", self.subject_prefix, self.instance_id);
        let payload = serde_json::to_vec(event)?;
        self.publisher.publish(&subject, &payload).await?;
        Ok(())
    }

    pub async fn emit_batch(&self, events: &[SdkEvent]) -> anyhow::Result<()> {
        for event in events {
            self.emit(event).await?;
        }
        Ok(())
    }

    pub async fn subscribe(&self, subject: &str) -> anyhow::Result<()> {
        let client = self.connection.client().await?;
        let subject_owned = subject.to_string();
        let mut subscriber = client.subscribe(subject_owned.clone()).await?;

        let instance_id = self.instance_id.clone();
        let tasks = self.tasks.clone();

        let task = tokio::spawn(async move {
            while let Some(msg) = subscriber.next().await {
                let text = String::from_utf8_lossy(&msg.payload);
                tracing::debug!(
                    "NATS message on {} (instance {}): {}",
                    msg.subject,
                    instance_id,
                    text
                );
            }
            tracing::debug!("NATS subscription ended: {}", subject_owned);
        });

        tasks.write().await.push(task);

        tracing::info!("NATS subscription: {}", subject);
        Ok(())
    }

    pub async fn subscribe_with_queue_group(
        &self,
        subject: &str,
        queue_group: &str,
    ) -> anyhow::Result<()> {
        let client = self.connection.client().await?;
        let subject_owned = subject.to_string();
        let queue_group_owned = queue_group.to_string();
        let mut subscriber = client
            .queue_subscribe(subject_owned.clone(), queue_group_owned.clone())
            .await?;

        let instance_id = self.instance_id.clone();
        let tasks = self.tasks.clone();

        let task = tokio::spawn(async move {
            while let Some(msg) = subscriber.next().await {
                let text = String::from_utf8_lossy(&msg.payload);
                tracing::debug!(
                    "NATS queue message on {} (instance {}, group {}): {}",
                    msg.subject,
                    instance_id,
                    queue_group_owned,
                    text
                );
            }
            tracing::debug!(
                "NATS queue subscription ended: {} group {}",
                subject_owned,
                queue_group_owned
            );
        });

        tasks.write().await.push(task);

        tracing::info!("NATS queue subscription: {} group {}", subject, queue_group);
        Ok(())
    }

    pub fn events_subject(&self) -> String {
        format!("{}.{}.out.events", self.subject_prefix, self.instance_id)
    }

    pub fn commands_subject(&self) -> String {
        format!("{}.{}.in.command", self.subject_prefix, self.instance_id)
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub async fn disconnect(&self) {
        let mut tasks = self.tasks.write().await;
        for task in tasks.drain(..) {
            task.abort();
        }
        drop(tasks);
        self.connection.disconnect().await;
    }
}

impl Drop for NatsTransport {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.tasks.try_write() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
    }
}
