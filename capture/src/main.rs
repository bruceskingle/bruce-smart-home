//! A Capture Device: an ESP32-S3 with PSRAM and an SD card, which polls what Bruce's Smart Home needs while the computer
//! it runs on sleeps, keeps it, and gives it to the app, which reads it from the device's store, see db.rs: the Tesla
//! gateway, see tesla.rs, myenergi's zappi, see myenergi.rs, and Octopus's smart charging, see octopus.rs, on a
//! Waveshare ESP32-S3-LCD-1.47B, whose small screen shows a graph of the gateway's power, and the device's status when
//! BOOT is pressed or it is tipped, a Waveshare ESP32-S3-RLCD-4.2, whose black and white screen shows the clock and
//! numbers, and the status when KEY is pressed, or a Guition JC3248W535C, whose 3.5" screen shows a graph of the
//! gateway's power, and the status when BOOT is pressed, see board and screen.rs. Boards without PSRAM, e.g. the CYD
//! and the ESP32-C6 boards, have too little memory for the store and HTTPS, and are not capture devices.

#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod battery;
#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod climate;
mod connections;
mod day_files;
mod db;
mod device;
// The kinds of records the device writes, and their items, shared with the app, see db.rs, some of which only the app
// uses.
#[allow(dead_code)]
#[path = "../../src/device_kinds.rs"]
mod device_kinds;
mod domain;
mod graph;
mod myenergi;
mod octopus;
mod records;
mod screen;
mod slots;
mod tesla;

use bruce_esp_idf::Esp32Platform;

/// What differs between the boards: the screen's orientation, and how much of the top of it the clock takes, the
/// numbers having the rest, see screen.rs.
#[cfg(feature = "wave-esp32s3-lcd-147")]
mod board {
    use std::sync::Arc;

    use bruce_embedded_std::DisplayOrientation;
    use bruce_esp_idf::Esp32PlatformBuilder;

    use crate::screen::Panel;

    /// The C6 Touch-LCD-1.47's screen, 320 by 172, on an ESP32-S3 without touch: the graph of the gateway's power, with
    /// a small digital clock, see screen.rs, which ticks every second, or until there is a graph, 8 lines of numbers.
    pub const ORIENTATION: DisplayOrientation = DisplayOrientation::Rotate270;
    pub const CLOCK_HEIGHT: u32 = 0;
    pub const GRAPH: bool = true;
    pub const SCREEN_SCHEDULE: &str = "* * * * * *";

    /// The screen's orientation, and BOOT, which shows the status, as a swipe does on the C6 board, having no touch, as
    /// does tipping the board, see screen.rs's Tip.
    pub fn configure(
        builder: Esp32PlatformBuilder,
        panel: &Arc<Panel>,
        _db: &Arc<crate::db::Db>,
    ) -> anyhow::Result<Esp32PlatformBuilder> {
        let panel = panel.clone();
        Ok(builder
            .with_display_orientation(ORIENTATION)?
            .with_boot_button(move |_platform| {
                Ok(Box::new(move || {
                    panel.show_status();
                    Ok(())
                }))
            }))
    }

    /// Nothing, as the board has no speaker.
    pub fn started(_runner: &bruce_esp_idf::Esp32PlatformRunner) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "guition-jc3248w535c")]
mod board {
    use std::sync::Arc;

    use bruce_esp_idf::Esp32PlatformBuilder;

    use crate::screen::Panel;

    /// 480 by 320, always landscape, with its USB port on the right, see the platform's axs15231b.rs: the graph of the
    /// gateway's power, with a small digital clock, see screen.rs, which ticks every second, or until there is a graph,
    /// the lines of numbers.
    pub const CLOCK_HEIGHT: u32 = 0;
    pub const GRAPH: bool = true;
    pub const SCREEN_SCHEDULE: &str = "* * * * * *";

    /// BOOT shows the status, as KEY does on the RLCD, as a swipe does, see screen.rs.
    pub fn configure(
        builder: Esp32PlatformBuilder,
        panel: &Arc<Panel>,
        _db: &Arc<crate::db::Db>,
    ) -> anyhow::Result<Esp32PlatformBuilder> {
        let panel = panel.clone();
        Ok(builder.with_boot_button(move |_platform| {
            Ok(Box::new(move || {
                panel.show_status();
                Ok(())
            }))
        }))
    }

    /// Nothing, as its speaker is not driven yet.
    pub fn started(_runner: &bruce_esp_idf::Esp32PlatformRunner) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "wave-esp32s3-rlcd-42")]
mod board {
    use std::sync::Arc;

    use bruce_esp_idf::features::analog_clock::AnalogClock;
    use bruce_esp_idf::{Esp32PlatformBuilder, Esp32PlatformRunner};
    use embedded_graphics::prelude::{Point, Size};
    use embedded_graphics::primitives::Rectangle;

    use crate::screen::Panel;

    /// 398 by 298, black and white: the clock, with 11 lines of numbers under it, in the board's smaller
    /// console font, as the graph's series would all be black, redrawn every other second, as the display is updated
    /// at most every 200 ms anyway.
    pub const CLOCK_HEIGHT: u32 = 130;
    pub const GRAPH: bool = false;
    pub const SCREEN_SCHEDULE: &str = "*/2 * * * * *";

    /// The clock, the log of the board's temperature and humidity sensor, see climate.rs, and the KEY button, which
    /// shows the status, as a swipe does on the touch screen.
    pub fn configure(
        builder: Esp32PlatformBuilder,
        panel: &Arc<Panel>,
        db: &Arc<crate::db::Db>,
    ) -> anyhow::Result<Esp32PlatformBuilder> {
        let panel = panel.clone();
        Ok(builder
            .with_feature(Box::new(crate::climate::ClimateCapture::new(panel.clone(), db.clone())))?
            // Always enabled, as the screen keeps its top for it, CLOCK_HEIGHT, which a new device would leave empty.
            .with_required_feature(Box::new(
                AnalogClock::builder()
                    .with_layout(|bounding_box: &Rectangle| {
                        Rectangle::new(
                            Point::new(bounding_box.top_left.x + 1, bounding_box.top_left.y + 1),
                            Size::new(bounding_box.size.width - 2, CLOCK_HEIGHT - 2),
                        )
                    })
                    .build()?,
            ))?
            .with_key_button(move |_platform| {
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
    pub fn started(runner: &Esp32PlatformRunner) -> anyhow::Result<()> {
        let speaker = runner.platform.speaker();
        std::thread::Builder::new()
            .stack_size(bruce_embedded_std::thread_stack(8192))
            .spawn(move || {
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
    let db = db::Db::new();
    // The Tesla gateway's, kept between polls, which the internet polls close for the RAM.
    let gateway = connections::KeptConnection::default();

    // The store first, so that it is open when the features start, and their first polls put their records in it.
    let builder = builder.with_feature(Box::new(db::StoreFeature::new(db.clone(), panel.clone())))?;
    let runner = board::configure(builder, &panel, &db)?
        .with_feature(Box::new(tesla::TeslaCapture::new(
            panel.clone(),
            db.clone(),
            gateway.clone(),
        )))?
        .with_feature(Box::new(myenergi::MyenergiCapture::new(
            panel.clone(),
            db.clone(),
            gateway.clone(),
        )))?
        .with_feature(Box::new(octopus::OctopusCapture::new(panel.clone(), db, gateway)))?
        .with_firmware(bruce_esp_idf::firmware!())
        .build()?;
    board::started(&runner)?;

    runner.start()
}
