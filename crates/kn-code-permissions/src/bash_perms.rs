pub const SAFE_ENV_VARS: &[&str] = &[
    "NODE_ENV",
    "LANG",
    "LC_ALL",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "TZ",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "GOEXPERIMENT",
    "GOOS",
    "GOARCH",
    "CGO_ENABLED",
    "GO111MODULE",
    "PYTHONUNBUFFERED",
    "PYTHONDONTWRITEBYTECODE",
    "CI",
    "CI_MERGE_REQUEST_ID",
    "GITHUB_ACTIONS",
];

pub const SAFE_WRAPPERS: &[&str] = &["timeout", "time", "nice", "stdbuf"];

pub const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "tree",
    "du",
    "stat",
    "file",
    "wc",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "grep",
    "rg",
    "ag",
    "ack",
    "which",
    "whereis",
    "locate",
    "jq",
    "cut",
    "sort",
    "uniq",
    "tr",
    "git status",
    "git log",
    "git diff",
    "git branch",
    "git remote",
    "git tag",
    "git show",
    "git describe",
    "cargo check",
    "cargo doc",
    "cargo metadata",
    "cargo tree",
    "cargo search",
    "npm ls",
    "npm view",
    "npm search",
    "go list",
    "go doc",
    "go version",
    "python -m pydoc",
    "df",
    "free",
    "uptime",
    "uname",
    "ps aux",
    "top -l",
    "whoami",
    "id",
    "echo",
    "printf",
];

pub const DANGEROUS_FLAGS: &[&str] = &["-exec", "-execdir", "-ok", "-delete", "-f", "-force"];

pub fn strip_safe_wrappers(command: &str) -> &str {
    let mut cmd = command.trim();
    loop {
        let found = SAFE_WRAPPERS.iter().find(|&wrapper| {
            cmd.starts_with(wrapper) && cmd[wrapper.len()..].starts_with(char::is_whitespace)
        });
        if let Some(wrapper) = found {
            cmd = cmd[wrapper.len()..].trim_start();
        } else {
            break;
        }
    }
    cmd
}

pub fn strip_env_vars(command: &str) -> String {
    let mut cmd = command.trim().to_string();
    while let Some(eq_pos) = cmd.find('=') {
        if eq_pos == 0 {
            break;
        }
        let before_eq = &cmd[..eq_pos];
        let last_token = before_eq
            .rsplit_once(|c: char| c.is_whitespace())
            .map(|(_, t)| t)
            .unwrap_or(before_eq);

        let is_safe_env = SAFE_ENV_VARS.contains(&last_token);
        if is_safe_env {
            let after_eq = &cmd[eq_pos + 1..];
            let mut rest_start = 0;
            let mut in_quote = false;
            let mut quote_char = '\0';
            let mut escaped = false;
            let chars: Vec<char> = after_eq.chars().collect();
            for (i, &c) in chars.iter().enumerate() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if in_quote {
                    if c == quote_char {
                        in_quote = false;
                    }
                    continue;
                }
                if c == '"' || c == '\'' {
                    in_quote = true;
                    quote_char = c;
                    continue;
                }
                if c.is_whitespace() {
                    rest_start = i + 1;
                    break;
                }
            }
            if rest_start == 0 {
                break;
            }
            let rest: String = chars[rest_start..].iter().collect();
            if rest.trim().is_empty() {
                break;
            }
            cmd = rest;
        } else {
            break;
        }
    }
    cmd
}

fn contains_shell_metacharacters(command: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for c in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }
        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if in_single_quote || in_double_quote {
            continue;
        }
        match c {
            ';' | '|' | '&' | '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>' | '~' | '!' => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn contains_dangerous_flags(command: &str) -> bool {
    let stripped = strip_safe_wrappers(command);
    DANGEROUS_FLAGS.iter().any(|&flag| stripped.contains(flag))
}

pub fn is_read_only_command(command: &str) -> bool {
    let stripped = strip_safe_wrappers(command);

    if contains_shell_metacharacters(stripped) {
        return false;
    }

    if contains_dangerous_flags(stripped) {
        return false;
    }

    READ_ONLY_COMMANDS
        .iter()
        .any(|&cmd| stripped.starts_with(cmd))
}
