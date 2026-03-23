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
use spore_core::task::{Scheduler, Task};
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
/// I2C SDA pin — override via SPORE_I2C_SDA env var (default: 8).
const I2C_SDA_PIN: i32 = const_parse_i32(match option_env!("SPORE_I2C_SDA") {
    Some(s) => s,
    None => "8",
});
/// I2C SCL pin — override via SPORE_I2C_SCL env var (default: 9).
const I2C_SCL_PIN: i32 = const_parse_i32(match option_env!("SPORE_I2C_SCL") {
    Some(s) => s,
    None => "9",
});
/// Default I2C clock speed.
const I2C_FREQ_HZ: u32 = 100_000;
/// I2C timeout in FreeRTOS ticks.
/// ESP-IDF default: configTICK_RATE_HZ=1000, so 1 tick = 1ms.
/// 1000 ticks = 1 second timeout.
const I2C_TIMEOUT_TICKS: u32 = 1000;

/// Maximum delay_ms per call, capped to limit per-call blocking (5 seconds).
/// Total scheduler duration is separately bounded by MAX_SCHEDULER_DURATION_MS.
const MAX_DELAY_MS: u32 = 5_000;

/// Max LEDC timers on ESP32-S3.
const MAX_PWM_TIMERS: usize = 4;

/// PWM channel assignment: pin → LEDC channel + timer.
struct PwmChannel {
    pin: i32,
    channel: sys::ledc_channel_t,
    timer: sys::ledc_timer_t,
}

/// PWM timer assignment: frequency → LEDC timer.
struct PwmTimer {
    freq_hz: u32,
    timer_num: sys::ledc_timer_t,
}

/// BME280 calibration data read from sensor NVM.
/// Per Bosch datasheet section 4.2.2.
struct BmeCalibration {
    dig_t1: u16,
    dig_t2: i16,
    dig_t3: i16,
    dig_p1: u16,
    dig_p2: i16,
    dig_p3: i16,
    dig_p4: i16,
    dig_p5: i16,
    dig_p6: i16,
    dig_p7: i16,
    dig_p8: i16,
    dig_p9: i16,
    dig_h1: u8,
    dig_h2: i16,
    dig_h3: u8,
    dig_h4: i16,
    dig_h5: i16,
    dig_h6: i8,
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
    /// Active PWM timers (frequency → timer mapping).
    pwm_timers: [Option<PwmTimer>; MAX_PWM_TIMERS],
    pwm_timer_count: usize,
    /// Current I2C target address (set by I2C_ADDR).
    i2c_addr: u8,
    /// Whether the I2C driver has been initialized.
    i2c_initialized: bool,
    /// Cached BME280 calibration data (loaded once per I2C address).
    bme_calibration: Option<(u8, BmeCalibration)>, // (addr, cal)
}

