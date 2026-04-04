use crate::traits::*;
use async_trait::async_trait;
use kn_code_permissions::SandboxType;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_OUTPUT_SIZE: usize = 50_000;
const MAX_BACKGROUND_TASKS: usize = 20;
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 3_600_000;

#[derive(Debug)]
pub struct BashTool {
    sandbox: SandboxType,
    background_tasks: Arc<Mutex<Vec<BackgroundTask>>>,
}

#[derive(Debug)]
struct BackgroundTask {
    task_id: String,
    child: tokio::process::Child,
    started_at: std::time::Instant,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            sandbox: SandboxType::detect(),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl BashTool {
    pub fn new(sandbox: SandboxType) -> Self {
        Self {
            sandbox,
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn kill_background_tasks(&self) -> usize {
        let mut tasks = self.background_tasks.lock().await;
        let count = tasks.len();
        for mut task in tasks.drain(..) {
            let _ = task.child.kill().await;
        }
        tracing::info!(count, "Killed background bash tasks");
        count
    }

    async fn reap_dead_tasks(&self) {
        let mut tasks = self.background_tasks.lock().await;
        let stale = std::time::Duration::from_secs(3600);
        tasks.retain_mut(|task| {
            if task.started_at.elapsed() > stale {
                tracing::warn!(task_id = %task.task_id, "Reaping stale background task");
                let _ = task.child.start_kill();
                false
            } else if let Ok(Some(_)) = task.child.try_wait() {
                tracing::debug!(task_id = %task.task_id, "Background task already exited");
                false
            } else {
                true
            }
        });
    }

    fn truncate_output(s: &str) -> String {
        if s.len() <= MAX_OUTPUT_SIZE {
            s.to_string()
        } else {
            let end = s
                .char_indices()
                .take_while(|(idx, _)| *idx <= MAX_OUTPUT_SIZE)
                .last()
                .map_or(0, |(idx, c)| idx + c.len_utf8());
            format!("{}\n... (truncated, {} total bytes)", &s[..end], s.len())
        }
    }
}

#[derive(Debug, Deserialize)]
struct BashInput {
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    description: Option<String>,
    #[serde(default)]
    run_in_background: Option<bool>,
}

#[derive(Debug, Serialize)]
struct BashOutput {
    stdout: String,
    stderr: String,
    return_code: i32,
    interrupted: bool,
    background_task_id: Option<String>,
    persisted_output_path: Option<String>,
    return_code_interpretation: String,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    fn prompt(&self) -> &str {
        "Use this to run shell commands. Commands run in a sandbox when available."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to execute" },
                "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds (default 120000)" },
                "description": { "type": "string", "description": "Human-readable description" },
                "run_in_background": { "type": "boolean", "description": "Run asynchronously" }
            },
            "required": ["command"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn is_destructive(&self) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> usize {
        MAX_OUTPUT_SIZE
    }

    async fn call(
        &self,
        input: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let bash_input: BashInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        let timeout_ms = bash_input
            .timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        let run_background = bash_input.run_in_background.unwrap_or(false);

        let command = &bash_input.command;

        let args = self.sandbox.sandbox_args(command);
        let mut cmd = tokio::process::Command::new(&args[0]);
        cmd.args(&args[1..]);
        cmd.current_dir(&context.cwd);

        if run_background {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        } else {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
        }

        if run_background {
            self.reap_dead_tasks().await;

            let mut tasks = self.background_tasks.lock().await;
            if tasks.len() >= MAX_BACKGROUND_TASKS {
                return Ok(ToolResult {
                    content: ToolContent::Text(format!(
                        "Too many background tasks (max {}). Kill some with kill_background_tasks first.",
                        MAX_BACKGROUND_TASKS
                    )),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                });
            }

            let task_id = format!("bg_{}", uuid::Uuid::new_v4());
            let child = cmd.spawn().map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to spawn background process: {}", e))
            })?;

            tasks.push(BackgroundTask {
                task_id: task_id.clone(),
                child,
                started_at: std::time::Instant::now(),
            });

            return Ok(ToolResult {
                content: ToolContent::Text(
                    serde_json::to_string(&BashOutput {
                        stdout: String::new(),
                        stderr: String::new(),
                        return_code: 0,
                        interrupted: false,
                        background_task_id: Some(task_id),
                        persisted_output_path: None,
                        return_code_interpretation: "Command started in background.".to_string(),
                    })
                    .unwrap_or_default(),
                ),
                new_messages: Vec::new(),
                persisted: false,
                persisted_path: None,
                structured_content: None,
            });
        }

        let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), cmd.output())
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(ToolResult {
                    content: ToolContent::Text(
                        serde_json::to_string(&BashOutput {
                            stdout: String::new(),
                            stderr: Self::truncate_output(&e.to_string()),
                            return_code: 1,
                            interrupted: false,
                            background_task_id: None,
                            persisted_output_path: None,
                            return_code_interpretation: "Command failed to execute.".to_string(),
                        })
                        .unwrap_or_default(),
                    ),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                });
            }
            Err(_) => {
                return Ok(ToolResult {
                    content: ToolContent::Text(
                        serde_json::to_string(&BashOutput {
                            stdout: String::new(),
                            stderr: Self::truncate_output(&format!(
                                "Command timed out after {}ms",
                                timeout_ms
                            )),
                            return_code: 124,
                            interrupted: true,
                            background_task_id: None,
                            persisted_output_path: None,
                            return_code_interpretation: "Command timed out.".to_string(),
                        })
                        .unwrap_or_default(),
                    ),
                    new_messages: Vec::new(),
                    persisted: false,
                    persisted_path: None,
                    structured_content: None,
                });
            }
        };

        let stdout = Self::truncate_output(&String::from_utf8_lossy(&output.stdout));
        let stderr = Self::truncate_output(&String::from_utf8_lossy(&output.stderr));
        let return_code = output.status.code().unwrap_or(-1);

        let interpretation = if return_code == 0 {
            "The command succeeded.".to_string()
        } else if output.status.code().is_none() {
            "The command was interrupted by a signal.".to_string()
        } else {
            format!("The command exited with non-zero status: {}", return_code)
        };

        Ok(ToolResult {
            content: ToolContent::Text(
                serde_json::to_string(&BashOutput {
                    stdout,
                    stderr,
                    return_code,
                    interrupted: output.status.code().is_none(),
                    background_task_id: None,
                    persisted_output_path: None,
                    return_code_interpretation: interpretation,
                })
                .unwrap_or_default(),
            ),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: None,
        })
    }
}

impl Drop for BashTool {
    fn drop(&mut self) {
        let background_tasks = self.background_tasks.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut tasks = background_tasks.lock().await;
                for mut task in tasks.drain(..) {
                    let _ = task.child.kill().await;
                }
            });
        }
    }
}
