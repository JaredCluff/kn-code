use serde::{Deserialize, Serialize};

/// Security classifier result for auto-mode permission decisions.
///
/// Mirrors Claude Code's YoloClassifier: a 2-stage classifier that uses
/// an LLM to decide whether a tool call is safe to auto-execute.
///
/// Stage 1 (Fast): Quick classification based on tool name and simple patterns.
/// Stage 2 (Thinking): Full LLM-based analysis of the command/input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierResult {
    pub allow: bool,
    pub confidence: f64,
    pub stage: ClassifierStage,
    pub reason: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ClassifierStage {
    Fast,
    Thinking,
}

/// Security classifier for auto-mode permission decisions.
///
/// When permission_mode is "auto", this classifier evaluates whether
/// destructive tool calls (especially bash commands) are safe to run
/// without user approval.
///
/// The classifier uses a tiered approach:
/// 1. Fast path: check against known-safe/known-dangerous patterns
/// 2. Thinking path: call an LLM to analyze the command
pub struct SecurityClassifier {
    pub enabled: bool,
    pub model: Option<String>,
    pub fast_allow_patterns: Vec<String>,
    pub fast_deny_patterns: Vec<String>,
}

impl Default for SecurityClassifier {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            fast_allow_patterns: vec![
                "ls ".to_string(),
                "cat ".to_string(),
                "head ".to_string(),
                "tail ".to_string(),
                "grep ".to_string(),
                "find ".to_string(),
                "git status".to_string(),
                "git log".to_string(),
                "git diff".to_string(),
                "git branch".to_string(),
                "cargo check".to_string(),
                "cargo build".to_string(),
                "cargo test".to_string(),
                "cargo clippy".to_string(),
                "cargo fmt".to_string(),
                "npm run build".to_string(),
                "npm test".to_string(),
                "npm run lint".to_string(),
                "python -m pytest".to_string(),
                "go test".to_string(),
                "go build".to_string(),
            ],
            fast_deny_patterns: vec![
                "rm -rf /".to_string(),
                "rm -rf ~".to_string(),
                " sudo ".to_string(),
                "\tsudo ".to_string(),
                "sudo ".to_string(),
                "curl ".to_string(),
                "curl\t".to_string(),
                "wget ".to_string(),
                "wget\t".to_string(),
                "chmod 777".to_string(),
                "dd if=".to_string(),
                "mkfs".to_string(),
                "shutdown".to_string(),
                "reboot".to_string(),
                "kill -9".to_string(),
                "pkill ".to_string(),
                "killall ".to_string(),
                "nc ".to_string(),
                "ncat ".to_string(),
                "netcat ".to_string(),
                "base64 -d".to_string(),
                "eval ".to_string(),
                "exec ".to_string(),
                "python -c".to_string(),
                "python3 -c".to_string(),
                "perl -e".to_string(),
                "ruby -e".to_string(),
                "node -e".to_string(),
                "node -p".to_string(),
                "nmap ".to_string(),
                "ssh ".to_string(),
                "scp ".to_string(),
                "rsync ".to_string(),
                "crontab".to_string(),
                "systemctl".to_string(),
                "service ".to_string(),
                "/usr/bin/sudo".to_string(),
                "/bin/bash -c".to_string(),
                "/bin/sh -c".to_string(),
                "bash -c".to_string(),
                "sh -c".to_string(),
                "fetch ".to_string(),
                "aria2c".to_string(),
            ],
        }
    }
}

impl SecurityClassifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_fast_allow_patterns(mut self, patterns: Vec<String>) -> Self {
        self.fast_allow_patterns = patterns;
        self
    }

    pub fn with_fast_deny_patterns(mut self, patterns: Vec<String>) -> Self {
        self.fast_deny_patterns = patterns;
        self
    }

    /// Classify a tool call for auto-mode.
    ///
    /// Returns a ClassifierResult with the decision and confidence.
    ///
    /// NOTE: This classifier is NEVER called when permission_mode is
    /// BypassPermissions. Bypass mode short-circuits ALL permission
    /// checks at the PermissionContext level before the classifier
    /// is even consulted. This prevents connectors, plugins, or any
    /// other subsystem from causing bypass mode to fail.
    pub async fn classify(&self, tool_name: &str, input: &serde_json::Value) -> ClassifierResult {
        if !self.enabled {
            return ClassifierResult {
                allow: true,
                confidence: 0.0,
                stage: ClassifierStage::Fast,
                reason: "Classifier disabled".to_string(),
                duration_ms: 0,
            };
        }

        // Stage 1: Fast path — check known patterns
        if let Some(result) = self.fast_classify(tool_name, input) {
            return result;
        }

        // Stage 2: Thinking path — would call LLM
        self.thinking_classify(tool_name, input).await
    }

    fn fast_classify(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<ClassifierResult> {
        let command = extract_command(tool_name, input);

        for pattern in &self.fast_deny_patterns {
            if command.contains(pattern.as_str()) {
                return Some(ClassifierResult {
                    allow: false,
                    confidence: 0.95,
                    stage: ClassifierStage::Fast,
                    reason: format!("Matched deny pattern: {}", pattern),
                    duration_ms: 0,
                });
            }
        }

        for pattern in &self.fast_allow_patterns {
            if command.starts_with(pattern.as_str()) {
                return Some(ClassifierResult {
                    allow: true,
                    confidence: 0.90,
                    stage: ClassifierStage::Fast,
                    reason: format!("Matched allow pattern: {}", pattern),
                    duration_ms: 0,
                });
            }
        }

        None
    }

    async fn thinking_classify(
        &self,
        _tool_name: &str,
        _input: &serde_json::Value,
    ) -> ClassifierResult {
        // TODO: Call LLM to analyze the tool call
        // Default to DENY when classifier model is not configured — fail-safe
        ClassifierResult {
            allow: false,
            confidence: 0.0,
            stage: ClassifierStage::Thinking,
            reason: "No classifier model configured, defaulting to deny for safety".to_string(),
            duration_ms: 0,
        }
    }
}

fn extract_command(tool_name: &str, input: &serde_json::Value) -> String {
    if tool_name == "Bash" {
        return input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }
    input.to_string()
}
