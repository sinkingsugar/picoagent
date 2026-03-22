//! The agent loop — the kernel of picoagent.
//!
//! Inspired by pi-mono's clean 349-line agent loop.
//! Synchronous, blocking, single-threaded. Perfect for ESP32.

use crate::agent::session::Session;
use crate::agent::types::{ContentBlock, Message};
use crate::config;
use crate::llm::{CompletionRequest, LlmProvider};
use crate::tools::ToolRegistry;
use anyhow::{bail, Result};
use log::{debug, info, warn};

/// Result of running the agent loop for one user message.
pub struct AgentResponse {
    pub text: String,
    pub tool_rounds: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Run the agent loop for a single user message.
///
/// 1. Push user message to session
/// 2. Call LLM with tools
/// 3. If LLM returns tool_use, execute and feed results back
/// 4. Repeat until end_turn or max iterations
/// 5. Compact if needed
/// 6. Return final text
pub fn run(
    message: &str,
    session: &mut Session,
    tools: &mut ToolRegistry,
    llm: &dyn LlmProvider,
) -> Result<AgentResponse> {
    session.push_user(message);

    let tool_defs = tools.tool_definitions();
    let mut tool_rounds: u32 = 0;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;

    loop {
        let system = session.system_prompt();
        let request = CompletionRequest {
            system: &system,
            messages: session.messages(),
            tools: &tool_defs,
            max_tokens: config::MAX_RESPONSE_TOKENS,
        };

        debug!("Calling LLM (round {})", tool_rounds + 1);
        let response = llm.complete(&request)?;

        total_input += response.usage.input_tokens as u64;
        total_output += response.usage.output_tokens as u64;

        // Push full assistant response
        session.push_assistant(Message::assistant(response.content.clone()));

        // Check for tool calls
        let tool_uses: Vec<_> = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some((id, name, input)),
                _ => None,
            })
            .collect();

        if tool_uses.is_empty() {
            // Done — extract text
            let text = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            if session.needs_compaction() {
                info!("Session needs compaction");
                if let Err(e) = session.compact(llm) {
                    warn!("Compaction failed: {e}");
                }
            }

            return Ok(AgentResponse {
                text,
                tool_rounds,
                input_tokens: total_input,
                output_tokens: total_output,
            });
        }

        // Execute tools
        tool_rounds += 1;
        info!("Tool round {}: {} call(s)", tool_rounds, tool_uses.len());

        let mut results = Vec::new();
        for (id, name, input) in &tool_uses {
            debug!("Executing: {name}");
            let result = match tools.execute(name, (*input).clone()) {
                Ok(out) => {
                    debug!("{name} -> {} bytes", out.content.len());
                    ContentBlock::tool_result(id.as_str(), out.content, !out.success)
                }
                Err(e) => {
                    warn!("{name} error: {e}");
                    ContentBlock::tool_result(id.as_str(), format!("Error: {e}"), true)
                }
            };
            results.push(result);
        }

        session.push_tool_results(results);

        if tool_rounds >= config::MAX_TOOL_ITERATIONS {
            warn!("Max tool iterations ({})", config::MAX_TOOL_ITERATIONS);
            // Push a synthetic assistant message so the session doesn't end
            // with dangling tool_result messages (API requires assistant after tool_result)
            session.push_assistant(Message::assistant_text(
                "I've reached the maximum number of tool iterations for this request."
            ));
            bail!("exceeded maximum tool iterations");
        }
    }
}
