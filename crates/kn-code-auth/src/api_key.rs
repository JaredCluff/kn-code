use secrecy::{ExposeSecret, Secret};

pub struct ApiKeyAuth {
    pub api_key: Secret<String>,
}

impl ApiKeyAuth {
    pub fn new(key: String) -> anyhow::Result<Self> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            anyhow::bail!("API key cannot be empty");
        }
        Ok(Self {
            api_key: Secret::new(trimmed.to_string()),
        })
    }

    pub fn from_env(env_var: &str) -> anyhow::Result<Option<Self>> {
        match std::env::var(env_var) {
            Ok(key) => Self::new(key).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(e) => anyhow::bail!("Failed to read env var {}: {}", env_var, e),
        }
    }

    pub fn api_key_str(&self) -> String {
        self.api_key.expose_secret().clone()
    }
}
