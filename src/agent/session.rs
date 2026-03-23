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
#[derive(Clone)]
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
        let mut prompt = format!(
            "You are an AI assistant running on an ESP32-S3 microcontroller ({}). \
             You control hardware and respond to the user via Telegram.\n\n\
             You have access to tools that interact with the physical world. \
             Use them when the user asks you to do something with hardware, \
             check sensors, or manage the system.\n\n\
             Be concise — your responses go through Telegram. \
             Prefer short, actionable answers.",
            crate::config::DEVICE_LABEL
        );

        if crate::config::SPORE_PROMPT_ENABLED {
            prompt.push_str("\n\n");
            prompt.push_str(SPORE_REFERENCE);
        }

        prompt.push_str("\n\n{summary}");

        Self {
            max_messages: 40,
            max_content_bytes: 32_768,
            system_prompt: prompt,
        }
    }
}

/// Spore language reference for the LLM system prompt.
///
/// Designed to be token-efficient while giving the LLM everything it needs
/// to generate correct Spore programs. Disable via SPORE_PROMPT=0 in .env.
const SPORE_REFERENCE: &str = "\
# Spore Language Reference

Spore is a stack-based language (Forth-inspired) you use via the deploy_spore tool. \
All tokens are UPPERCASE and space-delimited. Values are pushed onto a stack; \
operations pop their arguments and push results.

## Types
- Int: LIT <n> (decimal or 0x hex). 32-bit signed.
- Float: FLIT <n> (e.g. FLIT 3.14). 32-bit.
- Bool: TRUE / FALSE
- String: STR \"text\" — pushes a string reference.

## Stack ( before -- after )
DUP (a -- a a) | DROP (a -- ) | SWAP (a b -- b a) | OVER (a b -- a b a)
ROT (a b c -- b c a) | NIP (a b -- b) | TUCK (a b -- b a b)
2DUP (a b -- a b a b) | 2DROP (a b -- ) | DEPTH ( -- n)

## Arithmetic (operands -- result)
ADD SUB MUL DIV MOD — pop two, push result. Float auto-promotes.
ABS NEG — pop one, push result.
MIN MAX — pop two, push lesser/greater.

## Comparison (a b -- bool)
EQ NEQ GT LT GTE LTE — push TRUE/FALSE.

## Logic
AND OR — bitwise on ints, logical on bools. NOT XOR SHL SHR — integer ops.

## Type Conversion
I>F F>I I>STR F>STR — pop one type, push converted.

## Control Flow
IF <true-body> THEN — pops bool, skips body if false.
IF <true-body> ELSE <false-body> THEN — branches.
LOOP <body> ENDLOOP — infinite loop.
BREAK — exit enclosing LOOP.
LIT <n> TIMES <body> ENDTIMES — repeat n times.
BEGIN <body> <condition> UNTIL — loop until condition is true.

## Variables
VAR <name> — declare (before use). STORE <name> — pop and save. FETCH <name> — push value.

## Words (Functions)
DEF <name> <body> END — define a reusable word. Call by name.

## Tasks & Scheduling
TASK <name> <body> ENDTASK — define a task. A task named 'main' auto-starts.
START <name> / STOP <name> — start/stop tasks.
YIELD — cooperatively yield to scheduler. YIELD_FOREVER — suspend until event.
EVERY <ms> <body> ENDEVERY — run body at interval (inside a task with YIELD loop).

## Events
DEF handler <body> END — define handler word first.
ON <event> <handler> — bind event to word. EMIT <event> — fire event.

## GPIO (pin -- )
LIT <pin> LIT <mode> GPIO_MODE — set pin mode (0=input, 1=output, 2=input_pullup, 3=input_pulldown).
LIT <pin> LIT <val> GPIO_WRITE — write 0/1.
LIT <pin> GPIO_READ — (pin -- value) read pin.
LIT <pin> GPIO_TOGGLE — toggle output.
LIT <pin> ADC_READ — (pin -- raw_value) read ADC.

## PWM
LIT <pin> LIT <freq_hz> PWM_INIT — initialize PWM on pin.
LIT <pin> LIT <duty> PWM_DUTY — set duty 0-1023.

## I2C (SDA=8, SCL=9 on this board)
LIT <addr> I2C_ADDR — set slave address (7-bit).
LIT <byte> I2C_WRITE — write one byte.
I2C_READ — ( -- byte) read one byte.
<buf> I2C_WRITE_BUF — write buffer.
LIT <len> I2C_READ_BUF — ( -- buf) read into new buffer.
LIT <addr> I2C_ADDR BME_READ — ( -- temp hum pressure) read BME280 sensor (floats).

## SPI
LIT <clk> LIT <mosi> LIT <miso> LIT <cs> SPI_INIT
<buf_in> SPI_TRANSFER — ( -- buf_out)

## WiFi
STR \"ssid\" STR \"pass\" WIFI_CONNECT | WIFI_STATUS ( -- int) | WIFI_DISCONNECT | WIFI_IP ( -- int)

## BLE
BLE_INIT | STR \"name\" BLE_ADVERTISE | BLE_STOP_ADV
LIT <handle> STR \"data\" BLE_NOTIFY | LIT <handle> BLE_READ ( -- buf)

## MQTT
STR \"broker\" LIT <port> MQTT_INIT
STR \"topic\" STR \"payload\" MQTT_PUB
STR \"topic\" MQTT_SUB | STR \"topic\" MQTT_UNSUB

## System
LIT <ms> DELAY_MS | MILLIS ( -- ms) | LIT <secs> DEEP_SLEEP | REBOOT
STR \"key\" NVS_GET ( -- int) | STR \"key\" LIT <val> NVS_SET
HEAP_FREE ( -- bytes) | STR \"msg\" LOG — print to device log.

## Comments
\\ backslash starts a comment to end of line.

## Examples
Blink LED on pin 2:
TASK main LIT 2 LIT 1 GPIO_MODE LOOP LIT 2 GPIO_TOGGLE LIT 500 DELAY_MS ENDLOOP ENDTASK

Read temperature and log:
LIT 0x76 I2C_ADDR BME_READ DROP DROP F>STR LOG

Conditional with variable:
VAR count LIT 0 STORE count LIT 10 TIMES FETCH count LIT 1 ADD STORE count ENDTIMES FETCH count I>STR LOG";

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
        match Self::load(storage, config.clone()) {
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
                Self::new(config)
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
