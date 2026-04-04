# SPEC 006: WASM Plugin System

## 1. Overview

kn-code supports third-party plugins compiled to WebAssembly (WASM) and executed in a sandboxed wasmtime runtime. Plugins can add tools, hooks, commands, and modify session behavior — all without risking host security.

## 2. Architecture

```rust
/// Plugin runtime using wasmtime
pub struct PluginRuntime {
    engine: Engine,
    linker: Linker<PluginHostState>,
    store: Store<PluginHostState>,
}

/// Host state shared with WASM modules
pub struct PluginHostState {
    pub plugin_id: String,
    pub capabilities: PluginCapabilities,
    pub log_tx: mpsc::Sender<PluginLog>,
    pub tool_registry: Arc<ToolRegistry>,
    pub session: Arc<SessionState>,
    pub http_client: reqwest::Client,
}

/// Plugin capability grants (set by user config)
pub struct PluginCapabilities {
    /// Allow filesystem access (read-only or read-write)
    pub filesystem: FsAccess,
    /// Allow network access (none, specific hosts, or any)
    pub network: NetworkAccess,
    /// Allow subprocess execution
    pub subprocess: bool,
    /// Allow environment variable access
    pub env_access: bool,
    /// Max memory (bytes)
    pub max_memory: usize,
    /// Max CPU time per call
    pub max_cpu_time: Duration,
}

pub enum FsAccess {
    None,
    ReadOnly(Vec<PathBuf>),    // Read-only paths
    ReadWrite(Vec<PathBuf>),   // Read-write paths
}

pub enum NetworkAccess {
    None,
    AllowHosts(Vec<String>),   // Specific hostnames
    Any,
}
```

## 3. Plugin Manifest

```toml
# kn-plugin.toml
[plugin]
name = "my-plugin"
version = "0.1.0"
description = "Does something useful"
author = "developer@example.com"
license = "MIT"

[dependencies]
# Plugin SDK version
kn-code-sdk = "0.1"

[tools]
# Tools this plugin provides
[[tools]]
name = "deploy"
description = "Deploy to production"

[[tools]]
name = "lint"
description = "Run linter"

[hooks]
# Hooks the plugin registers
[[hooks]]
type = "pre_tool_use"
tool = "Bash"

[[hooks]]
type = "post_tool_use"
tool = "FileWrite"

[capabilities]
# Required capabilities (user must approve)
filesystem = { read_write = ["./dist", "./build"] }
network = { hosts = ["api.example.com"] }
subprocess = true
env_access = false
max_memory = "64MB"
max_cpu_time = "30s"
```

## 4. WASM Interface (WIT Definition)

```wit
// kn-code-plugin.wit
package kn-code:plugin@0.1.0;

interface types {
    record tool-input {
        tool-name: string,
        input: string,  // JSON
    }

    record tool-result {
        content: string,
        structured-content: string,  // JSON
    }

    record permission-decision {
        allow: bool,
        message: string,
    }

    record tool-context {
        cwd: string,
        session-id: string,
        turn-number: u64,
    }
}

interface tools {
    use types.{tool-input, tool-result, tool-context};

    /// Called when the plugin's tool is invoked by the LLM
    call-tool: func(input: tool-input, context: tool-context) -> tool-result;

    /// Returns the tool's JSON schema for input validation
    get-input-schema: func(tool-name: string) -> string;  // JSON schema

    /// Returns the tool's description for the LLM
    get-description: func(tool-name: string) -> string;

    /// Returns the tool's system prompt
    get-prompt: func(tool-name: string) -> string;
}

interface hooks {
    use types.{tool-input, tool-result, permission-decision, tool-context};

    /// Called before a tool use (can modify input or block)
    pre-tool-use: func(tool: tool-input, context: tool-context) -> permission-decision;

    /// Called after a tool use (can observe result)
    post-tool-use: func(tool: tool-input, result: tool-result, context: tool-context);
}

interface lifecycle {
    /// Called when the plugin is loaded
    on-load: func(config: string) -> string;  // config JSON -> status JSON

    /// Called when the plugin is unloaded
    on-unload: func();

    /// Called on each session start
    on-session-start: func(session-id: string, cwd: string);

    /// Called on session end
    on-session-end: func(session-id: string);
}

world plugin {
    import tools;
    import hooks;
    import lifecycle;

    export log: func(level: string, message: string);
    export read-file: func(path: string) -> result<string, string>;
    export write-file: func(path: string, content: string) -> result<_, string>;
    export http-get: func(url: string) -> result<string, string>;
    export http-post: func(url: string, body: string, content-type: string) -> result<string, string>;
    export get-env: func(key: string) -> result<string, string>;
    export get-cwd: func() -> string;
    export get-session-id: func() -> string;
}
```

## 5. Plugin Lifecycle

