/// NATS subject validation with per-tool allowlists.
///
/// Each tool is restricted to a specific set of subject prefixes.
/// This prevents agents from publishing to internal control subjects,
/// eavesdropping on other agents, or hijacking tasks.
///
/// Subject prefix allowlists for each NATS tool.
///
/// Format: (tool_name, allowed_prefixes)
/// Prefixes are matched with `starts_with` for efficiency.
/// Wildcards in prefixes are treated literally (not as NATS wildcards).
const SUBJECT_ALLOWLISTS: &[(&str, &[&str])] = &[
    (
        "nats_publish",
        &[
            "agent.events.",
            "agent.output.",
            "agent.log.",
            "kn-code.events.",
        ],
    ),
    (
        "nats_subscribe",
        &[
            "agent.events.",
            "agent.output.",
            "agent.broadcast.",
            "kn-code.events.",
        ],
    ),
    (
        "nats_request",
        &["agent.request.", "agent.rpc.", "kn-code.rpc."],
    ),
    ("js_publish", &["agent.stream.", "kn-code.stream."]),
    ("js_consume", &["agent.stream.", "kn-code.stream."]),
    ("kv_put", &["agent.kv.", "kn-code.kv."]),
    ("kv_get", &["agent.kv.", "kn-code.kv."]),
];

/// Validate that a subject is allowed for the given tool.
///
/// Returns `Ok(())` if the subject matches one of the allowed prefixes,
/// or `Err` with a descriptive message.
pub fn validate_subject(tool: &str, subject: &str) -> Result<(), String> {
    if subject.is_empty() {
        return Err("Subject cannot be empty".to_string());
    }

    if subject.len() > 256 {
        return Err(format!(
            "Subject too long ({} chars, max 256)",
            subject.len()
        ));
    }

    let allowlist = SUBJECT_ALLOWLISTS
        .iter()
        .find(|(name, _)| *name == tool)
        .ok_or_else(|| format!("No subject allowlist configured for tool: {}", tool))?;

    let (_, prefixes) = allowlist;

    for prefix in *prefixes {
        if subject.starts_with(prefix) {
            return Ok(());
        }
    }

    Err(format!(
        "Subject '{}' not allowed for tool '{}'. Allowed prefixes: {:?}",
        subject, tool, prefixes
    ))
}

/// Validate a JetStream stream name.
///
/// Stream names must match the pattern: alphanumeric, hyphens, underscores only.
pub fn validate_stream_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Stream name cannot be empty".to_string());
    }
    if name.len() > 128 {
        return Err(format!(
            "Stream name too long ({} chars, max 128)",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Stream name '{}' contains invalid characters. Only alphanumeric, hyphens, and underscores are allowed",
            name
        ));
    }
    Ok(())
}

/// Validate a KV bucket name.
///
/// Bucket names must be alphanumeric with underscores and hyphens.
pub fn validate_bucket_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Bucket name cannot be empty".to_string());
    }
    if name.len() > 128 {
        return Err(format!(
            "Bucket name too long ({} chars, max 128)",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Bucket name '{}' contains invalid characters",
            name
        ));
    }
    Ok(())
}

/// Validate a KV key.
///
/// Keys must be non-empty and cannot contain `..` (path traversal).
pub fn validate_kv_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Key cannot be empty".to_string());
    }
    if key.len() > 1024 {
        return Err(format!("Key too long ({} chars, max 1024)", key.len()));
    }
    if key.contains("..") {
        return Err("Key cannot contain '..'".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_subject_allowed() {
        assert!(validate_subject("nats_publish", "agent.events.task_complete").is_ok());
        assert!(validate_subject("nats_publish", "kn-code.events.startup").is_ok());
        assert!(validate_subject("nats_subscribe", "agent.broadcast.announcements").is_ok());
        assert!(validate_subject("nats_request", "agent.rpc.healthcheck").is_ok());
    }

    #[test]
    fn test_validate_subject_denied() {
        assert!(validate_subject("nats_publish", ">").is_err());
        assert!(validate_subject("nats_publish", "animus.in.permission_request").is_err());
        assert!(validate_subject("nats_subscribe", "nuntius.claim").is_err());
        assert!(validate_subject("nats_subscribe", "$JS.API.CONSUMER.MSG.NEXT").is_err());
        assert!(validate_subject("kv_put", "agent.registry").is_err());
    }

    #[test]
    fn test_validate_subject_empty() {
        assert!(validate_subject("nats_publish", "").is_err());
    }

    #[test]
    fn test_validate_subject_long() {
        let long_subject = "a".repeat(257);
        assert!(validate_subject("nats_publish", &long_subject).is_err());
    }

    #[test]
    fn test_validate_subject_unknown_tool() {
        assert!(validate_subject("unknown_tool", "agent.events.x").is_err());
    }

    #[test]
    fn test_validate_stream_name() {
        assert!(validate_stream_name("my-stream").is_ok());
        assert!(validate_stream_name("my_stream_123").is_ok());
        assert!(validate_stream_name("").is_err());
        assert!(validate_stream_name("bad stream!").is_err());
    }

    #[test]
    fn test_validate_bucket_name() {
        assert!(validate_bucket_name("my-bucket").is_ok());
        assert!(validate_bucket_name("").is_err());
        assert!(validate_bucket_name("bad bucket!").is_err());
    }

    #[test]
    fn test_validate_kv_key() {
        assert!(validate_kv_key("my-key").is_ok());
        assert!(validate_kv_key("").is_err());
        assert!(validate_kv_key("path/../secret").is_err());
    }
}
