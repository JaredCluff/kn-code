use chrono::{DateTime, Utc};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const SALT_LEN: usize = 16;
const SALT_FILE: &str = ".salt";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub provider_id: String,
    pub auth_type: AuthType,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub api_key: Option<Secret<String>>,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub access_token: Option<Secret<String>>,
    #[serde(skip_serializing, skip_deserializing, default)]
    pub refresh_token: Option<Secret<String>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub account_uuid: Option<String>,
    pub user_email: Option<String>,
    pub organization_uuid: Option<String>,
}

impl Credentials {
    pub fn api_key_str(&self) -> Option<String> {
        self.api_key.as_ref().map(|s| s.expose_secret().clone())
    }

    pub fn access_token_str(&self) -> Option<String> {
        self.access_token
            .as_ref()
            .map(|s| s.expose_secret().clone())
    }

    pub fn refresh_token_str(&self) -> Option<String> {
        self.refresh_token
            .as_ref()
            .map(|s| s.expose_secret().clone())
    }

    pub fn set_api_key(&mut self, key: String) {
        self.api_key = Some(Secret::new(key));
    }

    pub fn set_access_token(&mut self, token: String) {
        self.access_token = Some(Secret::new(token));
    }

    pub fn set_refresh_token(&mut self, token: String) {
        self.refresh_token = Some(Secret::new(token));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    ApiKey,
    OAuth,
    OAuthDerivedApiKey,
}

#[async_trait::async_trait]
pub trait TokenStore: Send + Sync {
    async fn store(&self, provider_id: &str, credentials: &Credentials) -> anyhow::Result<()>;
    async fn load(&self, provider_id: &str) -> anyhow::Result<Option<Credentials>>;
    async fn delete(&self, provider_id: &str) -> anyhow::Result<()>;
    async fn list_providers(&self) -> anyhow::Result<Vec<String>>;
}

pub struct FileTokenStore {
    dir: PathBuf,
    lock: Arc<Mutex<()>>,
    salt: [u8; SALT_LEN],
    rng: SystemRandom,
}

impl FileTokenStore {
    pub fn new(path: PathBuf) -> Self {
        let (dir, salt) = if path.extension().and_then(|e| e.to_str()) == Some("enc") {
            let dir = path.parent().unwrap_or(&path).to_path_buf();
            let salt = Self::load_or_generate_salt(&path);
            (dir, salt)
        } else {
            let salt = Self::load_or_generate_salt(&path.join(SALT_FILE));
            (path, salt)
        };

        Self {
            dir,
            lock: Arc::new(Mutex::new(())),
            salt,
            rng: SystemRandom::new(),
        }
    }

    fn provider_path(&self, provider_id: &str) -> PathBuf {
        self.dir.join(format!("{}.enc", provider_id))
    }

    #[allow(clippy::disallowed_methods)]
    fn load_or_generate_salt(salt_path: &PathBuf) -> [u8; SALT_LEN] {
        if salt_path.exists()
            && let Ok(bytes) = std::fs::read(salt_path)
            && bytes.len() >= SALT_LEN
        {
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&bytes[..SALT_LEN]);
            return salt;
        }
        let mut salt = [0u8; SALT_LEN];
        if SystemRandom::new().fill(&mut salt).is_err() {
            tracing::error!("Failed to generate salt — using zeroed salt");
        }
        if let Some(parent) = salt_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(salt_path, salt) {
            tracing::warn!("Failed to persist salt file: {}", e);
        }
        salt
    }