```rust
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub module: Module,
    pub instance: Option<Instance>,
    pub state: PluginState,
}

pub enum PluginState {
    Installed,     // Downloaded but not loaded
    Loaded,        // WASM module loaded, not initialized
    Initialized,   // on-load called, ready to use
    Error,         // Failed to load/initialize
    Disabled,      // User-disabled
}

impl Plugin {
    /// Load WASM module from disk
    pub async fn load(path: &Path) -> Result<Self> {
        let wasm_bytes = tokio::fs::read(path).await?;
        let module = Module::from_binary(&engine, &wasm_bytes)?;
        // Validate module doesn't exceed memory limits
        // Validate required imports are available
        Ok(Self { module, ... })
    }

    /// Instantiate and initialize the plugin
    pub async fn initialize(&mut self, config: &PluginConfig) -> Result<()> {
        // 1. Create store with host state
        let mut store = Store::new(&engine, PluginHostState {
            plugin_id: self.id.clone(),
            capabilities: config.capabilities.clone(),
            ...
        });

        // 2. Set up linker with host functions
        let mut linker = Linker::new(&engine);
        define_host_functions(&mut linker)?;

        // 3. Instantiate module
        let instance = linker.instantiate_async(&mut store, &self.module).await?;

        // 4. Call on-load
        let on_load = instance.get_typed_func::<String, String>(&mut store, "on-load")?;
        let status = on_load.call_async(&mut store, &config.to_json_string()).await?;

        // 5. Register tools and hooks
        self.register_tools(&instance, &mut store).await?;
        self.register_hooks(&instance, &mut store).await?;

        self.instance = Some(instance);
        self.state = PluginState::Initialized;
        Ok(())
    }

    /// Call a plugin tool
    pub async fn call_tool(&self, tool_name: &str, input: &str, context: &ToolContext) -> Result<ToolResult> {
        let instance = self.instance.as_ref().ok_or(PluginError::NotInitialized)?;
        // Call WASM call-tool function
        // Set resource limits (fuel, memory, time)
        // Execute with timeout
        // Parse result
    }
}
```

## 6. Host Functions (Exported to WASM)

```rust
/// Functions available to WASM plugins
fn define_host_functions(linker: &mut Linker<PluginHostState>) -> Result<()> {
    // Logging
    linker.func_wrap("log", |mut caller: Caller<'_, PluginHostState>,
                             level_ptr: u32, level_len: u32,
                             msg_ptr: u32, msg_len: u32| {
        // Read strings from WASM memory
        // Log to host logger with plugin ID prefix
    })?;

    // File read (capability-gated)
    linker.func_wrap("read-file", |mut caller: Caller<'_, PluginHostState>,
                                   path_ptr: u32, path_len: u32,
                                   result_ptr: u32| {
        // Check filesystem capability
        // Resolve path (must be within allowed paths)
        // Read file and copy to WASM memory
    })?;

    // File write (capability-gated)
    linker.func_wrap("write-file", |...| {
        // Check filesystem capability (must be ReadWrite)
        // Stage the change through the atomic system
    })?;

    // HTTP GET (capability-gated)
    linker.func_wrap("http-get", |...| {
        // Check network capability
        // Validate hostname against allowed hosts
        // Make request with timeout and size limit
    })?;

    // HTTP POST (capability-gated)
    linker.func_wrap("http-post", |...| {
        // Same as http-get but with body
    })?;

    // Environment variable (capability-gated)
    linker.func_wrap("get-env", |...| {
        // Check env_access capability
        // Return env var value
    })?;

    // Get current working directory
    linker.func_wrap("get-cwd", |...| { ... })?;

    // Get current session ID
    linker.func_wrap("get-session-id", |...| { ... })?;
}
```

## 7. Resource Limits

```rust
/// Enforced per-plugin, per-call limits
pub struct ResourceLimits {
    /// Max WASM memory (default: 64 MB)
    pub max_memory: usize,

    /// Max fuel (wasmtime computation units)
    pub max_fuel: u64,

    /// Max wall-clock time per call (default: 30s)
    pub max_wall_time: Duration,

    /// Max HTTP response size (default: 10 MB)
    pub max_http_response: usize,

    /// Max file read size (default: 10 MB)
    pub max_file_read: usize,

    /// Max file write size (default: 10 MB)
    pub max_file_write: usize,

    /// Max log output per call (default: 1 MB)
    pub max_log_output: usize,
}

/// Fuel is wasmtime's computation accounting
/// Set before each call, check after
impl PluginRuntime {
    pub async fn with_limits<T, F: Future<Output = Result<T>>>(&mut self, limits: ResourceLimits, f: F) -> Result<T> {
        // Set fuel limit
        self.store.set_fuel(limits.max_fuel)?;

        // Set memory limit
        let memory = self.instance.get_memory(&mut self.store, "memory")
            .ok_or(PluginError::NoMemory)?;
        memory.grow(&mut self.store, ...) // enforce max

        // Execute with timeout
        tokio::time::timeout(limits.max_wall_time, f).await?
    }
}
```

