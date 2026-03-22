//! Storage abstraction.
//!
//! Trait-based so you can swap implementations later.
//! For v1, we use SPIFFS on the "storage" partition.

pub mod spiffs;

use anyhow::Result;

/// Simple key-value storage interface.
///
/// Keys are path-like strings ("session.json", "config/ph_min").
/// Values are UTF-8 strings (usually JSON).
pub trait Storage {
    fn read(&self, key: &str) -> Result<String>;
    fn write(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
    fn exists(&self, key: &str) -> bool;
}
