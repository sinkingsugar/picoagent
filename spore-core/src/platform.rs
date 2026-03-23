//! Platform trait — HAL abstraction for hardware backends.
//!
//! The VM core knows nothing about hardware. All peripheral access goes
//! through this trait. Backends (ESP32, desktop mock, WASM) implement it.

use crate::VmError;

/// Result type for platform operations.
pub type PlatformResult<T> = Result<T, VmError>;

/// Hardware abstraction layer.
///
/// All methods have default implementations that return `PlatformError`,
/// so backends only need to implement what they support.
pub trait Platform {
    // --- GPIO ---
    fn gpio_mode(&mut self, _pin: i32, _mode: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn gpio_write(&mut self, _pin: i32, _val: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn gpio_read(&mut self, _pin: i32) -> PlatformResult<i32> {
        Err(VmError::PlatformError)
    }
    fn gpio_toggle(&mut self, _pin: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn adc_read(&mut self, _pin: i32) -> PlatformResult<i32> {
        Err(VmError::PlatformError)
    }
    fn pwm_init(&mut self, _pin: i32, _freq: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn pwm_duty(&mut self, _pin: i32, _duty: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }

    // --- I2C ---
    fn i2c_set_addr(&mut self, _addr: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn i2c_write_byte(&mut self, _byte: u8) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn i2c_read_byte(&mut self) -> PlatformResult<u8> {
        Err(VmError::PlatformError)
    }
    fn i2c_write_buf(&mut self, _buf: &[u8]) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn i2c_read_buf(&mut self, _buf: &mut [u8]) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn bme_read(&mut self) -> PlatformResult<(f32, f32, f32)> {
        Err(VmError::PlatformError)
    }

    // --- SPI ---
    fn spi_init(&mut self, _clk: i32, _mosi: i32, _miso: i32, _cs: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    /// Full-duplex SPI transfer: send `in_buf` (MOSI), receive into `out_buf` (MISO).
    fn spi_transfer(&mut self, _in_buf: &[u8], _out_buf: &mut [u8]) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }

    // --- WiFi ---
    fn wifi_connect(&mut self, _ssid: &str, _pass: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn wifi_status(&self) -> PlatformResult<i32> {
        Err(VmError::PlatformError)
    }
    fn wifi_disconnect(&mut self) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    /// Returns the IP address as a packed i32 (network byte order).
    fn wifi_ip(&mut self) -> PlatformResult<i32> {
        Err(VmError::PlatformError)
    }

    // --- BLE ---
    fn ble_init(&mut self) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn ble_advertise(&mut self, _name: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn ble_stop_adv(&mut self) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn ble_notify(&mut self, _handle: i32, _data: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn ble_read(&mut self, _handle: i32) -> PlatformResult<u16> {
        Err(VmError::PlatformError)
    }

    // --- MQTT ---
    fn mqtt_init(&mut self, _broker: &str, _port: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn mqtt_pub(&mut self, _topic: &str, _payload: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn mqtt_sub(&mut self, _topic: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn mqtt_unsub(&mut self, _topic: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }

    // --- System ---
    fn delay_ms(&mut self, _ms: u32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn millis(&self) -> PlatformResult<u32> {
        Err(VmError::PlatformError)
    }
    fn deep_sleep(&mut self, _seconds: u32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn reboot(&mut self) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn nvs_get(&self, _key: &str) -> PlatformResult<i32> {
        Err(VmError::PlatformError)
    }
    fn nvs_set(&mut self, _key: &str, _val: i32) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn heap_free(&self) -> PlatformResult<u32> {
        Err(VmError::PlatformError)
    }
    fn log(&mut self, _msg: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }

    // --- OTA ---
    fn ota_recv(&mut self) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
    fn ota_load(&mut self, _program: &str) -> PlatformResult<()> {
        Err(VmError::PlatformError)
    }
}
