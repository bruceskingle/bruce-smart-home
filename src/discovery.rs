//! Finding the boards on the network by mDNS, with bruce-embedded-flash's discovery, which the flasher shares, see its
//! discovery.rs, and telling the capture devices among them by their firmware.

pub use bruce_embedded_flash::discovery::Found;

/// The capture firmware's name, in a board's `fw` fact, which tells a capture device from another board.
pub const CAPTURE_FIRMWARE: &str = "bruce-smart-home-capture";

/// Whether a board found runs the capture firmware.
pub trait CaptureDevice {
    fn is_capture_device(&self) -> bool;
}

impl CaptureDevice for Found {
    fn is_capture_device(&self) -> bool {
        self.fact("fw") == Some(CAPTURE_FIRMWARE)
    }
}

/// The boards which answer, see bruce_embedded_flash::discovery::browse, on a thread of their own, as it blocks while
/// it listens.
pub async fn browse() -> anyhow::Result<Vec<Found>> {
    tokio::task::spawn_blocking(bruce_embedded_flash::discovery::browse).await?
}
