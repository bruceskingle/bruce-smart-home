//! A Capture Device: an ESP32 which polls what Marco Sparko needs while the computer it runs on sleeps, keeps it, and
//! gives it to the app, which reads it from the device's /api/capture/... pages: the Tesla gateway, see tesla.rs,
//! myenergi's zappi, see myenergi.rs, and Octopus's smart charging, see octopus.rs, on a CYD, whose screen shows a graph
//! of the gateway's power, as it has too little memory for the internet polls, a Waveshare ESP32-C6-Touch-LCD-1.47, whose small screen shows a graph of
//! the gateway's power, and the device's status when it is swiped, or a Waveshare ESP32-S3-RLCD-4.2, whose black and
//! white screen shows the clock and numbers, and the status when KEY is pressed, see board and screen.rs.

#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod battery;
#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod climate;
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
#[cfg(feature = "cyd-esp32-28")]
mod board {
    use std::sync::Arc;

    use sparko_embedded_std::DisplayOrientation;
    use sparko_esp_idf::Esp32PlatformBuilder;

    use crate::screen::Panel;

    /// 320 by 240, on its side, the way up it stands, with its USB port on the right (Rotate90 has it on the left): the
    /// graph of the gateway's power, with a small digital clock, as the Waveshare's, see screen.rs, which ticks every
    /// second, or until there is a graph, the lines of numbers. Without its internet polls, which it has too little
    /// memory for alongside the app's HTTPS requests, it is mostly a display of the graph.
    pub const ORIENTATION: DisplayOrientation = DisplayOrientation::Rotate270;
    pub const CLOCK_HEIGHT: u32 = 0;
    pub const GRAPH: bool = true;
    pub const SCREEN_SCHEDULE: &str = "* * * * * *";

    /// The screen's orientation.
    pub fn configure(builder: Esp32PlatformBuilder, _panel: &Arc<Panel>) -> anyhow::Result<Esp32PlatformBuilder> {
        builder.with_display_orientation(ORIENTATION)
    }

    /// Nothing, as the board has no speaker.
    pub fn started(_platform: &sparko_esp_idf::Esp32PlatformRunner) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "wave-esp32c6-touch-147")]
mod board {
    use std::sync::Arc;

    use sparko_embedded_std::DisplayOrientation;
    use sparko_esp_idf::Esp32PlatformBuilder;

    use crate::screen::Panel;

    /// 320 by 172, too small for the analog clock too, so the graph of the gateway's power, with a small digital
    /// clock, see screen.rs, which ticks every second, or until there is a graph, 8 lines of numbers.
    pub const ORIENTATION: DisplayOrientation = DisplayOrientation::Rotate270;
    pub const CLOCK_HEIGHT: u32 = 0;
    pub const GRAPH: bool = true;
    pub const SCREEN_SCHEDULE: &str = "* * * * * *";

    /// The screen's orientation; swipes show the status, see screen.rs.
    pub fn configure(builder: Esp32PlatformBuilder, _panel: &Arc<Panel>) -> anyhow::Result<Esp32PlatformBuilder> {
        builder.with_display_orientation(ORIENTATION)
    }

    /// Nothing, as the board has no speaker.
    pub fn started(_platform: &sparko_esp_idf::Esp32PlatformRunner) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod board {
    use std::sync::Arc;

    use embedded_graphics::prelude::{Point, Size};
    use embedded_graphics::primitives::Rectangle;
    use sparko_esp_idf::features::analog_clock::AnalogClock;
    use sparko_esp_idf::{Esp32PlatformBuilder, Esp32PlatformRunner};

    use crate::screen::Panel;

    /// 398 by 298, black and white: the clock, with 11 lines of numbers under it, in the board's smaller
    /// console font, as the graph's series would all be black, redrawn every other second, as the display is updated
    /// at most every 200 ms anyway.
    pub const CLOCK_HEIGHT: u32 = 130;
    pub const GRAPH: bool = false;
    pub const SCREEN_SCHEDULE: &str = "*/2 * * * * *";

    /// The clock, the log of the board's temperature and humidity sensor, see climate.rs, and the KEY button, which
    /// shows the status, as a swipe does on the touch screen.
    pub fn configure(builder: Esp32PlatformBuilder, panel: &Arc<Panel>) -> anyhow::Result<Esp32PlatformBuilder> {
        let panel = panel.clone();
        Ok(builder
            .with_feature(Box::new(crate::climate::ClimateCapture::new(panel.clone())))?
            .with_feature(Box::new(
                AnalogClock::builder()
                    .with_layout(|bounding_box: &Rectangle| {
                        Rectangle::new(
                            Point::new(bounding_box.top_left.x + 1, bounding_box.top_left.y + 1),
                            Size::new(bounding_box.size.width - 2, CLOCK_HEIGHT - 2),
                        )
                    })
                    .build()?,
            ))?
            .with_key_button(move |_sparko| {
                Ok(Box::new(move || {
                    panel.show_status();
                    Ok(())
                }))
            }))
    }

    /// Played on the speaker as the board starts.
    static STARTUP_SOUND: &[u8] = include_bytes!("../sounds/Startup.wav");

    /// Plays STARTUP_SOUND, at once, while the board joins the network, so in a thread of its own, at the volume set
    /// on the /config page.
    pub fn started(platform: &Esp32PlatformRunner) -> anyhow::Result<()> {
        let speaker = platform.sparko_std.speaker();
        std::thread::Builder::new().stack_size(8192).spawn(move || {
            if let Err(error) = speaker.play_wav(STARTUP_SOUND) {
                log::error!("Unable to play the startup sound: {error:#}");
            }
        })?;
        Ok(())
    }
}

pub use board::CLOCK_HEIGHT;

fn main() -> anyhow::Result<()> {
    let (builder, _remainder) = Esp32Platform::builder()?;
    let panel = screen::Panel::new();
    // The Tesla gateway's, kept between polls, which the internet polls close for the RAM.
    let gateway = connections::KeptConnection::default();

    let sparko_esp32 = board::configure(builder, &panel)?
        .with_feature(Box::new(tesla::TeslaCapture::new(panel.clone(), gateway.clone())))?
        .with_feature(Box::new(myenergi::MyenergiCapture::new(panel.clone(), gateway.clone())))?
        .with_feature(Box::new(octopus::OctopusCapture::new(panel.clone(), gateway)))?
        .with_firmware(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .build()?;
    board::started(&sparko_esp32)?;

    sparko_esp32.start()
}
