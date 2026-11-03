//! What the app is told about the device with each feature's records, e.g. when and why it last started.

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;

use crate::day_files::DayFiles;
use crate::records::format_time;

/// The firmware which took the records, in each one's firmware.
pub const FIRMWARE: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));

/// What took the records, in each one's capturedBy, which the app shows as their source: the device's name on the
/// network, e.g. cyd, or if it has none, the firmware.
pub fn captured_by(sparko: &sparko_esp_idf::Esp32Platform) -> String {
    sparko.hostname().unwrap_or_else(|| FIRMWARE.to_string())
}

/// Logs the free heap and its largest free block before an internet connection, whose TLS needs tens of KB, so that a
/// crash for want of memory can be seen coming, see sparko_esp_idf::panics.
pub fn log_memory(what: &str) {
    let free = unsafe { esp_idf_sys::esp_get_free_heap_size() };
    let largest = unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_DEFAULT) };
    log::info!("{what}: free heap {free}, largest block {largest}");
}

pub struct Device {
    /// When the device last started, and why, e.g. Brownout, which shows power dips.
    boot_time: DateTime<Utc>,
    reset_reason: &'static str,
    /// When the feature polls, a cron spec, so that the app can read its records soon after, when the device is not
    /// busy polling.
    schedule: String,
}

impl Device {
    pub fn new(schedule: &str) -> Device {
        let up_time = TimeDelta::microseconds(unsafe { esp_idf_sys::esp_timer_get_time() });
        Device { boot_time: Utc::now() - up_time, reset_reason: sparko_esp_idf::reset_reason(), schedule: schedule.to_string() }
    }

    /// The details, with where the feature's records are kept, and how many of them are in RAM, out of how many.
    pub fn to_json(&self, files: &DayFiles, in_ram: usize, ram_capacity: usize) -> serde_json::Value {
        json!({
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
            "now": format_time(Utc::now()),
            "bootTime": format_time(self.boot_time),
            "resetReason": self.reset_reason,
            "schedule": self.schedule,
            "storage": files.storage(),
            "storageProblem": files.problem(),
            "recordsInRam": in_ram,
            "ramCapacity": ram_capacity,
            "freeHeap": unsafe { esp_idf_sys::esp_get_free_heap_size() },
            "minimumFreeHeap": unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() },
            "largestFreeBlock": unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_DEFAULT) },
            "failedAllocations": sparko_esp_idf::panics::failed_allocations(),
            // Why it crashed before this start, if it did.
            "lastCrash": sparko_esp_idf::panics::last(),
        })
    }
}
