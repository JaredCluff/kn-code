use serde::{Deserialize, Serialize};

/// SDK control protocol requests (stdin → kn-code).
/// Used by orchestrators to interact with a running kn-code process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SdkControlRequest {
    /// Grant permission for a pending tool call
    PermissionGrant { request_id: String },
    /// Deny permission for a pending tool call
    PermissionDeny {
        request_id: String,
        message: Option<String>,
    },
    /// Cancel the current run
    Cancel,
    /// Send a follow-up message to the session
    Message { content: String },
    /// Update permission mode mid-session
    SetMode { mode: String },
    /// Ping (keepalive)
    Ping,
}

/// SDK control protocol responses (kn-code → stdout).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SdkControlResponse {
    /// Permission was granted/denied and tool execution proceeded
    PermissionResolved { request_id: String, granted: bool },
    /// Run was cancelled
    Cancelled,
    /// Message received and added to conversation
    MessageReceived,
    /// Mode updated
    ModeUpdated,
    /// Pong (keepalive response)
    Pong,
    /// Control protocol error
    Error { message: String },
}

/// Bidirectional control channel for headless mode.
///
/// This enables orchestrators (like Paperclip) to:
/// - Grant/deny permissions mid-run
/// - Cancel runs
/// - Send follow-up messages
/// - Change permission modes dynamically
pub struct ControlChannel {
    pub request_rx: tokio::sync::mpsc::Receiver<SdkControlRequest>,
    pub response_tx: tokio::sync::mpsc::Sender<SdkControlResponse>,
}

impl ControlChannel {
    pub fn new() -> (ControlSender, Self) {
        let (req_tx, req_rx) = tokio::sync::mpsc::channel(100);
        let (res_tx, res_rx) = tokio::sync::mpsc::channel(100);
        let sender = ControlSender {
            request_tx: req_tx,
            response_rx: res_rx,
        };
        let channel = Self {
            request_rx: req_rx,
            response_tx: res_tx,
        };
        (sender, channel)
    }

    pub async fn send_response(&self, response: SdkControlResponse) {
        let _ = self.response_tx.send(response).await;
    }
}

impl Default for ControlChannel {
    fn default() -> Self {
        Self::new().1
    }
}

pub struct ControlSender {
    pub request_tx: tokio::sync::mpsc::Sender<SdkControlRequest>,
    pub response_rx: tokio::sync::mpsc::Receiver<SdkControlResponse>,
}

impl ControlSender {
    pub async fn grant_permission(&self, request_id: &str) {
        let _ = self
            .request_tx
            .send(SdkControlRequest::PermissionGrant {
                request_id: request_id.to_string(),
            })
            .await;
    }

    pub async fn deny_permission(&self, request_id: &str, message: Option<String>) {
        let _ = self
            .request_tx
            .send(SdkControlRequest::PermissionDeny {
                request_id: request_id.to_string(),
                message,
            })
            .await;
    }

    pub async fn cancel(&self) {
        let _ = self.request_tx.send(SdkControlRequest::Cancel).await;
    }

    pub async fn send_message(&self, content: &str) {
        let _ = self
            .request_tx
            .send(SdkControlRequest::Message {
                content: content.to_string(),
            })
            .await;
    }
}
