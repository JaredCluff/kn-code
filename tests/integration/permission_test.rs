//! Integration tests for permission system

use kn_code_permissions::{PermissionContext, PermissionMode, PermissionRuleSource, ToolRule};

#[tokio::test]
async fn test_bypass_always_allows() {
    let ctx = PermissionContext::bypass();
    let decision = ctx.resolve_permission("Bash", &serde_json::json!({}), false, true, true);
    assert!(decision.is_allowed());
}

#[tokio::test]
async fn test_auto_mode_allows() {
    let ctx = PermissionContext::auto();
    let decision = ctx.resolve_permission("Bash", &serde_json::json!({}), false, true, true);
    assert!(decision.is_allowed());
}

#[tokio::test]
async fn test_plan_mode_denies_writes() {
    let ctx = PermissionContext::plan();
    let decision = ctx.resolve_permission("FileWrite", &serde_json::json!({}), false, true, true);
    assert!(!decision.is_allowed());
}

#[tokio::test]
async fn test_deny_rule_blocks() {
    let mut ctx = PermissionContext::default();
    ctx.always_deny.rules.push((
        ToolRule::ToolName("Bash".to_string()),
        PermissionRuleSource::CliArg,
    ));
    let decision = ctx.resolve_permission("Bash", &serde_json::json!({}), false, true, true);
    assert!(!decision.is_allowed());
}

#[tokio::test]
async fn test_allow_rule_overrides_default() {
    let mut ctx = PermissionContext::default();
    ctx.always_allow.rules.push((
        ToolRule::ToolName("Bash".to_string()),
        PermissionRuleSource::CliArg,
    ));
    let decision = ctx.resolve_permission("Bash", &serde_json::json!({}), false, true, true);
    assert!(decision.is_allowed());
}

#[tokio::test]
async fn test_read_only_tool_always_allowed() {
    let ctx = PermissionContext::default();
    let decision = ctx.resolve_permission("FileRead", &serde_json::json!({}), true, false, true);
    assert!(decision.is_allowed());
}
