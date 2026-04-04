use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasmtime::{Config, Engine, Module, ResourceLimiter, Store};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub capabilities: PluginCapabilities,
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginCapabilities {
    pub filesystem: Option<String>,
    pub network: Option<String>,
    pub subprocess: bool,
    pub env_access: bool,
    pub max_memory: String,
    pub max_cpu_time: String,
}

pub struct PluginRuntime {
    pub engine: Engine,
    pub max_memory_bytes: u64,
    pub max_table_elements: u32,
}

impl PluginRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.cranelift_debug_verifier(false);
        config.epoch_interruption(true);
        config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);

        let engine = Engine::new(&config)
            .map_err(|e| anyhow::anyhow!("Failed to create WASM engine: {}", e))?;

        Ok(Self {
            engine,
            max_memory_bytes: 64 * 1024 * 1024,
            max_table_elements: 10_000,
        })
    }

    pub fn load_module(&self, wasm_bytes: &[u8]) -> anyhow::Result<Module> {
        let module = Module::from_binary(&self.engine, wasm_bytes)?;
        Ok(module)
    }

    pub fn create_store(&self) -> anyhow::Result<Store<PluginLimiter>> {
        let data = PluginLimiter {
            max_memory: self.max_memory_bytes as usize,
            max_table_elements: self.max_table_elements,
        };
        let mut store = Store::new(&self.engine, data);
        store.limiter(|s| s);
        store.set_epoch_deadline(1);
        Ok(store)
    }

    pub fn advance_epoch(&self) {
        self.engine.increment_epoch();
    }
}

pub struct PluginLimiter {
    max_memory: usize,
    max_table_elements: u32,
}

impl ResourceLimiter for PluginLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let limit = maximum.unwrap_or(self.max_memory);
        Ok(desired <= limit)
    }

    fn table_growing(
        &mut self,
        _current: u32,
        desired: u32,
        maximum: Option<u32>,
    ) -> anyhow::Result<bool> {
        let limit = maximum.unwrap_or(self.max_table_elements);
        Ok(desired <= limit)
    }
}
