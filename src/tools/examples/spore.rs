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
use spore_core::{parse, VmError};

use esp_idf_svc::sys;
use log::{info, warn};
use std::string::String;
use std::thread;
use std::time::Duration;
use std::vec::Vec;

// ---------------------------------------------------------------------------
// ESP32 Platform — raw esp-idf hardware access
// ---------------------------------------------------------------------------

/// Max PWM channels (ESP32-S3 LEDC has 8).
const MAX_PWM_CHANNELS: usize = 8;

/// Default I2C port.
const I2C_PORT: sys::i2c_port_t = 0;
/// Default I2C SDA pin (common on ESP32-S3 dev boards).
const I2C_SDA_PIN: i32 = 8;
/// Default I2C SCL pin.
const I2C_SCL_PIN: i32 = 9;
/// Default I2C clock speed.
const I2C_FREQ_HZ: u32 = 100_000;
/// I2C timeout in ticks.
const I2C_TIMEOUT_MS: u32 = 1000;

/// PWM channel assignment: pin → LEDC channel.
struct PwmChannel {
    pin: i32,
    channel: sys::ledc_channel_t,
}

/// Spore platform backend for ESP32-S3.
///
/// Uses raw esp-idf syscalls for GPIO, PWM (LEDC), ADC, and I2C.
/// No HAL ownership — pin numbers are just integers, Claude picks them.
pub struct Esp32Platform {
    /// Log output captured during execution for feedback to Claude.
    logs: Vec<String>,
    /// Active PWM channels.
    pwm_channels: [Option<PwmChannel>; MAX_PWM_CHANNELS],
    pwm_count: usize,
    /// Current I2C target address (set by I2C_ADDR).
    i2c_addr: u8,
    /// Whether the I2C driver has been initialized.
    i2c_initialized: bool,
}

impl Esp32Platform {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            pwm_channels: [const { None }; MAX_PWM_CHANNELS],
            pwm_count: 0,
            i2c_addr: 0,
            i2c_initialized: false,
        }
    }

    pub fn take_logs(&mut self) -> Vec<String> {
        core::mem::take(&mut self.logs)
    }

    /// Find or allocate a LEDC channel for a pin.
    fn find_or_alloc_pwm_channel(&mut self, pin: i32) -> Option<sys::ledc_channel_t> {
        // Check if pin already has a channel
        for ch in &self.pwm_channels {
            if let Some(c) = ch {
                if c.pin == pin {
                    return Some(c.channel);
                }
            }
        }
        // Allocate new channel
        if self.pwm_count >= MAX_PWM_CHANNELS {
            return None;
        }
        let channel = self.pwm_count as sys::ledc_channel_t;
        self.pwm_channels[self.pwm_count] = Some(PwmChannel { pin, channel });
        self.pwm_count += 1;
        Some(channel)
    }

    /// Ensure the I2C driver is initialized.
    fn ensure_i2c(&mut self) -> PlatformResult<()> {
        if self.i2c_initialized {
            return Ok(());
        }

        let conf = sys::i2c_config_t {
            mode: sys::i2c_mode_t_I2C_MODE_MASTER,
            sda_io_num: I2C_SDA_PIN,
            scl_io_num: I2C_SCL_PIN,
            sda_pullup_en: true,
            scl_pullup_en: true,
            __bindgen_anon_1: sys::i2c_config_t__bindgen_ty_1 {
                master: sys::i2c_config_t__bindgen_ty_1__bindgen_ty_1 {
                    clk_speed: I2C_FREQ_HZ,
                },
            },
            clk_flags: 0,
        };

        let ret = unsafe { sys::i2c_param_config(I2C_PORT, &conf) };
        if ret != sys::ESP_OK {
            warn!("[spore] i2c_param_config failed: {}", ret);
            return Err(VmError::PlatformError);
        }

        let ret = unsafe {
            sys::i2c_driver_install(I2C_PORT, sys::i2c_mode_t_I2C_MODE_MASTER, 0, 0, 0)
        };
        if ret != sys::ESP_OK {
            warn!("[spore] i2c_driver_install failed: {}", ret);
            return Err(VmError::PlatformError);
        }

        self.i2c_initialized = true;
        info!("[spore] I2C initialized on SDA={} SCL={}", I2C_SDA_PIN, I2C_SCL_PIN);
        Ok(())
    }
}

