use axum::Json;
use axum::extract::State;
use kn_code_auth::{FileTokenStore, TokenStore};
use kn_code_providers::anthropic::AnthropicProvider;
use kn_code_providers::openai::OpenAIProvider;
use kn_code_providers::traits::Provider;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub context_window: usize,
}

pub struct ModelsState {
    pub token_store: Arc<FileTokenStore>,
}

pub async fn list_models(state: State<Arc<ModelsState>>) -> Json<Vec<ModelInfo>> {
    let store = &state.0.token_store;

    let anthropic_creds = store.load("anthropic").await.ok().flatten();
    let openai_creds = store.load("openai").await.ok().flatten();

    let mut all_models = Vec::new();

    let anthropic = AnthropicProvider::default();
    if let Some(creds) = &anthropic_creds {
        if let Ok(models) = anthropic.list_models(creds).await {
            for m in models {
                all_models.push(ModelInfo {
                    id: format!("{}/{}", m.provider, m.id),
                    provider: m.provider,
                    name: m.name,
                    context_window: m.context_window,
                });
            }
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
