use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A block in the system prompt.
///
/// The `cache_control` flag tells the provider to cache this block
/// (Anthropic supports prompt caching with cache_control breakpoints).
/// Blocks that don't change between turns should be cached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    pub content: String,
    pub cache_control: bool,
}

/// Builder for the dynamic system prompt.
///
/// The prompt is rebuilt each turn based on:
/// 1. Core identity (always first, always cached)
/// 2. Custom instructions from project files (cached if unchanged)
/// 3. Tool descriptions (cached if tool set unchanged)
/// 4. Permission mode context
/// 5. File state context (never cached — changes frequently)
/// 6. Plugin prompts
/// 7. Skill prompts
pub struct SystemPromptBuilder {
    pub core_identity: String,
    pub custom_instructions: Option<String>,
    pub custom_instructions_hash: Option<String>,
    pub tool_descriptions: Vec<String>,
    pub tool_descriptions_hash: Option<String>,
    pub permission_mode_prompt: String,
    pub file_state_context: Option<String>,
    pub plugin_prompts: Vec<String>,
    pub skill_prompts: Vec<String>,
    pub working_directory: PathBuf,
}

impl SystemPromptBuilder {
    pub fn new(working_directory: PathBuf) -> Self {
        Self {
            core_identity: Self::default_core_identity(),
            custom_instructions: None,
            custom_instructions_hash: None,
            tool_descriptions: Vec::new(),
            tool_descriptions_hash: None,
            permission_mode_prompt: String::new(),
            file_state_context: None,
            plugin_prompts: Vec::new(),
            skill_prompts: Vec::new(),
            working_directory,
        }
    }

    fn default_core_identity() -> String {
        r#"You are kn-code, an AI coding agent running in a terminal environment.

You are a highly capable, detail-oriented coding assistant that helps users with software engineering tasks.
You have access to tools that let you execute shell commands, read and write files, search codebases, and more.

Guidelines:
- Always read files before editing them
- Use the appropriate tool for each task
- Be thorough but concise
- When making changes, ensure the code compiles and tests pass
- If you're unsure about something, ask the user
- Never make assumptions about file contents — always read first"#.to_string()
    }

    /// Load custom instructions from project files.
    ///
    /// Priority order (first match wins):
    /// 1. .kn-code/AGENTS.md
    /// 2. AGENTS.md
    /// 3. .claude/CLAUDE.md (Claude Code compatibility)
    /// 4. CLAUDE.md
    /// 5. .cursor/rules/ (Cursor compatibility)
    /// 6. .github/copilot-instructions.md
    pub async fn load_custom_instructions(cwd: &Path) -> anyhow::Result<Option<(String, String)>> {
        let candidates = [
            cwd.join(".kn-code/AGENTS.md"),
            cwd.join("AGENTS.md"),
            cwd.join(".claude/CLAUDE.md"),
            cwd.join("CLAUDE.md"),
            cwd.join(".cursorrules"),
            cwd.join(".github/copilot-instructions.md"),
        ];

        for path in &candidates {
            if path.exists() {
                let content = tokio::fs::read_to_string(path).await?;
                let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
                return Ok(Some((content, hash)));
            }
        }

        Ok(None)
    }

    /// Build the system prompt as a list of cache-aware blocks.
    pub async fn build(&self) -> Vec<SystemBlock> {
        let mut blocks = Vec::new();

        // 1. Core identity (always first, always cached)
        blocks.push(SystemBlock {
            content: self.core_identity.clone(),
            cache_control: true,
        });

        // 2. Custom instructions (cached if present)
        if let Some(instructions) = &self.custom_instructions {
            blocks.push(SystemBlock {
                content: instructions.clone(),
                cache_control: true,
            });
        }

        // 3. Tool descriptions (cached if tool set unchanged)
        if !self.tool_descriptions.is_empty() {
            let tools_content = self.tool_descriptions.join("\n\n");
            blocks.push(SystemBlock {
                content: tools_content,
                cache_control: true,
            });
        }

        // 4. Permission mode
        if !self.permission_mode_prompt.is_empty() {
            blocks.push(SystemBlock {
                content: self.permission_mode_prompt.clone(),
                cache_control: false,
            });
        }

        // 5. File state context (never cached — changes frequently)
        if let Some(file_state) = &self.file_state_context {
            blocks.push(SystemBlock {
                content: file_state.clone(),
                cache_control: false,
            });
        }

        // 6. Plugin prompts
        for prompt in &self.plugin_prompts {
            blocks.push(SystemBlock {
                content: prompt.clone(),
                cache_control: false,
            });
        }

        // 7. Skill prompts
        for prompt in &self.skill_prompts {
            blocks.push(SystemBlock {
                content: prompt.clone(),
                cache_control: false,
            });
        }

        blocks
    }

    /// Build the full system prompt as a single string (for providers that don't support blocks).
    pub async fn build_string(&self) -> String {
        let blocks = self.build().await;
        blocks
            .iter()
            .map(|b| b.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}

/// Represents the state of cached blocks between turns.
/// Used to determine which blocks need cache_control reset.
#[derive(Debug, Clone, Default)]
pub struct PromptCacheState {
    pub core_identity_hash: String,
    pub custom_instructions_hash: Option<String>,
    pub tool_descriptions_hash: Option<String>,
}

impl PromptCacheState {
    pub fn needs_refresh(&self, new_instructions_hash: Option<&str>) -> bool {
        match (&self.custom_instructions_hash, new_instructions_hash) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(_), None) => true,
            (Some(old), Some(new)) => old != new,
        }
    }
}
