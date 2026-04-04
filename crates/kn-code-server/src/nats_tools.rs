use kn_code_nats::{AgentRegistry, NatsConfig, NatsConnection, PermissionGate};
use kn_code_tools::traits::{Tool, ToolContent, ToolContext, ToolError, ToolResult};
use serde::Deserialize;
use std::sync::Arc;

pub struct NatsTools {
    pub connection: NatsConnection,
    pub registry: Arc<AgentRegistry>,
    pub permission_gate: Arc<PermissionGate>,
}

impl NatsTools {
    pub fn new(config: NatsConfig) -> Self {
        let instance_id = config.instance_id();
        let connection = NatsConnection::new(config.clone());
        let registry = Arc::new(AgentRegistry::new(connection.clone(), instance_id.clone()));
        let permission_gate = Arc::new(PermissionGate::new(connection.clone(), instance_id));
        Self {
            connection,
            registry,
            permission_gate,
        }
    }

    pub fn all_tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(NatsPublishTool {
                connection: self.connection.clone(),
            }),
            Box::new(NatsSubscribeTool {
                connection: self.connection.clone(),
            }),
            Box::new(NatsRequestTool {
                connection: self.connection.clone(),
            }),
            Box::new(JetStreamPublishTool {
                connection: self.connection.clone(),
            }),
            Box::new(JetStreamConsumeTool {
                connection: self.connection.clone(),
            }),
            Box::new(KvPutTool {
                connection: self.connection.clone(),
            }),
            Box::new(KvGetTool {
                connection: self.connection.clone(),
            }),
            Box::new(AgentAnnounceTool {
                registry: self.registry.clone(),
            }),
            Box::new(AgentDiscoverTool {
                registry: self.registry.clone(),
            }),
            Box::new(AgentClaimTool {
                connection: self.connection.clone(),
            }),
            Box::new(RequestPermissionTool {
                gate: self.permission_gate.clone(),
            }),
        ]
    }
}

#[derive(Debug)]
pub struct NatsPublishTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct NatsPublishInput {
    subject: String,
    payload: String,
    #[allow(dead_code)]
    reply_to: Option<String>,
}

#[async_trait::async_trait]
impl Tool for NatsPublishTool {
    fn name(&self) -> &str {
        "nats_publish"
    }
    fn description(&self) -> &str {
        "Publish a message to a NATS subject (fire-and-forget)"
    }
    fn prompt(&self) -> &str {
        "Use this to send events or notifications via NATS."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "NATS subject" },
                "payload": { "type": "string", "description": "Message body" },
                "reply_to": { "type": "string", "description": "Optional reply subject" }
            },
            "required": ["subject", "payload"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: NatsPublishInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_subject("nats_publish", &parsed.subject)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let publisher = kn_code_nats::Publisher::new(self.connection.clone());
        publisher
            .publish(&parsed.subject, parsed.payload.as_bytes())
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({"ok": true})).unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct NatsSubscribeTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct NatsSubscribeInput {
    subject: String,
    #[serde(default)]
    queue_group: Option<String>,
}

#[async_trait::async_trait]
impl Tool for NatsSubscribeTool {
    fn name(&self) -> &str {
        "nats_subscribe"
    }
    fn description(&self) -> &str {
        "Subscribe to a NATS subject for real-time message delivery"
    }
    fn prompt(&self) -> &str {
        "Use this to listen to NATS subjects."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "NATS subject (supports * and > wildcards)" },
                "queue_group": { "type": "string", "description": "Optional queue group for load balancing" }
            },
            "required": ["subject"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: NatsSubscribeInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_subject("nats_subscribe", &parsed.subject)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        if let Some(qg) = &parsed.queue_group
            && qg.len() > 128 {
                return Err(ToolError::ValidationFailed {
                    message: "Queue group name too long (max 128 chars)".to_string(),
                });
            }

        let manager = kn_code_nats::SubscriptionManager::new(self.connection.clone());
        let (id, _rx) = if let Some(qg) = &parsed.queue_group {
            manager
                .subscribe_with_queue_group(&parsed.subject, qg, 100)
                .await
        } else {
            manager.subscribe(&parsed.subject, 100).await
        }
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({
                    "subscription_id": id,
                    "subject": parsed.subject,
                }))
                .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct NatsRequestTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct NatsRequestInput {
    subject: String,
    payload: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait::async_trait]
