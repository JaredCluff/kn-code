# SPEC 008: Paperclip Adapter Compatibility

## 1. Overview

kn-code must be a drop-in replacement for the `opencode_local` adapter in Paperclip. This means it must accept the same CLI arguments, produce the same JSONL output format, support session resume, and handle skill injection identically.

## 2. CLI Compatibility

### 2.1 Run Command (Primary)

```bash
kn-code run \
    --format json \
    [--session <session_id>] \
    --model <provider/model> \
    [--variant <effort>] \
    [--output-format json] \
    [--print -] \
    <prompt>
```

This mirrors the `opencode run --format json` command that Paperclip's `opencode_local` adapter calls.

### 2.2 Argument Mapping

| Paperclip Arg | kn-code Arg | Notes |
|---------------|-------------|-------|
| `--format json` | `--format json` | JSONL output to stdout |
| `--session <id>` | `--session <id>` | Resume session |
| `--model <model>` | `--model <provider/model>` | kn-code uses provider/model format |
| `--variant <effort>` | `--variant <effort>` | minimal\|low\|medium\|high\|max |
| `--print -` | `--print -` | Read prompt from stdin |
| (extra args) | (extra args) | Passed through |

### 2.3 Model Format

Paperclip's `opencode_local` adapter passes models in `provider/model` format:
- `anthropic/claude-sonnet-4-5`
- `openai/gpt-4o`
- `github_copilot/gpt-4o`

kn-code natively uses this format, so no translation is needed.

## 3. JSONL Output Format

kn-code MUST produce the exact same event types that Paperclip's parser expects. The parser in Paperclip's `opencode_local` adapter looks for these events:

### 3.1 Required Events

```json
// Session initialization
{"type": "system", "subtype": "init", "session_id": "abc123", "model": "claude-sonnet-4-5"}

// Assistant text output
{"type": "text", "content": "I'll start by examining the codebase..."}

// Tool usage
{"type": "tool_use", "id": "tool_1", "name": "Bash", "input": {"command": "ls -la"}}

// Tool result
{"type": "tool_result", "id": "tool_1", "output": {"stdout": "total 42\n...", "stderr": "", "return_code": 0}}

// Tool error
{"type": "tool_use", "id": "tool_2", "name": "FileEdit", "error": "File not found"}

// Final result (REQUIRED — Paperclip looks for this to know the run is done)
{"type": "result", "subtype": "success", "session_id": "abc123", "usage": {"input_tokens": 5000, "output_tokens": 3000, "cached_input_tokens": 4000}, "cost_usd": 0.042}

// Error result
{"type": "result", "subtype": "error", "session_id": "abc123", "error": "Model not found"}
```

### 3.2 Paperclip Parser Expectations

From Paperclip's `parseOpenCodeJsonl()`:

| Event Type | What Paperclip Extracts |
|------------|------------------------|
| `text` | Collected into summary string |
| `step_finish` | Token usage (input, cache.read, output+reasoning) and cost |
| `tool_use` | Tool errors captured |
| `error` | Error messages captured |
| `result` | Session ID, final usage, cost, summary |

**Critical**: kn-code must emit `step_finish` events for token tracking:
```json
{"type": "step_finish", "usage": {"input_tokens": 1000, "cache_read_tokens": 500, "output_tokens": 800, "reasoning_tokens": 200}, "cost_usd": 0.012}
```

### 3.3 Unknown Session Detection

Paperclip detects "unknown session" errors to trigger session retry. kn-code must produce recognizable errors:
```json
{"type": "error", "message": "unknown session abc123"}
```

The regex Paperclip uses:
```
/unknown\s+session|session\b.*\bnot\s+found|resource\s+not\s+found:.*[\\/]session[\\/].*\.json|notfounderror|no session/i
```

## 4. Session Resume Compatibility

### 4.1 Session Storage Location

Paperclip's `opencode_local` adapter expects sessions to be stored in OpenCode's default location. kn-code must either:

**Option A**: Store sessions in the same location as OpenCode (`~/.config/opencode/sessions/`)
**Option B**: Accept a `--session-dir` flag to specify session location

We use **Option B** for flexibility, defaulting to `~/.kn-code/sessions/`.

### 4.2 Session Resume Flow

```
1. Paperclip calls: kn-code run --format json --session <id> --model <model> <prompt>
2. kn-code loads session from disk
3. kn-code verifies session cwd matches current cwd
4. kn-code adds handoff message
5. kn-code runs agent loop
6. kn-code outputs JSONL events
7. kn-code exits with code 0 (success) or 1 (error)
8. Paperclip parses output, extracts session_id for next heartbeat
```

### 4.3 Session ID Format

