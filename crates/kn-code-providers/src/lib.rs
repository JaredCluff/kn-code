pub mod anthropic;
pub mod compatible;
pub mod github_copilot;
pub mod openai;
pub mod traits;

pub use traits::{Provider, ProviderError, ModelInfo};
pub use anthropic::AnthropicProvider;
pub use github_copilot::GitHubCopilotProvider;
pub use openai::OpenAIProvider;
pub use compatible::CompatibleProvider;