impl Tool for NatsRequestTool {
    fn name(&self) -> &str {
        "nats_request"
    }
    fn description(&self) -> &str {
        "Send a request-reply message via NATS and wait for response"
    }
    fn prompt(&self) -> &str {
        "Use this for synchronous RPC via NATS request-reply."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "NATS subject" },
                "payload": { "type": "string", "description": "Request body" },
                "timeout_ms": { "type": "integer", "description": "Timeout in ms" }
            },
            "required": ["subject", "payload"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: NatsRequestInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_subject("nats_request", &parsed.subject)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let publisher = kn_code_nats::Publisher::new(self.connection.clone());
        let response = publisher
            .request(
                &parsed.subject,
                parsed.payload.as_bytes(),
                parsed.timeout_ms,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let text = String::from_utf8_lossy(&response).to_string();
        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({
                    "subject": parsed.subject,
                    "payload": text,
                }))
                .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct JetStreamPublishTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct JsPublishInput {
    subject: String,
    payload: String,
    #[serde(default)]
    msg_id: Option<String>,
}

#[async_trait::async_trait]
impl Tool for JetStreamPublishTool {
    fn name(&self) -> &str {
        "js_publish"
    }
    fn description(&self) -> &str {
        "Publish to a JetStream stream (durable, with ack)"
    }
    fn prompt(&self) -> &str {
        "Use this for persistent, durable message publishing."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "Subject matching stream config" },
                "payload": { "type": "string", "description": "Message body" },
                "msg_id": { "type": "string", "description": "Deduplication ID" }
            },
            "required": ["subject", "payload"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: JsPublishInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_subject("js_publish", &parsed.subject)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let js = kn_code_nats::JetStreamManager::new(self.connection.clone());
        let seq = js
            .publish(
                &parsed.subject,
                parsed.payload.as_bytes(),
                parsed.msg_id.as_deref(),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({
                    "seq": seq,
                    "duplicate": false,
                }))
                .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct JetStreamConsumeTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct JsConsumeInput {
    stream: String,
    #[serde(default)]
    consumer_name: Option<String>,
    #[serde(default)]
    batch: Option<usize>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait::async_trait]
impl Tool for JetStreamConsumeTool {
    fn name(&self) -> &str {
        "js_consume"
    }
    fn description(&self) -> &str {
        "Pull messages from a JetStream stream"
    }
    fn prompt(&self) -> &str {
        "Use this to consume persisted messages from a stream."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "stream": { "type": "string", "description": "Stream name" },
                "consumer_name": { "type": "string", "description": "Durable consumer name" },
                "batch": { "type": "integer", "description": "Max messages to pull" },
                "timeout_ms": { "type": "integer", "description": "Wait timeout" }
            },
            "required": ["stream"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: JsConsumeInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_stream_name(&parsed.stream)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let js = kn_code_nats::JetStreamManager::new(self.connection.clone());
        let messages = js
            .consume(
                &parsed.stream,
                parsed.consumer_name.as_deref(),
                parsed.batch.unwrap_or(1),
                parsed.timeout_ms.unwrap_or(5000),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "subject": m.subject,
                    "payload": String::from_utf8_lossy(&m.payload).to_string(),
                    "seq": m.seq,
                })
            })
            .collect();

        Ok(ToolResult {
            content: ToolContent::Text(serde_json::to_string(&msgs).unwrap_or_default()),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct KvPutTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct KvPutInput {
    bucket: String,
    key: String,
    value: String,
}

#[async_trait::async_trait]
impl Tool for KvPutTool {
    fn name(&self) -> &str {
        "kv_put"
    }
    fn description(&self) -> &str {
        "Store a value in a NATS KV bucket"
    }
    fn prompt(&self) -> &str {
        "Use this for shared key-value state across agents."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "bucket": { "type": "string" },
                "key": { "type": "string" },
                "value": { "type": "string" }
            },
            "required": ["bucket", "key", "value"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: KvPutInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_bucket_name(&parsed.bucket)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;
        kn_code_nats::subjects::validate_kv_key(&parsed.key)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let kv = kn_code_nats::KvStore::new(self.connection.clone());
        let revision = kv
            .put_string(&parsed.bucket, &parsed.key, &parsed.value)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({"revision": revision}))
                    .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct KvGetTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct KvGetInput {
    bucket: String,
    key: String,
}

#[async_trait::async_trait]
impl Tool for KvGetTool {
    fn name(&self) -> &str {
        "kv_get"
    }
    fn description(&self) -> &str {
        "Retrieve a value from a NATS KV bucket"
    }
    fn prompt(&self) -> &str {
        "Use this to read shared state."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "bucket": { "type": "string" },
                "key": { "type": "string" }
            },
            "required": ["bucket", "key"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: KvGetInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        kn_code_nats::subjects::validate_bucket_name(&parsed.bucket)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;
        kn_code_nats::subjects::validate_kv_key(&parsed.key)
            .map_err(|e| ToolError::ValidationFailed { message: e })?;

        let kv = kn_code_nats::KvStore::new(self.connection.clone());
        match kv.get_string(&parsed.bucket, &parsed.key).await {
            Ok(Some(value)) => Ok(ToolResult {
                content: ToolContent::Text(
                    serde_json::to_string(&serde_json::json!({
                        "key": parsed.key,
                        "value": value,
                    }))
                    .unwrap_or_default(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            }),
            Ok(None) => Ok(ToolResult {
                content: ToolContent::Text("Key not found".to_string()),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            }),
            Err(e) => Err(ToolError::ExecutionFailed(e.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct AgentAnnounceTool {
    pub registry: Arc<AgentRegistry>,
}

#[derive(Debug, Deserialize)]
struct AgentAnnounceInput {
    #[allow(dead_code)]
    agent_id: String,
    capabilities: Vec<String>,
    #[serde(default)]
    metadata: Option<std::collections::HashMap<String, String>>,
}

#[async_trait::async_trait]
impl Tool for AgentAnnounceTool {
    fn name(&self) -> &str {
        "agent_announce"
    }
    fn description(&self) -> &str {
        "Register an agent in the shared NATS registry"
    }
    fn prompt(&self) -> &str {
        "Use this to announce agent capabilities to peers."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string" },
                "capabilities": { "type": "array", "items": { "type": "string" } },
                "metadata": { "type": "object" }
            },
            "required": ["agent_id", "capabilities"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: AgentAnnounceInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        self.registry
            .announce(parsed.capabilities, parsed.metadata.unwrap_or_default())
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&serde_json::json!({"ok": true})).unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct AgentDiscoverTool {
    pub registry: Arc<AgentRegistry>,
}

#[derive(Debug, Deserialize)]
struct AgentDiscoverInput {
    #[serde(default)]
    capability: Option<String>,
}

#[async_trait::async_trait]
impl Tool for AgentDiscoverTool {
    fn name(&self) -> &str {
        "agent_discover"
    }
    fn description(&self) -> &str {
        "Discover active agents in the NATS registry"
    }
    fn prompt(&self) -> &str {
        "Use this to find peer agents and their capabilities."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "capability": { "type": "string", "description": "Filter by capability" }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: AgentDiscoverInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let agents = self
            .registry
            .discover(parsed.capability.as_deref())
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolResult {
            content: ToolContent::Text(serde_json::to_string(&agents).unwrap_or_default()),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

#[derive(Debug)]
pub struct AgentClaimTool {
    pub connection: NatsConnection,
}

#[derive(Debug, Deserialize)]
struct AgentClaimInput {
    subject: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait::async_trait]
impl Tool for AgentClaimTool {
    fn name(&self) -> &str {
        "agent_claim"
    }
    fn description(&self) -> &str {
        "Claim a task message from a NATS subject via queue group"
    }
    fn prompt(&self) -> &str {
        "Use this for atomic work distribution — only one agent gets each task."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string" },
                "timeout_ms": { "type": "integer" }
            },
            "required": ["subject"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: AgentClaimInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let manager = kn_code_nats::SubscriptionManager::new(self.connection.clone());
        let (_id, mut rx) = manager
            .subscribe_with_queue_group(&parsed.subject, "nuntius.claim", 1)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let timeout = std::time::Duration::from_millis(parsed.timeout_ms.unwrap_or(1000));
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(msg)) => {
                let task: serde_json::Value = serde_json::from_slice(&msg.payload)
                    .unwrap_or(serde_json::json!({"raw": String::from_utf8_lossy(&msg.payload)}));
                Ok(ToolResult {
                    content: ToolContent::Text(
                        serde_json::to_string(&serde_json::json!({
                            "claimed": true,
                            "task": task,
                        }))
                        .unwrap_or_default(),
                    ),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                })
            }
            _ => Ok(ToolResult {
                content: ToolContent::Text(
                    serde_json::to_string(&serde_json::json!({"claimed": false}))
                        .unwrap_or_default(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            }),
        }
    }
}

#[derive(Debug)]
pub struct RequestPermissionTool {
    pub gate: Arc<PermissionGate>,
}

#[derive(Debug, Deserialize)]
struct RequestPermissionInput {
    action: String,
    details: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[async_trait::async_trait]
impl Tool for RequestPermissionTool {
    fn name(&self) -> &str {
        "request_permission"
    }
    fn description(&self) -> &str {
        "Request permission from supervisor before executing a sensitive action"
    }
    fn prompt(&self) -> &str {
        "Use this before destructive operations. Blocks until Animus approves or denies."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["shell_exec", "file_delete", "network_request", "write_file", "git_push", "deploy"] },
                "details": { "type": "string" },
                "timeout_ms": { "type": "integer" }
            },
            "required": ["action", "details"]
        })
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: RequestPermissionInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let response = self
            .gate
            .request(&parsed.action, &parsed.details, parsed.timeout_ms)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        if response.approved {
            Ok(ToolResult {
                content: ToolContent::Text(
                    serde_json::to_string(&serde_json::json!({
                        "approved": true,
                        "reason": response.reason,
                    }))
                    .unwrap_or_default(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            })
        } else {
            Err(ToolError::PermissionDenied {
                message: format!("Permission denied: {}", response.reason.unwrap_or_default()),
            })
        }
    }
}
