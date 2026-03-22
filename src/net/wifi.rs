//! WiFi connection management.
//!
//! Follows the same pattern as bme680-monitor — proven to work
//! on the Freenove ESP32-S3 WROOM Lite.

use anyhow::{bail, Result};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, EspWifi};
use log::info;

pub struct WifiManager<'d> {
    wifi: EspWifi<'d>,
}

impl<'d> WifiManager<'d> {
    pub fn new(
        modem: impl Peripheral<P = Modem> + 'd,
        sys_loop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self> {
        let wifi = EspWifi::new(modem, sys_loop, nvs)?;
        Ok(Self { wifi })
    }

    pub fn connect(&mut self, ssid: &str, password: &str) -> Result<()> {
        let ssid = ssid
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid SSID"))?;
        let password_h = password
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid password"))?;

        let auth_method = if password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPAWPA2Personal
        };

        let config = Configuration::Client(ClientConfiguration {
            ssid,
            bssid: None,
            auth_method,
            password: password_h,
            channel: None,
            ..Default::default()
        });

        self.wifi.set_configuration(&config)?;
        self.wifi.start()?;
        info!("WiFi started");

        self.wifi.connect()?;

        // Poll for connection with timeout
        let timeout_ms = 30_000u64;
        let poll_interval = 500u64;
        let mut elapsed = 0u64;

        while !self.wifi.is_connected()? {
            if elapsed >= timeout_ms {
                bail!("WiFi connection timeout after {}ms", timeout_ms);
            }
            std::thread::sleep(std::time::Duration::from_millis(poll_interval));
            elapsed += poll_interval;
            if elapsed % 5000 == 0 {
                info!("Still connecting... {}ms", elapsed);
            }
        }
        info!("WiFi connected after {}ms", elapsed);

        // Wait for IP
        info!("Waiting for IP address...");
        elapsed = 0;
        loop {
            let ip_info = self.wifi.sta_netif().get_ip_info()?;
            if !ip_info.ip.is_unspecified() {
                info!("Got IP: {:?}", ip_info.ip);
                break;
            }
            if elapsed >= timeout_ms {
                bail!("DHCP timeout after {}ms", timeout_ms);
            }
            std::thread::sleep(std::time::Duration::from_millis(poll_interval));
            elapsed += poll_interval;
        }

        Ok(())
    }

    pub fn ensure_connected(&mut self) -> Result<()> {
        if self.wifi.is_connected().unwrap_or(false) {
            return Ok(());
        }

        info!("WiFi disconnected, attempting to reconnect...");

        if let Err(e) = self.wifi.connect() {
            info!("WiFi connect() failed: {:?}, attempting full restart...", e);
            let _ = self.wifi.disconnect();
            let _ = self.wifi.stop();
            std::thread::sleep(std::time::Duration::from_millis(1000));
            self.wifi.start()?;
            self.wifi.connect()?;
        }

        let timeout_ms = 30_000u64;
        let poll_interval = 500u64;
        let mut elapsed = 0u64;

        while !self.wifi.is_connected()? {
            if elapsed >= timeout_ms {
                bail!("WiFi reconnection timeout after {}ms", timeout_ms);
            }
            std::thread::sleep(std::time::Duration::from_millis(poll_interval));
            elapsed += poll_interval;
            if elapsed % 5000 == 0 {
                info!("Reconnecting... {}ms", elapsed);
            }
        }
        info!("WiFi reconnected after {}ms", elapsed);

        // Wait for IP
        elapsed = 0;
        loop {
            let ip_info = self.wifi.sta_netif().get_ip_info()?;
            if !ip_info.ip.is_unspecified() {
                info!("Got IP: {:?}", ip_info.ip);
                break;
            }
            if elapsed >= timeout_ms {
                bail!("DHCP timeout after {}ms", timeout_ms);
            }
            std::thread::sleep(std::time::Duration::from_millis(poll_interval));
            elapsed += poll_interval;
        }

        Ok(())
    }
}
