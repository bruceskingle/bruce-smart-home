//! What the app is told about the device with each feature's status, at /api/store/info, see db.rs, e.g. when and why it
//! last started.

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;

use crate::db::Db;
use crate::records::{LastPoll, format_time};

/// The firmware which took the records, in each one's firmware.
pub const FIRMWARE: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));

/// What took the records, in each one's capturedBy, which the app shows as their source: the device's name on the
/// network, e.g. cyd, or if it has none, the firmware.
pub fn captured_by(sparko: &bruce_esp_idf::Esp32Platform) -> String {
    sparko.hostname().unwrap_or_else(|| FIRMWARE.to_string())
}

/// Logs the free heap and its largest free block before an internet connection, whose TLS needs tens of KB, so that a
/// crash for want of memory can be seen coming, see bruce_esp_idf::panics.
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
        Device { boot_time: Utc::now() - up_time, reset_reason: bruce_esp_idf::reset_reason(), schedule: schedule.to_string() }
    }

    /// The details, with whether the feature's records are kept in the store, db, and how its latest poll went, as the
    /// screen shows it, so that the app can show a feature which is failing.
    pub fn to_json(&self, db: &Db, last_poll: Option<&LastPoll>) -> serde_json::Value {
        json!({
            "name": env!("CARGO_PKG_NAME"),
            "version": env!("CARGO_PKG_VERSION"),
            "now": format_time(Utc::now()),
            "bootTime": format_time(self.boot_time),
            "resetReason": self.reset_reason,
            "schedule": self.schedule,
            "storage": if db.store().is_some() { "store" } else { "none" },
            "storageProblem": db.problem(),
            // The SD card's size and free space, if the records are kept on one.
            "sdCard": db.sd_card(),
            "freeHeap": unsafe { esp_idf_sys::esp_get_free_heap_size() },
            "minimumFreeHeap": unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() },
            "largestFreeBlock": unsafe { esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_DEFAULT) },
            "failedAllocations": bruce_esp_idf::panics::failed_allocations(),
            // Why it crashed before this start, if it did.
            "lastCrash": bruce_esp_idf::panics::last(),
            "lastPoll": last_poll.map(LastPoll::to_json),
        })
    }
}