    fn derive_key(salt: &[u8; SALT_LEN]) -> anyhow::Result<LessSafeKey> {
        let machine_id = Self::machine_id();
        let mut key_bytes = [0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(machine_id.as_bytes(), salt, &mut key_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to derive key with Argon2: {}", e))?;

        let unbound_key = UnboundKey::new(&CHACHA20_POLY1305, &key_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create encryption key: {}", e))?;
        Ok(LessSafeKey::new(unbound_key))
    }

    fn generate_random_nonce(&self) -> anyhow::Result<Nonce> {
        let mut nonce_bytes = [0u8; 12];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to generate random nonce: {}", e))?;
        Ok(Nonce::assume_unique_for_key(nonce_bytes))
    }

    fn encrypt_data(&self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        let key = Self::derive_key(&self.salt)?;
        let nonce = self.generate_random_nonce()?;
        let nonce_bytes = nonce.as_ref().to_vec();
        let mut in_out = data.to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        let mut result = nonce_bytes;
        result.extend_from_slice(&in_out);
        Ok(result)
    }

    fn decrypt_data(&self, ciphertext: &[u8]) -> anyhow::Result<Vec<u8>> {
        if ciphertext.len() < 12 {
            anyhow::bail!("Ciphertext too short to contain nonce");
        }
        let key = Self::derive_key(&self.salt)?;
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes.copy_from_slice(&ciphertext[..12]);
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = ciphertext[12..].to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;
        Ok(plaintext.to_vec())
    }

    async fn write_provider_file(
        &self,
        provider_id: &str,
        credentials: &EncryptedCredentials,
    ) -> anyhow::Result<()> {
        if !self.dir.exists() {
            tokio::fs::create_dir_all(&self.dir).await?;
        }

        let data = serde_json::to_vec(credentials)?;
        let encrypted = self.encrypt_data(&data)?;

        let path = self.provider_path(provider_id);
        let tmp_path = path.with_extension("tmp");
        tokio::fs::write(&tmp_path, &encrypted).await?;

        #[cfg(unix)]
        #[allow(clippy::disallowed_methods)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&tmp_path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&tmp_path, perms)?;
        }

        tokio::fs::rename(&tmp_path, &path).await?;
        Ok(())
    }

    async fn read_provider_file(
        &self,
        provider_id: &str,
    ) -> anyhow::Result<Option<EncryptedCredentials>> {
        let path = self.provider_path(provider_id);
        if !path.exists() {
            return Ok(None);
        }
        let encrypted = tokio::fs::read(&path).await?;
        if encrypted.is_empty() {
            return Ok(None);
        }
        let decrypted = self.decrypt_data(&encrypted)?;
        let creds: EncryptedCredentials = serde_json::from_slice(&decrypted)?;
        Ok(Some(creds))
    }

    fn machine_id() -> String {
        let hostname = std::env::var("HOSTNAME")
            .ok()
            .or_else(|| std::env::var("COMPUTERNAME").ok())
            .unwrap_or_else(|| "unknown".to_string());
        let uid = std::env::var("UID").unwrap_or_else(|_| "0".to_string());
        format!("{}:{}", hostname, uid)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedCredentials {
    pub provider_id: String,
    pub auth_type: AuthType,
    pub api_key: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub account_uuid: Option<String>,
    pub user_email: Option<String>,
    pub organization_uuid: Option<String>,
}

impl From<&Credentials> for EncryptedCredentials {
    fn from(c: &Credentials) -> Self {
        Self {
            provider_id: c.provider_id.clone(),
            auth_type: c.auth_type.clone(),
            api_key: c.api_key_str(),
            access_token: c.access_token_str(),
            refresh_token: c.refresh_token_str(),
            expires_at: c.expires_at,
            account_uuid: c.account_uuid.clone(),
            user_email: c.user_email.clone(),
            organization_uuid: c.organization_uuid.clone(),
        }
    }
}

impl From<&EncryptedCredentials> for Credentials {
    fn from(e: &EncryptedCredentials) -> Self {
        let mut c = Credentials {
            provider_id: e.provider_id.clone(),
            auth_type: e.auth_type.clone(),
            api_key: None,
            access_token: None,
            refresh_token: None,
            expires_at: e.expires_at,
            account_uuid: e.account_uuid.clone(),
            user_email: e.user_email.clone(),
            organization_uuid: e.organization_uuid.clone(),
        };
        if let Some(key) = &e.api_key {
            c.set_api_key(key.clone());
        }
        if let Some(token) = &e.access_token {
            c.set_access_token(token.clone());
        }
        if let Some(token) = &e.refresh_token {
            c.set_refresh_token(token.clone());
        }
        c
    }
}

#[async_trait::async_trait]
impl TokenStore for FileTokenStore {
    async fn store(&self, provider_id: &str, credentials: &Credentials) -> anyhow::Result<()> {
        let _lock = self.lock.lock().await;
        let encrypted = EncryptedCredentials::from(credentials);
        self.write_provider_file(provider_id, &encrypted).await
    }

    async fn load(&self, provider_id: &str) -> anyhow::Result<Option<Credentials>> {
        let _lock = self.lock.lock().await;
        match self.read_provider_file(provider_id).await? {
            Some(e) => Ok(Some(Credentials::from(&e))),
            None => Ok(None),
        }
    }

    async fn delete(&self, provider_id: &str) -> anyhow::Result<()> {
        let _lock = self.lock.lock().await;
        let path = self.provider_path(provider_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    async fn list_providers(&self) -> anyhow::Result<Vec<String>> {
        let _lock = self.lock.lock().await;
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut providers = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("enc")
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                providers.push(name.to_string());
            }
        }
        Ok(providers)
    }
}

pub struct TokenManager {
    store: Box<dyn TokenStore>,
    refresh_buffer_secs: i64,
}

impl TokenManager {
    pub fn new(store: Box<dyn TokenStore>) -> Self {
        Self {
            store,
            refresh_buffer_secs: 300,
        }
    }

    pub async fn get_credentials(&self, provider_id: &str) -> anyhow::Result<Option<Credentials>> {
        self.store.load(provider_id).await
    }

    pub fn needs_refresh(&self, credentials: &Credentials) -> bool {
        match &credentials.expires_at {
            Some(expires_at) => {
                let now = Utc::now();
                let threshold = *expires_at - chrono::Duration::seconds(self.refresh_buffer_secs);
                now >= threshold
            }
            None => true,
        }
    }
}
