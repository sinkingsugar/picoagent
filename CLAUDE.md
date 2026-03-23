# picoagent — Claude Code Project Guide

## What is this?

An AI-OS for ESP32-S3 microcontrollers. Telegram bot → Claude API → tool execution on hardware. Single-threaded, synchronous, blocking — no async runtime. Written in Rust targeting `xtensa-esp32s3-espidf` via esp-rs (std, not no_std).

## Hardware

- **Board:** Freenove ESP32-S3 WROOM Lite (8MB flash)
- **Toolchain:** `channel = "esp"` (Xtensa Rust)
- **ESP-IDF:** v5.3.3

## Build & Deploy

Requires `.env` file (see `.env.example`). Always build via justfile — `env!()` macros need the process env vars that `set dotenv-load` provides.

```bash
just build          # compile
just flash          # flash to device
just flash-monitor  # flash + serial monitor
just monitor        # serial monitor only
just erase          # erase flash (reset SPIFFS)
```

First build on a clean tree runs twice — the justfile handles the partition table copy to OUT_DIR automatically.

## Architecture

```
User ↔ Telegram (long poll) ↔ Agent Loop ↔ Claude API
                                  ↕
                             Tool Registry
                              ↕       ↕
                         Hardware   SPIFFS
```

- **Agent loop** (`src/agent/loop_.rs`): The kernel. Push user msg → call LLM → execute tool calls → loop until no tool calls or max iterations (15). ~125 lines.
- **Session** (`src/agent/session.rs`): Conversation history with compaction. Persists to SPIFFS as `session.json`. Compaction summarizes old messages via LLM, keeps last 4.
- **Tools** (`src/tools/mod.rs`): The `Tool` trait is the primary extension point. Implement it, register in `main()`. That's the whole API.
- **Claude client** (`src/llm/claude.rs`): Dual auth — standard API key (`x-api-key`) or OAuth token (`sk-ant-oat*` → `Authorization: Bearer` + beta headers + Claude Code trick).
- **Telegram** (`src/telegram/polling.rs`): Long-poll `getUpdates`, single-user auth via chat_id, message chunking at 4096 chars.
- **HTTP** (`src/net/http.rs`): Raw `EspHttpConnection` with `initiate_request`/`initiate_response`. No external HTTP crate. Response body capped at 128KB to prevent OOM.
- **Storage** (`src/storage/spiffs.rs`): SPIFFS on flash, auto-formats on first mount. Key-value via `Storage` trait. Keys validated against path traversal (`..`, leading `/`).
- **WDT** (`src/wdt.rs`): Main task subscribes to TWDT, deregisters around long operations (Telegram poll, LLM calls). Idle tasks not monitored.
- **WiFi** (`src/net/wifi.rs`): Connect + reconnect with poll-based timeout. Pattern proven from bme680-monitor.

## Key Design Decisions

- **Synchronous everything.** Single thread, no async. ESP32 doesn't need it and it saves flash/RAM.
- **Compile-time config.** `.env` → `env!()` macros. No runtime config parsing. Change config = rebuild + reflash.
- **Tool trait as syscall interface.** All extensibility goes through `Tool`. The agent can only interact with hardware through registered tools.
- **Session compaction.** When history exceeds 40 messages or 32KB, old messages are summarized by Claude and replaced with a summary in the system prompt. This keeps SPIFFS usage bounded.
- **OAuth "Claude Code trick."** When using `sk-ant-oat*` tokens, the system prompt is sent as two content blocks — first block is `"You are Claude Code, Anthropic's official CLI for Claude."` with `cache_control: {type: "ephemeral"}`. Required for OAuth to work. See edge-talk's `prompter-antro.shs` for reference.

## Extending

1. Create a new file in `src/tools/examples/` (or anywhere)
2. Implement the `Tool` trait:
   ```rust
   impl Tool for MyTool {
       fn name(&self) -> &'static str { "my_tool" }
       fn description(&self) -> &'static str { "What it does" }
       fn parameters_schema(&self) -> Value { json!({...}) }
       fn execute(&mut self, params: Value) -> Result<ToolOutput> { ... }
   }
   ```
3. Register in `main.rs`: `tools.register(MyTool::new());`
4. Claude will see it in the tool definitions and invoke it when appropriate.

## Crate Versions (pinned to match bme680-monitor)

- `esp-idf-svc` 0.51 (features: experimental)
- `esp-idf-hal` 0.45
- `embedded-hal` 1.0
- `heapless` 0.8 (not 0.9 — API incompatibility)
- `serde` 1 (no_default_features + derive + alloc)
- `serde_json` 1 (no_default_features + alloc)
- `embuild` 0.33 (build dep)

## Flash Layout

| Partition | Type   | Offset     | Size  |
|-----------|--------|------------|-------|
| nvs       | data   | 0x9000     | 24KB  |
| phy_init  | data   | 0xF000     | 4KB   |
| factory   | app    | 0x10000    | 3MB   |
| storage   | spiffs | 0x310000   | ~5MB  |

## Telegram Commands

- `/start` — boot info
- `/clear` — wipe session
- `/status` — heap, session stats, tool call count
- Anything else → routed to agent loop

## Known Constraints

- WDT timeout is 60s (ESP-IDF v5.3.3 max). LLM calls can take up to 90s, so the main task is deregistered from TWDT around long operations and re-registered after. Idle tasks are not monitored (main task starves CPU0's idle during blocking reads). See `src/wdt.rs`.
- SPIFFS is slow and has limited write cycles. Session saves happen after each agent response.
- Binary must fit in 3MB (factory partition). Currently ~1.2MB with release optimizations.
- No OTA yet — reflash via USB.
