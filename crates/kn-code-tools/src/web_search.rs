use crate::traits::*;
use async_trait::async_trait;
use serde::Deserialize;

#[derive(Debug)]
pub struct WebSearchTool;

impl Default for WebSearchTool {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct WebSearchInput {
    query: String,
    #[serde(default = "default_num_results")]
    num_results: usize,
}

fn default_num_results() -> usize {
    10
}

#[derive(Debug, serde::Serialize)]
#[allow(dead_code)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }
    fn description(&self) -> &str {
        "Search the web"
    }
    fn prompt(&self) -> &str {
        "Use this to search the web for information."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "num_results": { "type": "integer", "description": "Number of results" }
            },
            "required": ["query"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _context: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let parsed: WebSearchInput =
            serde_json::from_value(input.clone()).map_err(|e| ToolError::ValidationFailed {
                message: e.to_string(),
            })?;

        // TODO: Integrate with actual search API (provider's built-in search or external API)
        // For now, return a placeholder indicating search is not yet configured
        Ok(ToolResult {
            content: ToolContent::Text(format!(
                "Web search for '{}' — search integration not yet configured. \
                 Configure a search API key to enable this tool.",
                parsed.query
            )),
            new_messages: Vec::new(),
            persisted: false,
            persisted_path: None,
            structured_content: Some(serde_json::json!({
                "query": parsed.query,
                "num_results": parsed.num_results,
                "status": "not_configured",
            })),
        })
    }
}