fn esp_ok(ret: sys::esp_err_t) -> PlatformResult<()> {
    if ret == sys::ESP_OK {
        Ok(())
    } else {
        Err(VmError::PlatformError)
    }
}

impl Platform for Esp32Platform {
    // --- System ---

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

    // --- GPIO ---

    fn gpio_mode(&mut self, pin: i32, mode: i32) -> PlatformResult<()> {
        unsafe {
            esp_ok(sys::gpio_reset_pin(pin))?;
        }
        let direction = match mode {
            0 => sys::gpio_mode_t_GPIO_MODE_INPUT,               // input
            1 => sys::gpio_mode_t_GPIO_MODE_OUTPUT,              // output
            2 => sys::gpio_mode_t_GPIO_MODE_INPUT,               // input + pullup
            3 => sys::gpio_mode_t_GPIO_MODE_INPUT,               // input + pulldown
            _ => return Err(VmError::PlatformError),
        };
        unsafe {
            esp_ok(sys::gpio_set_direction(pin, direction))?;
        }
        // Set pull resistor
        match mode {
            2 => unsafe { esp_ok(sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY))? },
            3 => unsafe { esp_ok(sys::gpio_set_pull_mode(pin, sys::gpio_pull_mode_t_GPIO_PULLDOWN_ONLY))? },
            _ => {}
        }
        Ok(())
    }

    fn gpio_write(&mut self, pin: i32, val: i32) -> PlatformResult<()> {
        unsafe { esp_ok(sys::gpio_set_level(pin, val as u32)) }
    }

    fn gpio_read(&mut self, pin: i32) -> PlatformResult<i32> {
        Ok(unsafe { sys::gpio_get_level(pin) })
    }

    fn gpio_toggle(&mut self, pin: i32) -> PlatformResult<()> {
        let current = unsafe { sys::gpio_get_level(pin) };
        let new_val = if current == 0 { 1 } else { 0 };
        unsafe { esp_ok(sys::gpio_set_level(pin, new_val)) }
    }

    // --- PWM (LEDC) ---

    fn pwm_init(&mut self, pin: i32, freq: i32) -> PlatformResult<()> {
        let channel = self
            .find_or_alloc_pwm_channel(pin)
            .ok_or(VmError::PlatformError)?;

        // Configure timer (use timer 0 for all channels — shared frequency)
        let timer_conf = sys::ledc_timer_config_t {
            speed_mode: sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
            duty_resolution: sys::ledc_timer_bit_t_LEDC_TIMER_10_BIT, // 0-1023
            timer_num: sys::ledc_timer_t_LEDC_TIMER_0,
            freq_hz: freq as u32,
            clk_cfg: sys::soc_periph_ledc_clk_src_legacy_t_LEDC_AUTO_CLK,
            deconfigure: false,
        };
        unsafe {
            esp_ok(sys::ledc_timer_config(&timer_conf))?;
        }

        // Configure channel
        let channel_conf = sys::ledc_channel_config_t {
            gpio_num: pin,
            speed_mode: sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
            channel,
            intr_type: sys::ledc_intr_type_t_LEDC_INTR_DISABLE,
            timer_sel: sys::ledc_timer_t_LEDC_TIMER_0,
            duty: 0,
            hpoint: 0,
            flags: unsafe { core::mem::zeroed() },
        };
        unsafe {
            esp_ok(sys::ledc_channel_config(&channel_conf))?;
        }

        info!("[spore] PWM init: pin={} freq={}Hz channel={}", pin, freq, channel);
        Ok(())
    }

    fn pwm_duty(&mut self, pin: i32, duty: i32) -> PlatformResult<()> {
        // Find the channel for this pin
        let channel = self
            .pwm_channels
            .iter()
            .find_map(|ch| {
                ch.as_ref()
                    .filter(|c| c.pin == pin)
                    .map(|c| c.channel)
            })
            .ok_or(VmError::PlatformError)?;

        let duty_clamped = duty.clamp(0, 1023) as u32;
        unsafe {
            esp_ok(sys::ledc_set_duty(
                sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
                channel,
                duty_clamped,
            ))?;
            esp_ok(sys::ledc_update_duty(
                sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
                channel,
            ))?;
        }
        Ok(())
    }

    // --- ADC ---

    fn adc_read(&mut self, pin: i32) -> PlatformResult<i32> {
        // Use the legacy adc1 one-shot API for simplicity.
        // ADC1 channels on ESP32-S3: GPIO1-10.
        // Channel mapping: GPIO1=ADC1_CH0, GPIO2=ADC1_CH1, etc.
        let channel = (pin - 1) as u32;

        unsafe {
            // Configure width and attenuation on first read
            let _ = sys::adc1_config_width(sys::adc_bits_width_t_ADC_WIDTH_BIT_12);
            let _ = sys::adc1_config_channel_atten(
                channel,
                sys::adc_atten_t_ADC_ATTEN_DB_12,
            );
            let raw = sys::adc1_get_raw(channel);
            if raw < 0 {
                return Err(VmError::PlatformError);
            }
            Ok(raw)
        }
    }

    // --- I2C ---

    fn i2c_set_addr(&mut self, addr: i32) -> PlatformResult<()> {
        self.i2c_addr = addr as u8;
        Ok(())
    }

    fn i2c_write_byte(&mut self, byte: u8) -> PlatformResult<()> {
        self.ensure_i2c()?;
        let data = [byte];
        let ret = unsafe {
            sys::i2c_master_write_to_device(
                I2C_PORT,
                self.i2c_addr,
                data.as_ptr(),
                1,
                I2C_TIMEOUT_MS,
            )
        };
        esp_ok(ret)
    }

    fn i2c_read_byte(&mut self) -> PlatformResult<u8> {
        self.ensure_i2c()?;
        let mut data = [0u8];
        let ret = unsafe {
            sys::i2c_master_read_from_device(
                I2C_PORT,
                self.i2c_addr,
                data.as_mut_ptr(),
                1,
                I2C_TIMEOUT_MS,
            )
        };
        esp_ok(ret)?;
        Ok(data[0])
    }

    fn i2c_write_buf(&mut self, buf: &[u8]) -> PlatformResult<()> {
        self.ensure_i2c()?;
        let ret = unsafe {
            sys::i2c_master_write_to_device(
                I2C_PORT,
                self.i2c_addr,
                buf.as_ptr(),
                buf.len(),
                I2C_TIMEOUT_MS,
            )
        };
        esp_ok(ret)
    }

    fn i2c_read_buf(&mut self, buf: &mut [u8]) -> PlatformResult<()> {
        self.ensure_i2c()?;
        let ret = unsafe {
            sys::i2c_master_read_from_device(
                I2C_PORT,
                self.i2c_addr,
                buf.as_mut_ptr(),
                buf.len(),
                I2C_TIMEOUT_MS,
            )
        };
        esp_ok(ret)
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
         memory, MILLIS for uptime. GPIO: GPIO_MODE (0=in,1=out,2=pullup,3=pulldown), \
         GPIO_WRITE, GPIO_READ, GPIO_TOGGLE, ADC_READ. PWM: PWM_INIT (pin freq), \
         PWM_DUTY (pin 0-1023). I2C: I2C_ADDR, I2C_WRITE, I2C_READ (SDA=8, SCL=9). \
         Define words with DEF...END, tasks with TASK...ENDTASK."
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
                        'STR \"hello\" LOG' logs a message. \
                        'LIT 12 LIT 25000 PWM_INIT LIT 12 LIT 512 PWM_DUTY' starts PWM."
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
