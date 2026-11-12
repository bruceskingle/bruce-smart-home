//! The RLCD board's battery on the screen, a line below the temperature and humidity, see Panel::set, with its voltage
//! and how full that says it is, see bruce_esp_idf::battery, read every 10 seconds, and in the log every minute.

use std::sync::Arc;

use log::{info, warn};
use bruce_embedded_std::platform::PlatformInitializer;
use bruce_embedded_std::task::scheduler::ScheduledTask;
use bruce_esp_idf::{Esp32Platform, Esp32PlatformInitializer};

use crate::screen::Panel;

/// Where its line goes on the screen, after the climate's, see climate.rs.
const PANEL_ORDER: u8 = 5;

/// Every 10 seconds.
const SCHEDULE: &str = "*/10 * * * * *";

/// How many readings go by between those in the log, a minute's.
const LOG_EVERY: u32 = 6;

struct BatteryTask {
    panel: Arc<Panel>,
    /// Readings since the last in the log.
    since_logged: u32,
}

/// Shows the battery on the screen from now on.
pub fn start(panel: Arc<Panel>, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<()> {
    initializer.add_task(Box::new(BatteryTask { panel, since_logged: LOG_EVERY }), SCHEDULE)
}

impl ScheduledTask<Esp32Platform> for BatteryTask {
    fn run(&mut self, sparko: &mut Esp32Platform) -> anyhow::Result<()> {
        let line = match sparko.battery().map(|battery| battery.read()) {
            Some(Ok(reading)) => {
                self.since_logged += 1;
                if self.since_logged >= LOG_EVERY {
                    self.since_logged = 0;
                    info!("Battery {:.2} V, {}%", reading.volts, reading.percent);
                }
                (format!("Battery {:.2} V  {}%", reading.volts, reading.percent), false)
            },
            Some(Err(error)) => {
                warn!("Unable to read the battery, {error:#}");
                (format!("Battery: {error:#}"), true)
            },
            None => (String::from("Battery: not readable"), true),
        };
        self.panel.set(PANEL_ORDER, vec![line]);
        Ok(())
    }

    fn name(&self) -> &str {
        "Battery"
    }
}
