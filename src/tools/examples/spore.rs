//! Spore tools — deploy and monitor AI-generated programs on the device.
//!
//! Two tools replace the need for individual hardware tool definitions:
//! - `deploy_spore`: parse + run a Spore program, return results
//! - `spore_status`: VM state, running tasks, heap

use crate::tools::{Tool, ToolOutput};
use anyhow::Result;
use serde_json::Value;

use spore_core::dict::Dict;
use spore_core::platform::{Platform, PlatformResult};
use spore_core::vm::Vm;
use spore_core::parse;

use esp_idf_svc::sys;
use log::info;
use std::string::String;
use std::thread;
use std::time::Duration;
use std::vec::Vec;

// ---------------------------------------------------------------------------
// ESP32 Platform
// ---------------------------------------------------------------------------

/// Spore platform backend for ESP32-S3.
///
/// Provides system services (millis, heap, delay, log) out of the box.
/// GPIO/I2C/SPI/WiFi/BLE/MQTT can be wired in as the project grows.
pub struct Esp32Platform {
    /// Log output captured during execution for feedback to Claude.
    logs: Vec<String>,
}

impl Esp32Platform {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
        }
    }

    pub fn take_logs(&mut self) -> Vec<String> {
        core::mem::take(&mut self.logs)
    }
}

impl Platform for Esp32Platform {
    fn log(&mut self, msg: &str) -> PlatformResult<()> {
        info!("[spore] {}", msg);
        self.logs.push(String::from(msg));
        Ok(())
    }

    fn millis(&self) -> PlatformResult<u32> {
        Ok(unsafe { sys::esp_log_timestamp() })
    }

    fn heap_free(&self) -> PlatformResult<u32> {
        Ok(unsafe { sys::esp_get_free_heap_size() })
    }

    fn delay_ms(&mut self, ms: u32) -> PlatformResult<()> {
        thread::sleep(Duration::from_millis(ms as u64));
        Ok(())
    }

    fn reboot(&mut self) -> PlatformResult<()> {
        esp_idf_svc::hal::reset::restart();
    }
}

// ---------------------------------------------------------------------------
// deploy_spore tool
// ---------------------------------------------------------------------------

/// Tool that deploys and runs a Spore program on the device.
///
/// Claude generates the token stream, this tool parses and executes it,
/// and returns the results (stack contents, logs, errors).
pub struct DeploySporeTool {
    vm: Vm<Esp32Platform>,
    dict: Dict<64>,
    /// Total spores deployed since boot.
    deploy_count: u32,
}

impl DeploySporeTool {
    pub fn new() -> Self {
        Self {
            vm: Vm::new(Esp32Platform::new()),
            dict: Dict::new(),
            deploy_count: 0,
        }
    }

    fn run_program(&mut self, program: &str) -> Result<ToolOutput> {
        // Reset string pool and dict for fresh parse
        self.vm.strings.clear();
        self.vm.buffers.clear();
        self.dict.clear();

        // Parse
        let result = parse(program, &mut self.vm.strings, &mut self.dict);
        let parsed = match result {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolOutput::err(format!("Parse error: {:?}", e)));
            }
        };

        if parsed.len == 0 {
            return Ok(ToolOutput::err("Empty program"));
        }

        // Load and run
        self.vm.load(&parsed.ops[..parsed.len]);
        self.vm.program_len = parsed.len;

        // If there's a main task entry, start there
        if parsed.entry > 0 {
            self.vm.ip = parsed.entry;
        }

        // Run with a step limit to prevent infinite loops from blocking
        // the agent. 100K steps is generous for any reasonable program.
        let step_result = self.vm.run_steps(100_000);

        self.deploy_count += 1;

        // Collect results
        let mut output = String::new();

        // Logs
        let logs = self.vm.platform.take_logs();
        if !logs.is_empty() {
            output.push_str("LOG:\n");
            for log in &logs {
                output.push_str("  ");
                output.push_str(log);
                output.push('\n');
            }
        }

        // Stack contents (top 8)
        let depth = self.vm.ds.depth();
        if depth > 0 {
            output.push_str(&format!("STACK ({depth} values, top first):\n"));
            for i in 0..depth.min(8) {
                if let Ok(val) = self.vm.ds.peek_at(i) {
                    let repr = format_value(val, &self.vm.strings);
                    output.push_str(&format!("  [{i}] {repr}\n"));
                }
            }
        }

        // Execution status
        match step_result {
            spore_core::StepResult::Halted => {
                output.push_str("STATUS: halted (ok)\n");
            }
            spore_core::StepResult::Continue => {
                output.push_str("STATUS: step limit reached (program may need more cycles)\n");
            }
            spore_core::StepResult::Yielded => {
                output.push_str("STATUS: yielded (task wants to be scheduled)\n");
            }
            spore_core::StepResult::YieldedForever => {
                output.push_str("STATUS: suspended (waiting for event)\n");
            }
            spore_core::StepResult::Error(e) => {
                return Ok(ToolOutput::err(format!(
                    "{}Runtime error: {:?}",
                    output, e
                )));
            }
        }

        Ok(ToolOutput::ok(output))
    }
}

