# picoagent — AI-OS for ESP32
# Usage: just <recipe>

# Load .env file automatically
set dotenv-load
set fallback := true

# Force sdkconfig.defaults and custom partition table
export ESP_IDF_SDKCONFIG_DEFAULTS := justfile_directory() / "sdkconfig.defaults"
export ESP_IDF_PARTITION_TABLE := justfile_directory() / "partitions.csv"

# Default recipe - show help
default:
    @just --list

# ============================================================================
# Build
# ============================================================================

# Build firmware (run twice on clean build to copy partition table)
build:
    #!/usr/bin/env bash
    set -e
    TARGET_DIR=$(cargo metadata --format-version=1 | jq -r '.target_directory')

    # First pass: try to build, which creates the OUT_DIR
    if ! cargo build --release 2>&1; then
        # If failed, copy partitions.csv to the newly created OUT_DIR
        OUT_DIR=$(find "$TARGET_DIR" -path "*/esp-idf-sys-*/out" -type d 2>/dev/null | head -1)
        if [ -n "$OUT_DIR" ]; then
            echo "Copying partitions.csv to $OUT_DIR"
            cp partitions.csv "$OUT_DIR/"
            # Retry build
            cargo build --release
        else
            echo "Build failed and couldn't find OUT_DIR"
            exit 1
        fi
    fi

# ============================================================================
# Flash
# ============================================================================

# Flash firmware
flash: build
    #!/usr/bin/env bash
    set -e
    TARGET_DIR=$(cargo metadata --format-version=1 | jq -r '.target_directory')
    APP="$TARGET_DIR/xtensa-esp32s3-espidf/release/picoagent"
    PORT_ARG="${ESP_PORT:+--port $ESP_PORT}"
    espflash flash --partition-table partitions.csv --ignore_app_descriptor $PORT_ARG "$APP"

# Flash and open monitor
flash-monitor: build
    #!/usr/bin/env bash
    set -e
    TARGET_DIR=$(cargo metadata --format-version=1 | jq -r '.target_directory')
    APP="$TARGET_DIR/xtensa-esp32s3-espidf/release/picoagent"
    PORT_ARG="${ESP_PORT:+--port $ESP_PORT}"
    espflash flash --partition-table partitions.csv --ignore_app_descriptor $PORT_ARG "$APP"
    espflash monitor $PORT_ARG

# ============================================================================
# Monitor & Debug
# ============================================================================

# Open serial monitor
monitor:
    espflash monitor

# Erase flash completely
erase:
    espflash erase-flash

# ============================================================================
# Analysis
# ============================================================================

# Show binary size
size: build
    #!/usr/bin/env bash
    TARGET_DIR=$(cargo metadata --format-version=1 | jq -r '.target_directory')
    ls -lh "$TARGET_DIR/xtensa-esp32s3-espidf/release/picoagent"

# Show cargo tree dependencies
deps:
    cargo tree --depth 1

# ============================================================================
# Code Quality
# ============================================================================

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt -- --check

# Run clippy lints
clippy:
    cargo clippy --all-targets -- -D warnings

# Check without building
check:
    cargo check --release

# ============================================================================
# Clean
# ============================================================================

# Clean build artifacts
clean:
    cargo clean

# Clean everything including ESP-IDF build cache
clean-all:
    cargo clean
    rm -rf .embuild

# ============================================================================
# Info
# ============================================================================

# Show current configuration
info:
    @echo "Target: Freenove ESP32-S3 WROOM Lite"
    @echo ""
    @echo "picoagent — AI-OS for ESP32"
    @echo "  Telegram Bot -> Claude API -> Tool execution"
    @echo ""
    @echo "Config: .env file"
    @echo "  WIFI_SSID, WIFI_PASS"
    @echo "  TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID"
    @echo "  CLAUDE_API_KEY"
    @echo "  DEVICE_LABEL (optional)"
    @echo "  CLAUDE_MODEL (optional)"

# ============================================================================
# Development Helpers
# ============================================================================

# Build and flash in one command
run: flash-monitor

# Quick rebuild (skip if unchanged)
rebuild:
    cargo build --release 2>&1 | tail -5