## 8. Plugin Discovery and Installation

```rust
/// Plugin sources
pub enum PluginSource {
    /// Local file path
    Local(PathBuf),
    /// URL to .wasm file
    Url(Url),
    /// Registry (future)
    Registry { name: String, version: String },
}

/// Plugin installation
pub struct PluginManager {
    pub plugins_dir: PathBuf,  // ~/.kn-code/plugins/
    pub loaded_plugins: HashMap<String, Plugin>,
}

impl PluginManager {
    /// Install a plugin from source
    pub async fn install(&mut self, source: PluginSource) -> Result<String> {
        // 1. Download/copy WASM file
        // 2. Parse manifest (embedded in WASM custom section or sidecar .toml)
        // 3. Validate manifest
        // 4. Show required capabilities to user for approval
        // 5. Write to plugins directory
        // 6. Return plugin ID
    }

    /// Load all installed plugins
    pub async fn load_all(&mut self) -> Result<()> {
        for entry in std::fs::read_dir(&self.plugins_dir)? {
            let path = entry?.path();
            if path.extension() == Some(OsStr::new("wasm")) {
                let mut plugin = Plugin::load(&path).await?;
                plugin.initialize(&self.config).await?;
                self.loaded_plugins.insert(plugin.id.clone(), plugin);
            }
        }
    }

    /// Register plugin tools and hooks with the main registry
    pub fn register_with_host(&self, tool_registry: &mut ToolRegistry, hooks: &mut HookRegistry) {
        for plugin in self.loaded_plugins.values() {
            for tool in plugin.tools() {
                tool_registry.register(Box::new(PluginToolWrapper::new(plugin, tool)));
            }
            for hook in plugin.hooks() {
                hooks.register(Box::new(PluginHookWrapper::new(plugin, hook)));
            }
        }
    }
}
```

## 9. Plugin SDK (Rust)

```rust
/// Plugins are written using the kn-code-plugin-sdk crate
/// Provides macros and helpers for easy plugin development

use kn_code_plugin_sdk::prelude::*;

#[plugin]
struct MyPlugin;

#[plugin_tool(name = "deploy", description = "Deploy to production")]
async fn deploy(input: DeployInput, ctx: ToolContext) -> ToolResult {
    // Read input
    let input: DeployInput = serde_json::from_str(&input.json)?;

    // Call host HTTP function
    let response = kn_code_plugin_sdk::http::post(
        "https://api.example.com/deploy",
        &serde_json::to_string(&input)?,
        "application/json",
    ).await?;

    ToolResult::text(&response)
}

#[plugin_hook(pre_tool_use, tool = "Bash")]
async fn check_bash_command(input: &Value, ctx: &ToolContext) -> PermissionDecision {
    let command = input["command"].as_str().unwrap();

    // Block dangerous commands
    if command.contains("rm -rf /") {
        return PermissionDecision::deny("Blocked: dangerous command");
    }

    PermissionDecision::allow()
}
```

## 10. Plugin Configuration

```jsonc
// In kn-code config (opencode.json style):
{
    "plugins": {
        "my-plugin": {
            "enabled": true,
            "capabilities": {
                "filesystem": { "read_write": ["./dist"] },
                "network": { "hosts": ["api.example.com"] },
                "subprocess": false,
                "env_access": false,
                "max_memory": "64MB",
                "max_cpu_time": "30s"
            },
            "config": {
                "api_key": "${DEPLOY_API_KEY}",  // Secret reference
                "environment": "production"
            }
        }
    }
}
```

## 11. Security Model

1. **WASM sandbox**: No direct access to host filesystem, network, or processes
2. **Capability-based**: Each capability must be explicitly granted by the user
3. **Fuel limits**: Prevents infinite loops and CPU exhaustion
4. **Memory limits**: Prevents memory exhaustion
5. **Time limits**: Prevents long-running calls from blocking the session
6. **Path restrictions**: File access limited to configured paths only
7. **Host filtering**: All host function calls validated before execution
8. **No FFI**: WASM modules cannot call arbitrary host functions
9. **Manifest review**: User reviews required capabilities before installation
10. **Signature verification** (future): Plugins can be signed and verified

## 12. Plugin Types

```rust
/// What a plugin can provide
pub enum PluginContribution {
    /// New tool(s)
    Tool { name: String, schema: Value, call: WasmFunc },

    /// Pre/post tool use hooks
    Hook { hook_type: HookType, tool_filter: String, func: WasmFunc },

    /// Skill definitions (SKILL.md content)
    Skill { name: String, content: String },

    /// Custom commands
    Command { name: String, prompt: String },

    /// System prompt additions
    SystemPrompt { content: String },

    /// Custom permission rules
    PermissionRule { rule: ToolRule },
}
```
