use crate::anthropic::AnthropicProvider;
use crate::github_copilot::GitHubCopilotProvider;
use crate::openai::OpenAIProvider;
use crate::traits::{ModelInfo, Provider};
use std::sync::Arc;

pub fn resolve_provider(model: &str) -> (Arc<dyn Provider>, Option<ModelInfo>) {
    let (prefix, model_id) = if let Some((p, m)) = model.split_once('/') {
        (p, m)
    } else {
        ("anthropic", model)
    };

    match prefix {
        "anthropic" => {
            let provider = Arc::new(AnthropicProvider::default());
            let model_info = provider
                .list_models_sync()
                .into_iter()
                .find(|m| m.id == model_id);
            (provider, model_info)
        }
        "openai" => {
            let provider = Arc::new(OpenAIProvider::default());
            let model_info = provider
                .list_models_sync()
                .into_iter()
                .find(|m| m.id == model_id);
            (provider, model_info)
        }
        "github_copilot" => {
            let provider = Arc::new(GitHubCopilotProvider::default());
            let model_info = provider
                .list_models_sync()
                .into_iter()
                .find(|m| m.id == model_id);
            (provider, model_info)
        }
        _ => {
            let provider = Arc::new(AnthropicProvider::default());
            let model_info = None;
            (provider, model_info)
        }
    }
}