impl Tool for DeploySporeTool {
    fn name(&self) -> &'static str {
        "deploy_spore"
    }

    fn description(&self) -> &'static str {
        "Deploy and run a Spore program on the device. Spore is a stack-based \
         language (Forth-inspired) with uppercase tokens. Returns stack contents, \
         log output, and execution status. Use LOG to print values, HEAP_FREE for \
         memory, MILLIS for uptime, GPIO_WRITE/GPIO_READ for pins, I2C_ADDR + \
         BME_READ for sensors. Define words with DEF...END, tasks with TASK...ENDTASK."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "program": {
                    "type": "string",
                    "description": "Spore token stream. Uppercase, space-delimited. \
                        Example: 'LIT 2 LIT 3 ADD' pushes 5. \
                        'HEAP_FREE' pushes free bytes. \
                        'STR \"hello\" LOG' logs a message."
                }
            },
            "required": ["program"]
        })
    }

    fn execute(&mut self, params: Value) -> Result<ToolOutput> {
        let program = params["program"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'program' parameter required"))?;

        info!("Deploying spore ({} bytes)...", program.len());
        self.run_program(program)
    }
}

// ---------------------------------------------------------------------------
// spore_status tool
// ---------------------------------------------------------------------------

/// Tool that reports device status — heap, uptime, firmware.
///
/// The deploy tool already returns full execution results (stack, logs, errors),
/// so this is for device-level health checks.
pub struct SporeStatusStandalone {
    boot_time: u32,
}

impl SporeStatusStandalone {
    pub fn new() -> Self {
        Self {
            boot_time: unsafe { sys::esp_log_timestamp() },
        }
    }
}

impl Tool for SporeStatusStandalone {
    fn name(&self) -> &'static str {
        "spore_status"
    }

    fn description(&self) -> &'static str {
        "Get device status: free heap, uptime, firmware info. Use this to check \
         the device health before or after deploying spores."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    fn execute(&mut self, _params: Value) -> Result<ToolOutput> {
        let free_heap = unsafe { sys::esp_get_free_heap_size() };
        let min_heap = unsafe { sys::esp_get_minimum_free_heap_size() };
        let now = unsafe { sys::esp_log_timestamp() };
        let uptime_s = now.wrapping_sub(self.boot_time) as u64 / 1000;

        let report = format!(
            "Free heap: {} KB ({} KB minimum)\n\
             Uptime: {}m {}s\n\
             Firmware: picoagent v{}",
            free_heap / 1024,
            min_heap / 1024,
            uptime_s / 60,
            uptime_s % 60,
            env!("CARGO_PKG_VERSION"),
        );

        Ok(ToolOutput::ok(report))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_value(val: spore_core::Value, strings: &spore_core::StringPool<2048, 128>) -> String {
    match val {
        spore_core::Value::I(v) => format!("Int({})", v),
        spore_core::Value::F(v) => format!("Float({:.2})", v),
        spore_core::Value::B(v) => format!("Bool({})", v),
        spore_core::Value::S(idx) => {
            if let Some(s) = strings.get(idx) {
                format!("Str(\"{}\")", s)
            } else {
                format!("Str(#{})", idx)
            }
        }
        spore_core::Value::Buf(idx) => format!("Buf(#{})", idx),
    }
}