impl Esp32Platform {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            pwm_channels: [const { None }; MAX_PWM_CHANNELS],
            pwm_count: 0,
            pwm_timers: [const { None }; MAX_PWM_TIMERS],
            pwm_timer_count: 0,
            i2c_addr: 0,
            i2c_initialized: false,
            bme_calibration: None,
        }
    }

    pub fn take_logs(&mut self) -> Vec<String> {
        core::mem::take(&mut self.logs)
    }

    /// Reset PWM channel and timer allocations. Called between deployments
    /// so new programs get a fresh pool. The underlying LEDC hardware is
    /// reconfigured on next `pwm_init`.
    pub fn reset_pwm(&mut self) {
        self.pwm_channels = [const { None }; MAX_PWM_CHANNELS];
        self.pwm_count = 0;
        self.pwm_timers = [const { None }; MAX_PWM_TIMERS];
        self.pwm_timer_count = 0;
    }

    /// Find or allocate a LEDC timer for a given frequency.
    fn find_or_alloc_pwm_timer(&mut self, freq_hz: u32) -> Option<sys::ledc_timer_t> {
        // Reuse existing timer with the same frequency
        for t in &self.pwm_timers {
            if let Some(t) = t {
                if t.freq_hz == freq_hz {
                    return Some(t.timer_num);
                }
            }
        }
        // Allocate new timer
        if self.pwm_timer_count >= MAX_PWM_TIMERS {
            return None;
        }
        let timer_num = self.pwm_timer_count as sys::ledc_timer_t;
        self.pwm_timers[self.pwm_timer_count] = Some(PwmTimer { freq_hz, timer_num });
        self.pwm_timer_count += 1;
        Some(timer_num)
    }

    /// Find or allocate a LEDC channel for a pin.
    fn find_or_alloc_pwm_channel(
        &mut self,
        pin: i32,
        timer: sys::ledc_timer_t,
    ) -> Option<sys::ledc_channel_t> {
        // Check if pin already has a channel
        for ch in &mut self.pwm_channels {
            if let Some(c) = ch {
                if c.pin == pin {
                    c.timer = timer; // Update timer assignment
                    return Some(c.channel);
                }
            }
        }
        // Allocate new channel
        if self.pwm_count >= MAX_PWM_CHANNELS {
            return None;
        }
        let channel = self.pwm_count as sys::ledc_channel_t;
        self.pwm_channels[self.pwm_count] = Some(PwmChannel {
            pin,
            channel,
            timer,
        });
        self.pwm_count += 1;
        Some(channel)
    }

    /// Read a block of I2C registers starting at `reg`.
    fn i2c_read_regs(&self, reg: u8, buf: &mut [u8]) -> PlatformResult<()> {
        let reg_buf = [reg];
        let ret = unsafe {
            sys::i2c_master_write_to_device(
                I2C_PORT,
                self.i2c_addr,
                reg_buf.as_ptr(),
                1,
                I2C_TIMEOUT_TICKS,
            )
        };
        esp_ok(ret)?;
        let ret = unsafe {
            sys::i2c_master_read_from_device(
                I2C_PORT,
                self.i2c_addr,
                buf.as_mut_ptr(),
                buf.len(),
                I2C_TIMEOUT_TICKS,
            )
        };
        esp_ok(ret)
    }

    /// Read BME280 calibration data from sensor NVM registers.
    /// Per Bosch BME280 datasheet section 4.2.2.
    fn read_bme_calibration(&mut self) -> PlatformResult<BmeCalibration> {
        // Temperature and pressure calibration: 0x88..0x9F (26 bytes)
        let mut tp = [0u8; 26];
        self.i2c_read_regs(0x88, &mut tp)?;

        // Humidity calibration part 1: 0xA1 (1 byte)
        let mut h1 = [0u8; 1];
        self.i2c_read_regs(0xA1, &mut h1)?;

        // Humidity calibration part 2: 0xE1..0xE7 (7 bytes)
        let mut h2 = [0u8; 7];
        self.i2c_read_regs(0xE1, &mut h2)?;

        Ok(BmeCalibration {
            dig_t1: u16::from_le_bytes([tp[0], tp[1]]),
            dig_t2: i16::from_le_bytes([tp[2], tp[3]]),
            dig_t3: i16::from_le_bytes([tp[4], tp[5]]),
            dig_p1: u16::from_le_bytes([tp[6], tp[7]]),
            dig_p2: i16::from_le_bytes([tp[8], tp[9]]),
            dig_p3: i16::from_le_bytes([tp[10], tp[11]]),
            dig_p4: i16::from_le_bytes([tp[12], tp[13]]),
            dig_p5: i16::from_le_bytes([tp[14], tp[15]]),
            dig_p6: i16::from_le_bytes([tp[16], tp[17]]),
            dig_p7: i16::from_le_bytes([tp[18], tp[19]]),
            dig_p8: i16::from_le_bytes([tp[20], tp[21]]),
            dig_p9: i16::from_le_bytes([tp[22], tp[23]]),
            dig_h1: h1[0],
            dig_h2: i16::from_le_bytes([h2[0], h2[1]]),
            dig_h3: h2[2],
            dig_h4: ((h2[3] as i16) << 4) | ((h2[4] as i16) & 0x0F),
            dig_h5: ((h2[5] as i16) << 4) | (((h2[4] as i16) >> 4) & 0x0F),
            dig_h6: h2[6] as i8,
        })
    }

    /// Get or read BME280 calibration for the current I2C address.
    fn ensure_bme_calibration(&mut self) -> PlatformResult<&BmeCalibration> {
        let addr = self.i2c_addr;
        if let Some((cached_addr, _)) = &self.bme_calibration {
            if *cached_addr == addr {
                return Ok(&self.bme_calibration.as_ref().unwrap().1);
            }
        }
        let cal = self.read_bme_calibration()?;
        self.bme_calibration = Some((addr, cal));
        Ok(&self.bme_calibration.as_ref().unwrap().1)
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

        let ret =
            unsafe { sys::i2c_driver_install(I2C_PORT, sys::i2c_mode_t_I2C_MODE_MASTER, 0, 0, 0) };
        if ret != sys::ESP_OK {
            warn!("[spore] i2c_driver_install failed: {}", ret);
            return Err(VmError::PlatformError);
        }

        self.i2c_initialized = true;
        info!(
            "[spore] I2C initialized on SDA={} SCL={}",
            I2C_SDA_PIN, I2C_SCL_PIN
        );
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
        let clamped = ms.min(MAX_DELAY_MS);
        thread::sleep(Duration::from_millis(clamped as u64));
        Ok(())
    }

    fn reboot(&mut self) -> PlatformResult<()> {
        // Reboot is not supported from Spore programs — calling restart() here
        // would bypass session save (which happens after the agent loop returns).
        warn!("[spore] REBOOT rejected: would bypass session save");
        Err(VmError::PlatformError)
    }

    // --- GPIO ---

    fn gpio_mode(&mut self, pin: i32, mode: i32) -> PlatformResult<()> {
        unsafe {
            esp_ok(sys::gpio_reset_pin(pin))?;
        }
        let direction = match mode {
            0 => sys::gpio_mode_t_GPIO_MODE_INPUT,  // input
            1 => sys::gpio_mode_t_GPIO_MODE_OUTPUT, // output
            2 => sys::gpio_mode_t_GPIO_MODE_INPUT,  // input + pullup
            3 => sys::gpio_mode_t_GPIO_MODE_INPUT,  // input + pulldown
            _ => return Err(VmError::PlatformError),
        };
        unsafe {
            esp_ok(sys::gpio_set_direction(pin, direction))?;
        }
        // Set pull resistor
        match mode {
            2 => unsafe {
                esp_ok(sys::gpio_set_pull_mode(
                    pin,
                    sys::gpio_pull_mode_t_GPIO_PULLUP_ONLY,
                ))?
            },
            3 => unsafe {
                esp_ok(sys::gpio_set_pull_mode(
                    pin,
                    sys::gpio_pull_mode_t_GPIO_PULLDOWN_ONLY,
                ))?
            },
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
        let freq_hz = freq as u32;

        // Find or allocate a timer for this frequency (up to 4 distinct frequencies)
        let timer_num = self
            .find_or_alloc_pwm_timer(freq_hz)
            .ok_or(VmError::PlatformError)?;

        let channel = self
            .find_or_alloc_pwm_channel(pin, timer_num)
            .ok_or(VmError::PlatformError)?;

        // Configure timer for this frequency
        let timer_conf = sys::ledc_timer_config_t {
            speed_mode: sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
            duty_resolution: sys::ledc_timer_bit_t_LEDC_TIMER_10_BIT, // 0-1023
            timer_num,
            freq_hz,
            clk_cfg: sys::soc_periph_ledc_clk_src_legacy_t_LEDC_AUTO_CLK,
            deconfigure: false,
        };
        unsafe {
            esp_ok(sys::ledc_timer_config(&timer_conf))?;
        }

        // Configure channel bound to the correct timer
        let channel_conf = sys::ledc_channel_config_t {
            gpio_num: pin,
            speed_mode: sys::ledc_mode_t_LEDC_LOW_SPEED_MODE,
            channel,
            intr_type: sys::ledc_intr_type_t_LEDC_INTR_DISABLE,
            timer_sel: timer_num,
            duty: 0,
            hpoint: 0,
            flags: unsafe { core::mem::zeroed() },
        };
        unsafe {
            esp_ok(sys::ledc_channel_config(&channel_conf))?;
        }

        info!(
            "[spore] PWM init: pin={} freq={}Hz channel={} timer={}",
            pin, freq, channel, timer_num
        );
        Ok(())
    }

    fn pwm_duty(&mut self, pin: i32, duty: i32) -> PlatformResult<()> {
        // Find the channel for this pin
        let channel = self
            .pwm_channels
            .iter()
            .find_map(|ch| ch.as_ref().filter(|c| c.pin == pin).map(|c| c.channel))
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
        if pin < 1 || pin > 10 {
            return Err(VmError::PlatformError);
        }
        let channel = (pin - 1) as u32;

        unsafe {
            // Configure width and attenuation on first read
            let _ = sys::adc1_config_width(sys::adc_bits_width_t_ADC_WIDTH_BIT_12);
            let _ = sys::adc1_config_channel_atten(channel, sys::adc_atten_t_ADC_ATTEN_DB_12);
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
                I2C_TIMEOUT_TICKS,
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
                I2C_TIMEOUT_TICKS,
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
                I2C_TIMEOUT_TICKS,
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
                I2C_TIMEOUT_TICKS,
            )
        };
        esp_ok(ret)
    }

    fn bme_read(&mut self) -> PlatformResult<(f32, f32, f32)> {
        // BME280 read via I2C. Requires I2C_ADDR to be set first (0x76 or 0x77).
        // Reads calibration data from NVM on first call, then applies Bosch
        // compensation formulas (datasheet section 4.2.3).
        self.ensure_i2c()?;

        // Ensure calibration data is loaded for this address
        self.ensure_bme_calibration()?;

        // Read raw data burst from 0xF7-0xFE (8 bytes)
        let mut data = [0u8; 8];
        self.i2c_read_regs(0xF7, &mut data)?;

        // Raw values (20-bit pressure/temp, 16-bit humidity)
        let raw_press =
            ((data[0] as i32) << 12) | ((data[1] as i32) << 4) | ((data[2] as i32) >> 4);
        let raw_temp = ((data[3] as i32) << 12) | ((data[4] as i32) << 4) | ((data[5] as i32) >> 4);
        let raw_hum = ((data[6] as i32) << 8) | (data[7] as i32);

        let cal = &self.bme_calibration.as_ref().unwrap().1;

        // Temperature compensation (Bosch datasheet 4.2.3)
        let var1 = (raw_temp as f32 / 16384.0 - cal.dig_t1 as f32 / 1024.0) * cal.dig_t2 as f32;
        let d = raw_temp as f32 / 131072.0 - cal.dig_t1 as f32 / 8192.0;
        let var2 = d * d * cal.dig_t3 as f32;
        let t_fine = var1 + var2;
        let temp = t_fine / 5120.0;

        // Pressure compensation
        let mut pvar1 = t_fine / 2.0 - 64000.0;
        let mut pvar2 = pvar1 * pvar1 * cal.dig_p6 as f32 / 32768.0;
        pvar2 += pvar1 * cal.dig_p5 as f32 * 2.0;
        pvar2 = pvar2 / 4.0 + cal.dig_p4 as f32 * 65536.0;
        pvar1 =
            (cal.dig_p3 as f32 * pvar1 * pvar1 / 524288.0 + cal.dig_p2 as f32 * pvar1) / 524288.0;
        pvar1 = (1.0 + pvar1 / 32768.0) * cal.dig_p1 as f32;
        let press = if pvar1 > 0.0 {
            let mut p = 1048576.0 - raw_press as f32;
            p = (p - pvar2 / 4096.0) * 6250.0 / pvar1;
            pvar1 = cal.dig_p9 as f32 * p * p / 2147483648.0;
            pvar2 = p * cal.dig_p8 as f32 / 32768.0;
            (p + (pvar1 + pvar2 + cal.dig_p7 as f32) / 16.0) / 100.0 // Pa → hPa
        } else {
            0.0
        };

        // Humidity compensation
        let mut h = t_fine - 76800.0;
        if h == 0.0 {
            return Ok((temp, 0.0, press));
        }
        h = (raw_hum as f32 - (cal.dig_h4 as f32 * 64.0 + cal.dig_h5 as f32 / 16384.0 * h))
            * (cal.dig_h2 as f32 / 65536.0
                * (1.0
                    + cal.dig_h6 as f32 / 67108864.0
                        * h
                        * (1.0 + cal.dig_h3 as f32 / 67108864.0 * h)));
        h *= 1.0 - cal.dig_h1 as f32 * h / 524288.0;
        let hum = if h > 100.0 {
            100.0
        } else if h < 0.0 {
            0.0
        } else {
            h
        };

        Ok((temp, hum, press))
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

/// Max scheduler ticks per deploy (prevents runaway multitasking).
const MAX_SCHEDULER_TICKS: u32 = 1000;
/// Steps per scheduler tick for each task.
const STEPS_PER_TICK: u32 = 100;
/// Max wall-clock time for the scheduler loop (30s, well under 60s WDT timeout).
const MAX_SCHEDULER_DURATION_MS: u32 = 30_000;

impl DeploySporeTool {
    pub fn new() -> Self {
        Self {
            vm: Vm::new(Esp32Platform::new()),
            dict: Dict::new(),
            deploy_count: 0,
        }
    }

    fn run_program(&mut self, program: &str) -> Result<ToolOutput> {
        // Reset string pool, buffers, dict, and PWM state for fresh deployment.
        // PWM channels/timers accumulate across deployments if not reset,
        // exhausting the pool after 8 distinct pin assignments.
        self.vm.strings.clear();
        self.vm.buffers.clear();
        self.dict.clear();
        self.vm.platform.reset_pwm();

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

        // Load program
        self.vm.load(&parsed.ops[..parsed.len]);

        // Set entry point if main task was found
        if let Some(entry) = parsed.entry {
            self.vm.ip = entry;
        }

        // Run with a step limit to prevent infinite loops from blocking
        // the agent. 100K steps is generous for any reasonable program.
        let step_result = self.vm.run_steps(100_000);

        // Process any VmActions through the scheduler (for TASK/START/STOP/ON/EMIT)
        let mut scheduler: Scheduler<Esp32Platform> = Scheduler::new();
        let mut has_tasks = false;
        let mut last_task_idx: Option<usize> = None;

        // Drain actions from the initial run
        for action in self.vm.drain_actions() {
            match action {
                spore_core::VmAction::StartTask(name_idx) => {
                    if let Some(offset) = self.dict.lookup(name_idx) {
                        let task = Task::new(name_idx, offset as usize);
                        if let Ok(idx) = scheduler.add_task(task) {
                            has_tasks = true;
                            last_task_idx = Some(idx);
                        }
                    }
                }
                spore_core::VmAction::EmitEvent(eid) => {
                    scheduler.emit_event(eid);
                }
                spore_core::VmAction::StopTask(_) => {
                    // No tasks running yet during initial execution
                }
                spore_core::VmAction::BindEvent {
                    event_id,
                    word_offset,
                } => {
                    // Bind to the most recently added task (not hardcoded 0).
                    // During initial execution there is no "current task", so
                    // the last-added task is the best approximation.
                    if let Some(tidx) = last_task_idx {
                        let _ = scheduler.bind_event(tidx, event_id, word_offset);
                    }
                }
            }
        }

        // If tasks were started, run the scheduler with a wall-clock time budget
        // to prevent unbounded blocking (delay_ms calls accumulate across ticks).
        if has_tasks {
            let start_ms = unsafe { sys::esp_log_timestamp() };
            for _ in 0..MAX_SCHEDULER_TICKS {
                let elapsed = unsafe { sys::esp_log_timestamp() }.wrapping_sub(start_ms);
                if elapsed > MAX_SCHEDULER_DURATION_MS {
                    warn!("[spore] Scheduler time budget exceeded ({}ms)", elapsed);
                    break;
                }
                match scheduler.tick(&mut self.vm, &self.dict, STEPS_PER_TICK) {
                    Ok(active) if active => continue,
                    _ => break,
                }
            }
        }

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
                return Ok(ToolOutput::err(format!("{}Runtime error: {:?}", output, e)));
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
        "Deploy and run a Spore program on the device. \
         Returns stack contents, log output, and execution status. \
         See the Spore Language Reference in your system prompt for full syntax."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "program": {
                    "type": "string",
                    "description": "Spore program. Uppercase, space-delimited tokens. \
                        See Spore Language Reference in system prompt."
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

/// Compile-time i32 parser for `option_env!()` values.
const fn const_parse_i32(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let mut result: i32 = 0;
    let mut i: usize = 0;
    let negative = bytes[0] == b'-';
    if negative {
        i = 1;
    }
    while i < bytes.len() {
        result = result * 10 + (bytes[i] - b'0') as i32;
        i += 1;
    }
    if negative {
        -result
    } else {
        result
    }
}

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
