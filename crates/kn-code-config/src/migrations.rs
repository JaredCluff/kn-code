use serde_json::Value;

pub fn migrate_config(config: &mut Value, from_version: u32, to_version: u32) {
    if from_version >= to_version {
        return;
    }

    for version in from_version..to_version {
        match version {
            0 => migrate_v0_to_v1(config),
            _ => tracing::warn!("No migration defined for version {}", version),
        }
    }
}

fn migrate_v0_to_v1(_config: &mut Value) {
    // Initial migration placeholder
}