kn-code uses UUID v4 for session IDs, same as OpenCode. This ensures compatibility.

## 5. Skill Injection Compatibility

Paperclip injects skills by symlinking them into `~/.claude/skills/`. kn-code must:

1. **Read skills from the same location**: `~/.claude/skills/` (shared with Claude Code and OpenCode)
2. **Support SKILL.md format**: Same format as Claude Code skills
3. **Support `--add-dir` flag**: For passing additional skill directories

```bash
kn-code run --add-dir /tmp/paperclip-skills-abc123 ...
```

### 5.1 Skill Discovery

```rust
pub async fn discover_skills(cwd: &Path, add_dirs: &[PathBuf]) -> Vec<Skill> {
    let mut skills = Vec::new();

    // 1. Global skills directory
    let global_skills = home_dir().join(".claude/skills/");
    skills.extend(load_skills_from_dir(&global_skills).await);

    // 2. Project-level skills
    let project_skills = cwd.join(".kn-code/skills/");
    skills.extend(load_skills_from_dir(&project_skills).await);

    // 3. Additional directories (from --add-dir)
    for dir in add_dirs {
        skills.extend(load_skills_from_dir(dir).await);
    }

    skills
}
```

## 6. Permission Bypass for Headless Mode

Paperclip runs kn-code with permissions bypassed (headless mode). kn-code supports:

```bash
kn-code run --permission-mode auto ...
```

Or via environment variable:
```bash
KN_CODE_PERMISSION_MODE=auto kn-code run ...
```

This auto-accepts all tool calls, equivalent to Claude Code's `--dangerously-skip-permissions`.

## 7. Environment Variables from Paperclip

Paperclip injects these env vars into every agent run. kn-code must read and use them:

| Variable | Purpose |
|----------|---------|
| `PAPERCLIP_RUN_ID` | Unique run identifier |
| `PAPERCLIP_TASK_ID` | Issue/task ID |
| `PAPERCLIP_WORKSPACE_PATH` | Absolute workspace path |
| `PAPERCLIP_WORKSPACE_RELATIVE` | Relative workspace path |
| `PAPERCLIP_COMPANY_ID` | Company ID |
| `PAPERCLIP_AGENT_ID` | Agent ID |
| `PAPERCLIP_API_KEY` | JWT auth token for API callbacks |
| `PAPERCLIP_HEARTBEAT_PROMPT` | The heartbeat prompt text |
| `OPENCODE_DISABLE_PROJECT_CONFIG` | Don't load project config (set by Paperclip) |

kn-code equivalent:
| Variable | Purpose |
|----------|---------|
| `KN_CODE_RUN_ID` | Same as PAPERCLIP_RUN_ID |
| `KN_CODE_TASK_ID` | Same as PAPERCLIP_TASK_ID |
| `KN_CODE_WORKSPACE` | Same as PAPERCLIP_WORKSPACE_PATH |
| `KN_CODE_COMPANY_ID` | Same as PAPERCLIP_COMPANY_ID |
| `KN_CODE_AGENT_ID` | Same as PAPERCLIP_AGENT_ID |
| `KN_CODE_AUTH_TOKEN` | Same as PAPERCLIP_API_KEY |
| `KN_CODE_DISABLE_PROJECT_CONFIG` | Same as OPENCODE_DISABLE_PROJECT_CONFIG |

kn-code reads **both** `PAPERCLIP_*` and `KN_CODE_*` prefixes for compatibility.

## 8. Paperclip Adapter Registration

To register kn-code as a Paperclip adapter, add to Paperclip's adapter registry:

```rust
// In Paperclip's server/src/adapters/registry.rs:
register_adapter("kn_code_local", kn_code_local::adapter());
```

### 8.1 Adapter Implementation

