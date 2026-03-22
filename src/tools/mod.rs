//! Tool system — the primary extension point of picoagent.
//!
//! Implement [`Tool`] to teach the agent new capabilities.
//! Register tools with [`ToolRegistry`] at boot.

pub mod examples;

use anyhow::Result;
use serde_json::Value;

/// Output from a tool execution.
pub struct ToolOutput {
    pub content: String,
    pub success: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            success: true,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            success: false,
        }
    }
}

/// A tool that the agent can invoke.
///
/// This is the primary extension point. Implement this trait to give
/// the agent control over hardware, sensors, actuators, or anything.
///
/// # Example
///
/// ```rust
/// struct LedTool { /* gpio handle */ }
///
/// impl Tool for LedTool {
///     fn name(&self) -> &'static str { "led" }
///     fn description(&self) -> &'static str { "Toggle an LED on/off" }
///     fn parameters_schema(&self) -> serde_json::Value { /* ... */ }
///     fn execute(&mut self, params: serde_json::Value) -> Result<ToolOutput> { /* ... */ }
/// }
/// ```
pub trait Tool: Send {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    fn execute(&mut self, params: Value) -> Result<ToolOutput>;
}

/// Registry of all available tools.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.push(Box::new(tool));
    }

    pub fn execute(&mut self, name: &str, params: Value) -> Result<ToolOutput> {
        let tool = self
            .tools
            .iter_mut()
            .find(|t| t.name() == name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
        tool.execute(params)
    }

    /// Generate the tools array for the Claude API.
    pub fn tool_definitions(&self) -> Value {
        let defs: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "input_schema": t.parameters_schema(),
                })
            })
            .collect();
        Value::Array(defs)
    }

    pub fn tool_names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }
}
