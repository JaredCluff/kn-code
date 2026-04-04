pub mod registry;
pub mod traits;

pub mod agent;
pub mod ask_user;
pub mod bash;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp;
pub mod skill;
pub mod todo_write;
pub mod web_fetch;
pub mod web_search;

pub use registry::ToolRegistry;
pub use traits::{Tool, ToolContent, ToolContext, ToolError, ToolResult};
