use std::env;

pub struct EnvConfig;

impl EnvConfig {
    pub fn get(key: &str) -> Option<String> {
        env::var(key).ok()
    }

    pub fn get_or(key: &str, default: &str) -> String {
        env::var(key).unwrap_or_else(|_| default.to_string())
    }

    pub fn paperclip_run_id() -> Option<String> {
        env::var("PAPERCLIP_RUN_ID")
            .or_else(|_| env::var("KN_CODE_RUN_ID"))
            .ok()
    }

    pub fn paperclip_task_id() -> Option<String> {
        env::var("PAPERCLIP_TASK_ID")
            .or_else(|_| env::var("KN_CODE_TASK_ID"))
            .ok()
    }

    pub fn paperclip_workspace() -> Option<String> {
        env::var("PAPERCLIP_WORKSPACE_PATH")
            .or_else(|_| env::var("KN_CODE_WORKSPACE"))
            .ok()
    }

    pub fn paperclip_company_id() -> Option<String> {
        env::var("PAPERCLIP_COMPANY_ID")
            .or_else(|_| env::var("KN_CODE_COMPANY_ID"))
            .ok()
    }

    pub fn paperclip_agent_id() -> Option<String> {
        env::var("PAPERCLIP_AGENT_ID")
            .or_else(|_| env::var("KN_CODE_AGENT_ID"))
            .ok()
    }

    pub fn paperclip_auth_token() -> Option<String> {
        env::var("PAPERCLIP_API_KEY")
            .or_else(|_| env::var("KN_CODE_AUTH_TOKEN"))
            .ok()
    }

    pub fn disable_project_config() -> bool {
        env::var("OPENCODE_DISABLE_PROJECT_CONFIG").is_ok()
            || env::var("KN_CODE_DISABLE_PROJECT_CONFIG").is_ok()
    }

    pub fn permission_mode() -> Option<String> {
        env::var("KN_CODE_PERMISSION_MODE").ok()
    }

    pub fn server_host() -> String {
        Self::get_or("KN_CODE_SERVER_HOST", "127.0.0.1")
    }

    pub fn server_port() -> u16 {
        Self::get_or("KN_CODE_SERVER_PORT", "3200")
            .parse()
            .unwrap_or(3200)
    }

    pub fn default_model() -> Option<String> {
        env::var("KN_CODE_DEFAULT_MODEL").ok()
    }

    pub fn auth_method() -> Option<String> {
        env::var("KN_CODE_AUTH_METHOD").ok()
    }
}
