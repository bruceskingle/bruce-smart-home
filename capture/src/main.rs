//! A Capture Device: an ESP32 which polls what Marco Sparko needs while the computer it runs on sleeps, keeps it, and
//! gives it to the app, which reads it from the device's /api/capture/... pages: the Tesla gateway, see tesla.rs,
//! myenergi's zappi, see myenergi.rs, and Octopus's smart charging, see octopus.rs, on a CYD, whose screen shows the
//! clock with the latest numbers below it, or a Waveshare ESP32-C6-Touch-LCD-1.47, whose small screen shows a graph of
//! the gateway's power, and the device's status when it is swiped, see board and screen.rs.

mod connections;
mod day_files;
mod device;
mod graph;
mod myenergi;
mod octopus;
mod records;
mod screen;
mod slots;
mod tesla;

use sparko_esp_idf::Esp32Platform;

/// What differs between the boards: the screen's orientation, and how much of the top of it the clock takes, the
/// numbers having the rest, see screen.rs.
#[cfg(feature = "cyd")]
mod board {
    use embedded_graphics::prelude::{Point, Size};
    use embedded_graphics::primitives::Rectangle;
    use sparko_embedded_std::DisplayOrientation;
    use sparko_esp_idf::Esp32PlatformBuilder;
    use sparko_esp_idf::features::analog_clock::AnalogClock;

    /// 240 by 320, 8 lines of numbers under the clock, and no graph, see screen.rs, redrawn every other second.
    pub const ORIENTATION: DisplayOrientation = DisplayOrientation::Rotate0;
    pub const CLOCK_HEIGHT: u32 = 160;
    pub const GRAPH: bool = false;
    pub const SCREEN_SCHEDULE: &str = "*/2 * * * * *";

    pub fn with_clock(builder: Esp32PlatformBuilder) -> anyhow::Result<Esp32PlatformBuilder> {
        builder.with_feature(Box::new(
            AnalogClock::builder()
                .with_layout(|bounding_box: &Rectangle| {
                    Rectangle::new(
                        Point::new(bounding_box.top_left.x + 1, bounding_box.top_left.y + 1),
                        Size::new(bounding_box.size.width - 2, CLOCK_HEIGHT - 2),
                    )
                })
                .build()?,
        ))
    }
}

#[cfg(feature = "waveshare-c6-touch")]
mod board {
    use sparko_embedded_std::DisplayOrientation;
    use sparko_esp_idf::Esp32PlatformBuilder;

    /// 320 by 172, too small for the analog clock too, so the graph of the gateway's power, with a small digital
    /// clock, see screen.rs, which ticks every second, or until there is a graph, 8 lines of numbers.
    pub const ORIENTATION: DisplayOrientation = DisplayOrientation::Rotate270;
    pub const CLOCK_HEIGHT: u32 = 0;
    pub const GRAPH: bool = true;
    pub const SCREEN_SCHEDULE: &str = "* * * * * *";

    pub fn with_clock(builder: Esp32PlatformBuilder) -> anyhow::Result<Esp32PlatformBuilder> {
        Ok(builder)
    }
}

pub use board::CLOCK_HEIGHT;

fn main() -> anyhow::Result<()> {
    let (builder, _remainder) = Esp32Platform::builder()?;
    let panel = screen::Panel::new();
    // The Tesla gateway's, kept between polls, which the internet polls close for the RAM.
    let gateway = connections::KeptConnection::default();

    let sparko_esp32 = board::with_clock(builder)?
        .with_feature(Box::new(tesla::TeslaCapture::new(panel.clone(), gateway.clone())))?
        .with_feature(Box::new(myenergi::MyenergiCapture::new(panel.clone(), gateway.clone())))?
        .with_feature(Box::new(octopus::OctopusCapture::new(panel.clone(), gateway)))?
        .with_display_orientation(board::ORIENTATION)?
        .with_firmware(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .build()?;

    sparko_esp32.start()
}
