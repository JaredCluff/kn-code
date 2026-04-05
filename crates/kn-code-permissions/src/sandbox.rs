use std::path::Path;

#[derive(Debug)]
pub enum SandboxType {
    None,
    Seatbelt,
    Firejail,
}

impl SandboxType {
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") {
            SandboxType::Seatbelt
        } else if Path::new("/usr/bin/firejail").exists() {
            SandboxType::Firejail
        } else {
            SandboxType::None
        }
    }

    pub fn sandbox_args(&self, command: &str) -> Vec<String> {
        match self {
            SandboxType::Seatbelt => {
                let cwd =
                    std::env::current_dir().unwrap_or_else(|_| Path::new("/tmp").to_path_buf());
                let cwd_str = cwd.to_string_lossy().replace('"', "\\\"");
                vec![
                    "sandbox-exec".to_string(),
                    "-f".to_string(),
                    format!(
                        "(version 1)\n\
                         (deny default)\n\
                         (allow process-exec)\n\
                         (allow file-read*)\n\
                         (allow file-write*\n  (subpath \"{}\")\n  (subpath \"/tmp\")\n  (subpath \"/var/tmp\")\n)\n\
                         (allow sysctl-read)\n\
                         (allow network-outbound\n  (remote tcp \"*.80\")\n  (remote tcp \"*.443\")\n)",
                        cwd_str
                    ),
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    command.to_string(),
                ]
            }
            SandboxType::Firejail => {
                let cwd =
                    std::env::current_dir().unwrap_or_else(|_| Path::new("/tmp").to_path_buf());
                let cwd_str = cwd.to_string_lossy();
                vec![
                    "firejail".to_string(),
                    "--quiet".to_string(),
                    "--noprofile".to_string(),
                    format!("--read-write={}", cwd_str),
                    "--net=none".to_string(),
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    command.to_string(),
                ]
            }
            SandboxType::None => {
                vec!["/bin/sh".to_string(), "-c".to_string(), command.to_string()]
            }
        }
    }
}
