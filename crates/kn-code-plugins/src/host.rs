use wasmtime::{Caller, Linker};

pub struct PluginHostState {
    pub plugin_id: String,
    pub capabilities: String,
}

pub fn define_host_functions(linker: &mut Linker<PluginHostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "log",
        |mut caller: Caller<'_, PluginHostState>,
         level_ptr: u32,
         level_len: u32,
         msg_ptr: u32,
         msg_len: u32| {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(mem)) => mem,
                _ => return,
            };

            let level_bytes = match memory
                .data(&caller)
                .get(level_ptr as usize..(level_ptr as usize + level_len as usize))
            {
                Some(b) => b.to_vec(),
                None => return,
            };
            let level = String::from_utf8_lossy(&level_bytes);

            let msg_bytes = match memory
                .data(&caller)
                .get(msg_ptr as usize..(msg_ptr as usize + msg_len as usize))
            {
                Some(b) => b.to_vec(),
                None => return,
            };
            let msg = String::from_utf8_lossy(&msg_bytes);

            let plugin_id = &caller.data().plugin_id;
            match level.as_ref() {
                "error" => tracing::error!("[plugin:{}] {}", plugin_id, msg),
                "warn" => tracing::warn!("[plugin:{}] {}", plugin_id, msg),
                "info" => tracing::info!("[plugin:{}] {}", plugin_id, msg),
                _ => tracing::debug!("[plugin:{}] {}", plugin_id, msg),
            }
        },
    )?;
    Ok(())
}
