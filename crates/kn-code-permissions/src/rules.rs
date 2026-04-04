use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Permission modes mirror Claude Code's behavior plus our enhancements.
///
/// Critical design rule: BypassPermissions MUST override ALL permission checks,
/// including those from plugins, connectors, MCP servers, hooks, and classifiers.
/// No subsystem may bypass BypassPermissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Ask for every destructive operation
    #[default]
    Ask,
    /// Auto-accept file edits, ask for bash commands
    AcceptEdits,
    /// Auto-accept everything (headless default)
    Auto,
    /// Plan mode — no modifications allowed
    Plan,
    /// Bypass ALL permission checks. This is the strongest mode and
    /// cannot be overridden by plugins, connectors, MCP servers, hooks,
    /// or classifiers. Used by orchestrators like Paperclip.
    BypassPermissions,
}

impl PermissionMode {
    pub fn is_headless_safe(&self) -> bool {
        matches!(self, Self::Auto | Self::BypassPermissions)
    }

    pub fn allows_writes(&self) -> bool {
        !matches!(self, Self::Plan)
    }

    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::Ask | Self::AcceptEdits)
    }
}

/// Tool permission rules, matching Claude Code's rule types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolRule {
    /// Exact tool name match: "Bash"
    ToolName(String),
    /// Tool with argument prefix: "Bash(git add:*)"
    ToolWithArgs { tool: String, arg_prefix: String },
    /// MCP server-level: "mcp__server1"
    McpServer(String),
    /// MCP wildcard: "mcp__server1__*"
    McpWildcard(String),
    /// Path-based rule
    Path { tool: String, path_pattern: String },
    /// Regex-based rule for advanced matching
    Regex { tool: String, pattern: String },
}

impl ToolRule {
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        match self {
            Self::ToolName(name) => name == tool_name,
            Self::ToolWithArgs { tool, .. } => tool == tool_name,
            Self::McpServer(server) => tool_name.starts_with(&format!("mcp__{}", server)),
            Self::McpWildcard(server) => tool_name.starts_with(&format!("mcp__{}__", server)),
            Self::Path { tool, .. } => tool == tool_name,
            Self::Regex { tool, .. } => tool == tool_name,
        }
    }
}

/// Where a permission rule came from. Higher-priority sources override lower ones.
/// Priority order (highest to lowest):
/// 1. CliArg — passed on command line
/// 2. Session — set during session (e.g. by skill/command)
/// 3. Command — from skill execution
/// 4. PolicySettings — from organizational policy
/// 5. UserSettings — from user's global config
/// 6. ProjectSettings — from project-level config
/// 7. LocalSettings — from local directory config
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    CliArg,
    Session,
    Command,
    PolicySettings,
    UserSettings,
    ProjectSettings,
    LocalSettings,
}

impl PermissionRuleSource {
    pub fn priority(&self) -> u8 {
        match self {
            Self::CliArg => 70,
            Self::Session => 60,
            Self::Command => 50,
            Self::PolicySettings => 40,
            Self::UserSettings => 30,
            Self::ProjectSettings => 20,
            Self::LocalSettings => 10,
        }
    }
}

/// Rules grouped by their source for tracking and debugging.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolPermissionRulesBySource {
    pub rules: Vec<(ToolRule, PermissionRuleSource)>,
}

/// The complete permission context for a session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionContext {
    pub mode: PermissionMode,
    pub additional_working_directories: Vec<PathBuf>,
    pub always_allow: ToolPermissionRulesBySource,
    pub always_deny: ToolPermissionRulesBySource,
    pub always_ask: ToolPermissionRulesBySource,
    pub is_bypass_permissions_mode_available: bool,
    pub is_auto_mode_available: bool,
    pub should_avoid_permission_prompts: bool,
    pub await_automated_checks_before_dialog: bool,
    /// Whether the classifier is enabled for auto-mode
    pub classifier_enabled: bool,
    /// Custom auto-mode config for the classifier
    pub auto_mode_config: Option<AutoModeConfig>,
}

/// Configuration for auto-mode behavior.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoModeConfig {
    /// Tools that are always allowed in auto-mode
    #[serde(default)]
    pub allow: Vec<String>,
    /// Tools that trigger a soft deny (warn but allow)
    #[serde(default)]
    pub soft_deny: Vec<String>,
    /// Environment-specific rules
    #[serde(default)]
    pub environment: Vec<ToolRule>,
    /// Whether to use the LLM classifier for bash commands
    #[serde(default = "default_classifier")]
    pub use_classifier: bool,
}

fn default_classifier() -> bool {
    true
}

/// A suggestion for how to update permission rules based on a user's decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionUpdate {
    AddAllowRule {
        rule: ToolRule,
        source: PermissionRuleSource,
    },
    AddDenyRule {
        rule: ToolRule,
        source: PermissionRuleSource,
    },
    AddAlwaysAskRule {
        rule: ToolRule,
        source: PermissionRuleSource,
    },
    SetMode(PermissionMode),
}

