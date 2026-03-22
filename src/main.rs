//! picoagent — AI-OS for ESP32
//!
//! A minimal agent runtime that turns an ESP32-S3 into an AI-powered
//! device controller. Telegram in, tool calls out, Claude in the middle.
//!
//! # Architecture
//!
//! ```text
//! User ↔ Telegram ↔ Agent Loop ↔ Claude API
//!                       ↕
//!                  Tool Registry
//!                   ↕       ↕
//!              Hardware   Storage
//! ```
//!
//! # Extending
//!
//! Implement the `Tool` trait and register it in `setup_tools()`.
//! That's the whole API.

mod agent;
mod config;
mod llm;
mod net;
mod storage;
mod telegram;
mod tools;

use agent::session::{Session, SessionConfig};
use llm::claude::ClaudeClient;
use storage::spiffs::SpiffsStorage;
use telegram::polling::TelegramClient;
use tools::examples::gpio::GpioTool;
use tools::examples::info::InfoTool;
use tools::ToolRegistry;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::link_patches;
use anyhow::Context;
use log::{error, info, warn};
use std::thread;
use std::time::Duration;

const FIRMWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("==============================");
    info!("picoagent v{}", FIRMWARE_VERSION);
    info!("AI-OS for ESP32");
    info!("==============================");

    if let Err(e) = run() {
        error!("Fatal error: {:?}", e);
        error!("Rebooting in 5 seconds...");
        thread::sleep(Duration::from_secs(5));
        esp_idf_svc::hal::reset::restart();
    }
}

fn run() -> anyhow::Result<()> {
    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs_partition = EspDefaultNvsPartition::take()?;

    // Connect WiFi
    info!("Connecting to WiFi...");
    let mut wifi = net::wifi::WifiManager::new(
        peripherals.modem,
        sys_loop,
        Some(nvs_partition),
    )?;
    wifi.connect(config::WIFI_SSID, config::WIFI_PASSWORD)?;

    // Mount flash storage
    let flash = SpiffsStorage::mount()?;

    // Set up tools
    let mut tools = ToolRegistry::new();

    let gpio_tool = GpioTool::new();
    // Uncomment and adjust for your board:
    // gpio_tool.add_output("led", peripherals.pins.gpio2.into())?;
    // gpio_tool.add_output("relay1", peripherals.pins.gpio4.into())?;
    tools.register(gpio_tool);

    let info_tool = InfoTool::new();
    tools.register(info_tool);

    info!("Tools registered: {:?}", tools.tool_names());

    // Load or create session
    let session_config = SessionConfig::default();
    let mut session = Session::load_or_new(&flash, session_config);

    // Set up Claude client
    let claude = match config::CLAUDE_MODEL {
        Some(model) => ClaudeClient::new(config::CLAUDE_API_KEY).with_model(model),
        None => ClaudeClient::new(config::CLAUDE_API_KEY),
    };

    // Set up Telegram client
    let chat_id: i64 = config::TELEGRAM_CHAT_ID
        .parse()
        .context("TELEGRAM_CHAT_ID must be a valid number")?;

    let mut telegram = TelegramClient::new(config::TELEGRAM_BOT_TOKEN, chat_id);

    // Send boot message
    info!("Sending boot notification...");
    if let Err(e) = telegram.send(chat_id, &format!(
        "{} online\npicoagent v{}\n{} tools loaded",
        config::DEVICE_LABEL, FIRMWARE_VERSION, tools.len()
    )) {
        error!("Failed to send boot message: {:?}", e);
    }

    info!("picoagent ready — polling Telegram");

    // Main loop
    loop {
        // Ensure WiFi
        if let Err(e) = wifi.ensure_connected() {
            error!("WiFi reconnect failed: {:?}", e);
            thread::sleep(Duration::from_secs(60));
            continue;
        }

        // Poll Telegram
        match telegram.poll() {
            Ok(Some(msg)) => {
                info!("Message from {}: {}", msg.from_name, msg.text);

                // Typing indicator
                let _ = telegram.send_typing(msg.chat_id);

                // Handle slash commands
                if let Some(response) = handle_command(&msg.text, &mut session, &flash) {
                    if let Err(e) = telegram.send(msg.chat_id, &response) {
                        error!("Failed to send response: {:?}", e);
                    }
                    continue;
                }

                // Run agent
                match agent::loop_::run(&msg.text, &mut session, &mut tools, &claude) {
                    Ok(response) => {
                        info!(
                            "Response: {} chars, {} tool rounds, {}/{} tokens",
                            response.text.len(),
                            response.tool_rounds,
                            response.input_tokens,
                            response.output_tokens,
                        );

                        if let Err(e) = telegram.send(msg.chat_id, &response.text) {
                            error!("Failed to send response: {:?}", e);
                        }

                        // Persist session
                        if let Err(e) = session.save(&flash) {
                            warn!("Failed to save session: {:?}", e);
                        }
                    }
                    Err(e) => {
                        error!("Agent error: {:?}", e);
                        let _ = telegram.send(msg.chat_id, &format!("Error: {e}"));
                    }
                }
            }
            Ok(None) => {
                // Normal — long poll timed out with no messages
            }
            Err(e) => {
                error!("Telegram poll error: {:?}", e);
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

/// Handle slash commands before they reach the agent.
fn handle_command(text: &str, session: &mut Session, storage: &SpiffsStorage) -> Option<String> {
    let text = text.trim();

    match text {
        "/start" => Some(format!(
            "picoagent v{}\n{}\nSay anything to talk to the AI agent.",
            FIRMWARE_VERSION, config::DEVICE_LABEL
        )),
        "/clear" => {
            session.clear();
            let _ = session.save(storage);
            Some("Session cleared.".into())
        }
        "/status" => {
            let stats = session.stats();
            let free_heap = unsafe { esp_idf_svc::sys::esp_get_free_heap_size() };
            Some(format!(
                "Device: {}\n\
                 Free heap: {} KB\n\
                 Session: {} messages, {} KB\n\
                 Summary: {} bytes\n\
                 Tool calls: {}",
                config::DEVICE_LABEL,
                free_heap / 1024,
                stats.message_count,
                stats.content_bytes / 1024,
                stats.summary_bytes,
                stats.total_tool_calls,
            ))
        }
        _ if text.starts_with('/') => {
            Some(format!("Unknown command: {text}\nAvailable: /start, /clear, /status"))
        }
        _ => None,
    }
}
