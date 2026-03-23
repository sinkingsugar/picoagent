//! SPIFFS-based storage on the "storage" partition.
//!
//! ESP-IDF mounts SPIFFS as a VFS, so we use std::fs after mounting.
//! We call esp_vfs_spiffs_register directly to enable format_if_mount_failed.

use crate::storage::Storage;
use anyhow::{bail, Context, Result};
use esp_idf_svc::sys;
use log::{debug, info, warn};
use std::ffi::CString;
use std::fs;
use std::path::Path;

const MOUNT_POINT: &str = "/storage";
const PARTITION_LABEL: &str = "storage";
const MAX_FDS: usize = 5;

/// SPIFFS storage on ESP32 flash.
pub struct SpiffsStorage {
    _mount_path: CString,
    _partition_label: CString,
}

impl SpiffsStorage {
    /// Mount the SPIFFS partition. Auto-formats on first use.
    pub fn mount() -> Result<Self> {
        let mount_path =
            CString::new(MOUNT_POINT).map_err(|_| anyhow::anyhow!("invalid mount path"))?;
        let partition_label = CString::new(PARTITION_LABEL)
            .map_err(|_| anyhow::anyhow!("invalid partition label"))?;

        let conf = sys::esp_vfs_spiffs_conf_t {
            base_path: mount_path.as_ptr(),
            partition_label: partition_label.as_ptr(),
            max_files: MAX_FDS as _,
            format_if_mount_failed: true,
        };

        let ret = unsafe { sys::esp_vfs_spiffs_register(&conf) };
        if ret != 0 {
            bail!("SPIFFS mount failed: ESP error {ret}");
        }

        // Log partition info
        let mut total: usize = 0;
        let mut used: usize = 0;
        let info_ret =
            unsafe { sys::esp_spiffs_info(partition_label.as_ptr(), &mut total, &mut used) };
        if info_ret == 0 {
            info!(
                "SPIFFS mounted at {MOUNT_POINT}: {} KB total, {} KB used",
                total / 1024,
                used / 1024
            );
        } else {
            warn!("SPIFFS mounted but couldn't read info");
        }

        Ok(Self {
            _mount_path: mount_path,
            _partition_label: partition_label,
        })
    }

    fn full_path(key: &str) -> Result<String> {
        if key.contains("..") || key.starts_with('/') {
            bail!("invalid storage key: {key}");
        }
        Ok(format!("{MOUNT_POINT}/{key}"))
    }
}

impl Drop for SpiffsStorage {
    fn drop(&mut self) {
        unsafe {
            sys::esp_vfs_spiffs_unregister(self._partition_label.as_ptr());
        }
    }
}

impl Storage for SpiffsStorage {
    fn read(&self, key: &str) -> Result<String> {
        let path = Self::full_path(key)?;
        fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))
    }

    fn write(&self, key: &str, value: &str) -> Result<()> {
        let path = Self::full_path(key)?;
        debug!("Writing {} bytes to {}", value.len(), path);
        fs::write(&path, value).with_context(|| format!("failed to write {path}"))
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = Self::full_path(key)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => bail!("failed to delete {path}: {e}"),
        }
    }

    fn exists(&self, key: &str) -> bool {
        match Self::full_path(key) {
            Ok(path) => Path::new(&path).exists(),
            Err(_) => false,
        }
    }
}
