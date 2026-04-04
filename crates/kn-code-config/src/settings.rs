use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub plugins: HashMap<String, PluginConfig>,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub atomic: AtomicConfig,
    #[serde(default)]
    pub additional_working_directories: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(
        skip_serializing,
        deserialize_with = "deserialize_secret_option",
        default
    )]
    pub api_key: Option<Secret<String>>,
    pub base_url: Option<String>,
    pub auth_method: Option<String>,
    pub default_model: Option<String>,
}

fn deserialize_secret_option<'de, D>(deserializer: D) -> Result<Option<Secret<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(s) => Ok(Some(Secret::new(s))),
        None => Ok(None),
    }
}

impl ProviderConfig {
    pub fn api_key_str(&self) -> Option<String> {
        self.api_key.as_ref().map(|s| s.expose_secret().clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginCapabilities {
    #[serde(default)]
    pub filesystem: Option<FsAccess>,
    #[serde(default)]
    pub network: Option<NetworkAccess>,
    #[serde(default)]
    pub subprocess: bool,
    #[serde(default)]
    pub env_access: bool,
    #[serde(default = "default_max_memory")]
    pub max_memory: String,
    #[serde(default = "default_max_cpu_time")]
    pub max_cpu_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsAccess {
    None,
    ReadOnly(Vec<PathBuf>),
    ReadWrite(Vec<PathBuf>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkAccess {
    None,
    AllowHosts(Vec<String>),
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub auth_mode: AuthMode,
    #[serde(default)]
    pub max_sessions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    None,
    Jwt,
    ApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AtomicConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub commit_strategy: CommitStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommitStrategy {
    #[default]
    AutoEndOfTurn,
    ExplicitTool,
    ManualApi,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Ask,
    AcceptEdits,
    Auto,
    Plan,
}

fn default_model() -> String {
    "anthropic/claude-sonnet-4-5".into()
}

fn default_true() -> bool {
    true
}

fn default_host() -> String {
    "127.0.0.1".into()
}

fn default_port() -> u16 {
    3200
}

fn default_max_memory() -> String {
    "64MB".into()
}

fn default_max_cpu_time() -> String {
    "30s".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            providers: HashMap::new(),
            plugins: HashMap::new(),
            default_model: default_model(),
            permission_mode: PermissionMode::default(),
            server: ServerConfig::default(),
            atomic: AtomicConfig {
                enabled: true,
                commit_strategy: CommitStrategy::default(),
            },
            additional_working_directories: Vec::new(),
        }
    }
}

impl Settings {
    pub async fn load() -> anyhow::Result<Self> {
        let config_dir = Self::config_dir();
        let config_path = config_dir.join("kn-code.json");
        if config_path.exists() {
            let content = tokio::fs::read_to_string(&config_path).await?;
            let settings: Settings = serde_json::from_str(&content)?;
            Ok(settings)
        } else {
            Ok(Settings::default())
        }
    }

    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp/.kn-code-fallback"))
            .join(".kn-code")
            .join("config")
    }
}
