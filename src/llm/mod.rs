//! LLM provider abstraction.

pub mod claude;

use crate::agent::types::{ApiResponse, ContentBlock, Message};
use anyhow::Result;
use serde_json::Value;

/// Configuration for an LLM request.
pub struct CompletionRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a Value,
    pub max_tokens: u32,
}

/// LLM provider interface.
pub trait LlmProvider {
    fn complete(&self, request: &CompletionRequest) -> Result<ApiResponse>;

    /// Summarize a conversation for compaction.
    fn summarize(&self, messages: &[Message], system_context: &str) -> Result<String> {
        let summary_system = format!(
            "You are summarizing a conversation for memory compaction on a constrained device. \
             Preserve: decisions made, current hardware state, active schedules, \
             problems detected, user preferences. Be concise but complete.\n\n\
             Original context: {system_context}"
        );

        // Build messages: include the actual conversation, then a user prompt to summarize
        let mut summary_messages: Vec<Message> = messages.to_vec();
        summary_messages.push(Message::user(
            "Summarize the conversation above. Focus on what matters for continuity: \
             decisions made, current hardware state, active schedules, problems detected, \
             user preferences.",
        ));

        let empty_tools = Value::Array(Vec::new());
        let req = CompletionRequest {
            system: &summary_system,
            messages: &summary_messages,
            tools: &empty_tools,
            max_tokens: 1024,
        };

        let response = self.complete(&req)?;
        Ok(response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""))
    }
}
