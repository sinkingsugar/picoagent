//! Telegram Bot API types.
//!
//! Minimal subset — only what we need for text messaging.
//! Add more as needed (photos, documents, inline keyboards, etc.)

use serde::Deserialize;

/// Response wrapper from the Telegram Bot API.
#[derive(Debug, Deserialize)]
pub struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
}

/// An incoming update from getUpdates.
#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

/// A Telegram message.
#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub chat: Chat,
    pub from: Option<User>,
    pub text: Option<String>,
    pub date: i64,
}

/// A Telegram chat.
#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
}

/// A Telegram user.
#[derive(Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

/// Parsed result from polling.
pub struct IncomingMessage {
    pub chat_id: i64,
    pub text: String,
    pub from_name: String,
    pub message_id: i64,
}

impl IncomingMessage {
    pub fn from_update(update: &Update) -> Option<Self> {
        let msg = update.message.as_ref()?;
        let text = msg.text.as_ref()?;

        let from_name = msg
            .from
            .as_ref()
            .map(|u| {
                let mut name = u.first_name.clone();
                if let Some(ref last) = u.last_name {
                    name.push(' ');
                    name.push_str(last);
                }
                name
            })
            .unwrap_or_else(|| String::from("Unknown"));

        Some(Self {
            chat_id: msg.chat.id,
            text: text.clone(),
            from_name,
            message_id: msg.message_id,
        })
    }
}
