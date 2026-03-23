//! Conversation session management.
//!
//! Handles message history, memory budgeting, and compaction.
//! Persists to flash via the storage trait.

use crate::agent::types::{ContentBlock, Message};
use crate::llm::LlmProvider;
use crate::storage::Storage;
use anyhow::{Context, Result};
use log::{info, warn};

/// How many recent messages to keep after compaction.
const KEEP_AFTER_COMPACT: usize = 4;

/// Session configuration.
pub struct SessionConfig {
    /// Maximum number of messages before triggering compaction.
    pub max_messages: usize,
    /// Maximum total estimated bytes of message content before compaction.
    pub max_content_bytes: usize,
    /// System prompt template.
    /// Use `{summary}` as placeholder for compacted history.
    pub system_prompt: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_messages: 40,
            max_content_bytes: 32_768,
            system_prompt: format!(
                "You are an AI assistant running on an ESP32-S3 microcontroller ({}). \
                 You control hardware and respond to the user via Telegram.\n\n\
                 You have access to tools that interact with the physical world. \
                 Use them when the user asks you to do something with hardware, \
                 check sensors, or manage the system.\n\n\
                 Be concise — your responses go through Telegram. \
                 Prefer short, actionable answers.\n\n\
                 {{summary}}",
                crate::config::DEVICE_LABEL
            ),
        }
    }
}

/// A conversation session with compaction support.
pub struct Session {
    summary: String,
    messages: Vec<Message>,
    config: SessionConfig,
    total_tool_calls: u32,
}

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            summary: String::new(),
            messages: Vec::new(),
            config,
            total_tool_calls: 0,
        }
    }

    /// Load session from storage, or create new if none exists.
    pub fn load_or_new(storage: &dyn Storage, config: SessionConfig) -> Self {
        match Self::load(storage, config) {
            Ok(session) => {
                info!(
                    "Restored session: {} messages, {} bytes summary",
                    session.messages.len(),
                    session.summary.len()
                );
                session
            }
            Err(e) => {
                warn!("No saved session ({}), starting fresh", e);
                Self::new(SessionConfig::default())
            }
        }
    }

    fn load(storage: &dyn Storage, config: SessionConfig) -> Result<Self> {
        let data = storage.read("session.json")?;
        let saved: SavedSession =
            serde_json::from_str(&data).context("failed to parse saved session")?;
        Ok(Self {
            summary: saved.summary,
            messages: saved.messages,
            config,
            total_tool_calls: saved.total_tool_calls,
        })
    }

    /// Persist current session to storage.
    pub fn save(&self, storage: &dyn Storage) -> Result<()> {
        let saved = SavedSession {
            summary: self.summary.clone(),
            messages: self.messages.clone(),
            total_tool_calls: self.total_tool_calls,
        };
        let data = serde_json::to_string(&saved).context("failed to serialize session")?;
        storage.write("session.json", &data)?;
        Ok(())
    }

    pub fn push_user(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    pub fn push_assistant(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn push_tool_results(&mut self, results: Vec<ContentBlock>) {
        let count = results.len() as u32;
        self.messages.push(Message::tool_results(results));
        self.total_tool_calls += count;
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn system_prompt(&self) -> String {
        let summary_block = if self.summary.is_empty() {
            String::new()
        } else {
            format!(
                "Previous conversation summary:\n{}\n\nContinue from where we left off.",
                self.summary
            )
        };
        self.config
            .system_prompt
            .replace("{summary}", &summary_block)
    }

    pub fn needs_compaction(&self) -> bool {
        if self.messages.len() > self.config.max_messages {
            return true;
        }
        let total_bytes: usize = self.messages.iter().map(|m| m.estimated_size()).sum();
        total_bytes > self.config.max_content_bytes
    }

    pub fn compact(&mut self, llm: &dyn LlmProvider) -> Result<()> {
        if self.messages.len() <= KEEP_AFTER_COMPACT {
            return Ok(());
        }

        let split_at = self.messages.len() - KEEP_AFTER_COMPACT;

        info!(
            "Compacting: {} messages -> summarize {} + keep {}",
            self.messages.len(),
            split_at,
            KEEP_AFTER_COMPACT
        );

        let system_context = if self.summary.is_empty() {
            String::from("No prior summary.")
        } else {
            format!("Prior summary: {}", self.summary)
        };

        let new_summary = llm.summarize(&self.messages[..split_at], &system_context)?;

        self.summary = new_summary;
        self.messages = self.messages.split_off(split_at);

        info!(
            "Compaction done: summary {} bytes, {} messages remaining",
            self.summary.len(),
            self.messages.len()
        );

        Ok(())
    }

    pub fn clear(&mut self) {
        self.summary.clear();
        self.messages.clear();
        self.total_tool_calls = 0;
    }

    pub fn stats(&self) -> SessionStats {
        SessionStats {
            message_count: self.messages.len(),
            summary_bytes: self.summary.len(),
            total_tool_calls: self.total_tool_calls,
            content_bytes: self.messages.iter().map(|m| m.estimated_size()).sum(),
        }
    }
}

pub struct SessionStats {
    pub message_count: usize,
    pub summary_bytes: usize,
    pub total_tool_calls: u32,
    pub content_bytes: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSession {
    summary: String,
    messages: Vec<Message>,
    total_tool_calls: u32,
}
