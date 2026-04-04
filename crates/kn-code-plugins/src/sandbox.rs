use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCapabilities {
    pub filesystem: Option<FsAccess>,
    pub network: Option<NetworkAccess>,
    pub subprocess: bool,
    pub env_access: bool,
    pub max_memory: usize,
    pub max_cpu_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsAccess {
    None,
    ReadOnly(Vec<String>),
    ReadWrite(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkAccess {
    None,
    AllowHosts(Vec<String>),
    Any,
}

impl Default for PluginCapabilities {
    fn default() -> Self {
        Self {
            filesystem: None,
            network: None,
            subprocess: false,
            env_access: false,
            max_memory: 64 * 1024 * 1024,
            max_cpu_time_ms: 30_000,
        }
    }
}