```rust
// packages/adapters/kn-code-local/src/server/execute.ts

const DEFAULT_COMMAND = "kn-code";
const DEFAULT_ARGS = ["run", "--format", "json", "--permission-mode", "auto"];

export async function executeKnCode(ctx: AdapterExecutionContext): Promise<AdapterExecutionResult> {
    // 1. Resolve config
    const cwd = resolveCwd(ctx);
    const model = ctx.config.model;  // provider/model format
    const variant = ctx.config.variant;  // effort level

    // 2. Inject skills (same as opencode: ~/.claude/skills/)
    const skillsDir = await injectSkills(ctx);

    // 3. Build environment
    const env = buildEnv(ctx);

    // 4. Resolve session
    const sessionId = resolveSession(ctx, cwd);

    // 5. Build prompt (bootstrap + heartbeat)
    const prompt = buildPrompt(ctx, sessionId);

    // 6. Build CLI args
    const args = [
        "run",
        "--format", "json",
        "--permission-mode", "auto",
        ...(sessionId ? ["--session", sessionId] : []),
        ...(model ? ["--model", model] : []),
        ...(variant ? ["--variant", variant] : []),
        ...(skillsDir ? ["--add-dir", skillsDir] : []),
        ...extraArgs,
    ];

    // 7. Execute
    const result = await runChildProcess({
        command: "kn-code",
        args,
        cwd,
        env,
        stdin: prompt,
    });

    // 8. Parse JSONL output (same parser as opencode_local)
    const parsed = parseKnCodeJsonl(result.stdout);

    // 9. Build result
    return {
        exitCode: result.exitCode,
        sessionId: parsed.sessionId,
        usage: parsed.usage,
        costUsd: parsed.costUsd,
        summary: parsed.summary,
        provider: inferProvider(model),
        biller: inferBiller(env),
        model: extractModel(model),
    };
}
```

### 8.2 JSONL Parser (Reuses opencode parser)

The parser is identical to `opencode_local` since kn-code produces the same event types:

```typescript
function parseKnCodeJsonl(stdout: string): ParsedResult {
    let sessionId: string | null = null;
    let summary = "";
    let usage = { inputTokens: 0, outputTokens: 0, cachedInputTokens: 0 };
    let costUsd = 0;
    let errorMessage: string | null = null;

    for (const line of stdout.trim().split("\n")) {
        if (!line.trim()) continue;
        const event = JSON.parse(line);

        switch (event.type) {
            case "system":
                if (event.subtype === "init") {
                    sessionId = event.session_id;
                }
                break;
            case "text":
                summary += event.content;
                break;
            case "step_finish":
                usage.inputTokens += event.usage?.input_tokens || 0;
                usage.cachedInputTokens += event.usage?.cache_read_tokens || 0;
                usage.outputTokens += event.usage?.output_tokens || 0;
                costUsd += event.cost_usd || 0;
                break;
            case "tool_use":
                if (event.error) {
                    errorMessage = event.error;
                }
                break;
            case "error":
                errorMessage = event.message;
                break;
            case "result":
                if (event.usage) {
                    // Final usage override
                }
                if (event.cost_usd !== undefined) {
                    costUsd = event.cost_usd;
                }
                break;
        }
    }

    return { sessionId, summary, usage, costUsd, errorMessage };
}
```

## 9. Model Discovery

Paperclip's `opencode_local` adapter runs `opencode models` to discover available models. kn-code provides the same:

```bash
kn-code models
```

Output format (one model per line):
```
anthropic/claude-sonnet-4-5
anthropic/claude-opus-4-5
anthropic/claude-haiku-4-5
github_copilot/gpt-4o
github_copilot/claude-sonnet-4
openai/gpt-4o
openai/o1
```

This allows Paperclip's model discovery to work without modification.

## 10. Test Environment Probe

Paperclip probes the agent environment with a "hello" test. kn-code must handle:

```bash
kn-code run --format json --permission-mode auto "Say hello and list the current directory"
```

Expected output:
- Clean JSONL with `system/init`, `text`, `tool_use`, `tool_result`, and `result` events
- Exit code 0
- No stderr noise

## 11. Differences from opencode_local

| Aspect | opencode_local | kn-code (this spec) |
|--------|---------------|---------------------|
| Command | `opencode run --format json` | `kn-code run --format json` |
| Permission bypass | Runtime config injection | `--permission-mode auto` |
| Config isolation | Temp XDG config dir | `KN_CODE_DISABLE_PROJECT_CONFIG` |
| Skills | `~/.claude/skills/` | `~/.claude/skills/` (same) |
| Model format | `provider/model` | `provider/model` (same) |
| Output format | JSONL (text, step_finish, tool_use, error, result) | Same JSONL format |
| Session resume | `--session <id>` | `--session <id>` (same) |
| Variant/effort | `--variant` | `--variant` (same) |
| Quota support | No | No (delegated to provider) |
| Max turns | No | `--max-turns` (new) |

## 12. docker-compose Integration

When running kn-code in Paperclip's Docker deployment:

```yaml
services:
  paperclip:
    environment:
      - KN_CODE_PATH=/usr/local/bin/kn-code
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
```

The kn-code binary is pre-installed in the Paperclip Docker image (alongside Claude Code, Codex, OpenCode).

## 13. Migration Path

To switch from `opencode_local` to `kn_code_local` in Paperclip:

1. Install kn-code binary in the deployment
2. Register `kn_code_local` adapter in Paperclip's registry
3. Update agent config: `"adapterType": "kn_code_local"`
4. No other changes needed — same CLI interface, same output format
