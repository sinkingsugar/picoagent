/// Configuration for picoagent — loaded from .env at compile time.

// WiFi credentials
pub const WIFI_SSID: &str = env!("WIFI_SSID");
pub const WIFI_PASSWORD: &str = env!("WIFI_PASS");

// Telegram Bot
pub const TELEGRAM_BOT_TOKEN: &str = env!("TELEGRAM_BOT_TOKEN");
pub const TELEGRAM_CHAT_ID: &str = env!("TELEGRAM_CHAT_ID");

// Claude API
pub const CLAUDE_API_KEY: &str = env!("CLAUDE_API_KEY");

// Optional: override model (default: claude-sonnet-4-6)
// Set CLAUDE_MODEL in .env to override.
pub const CLAUDE_MODEL: Option<&str> = option_env!("CLAUDE_MODEL");

// Device label for the system prompt
pub const DEVICE_LABEL: &str = match option_env!("DEVICE_LABEL") {
    Some(v) => v,
    None => "ESP32-S3 Agent",
};

// Agent limits
pub const MAX_TOOL_ITERATIONS: u32 = 15;
pub const MAX_RESPONSE_TOKENS: u32 = 2048;

// Spore language reference in system prompt (set SPORE_PROMPT=0 to disable)
pub const SPORE_PROMPT_ENABLED: bool = {
    match option_env!("SPORE_PROMPT") {
        Some(v) => !v.is_empty() && v.as_bytes()[0] != b'0',
        None => true,
    }
};
