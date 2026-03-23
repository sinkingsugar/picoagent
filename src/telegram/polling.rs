//! Telegram Bot API client — long polling.
//!
//! Uses the same HTTP pattern as bme680-monitor.
//! Simple, NAT-friendly, no webhook needed.

use crate::net::http;
use crate::telegram::types::{IncomingMessage, TelegramResponse, Update};
use anyhow::{bail, Context, Result};
use log::{debug, warn};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org/bot";

/// Long poll timeout — Telegram holds the connection open this long.
const POLL_TIMEOUT_SECS: u32 = 30;

/// HTTP timeout — must be longer than poll timeout.
const HTTP_TIMEOUT_SECS: u64 = 40;

/// Telegram send timeout.
const SEND_TIMEOUT_SECS: u64 = 15;

/// Telegram polling client.
pub struct TelegramClient {
    bot_token: String,
    /// Only process messages from this chat ID. Single-user auth.
    allowed_chat_id: i64,
    /// Offset for long polling (tracks last processed update).
    offset: i64,
}

impl TelegramClient {
    pub fn new(bot_token: impl Into<String>, allowed_chat_id: i64) -> Self {
        Self {
            bot_token: bot_token.into(),
            allowed_chat_id,
            offset: 0,
        }
    }

    /// Poll for new messages. Blocks up to POLL_TIMEOUT_SECS seconds.
    pub fn poll(&mut self) -> Result<Option<IncomingMessage>> {
        let url = format!(
            "{}{}/getUpdates?offset={}&timeout={}&allowed_updates=[\"message\"]",
            TELEGRAM_API_BASE, self.bot_token, self.offset, POLL_TIMEOUT_SECS
        );

        let (status, body) = http::get(&url, HTTP_TIMEOUT_SECS)?;

        if status == 429 {
            warn!("Telegram rate limited, backing off 10s");
            std::thread::sleep(std::time::Duration::from_secs(10));
            return Ok(None);
        }
        if status >= 500 {
            warn!("Telegram server error ({status}), backing off 5s");
            std::thread::sleep(std::time::Duration::from_secs(5));
            return Ok(None);
        }
        if status != 200 {
            bail!("Telegram getUpdates failed with status {status}");
        }

        let response: TelegramResponse<Vec<Update>> =
            serde_json::from_str(&body).context("failed to parse Telegram response")?;

        if !response.ok {
            let desc = response.description.unwrap_or_default();
            bail!("Telegram API error: {desc}");
        }

        let updates = match response.result {
            Some(u) => u,
            None => return Ok(None),
        };

        for update in &updates {
            // Always advance offset
            self.offset = update.update_id + 1;

            if let Some(msg) = IncomingMessage::from_update(update) {
                if msg.chat_id != self.allowed_chat_id {
                    warn!(
                        "Ignoring message from unauthorized chat {} ({})",
                        msg.chat_id, msg.from_name
                    );
                    continue;
                }

                debug!("Received from {}: {}", msg.from_name, msg.text);
                return Ok(Some(msg));
            }
        }

        Ok(None)
    }

    /// Send a text message. Handles Telegram's 4096-char limit by chunking.
    pub fn send(&self, chat_id: i64, text: &str) -> Result<()> {
        const MAX_MSG_LEN: usize = 4096;

        if text.len() <= MAX_MSG_LEN {
            return self.send_chunk(chat_id, text);
        }

        let mut remaining = text;
        while !remaining.is_empty() {
            let chunk_end = if remaining.len() <= MAX_MSG_LEN {
                remaining.len()
            } else {
                // Find a safe char boundary at or before MAX_MSG_LEN
                let mut boundary = MAX_MSG_LEN;
                while boundary > 0 && !remaining.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                // Try to split at a newline for cleaner chunks
                remaining[..boundary]
                    .rfind('\n')
                    .map(|p| p + 1)
                    .unwrap_or(boundary)
            };

            self.send_chunk(chat_id, &remaining[..chunk_end])?;
            remaining = &remaining[chunk_end..];
        }

        Ok(())
    }

    /// Send typing indicator.
    pub fn send_typing(&self, chat_id: i64) -> Result<()> {
        let url = format!("{}{}/sendChatAction", TELEGRAM_API_BASE, self.bot_token);
        let payload = format!(r#"{{"chat_id":{},"action":"typing"}}"#, chat_id);
        let _ = http::post_json(&url, &payload, &[], SEND_TIMEOUT_SECS);
        Ok(())
    }

    fn send_chunk(&self, chat_id: i64, text: &str) -> Result<()> {
        let url = format!("{}{}/sendMessage", TELEGRAM_API_BASE, self.bot_token);
        let escaped = escape_json(text);
        let payload = format!(
            r#"{{"chat_id":{},"text":"{}","disable_notification":false}}"#,
            chat_id, escaped
        );

        let (status, _response_body) = http::post_json(&url, &payload, &[], SEND_TIMEOUT_SECS)?;

        if !(200..300).contains(&status) {
            // If markdown parsing fails, Telegram returns 400 — retry plain
            if status == 400 {
                debug!("Send failed ({}), retrying without formatting", status);
                let payload_plain = format!(r#"{{"chat_id":{},"text":"{}"}}"#, chat_id, escaped);
                let (status2, _) = http::post_json(&url, &payload_plain, &[], SEND_TIMEOUT_SECS)?;
                if !(200..300).contains(&status2) {
                    bail!("Telegram send failed with status {status2}");
                }
                return Ok(());
            }
            bail!("Telegram send failed with status {status}");
        }

        Ok(())
    }
}

/// Escape special characters for JSON string values.
fn escape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c < '\x20' => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}
