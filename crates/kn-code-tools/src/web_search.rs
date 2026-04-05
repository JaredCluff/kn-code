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
#[allow(dead_code)]
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

        Err(ToolError::ExecutionFailed(format!(
            "Web search for '{}' — search API integration is not yet configured",
            parsed.query
        )))
    }
}
