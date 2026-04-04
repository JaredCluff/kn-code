use crate::connection::NatsConnection;
use chrono::Utc;
use serde::{Deserialize, Serialize};

const PERMISSION_REQUEST_SUBJECT: &str = "animus.in.permission_request";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub from: String,
    pub action: String,
    pub details: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResponse {
    pub approved: bool,
    pub reason: Option<String>,
}

pub struct PermissionGate {
    connection: NatsConnection,
    instance_id: String,
}

impl std::fmt::Debug for PermissionGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionGate")
            .field("instance_id", &self.instance_id)
            .finish()
    }
}

impl PermissionGate {
    pub fn new(connection: NatsConnection, instance_id: String) -> Self {
        Self {
            connection,
            instance_id,
        }
    }

    pub async fn request(
        &self,
        action: &str,
        details: &str,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<PermissionResponse> {
        let request_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

        let request = PermissionRequest {
            request_id: request_id.clone(),
            from: self.instance_id.clone(),
            action: action.to_string(),
            details: details.to_string(),
            timestamp: Utc::now().to_rfc3339(),
        };

        let payload = serde_json::to_vec(&request)?;
        let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(30000));

        tracing::info!("Permission request: {} — {}", action, details);

        let response_bytes = tokio::time::timeout(
            timeout,
            self.connection
                .client()
                .await?
                .request(PERMISSION_REQUEST_SUBJECT, payload.into()),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Permission request timed out after {}ms",
                timeout.as_millis()
            )
        })?
        .map_err(|e| anyhow::anyhow!("Permission request failed: {}", e))?;

        let response: PermissionResponse = serde_json::from_slice(&response_bytes.payload)
            .map_err(|e| anyhow::anyhow!("Failed to parse permission response: {}", e))?;

        if response.approved {
            tracing::info!(
                "Permission granted: {} — {}",
                action,
                response.reason.as_deref().unwrap_or("")
            );
        } else {
            tracing::warn!(
                "Permission denied: {} — {}",
                action,
                response.reason.as_deref().unwrap_or("")
            );
        }

        Ok(response)
    }
}
