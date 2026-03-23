# picoagent

A minimal AI agent runtime for ESP32-S3 microcontrollers. Talk to your hardware through Telegram, powered by Claude.

```
User ↔ Telegram ↔ Agent Loop ↔ Claude API
                      ↕
                 Tool Registry
                  ↕       ↕
             Hardware   Flash Storage
```

## What it does

picoagent turns an ESP32-S3 into an AI-powered device controller. You message it on Telegram, Claude figures out what tools to call, the ESP32 executes them, and you get the result back. Session history persists across reboots via SPIFFS.

Think of it as a tiny AI-OS where the `Tool` trait is the syscall interface.

## Requirements

- [Rust (esp channel)](https://github.com/esp-rs/rust-build) — `rustup toolchain install esp`
- [espflash](https://github.com/esp-rs/espflash) — `cargo install espflash`
- [just](https://github.com/casey/just) — `cargo install just`
- A Freenove ESP32-S3 WROOM (or compatible, 8MB flash)
- A Telegram bot token (from [@BotFather](https://t.me/botfather))
- A Claude API key or OAuth token

## Quick Start

```bash
# Clone and configure
cp .env.example .env
# Edit .env with your credentials

# Build and flash
just flash-monitor
```

## Configuration

All config is compile-time via `.env`:

```env
WIFI_SSID="your_network"
WIFI_PASS="your_password"
TELEGRAM_BOT_TOKEN="123456:ABC..."
TELEGRAM_CHAT_ID="your_chat_id"
CLAUDE_API_KEY="sk-ant-..."

# Optional
CLAUDE_MODEL="claude-sonnet-4-20250514"
DEVICE_LABEL="Grow Room Controller"
```

Both standard API keys (`sk-ant-api...`) and OAuth tokens (`sk-ant-oat...`) are supported. OAuth tokens are auto-detected and use the appropriate auth headers.

## Adding Tools

The `Tool` trait is the only API you need:

```rust
use crate::tools::{Tool, ToolOutput};

struct TemperatureTool { sensor: Bme680 }

impl Tool for TemperatureTool {
    fn name(&self) -> &'static str { "read_temperature" }
    fn description(&self) -> &'static str { "Read ambient temperature in Celsius" }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    fn execute(&mut self, _params: serde_json::Value) -> anyhow::Result<ToolOutput> {
        let temp = self.sensor.read()?;
        Ok(ToolOutput::ok(format!("{:.1}°C", temp)))
    }
}
```

Register it in `main.rs`:

```rust
tools.register(TemperatureTool::new(sensor));
```

Claude will automatically see the tool and invoke it when relevant.

## Built-in Tools

| Tool | Description |
|------|-------------|
| `gpio` | Digital pin control — set outputs, read inputs |
| `system_info` | Device status — free heap, uptime, firmware version |

## Telegram Commands

| Command | Description |
|---------|-------------|
| `/start` | Show device info |
| `/clear` | Wipe conversation history |
| `/status` | Memory, session stats, tool call count |

## Build Recipes

```bash
just build          # Compile firmware
just flash          # Flash to device
just flash-monitor  # Flash + open serial monitor
just monitor        # Serial monitor only
just erase          # Erase flash (reset storage)
just size           # Show binary size
just clean          # Clean build artifacts
```

## Architecture

- **Synchronous, single-threaded.** No async runtime. The main loop polls Telegram, runs the agent, sends the response. Simple.
- **Session compaction.** Conversation history is bounded (40 messages / 32KB). When exceeded, older messages are summarized by Claude and folded into the system prompt.
- **SPIFFS persistence.** Session survives reboots. ~5MB storage partition on flash.
- **Dual Claude auth.** Standard API keys and OAuth tokens (Claude Code compatible). Auto-detected from the key prefix.

## Spore — Embedded VM

picoagent includes **spore-core**, a tiny stack-based VM for AI-generated embedded programs. Claude generates Spore token streams that run directly on the ESP32 — cooperative multitasking, GPIO/I2C/SPI/BLE/MQTT, event-driven tasks, all in `#![no_std]` with zero allocations.

```
DEF read_sensor
  LIT 0x76 I2C_ADDR BME_READ
END

TASK monitor
  EVERY 30000
    read_sensor
    DUP FLIT 35.0 GT IF
      STR "alert/temp" SWAP F>STR MQTT_PUB
    THEN
  ENDEVERY
ENDTASK
```

A spore is a program — a self-contained capsule of instructions that lands on a microcontroller and starts doing things. The name fits: biological, microscopic, produced and dispersed by AI rather than hand-crafted.

See `spore-core/CLAUDE.md` for the full architecture.

## Flash Layout

3MB for the app, ~5MB for SPIFFS storage. Binary is ~1.2MB after release optimizations.

## License

MIT
