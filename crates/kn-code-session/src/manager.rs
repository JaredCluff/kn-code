use crate::store::SessionStore;
use std::path::PathBuf;

pub struct SessionManager {
    pub store: SessionStore,
}

impl SessionManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            store: SessionStore::new(data_dir.join("sessions")),
        }
    }

    pub async fn create_session(
        &self,
        cwd: PathBuf,
        model: String,
    ) -> anyhow::Result<crate::store::SessionRecord> {
        self.store.create_session(cwd, model).await
    }

    pub async fn resume_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::store::SessionRecord> {
        self.store
            .load_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))
    }
}
