use crate::runtime::{PluginConfig, PluginRuntime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use wasmtime::Module;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginState {
    Installed,
    Loaded,
    Initialized,
    Error,
    Disabled,
}

pub struct Plugin {
    pub id: String,
    pub name: String,
    pub state: PluginState,
    pub module: Option<Module>,
    pub checksum: String,
}

impl Plugin {
    pub async fn load(path: &Path, runtime: &PluginRuntime) -> anyhow::Result<Self> {
        let wasm_bytes = tokio::fs::read(path).await?;

        let checksum = format!("{:x}", Sha256::digest(&wasm_bytes));

        let module = runtime.load_module(&wasm_bytes)?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let id = format!("{}:{}", name, &checksum[..12]);

        Ok(Self {
            id,
            name,
            state: PluginState::Loaded,
            module: Some(module),
            checksum,
        })
    }

    pub async fn initialize(&mut self, _config: &PluginConfig) -> anyhow::Result<()> {
        self.state = PluginState::Initialized;
        Ok(())
    }
}
