use crate::traits::Tool;
use std::collections::HashMap;

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    aliases: HashMap<String, String>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            tracing::warn!(tool_name = %name, "Tool registration overwriting existing tool");
        }
        for alias in tool.aliases() {
            self.aliases.insert(alias.to_string(), name.clone());
        }
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref()).or_else(|| {
            self.aliases
                .get(name)
                .and_then(|canonical| self.tools.get(canonical))
                .map(|t| t.as_ref())
        })
    }

    pub fn get_all(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).collect()
    }

    pub fn get_enabled(&self) -> Vec<&dyn Tool> {
        self.get_all()
            .into_iter()
            .filter(|t| t.is_enabled())
            .collect()
    }
}
