//! OTA (Over-The-Air) firmware updates.
//!
//! Downloads a firmware binary via HTTPS and flashes it.
//! Placeholder for now — enable when OTA partitions are added.

use anyhow::{bail, Result};
use log::info;

/// Perform OTA update from a URL.
///
/// Currently a stub — the partition table uses factory-only layout.
/// To enable OTA:
/// 1. Switch partitions.csv to dual OTA layout
/// 2. Uncomment the implementation below
pub fn update_from_url(_url: &str) -> Result<usize> {
    bail!(
        "OTA not enabled — partition table uses factory layout. \
           Switch to OTA partitions first."
    )
}

/// Reboot the device.
pub fn reboot() -> ! {
    info!("Rebooting...");
    esp_idf_svc::hal::reset::restart();
}