/// The result of a permission check.
///
/// CRITICAL: BypassPermissions ALWAYS returns Allow, regardless of any
/// plugin, connector, MCP server, hook, or classifier override attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionDecision {
    Allow {
        updated_input: Option<serde_json::Value>,
        reason: PermissionReason,
    },
    Deny {
        message: String,
        suggestions: Vec<PermissionUpdate>,
        reason: PermissionReason,
    },
    Ask {
        message: String,
        suggestions: Vec<PermissionUpdate>,
        reason: PermissionReason,
    },
    /// Pass through to the caller without deciding (used by headless mode
    /// to return permission requests to the orchestrator)
    Passthrough {
        message: Option<String>,
        tool_name: String,
        input: serde_json::Value,
    },
}

/// Why a permission decision was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReason {
    /// Matched an allow rule
    Rule,
    /// Determined by permission mode
    Mode,
    /// Result of sub-command evaluation
    SubcommandResults,
    /// Permission prompt tool (headless passthrough)
    PermissionPromptTool,
    /// Hook decided
    Hook,
    /// Async agent context
    AsyncAgent,
    /// Sandbox override (sandbox deemed it safe)
    SandboxOverride,
    /// LLM classifier decision
    Classifier,
    /// Working directory constraint
    WorkingDir,
    /// Safety check passed/failed
    SafetyCheck,
    /// BypassPermissions mode — always allow
    Bypass,
    /// Default fallback
    Other,
}

impl PermissionDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    pub fn deny_message(&self) -> Option<&str> {
        match self {
            Self::Deny { message, .. } => Some(message),
            _ => None,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        matches!(self, Self::Passthrough { .. })
    }

    /// Bypass mode always returns Allow — no exceptions.
    pub fn bypass(_tool_name: &str) -> Self {
        Self::Allow {
            updated_input: None,
            reason: PermissionReason::Bypass,
        }
    }
}

impl PermissionContext {
    /// Create a context with BypassPermissions mode.
    /// This is the strongest mode — nothing can override it.
    pub fn bypass() -> Self {
        Self {
            mode: PermissionMode::BypassPermissions,
            is_bypass_permissions_mode_available: true,
            ..Default::default()
        }
    }

    /// Create a context with Auto mode (headless default).
    pub fn auto() -> Self {
        Self {
            mode: PermissionMode::Auto,
            is_auto_mode_available: true,
            ..Default::default()
        }
    }

    /// Create a context for AcceptEdits mode.
    pub fn accept_edits() -> Self {
        Self {
            mode: PermissionMode::AcceptEdits,
            ..Default::default()
        }
    }

    /// Create a context for Plan mode.
    pub fn plan() -> Self {
        Self {
            mode: PermissionMode::Plan,
            ..Default::default()
        }
    }

