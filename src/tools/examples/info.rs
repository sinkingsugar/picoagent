//! System info tool — reports device status.

use crate::tools::{Tool, ToolOutput};
use anyhow::Result;
use esp_idf_svc::sys;
use serde_json::Value;

/// System information tool.
pub struct InfoTool {
    boot_time: u64,
}

impl InfoTool {
    pub fn new() -> Self {
        Self {
            boot_time: unsafe { sys::esp_log_timestamp() } as u64,
        }
    }
}

impl Tool for InfoTool {
    fn name(&self) -> &'static str {
        "system_info"
    }

    fn description(&self) -> &'static str {
        "Get system status: free memory, uptime."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    fn execute(&mut self, _params: Value) -> Result<ToolOutput> {
        let free_heap = unsafe { sys::esp_get_free_heap_size() };
        let min_heap = unsafe { sys::esp_get_minimum_free_heap_size() };
        let now = unsafe { sys::esp_log_timestamp() } as u64;
        let uptime_ms = now.wrapping_sub(self.boot_time);
        let uptime_s = uptime_ms / 1000;

        let report = format!(
            "System Status:\n\
             - Device: {}\n\
             - Free heap: {} KB (min: {} KB)\n\
             - Uptime: {}h {}m {}s\n\
             - Firmware: v{}",
            crate::config::DEVICE_LABEL,
            free_heap / 1024,
            min_heap / 1024,
            uptime_s / 3600,
            (uptime_s / 60) % 60,
            uptime_s % 60,
            env!("CARGO_PKG_VERSION"),
        );

        Ok(ToolOutput::ok(report))
    }
}
