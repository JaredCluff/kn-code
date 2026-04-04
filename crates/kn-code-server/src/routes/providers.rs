use axum::Json;
use kn_code_auth::{Credentials, FileTokenStore, TokenStore};
use kn_code_config::Settings;
use kn_code_providers::anthropic::AnthropicProvider;
use kn_code_providers::openai::OpenAIProvider;
use kn_code_providers::traits::Provider;
use serde::Serialize;

#[derive(Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub auth_methods: Vec<String>,
    pub authenticated: bool,
    pub models: Vec<String>,
}

async fn load_provider_creds(store: &FileTokenStore, provider: &str) -> Option<Credentials> {
    store.load(provider).await.ok().flatten()
}

pub async fn list_providers() -> Json<Vec<ProviderInfo>> {
    let token_store = Settings::config_dir().join("tokens.enc");
    let store = FileTokenStore::new(token_store);

    let anthropic_creds = load_provider_creds(&store, "anthropic").await;
    let openai_creds = load_provider_creds(&store, "openai").await;

    let mut providers = Vec::new();

    let anthropic = AnthropicProvider::default();
    let anthropic_authed = anthropic_creds.is_some();
    let anthropic_models = match &anthropic_creds {
        Some(creds) => anthropic
            .list_models(creds)
            .await
            .map(|m| m.into_iter().map(|mi| mi.id).collect())
            .unwrap_or_else(|_| {
                vec![
                    "claude-sonnet-4-5".to_string(),
                    "claude-opus-4-5".to_string(),
                    "claude-haiku-4-5".to_string(),
                ]
            }),
        None => vec![
            "claude-sonnet-4-5".to_string(),
            "claude-opus-4-5".to_string(),
            "claude-haiku-4-5".to_string(),
        ],
    };
    providers.push(ProviderInfo {
        id: anthropic.id().to_string(),
        name: anthropic.name().to_string(),
        auth_methods: anthropic.auth_methods(),
        authenticated: anthropic_authed,
        models: anthropic_models,
    });

    let openai = OpenAIProvider::default();
    let openai_authed = openai_creds.is_some();
    let openai_models = match &openai_creds {
        Some(creds) => openai
            .list_models(creds)
            .await
            .map(|m| m.into_iter().map(|mi| mi.id).collect())
            .unwrap_or_else(|_| vec!["gpt-4o".to_string(), "o1".to_string()]),
        None => vec!["gpt-4o".to_string(), "o1".to_string()],
    };
    providers.push(ProviderInfo {
        id: openai.id().to_string(),
        name: openai.name().to_string(),
        auth_methods: openai.auth_methods(),
        authenticated: openai_authed,
        models: openai_models,
    });

    Json(providers)
}