    /// The core permission resolution logic.
    ///
    /// Resolution order (first match wins):
    /// 1. BypassPermissions mode → ALWAYS Allow (cannot be overridden)
    /// 2. Always-deny rules (checked across all sources, highest priority first)
    /// 3. Always-ask rules
    /// 4. Read-only tool check
    /// 5. Always-allow rules
    /// 6. Plan mode → Deny all writes
    /// 7. AcceptEdits mode → Allow reads/file edits, ask for bash
    /// 8. Auto mode → Use classifier if enabled, otherwise allow
    /// 9. Default → Ask (or passthrough in headless)
    pub fn resolve_permission(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        is_read_only: bool,
        is_destructive: bool,
        is_headless: bool,
    ) -> PermissionDecision {
        // 1. BypassPermissions — ALWAYS allow, no exceptions
        if self.mode == PermissionMode::BypassPermissions {
            return PermissionDecision::bypass(tool_name);
        }

        // 2. Check always-deny rules (highest priority source wins)
        if self.is_denied(tool_name, input) {
            return PermissionDecision::Deny {
                message: format!("Tool '{}' is denied by permission rule", tool_name),
                suggestions: vec![PermissionUpdate::AddAllowRule {
                    rule: ToolRule::ToolName(tool_name.to_string()),
                    source: PermissionRuleSource::Session,
                }],
                reason: PermissionReason::Rule,
            };
        }

        // 3. Check always-ask rules
        if self.is_always_ask(tool_name, input) {
            return PermissionDecision::Ask {
                message: format!("Tool '{}' requires approval", tool_name),
                suggestions: vec![
                    PermissionUpdate::AddAllowRule {
                        rule: ToolRule::ToolName(tool_name.to_string()),
                        source: PermissionRuleSource::Session,
                    },
                    PermissionUpdate::AddAlwaysAskRule {
                        rule: ToolRule::ToolName(tool_name.to_string()),
                        source: PermissionRuleSource::Session,
                    },
                ],
                reason: PermissionReason::Rule,
            };
        }

        // 4. Read-only tools are generally safe
        if is_read_only {
            return PermissionDecision::Allow {
                updated_input: None,
                reason: PermissionReason::SafetyCheck,
            };
        }

        // 5. Check always-allow rules
        if self.is_explicitly_allowed(tool_name, input) {
            return PermissionDecision::Allow {
                updated_input: None,
                reason: PermissionReason::Rule,
            };
        }

        // 6. Plan mode — deny all writes
        if self.mode == PermissionMode::Plan {
            return PermissionDecision::Deny {
                message: "Cannot modify files in plan mode".to_string(),
                suggestions: vec![],
                reason: PermissionReason::Mode,
            };
        }

        // 7. AcceptEdits mode — allow file operations, ask for bash
        if self.mode == PermissionMode::AcceptEdits {
            let is_file_tool =
                tool_name.starts_with("File") || tool_name == "Glob" || tool_name == "Grep";
            if is_file_tool && !is_destructive {
                return PermissionDecision::Allow {
                    updated_input: None,
                    reason: PermissionReason::Mode,
                };
            }
            // Fall through to ask for bash/other destructive tools
        }

        // 8. Auto mode — use classifier or deny
        if self.mode == PermissionMode::Auto {
            if self.classifier_enabled && is_destructive {
                // Classifier would be called here; for now, deny for safety
                return PermissionDecision::Deny {
                    message: format!(
                        "Tool '{}' requires classifier evaluation in auto mode",
                        tool_name
                    ),
                    suggestions: vec![PermissionUpdate::AddAllowRule {
                        rule: ToolRule::ToolName(tool_name.to_string()),
                        source: PermissionRuleSource::Session,
                    }],
                    reason: PermissionReason::Classifier,
                };
            }
            return PermissionDecision::Deny {
                message: format!("Tool '{}' requires approval in auto mode", tool_name),
                suggestions: vec![PermissionUpdate::AddAllowRule {
                    rule: ToolRule::ToolName(tool_name.to_string()),
                    source: PermissionRuleSource::Session,
                }],
                reason: PermissionReason::Mode,
            };
        }

        // 9. Default: ask (or passthrough in headless)
        if is_headless && self.should_avoid_permission_prompts {
            return PermissionDecision::Passthrough {
                message: Some(format!("Tool '{}' requires approval", tool_name)),
                tool_name: tool_name.to_string(),
                input: input.clone(),
            };
        }

        PermissionDecision::Ask {
            message: format!("Tool '{}' requires approval", tool_name),
            suggestions: vec![PermissionUpdate::AddAllowRule {
                rule: ToolRule::ToolName(tool_name.to_string()),
                source: PermissionRuleSource::Session,
            }],
            reason: PermissionReason::Other,
        }
    }

    fn is_explicitly_allowed(&self, tool_name: &str, input: &serde_json::Value) -> bool {
        self.always_allow.rules.iter().any(|(rule, _source)| {
            rule.matches_tool(tool_name) && self.rule_matches_args(rule, input)
        })
    }

    fn is_denied(&self, tool_name: &str, input: &serde_json::Value) -> bool {
        // Sort by source priority, highest first
        let mut rules: Vec<_> = self.always_deny.rules.iter().collect();
        rules.sort_by(|a, b| b.1.priority().cmp(&a.1.priority()));
        rules.iter().any(|(rule, _source)| {
            rule.matches_tool(tool_name) && self.rule_matches_args(rule, input)
        })
    }

    fn is_always_ask(&self, tool_name: &str, input: &serde_json::Value) -> bool {
        self.always_ask.rules.iter().any(|(rule, _source)| {
            rule.matches_tool(tool_name) && self.rule_matches_args(rule, input)
        })
    }

    fn rule_matches_args(&self, rule: &ToolRule, input: &serde_json::Value) -> bool {
        match rule {
            ToolRule::ToolWithArgs { arg_prefix, .. } => {
                // Check if any argument value starts with the prefix
                if let Some(obj) = input.as_object() {
                    obj.values().any(|v| {
                        v.as_str()
                            .map(|s| s.starts_with(arg_prefix))
                            .unwrap_or(false)
                    })
                } else {
                    true
                }
            }
            ToolRule::Path { path_pattern, .. } => {
                // Check if file_path argument matches the pattern
                if let Some(file_path) = input.get("file_path").and_then(|v| v.as_str()) {
                    return file_path.contains(path_pattern);
                }
                if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                    return path.contains(path_pattern);
                }
                true
            }
            ToolRule::Regex { pattern, .. } => {
                if let Ok(regex) = regex::Regex::new(pattern) {
                    return regex.is_match(&input.to_string());
                }
                false
            }
            _ => true,
        }
    }
}
