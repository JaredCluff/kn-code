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
            SandboxType::Seatbelt => vec![
                "sandbox-exec".to_string(),
                "-f".to_string(),
                format!(
                    "(version 1)\n(deny default)\n(allow process-exec)\n(allow file-read*)\n(allow file-write*\n  (subpath \"/tmp\")\n  (subpath \"/var/tmp\")\n)\n(allow sysctl-read)\n(allow network-outbound\n  (remote tcp \"*.80\")\n  (remote tcp \"*.443\")\n)"
                ),
                "/bin/sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ],
            SandboxType::Firejail => vec![
                "firejail".to_string(),
                "--quiet".to_string(),
                "--noprofile".to_string(),
                "--private=/tmp".to_string(),
                "--net=none".to_string(),
                "/bin/sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ],
            SandboxType::None => vec!["/bin/sh".to_string(), "-c".to_string(), command.to_string()],
        }
    }
}
