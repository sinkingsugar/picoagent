//! Task watchdog management for the main task.
//!
//! The main task is explicitly subscribed to the TWDT so that hangs in
//! non-blocking code (between long operations) are caught. Before known
//! long operations (Telegram poll, LLM calls), the task is deregistered
//! and re-registered afterwards.

use esp_idf_svc::sys;
use log::debug;

/// Subscribe the current task to the Task Watchdog Timer.
pub fn subscribe() {
    unsafe {
        let handle = sys::xTaskGetCurrentTaskHandle();
        let err = sys::esp_task_wdt_add(handle);
        if err != 0 {
            log::warn!("esp_task_wdt_add failed: {err}");
        } else {
            debug!("Main task subscribed to TWDT");
        }
    }
}

/// Unsubscribe the current task from the Task Watchdog Timer.
/// Call before operations that block longer than the WDT timeout (60s).
pub fn unsubscribe() {
    unsafe {
        let handle = sys::xTaskGetCurrentTaskHandle();
        let err = sys::esp_task_wdt_delete(handle);
        if err != 0 {
            log::warn!("esp_task_wdt_delete failed: {err}");
        }
    }
}

/// Feed the watchdog for the current task.
pub fn feed() {
    unsafe {
        sys::esp_task_wdt_reset();
    }
}
