use axum::Json;
use kn_code_auth::{FileTokenStore, TokenStore};
use kn_code_config::Settings;
use kn_code_providers::anthropic::AnthropicProvider;
use kn_code_providers::openai::OpenAIProvider;
use kn_code_providers::traits::Provider;
use serde::Serialize;

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub context_window: usize,
}

pub async fn list_models() -> Json<Vec<ModelInfo>> {
    let token_store = Settings::config_dir().join("tokens.enc");
    let store = FileTokenStore::new(token_store);

    let anthropic_creds = store.load("anthropic").await.ok().flatten();
    let openai_creds = store.load("openai").await.ok().flatten();

    let mut all_models = Vec::new();

    let anthropic = AnthropicProvider::default();
    if let Some(creds) = &anthropic_creds
        && let Ok(models) = anthropic.list_models(creds).await
    {
        for m in models {
            all_models.push(ModelInfo {
                id: format!("{}/{}", m.provider, m.id),
                provider: m.provider,
                name: m.name,
                context_window: m.context_window,
            });
        }
    }
    if anthropic_creds.is_none() {
        for model_id in &["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"] {
            all_models.push(ModelInfo {
                id: format!("anthropic/{}", model_id),
                provider: "anthropic".to_string(),
                name: model_id.to_string(),
                context_window: 200_000,
            });
        }
    }

    let openai = OpenAIProvider::default();
    if let Some(creds) = &openai_creds
        && let Ok(models) = openai.list_models(creds).await
    {
        for m in models {
            all_models.push(ModelInfo {
                id: format!("{}/{}", m.provider, m.id),
                provider: m.provider,
                name: m.name,
                context_window: m.context_window,
            });
        }
    }
    if openai_creds.is_none() {
        for model_id in &["gpt-4o", "o1"] {
            all_models.push(ModelInfo {
                id: format!("openai/{}", model_id),
                provider: "openai".to_string(),
                name: model_id.to_string(),
                context_window: 128_000,
            });
        }
    }

    Json(all_models)
}
