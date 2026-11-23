//! What the screen shows below the clock, see main.rs: the latest numbers, of which each feature which captures sets
//! its own lines, e.g. the power of each of the gateway's meters, see Panel::set, or, until any does, e.g. as none is
//! set up yet, where to find the device, so that it can be adopted, see Screen::unconfigured.
//!
//! On a board with a graph, see main.rs's board, it shows instead the graph of the gateway's power, see graph.rs, under
//! the latest power of each meter and a small clock, and, for a while after a swipe of the screen, the device's status,
//! e.g. its address, see Swipe, or, on a board with a motion sensor and no touch screen, after it is tipped, see Tip.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bruce_embedded_std::graphics::{Color, DisplayManager, DrawTargetResultExt};
use bruce_embedded_std::platform::PlatformInitializer;
use bruce_embedded_std::task::scheduler::ScheduledTask;
use bruce_esp_idf::{Esp32Platform, Esp32PlatformInitializer};
use chrono::Local;
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use rgb::RGB8;

use crate::device::captured_by;
use crate::graph::{self, History, Palette, Sample};

use crate::CLOCK_HEIGHT;

/// The longest a line is kept, the most which fit across either board in its console's font, see
/// DisplayManager::console: FONT_10X20, which fits 24 across the CYD's 240 pixels, and 32 across the Waveshare's 320.
const LINE_LENGTH: usize = 32;
/// The section of the gateway's meters' lines, see Panel::set, which the graph screen leaves out, as it shows them
/// above the graph.
pub const METERS_SECTION: u8 = 0;
/// How long the status shows after a swipe, see Swipe.
const STATUS_TIME: Duration = Duration::from_secs(5);
/// The height of the row of numbers above the graph, a line of FONT_10X20 and a gap.
const NUMBERS_HEIGHT: u32 = 22;
/// The row above the graph, in the console's font: the clock, "HH:MM:SS", a little in from the left, as the corners of
/// the Waveshare's screen are hidden, then the four numbers, each of 5 characters, e.g. "-12.3", spread over the rest,
/// up to graph::RIGHT_MARGIN from the right: 80, then 4 of 50 and the gaps, across 320 with FONT_10X20.
const CLOCK_LEFT: i32 = 2;
const CLOCK_CHARACTERS: u32 = 8;
const NUMBER_CHARACTERS: u32 = 5;

/// A line, and whether it is a warning, shown in yellow.
pub type Line = (String, bool);

/// What the screen shows below the clock: each feature's lines, in the order of the number each sets them with, and
/// the samples for the graph.
pub struct Panel {
    sections: Mutex<BTreeMap<u8, Vec<Line>>>,
    /// A problem which every screen shows, in yellow, e.g. that there is no SD card, see Panel::set_warning.
    warning: Mutex<Option<String>>,
    history: Mutex<History>,
    /// When the status, shown after a swipe, is to go, see Swipe.
    status_until: Mutex<Option<Instant>>,
    /// Changed whenever the lines or samples are, so the screen only redraws when it must.
    generation: AtomicU32,
    started: AtomicBool,
}

impl Panel {
    pub fn new() -> Arc<Panel> {
        Arc::new(Panel {
            sections: Mutex::new(BTreeMap::new()),
            warning: Mutex::new(None),
            history: Mutex::new(History::default()),
            status_until: Mutex::new(None),
            generation: AtomicU32::new(0),
            started: AtomicBool::new(false),
        })
    }

    /// Sets a feature's lines, cut to the width of the screen, order being where they go.
    pub fn set(&self, order: u8, mut lines: Vec<Line>) {
        for (line, _) in &mut lines {
            if let Some((index, _)) = line.char_indices().nth(LINE_LENGTH) {
                line.truncate(index);
            }
        }
        self.sections.lock().unwrap().insert(order, lines);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Sets the problem which every screen shows, in yellow, along the bottom of the graph, or first among the lines,
    /// e.g. that there is no SD card, so that it is seen without the status being asked for.
    pub fn set_warning(&self, warning: Option<String>) {
        *self.warning.lock().unwrap() = warning;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Adds a sample of the gateway's power to the graph.
    pub fn add_sample(&self, sample: Sample) {
        self.history.lock().unwrap().add(sample);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Shows the status for a while, see STATUS_TIME, after a swipe, or a press of the RLCD's KEY button.
    pub fn show_status(&self) {
        *self.status_until.lock().unwrap() = Some(Instant::now() + STATUS_TIME);
    }

    /// Starts drawing it, the first time a feature asks, from its init, which is whether or not it is enabled, so that
    /// a device with none enabled shows where to find it.
    pub fn start(self: &Arc<Self>, initializer: &mut Esp32PlatformInitializer) -> anyhow::Result<()> {
        if !self.started.swap(true, Ordering::SeqCst) {
            let screen = Screen {
                panel: self.clone(),
                shown: None,
                clock_shown: None,
                status_overlay: false,
                #[cfg(feature = "touch")]
                swipe: None,
                #[cfg(feature = "tip")]
                tip: None,
            };
            initializer.add_task(Box::new(screen), crate::board::SCREEN_SCHEDULE)?;
            // The RLCD's battery, below the temperature and humidity.
            #[cfg(feature = "wave-esp32s3-rlcd-42")]
            crate::battery::start(self.clone(), initializer)?;
        }
        Ok(())
    }
}

/// What the screen is showing.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Each feature's lines, below the clock, if there is one.
    Lines,
    Graph,
    Status,
}

struct Screen {
    panel: Arc<Panel>,
    /// What was last shown: the mode, the panel's and display's generations, and whether the device was adopted, so
    /// that it is only redrawn when any changes, e.g. after adoption's pairing code has been shown, see
    /// DisplayManager::generation.
    shown: Option<(Mode, u32, u32, bool)>,
    /// The time the graph's clock last showed, so it is drawn once a second.
    clock_shown: Option<String>,
    /// Whether the status is being shown, as an overlay, see run.
    status_overlay: bool,
    /// Kept, as the touch driver only keeps a weak reference to it, see Swipe.
    #[cfg(feature = "touch")]
    swipe: Option<Arc<dyn bruce_embedded_std::listener::Listener<bruce_esp_idf::touch::TouchPoint>>>,
    /// Whether the board has been tipped, on a board with a motion sensor, None until the sensor has been started.
    #[cfg(feature = "tip")]
    tip: Option<Tip>,
}

impl Screen {
    /// Where to find the device, and what it needs, shown while no feature has set any lines, e.g. as none is
    /// enabled, as it has not been set up yet.
    fn unconfigured(platform: &Esp32Platform, adopted: bool) -> Vec<Line> {
        let needs = if adopted {
            "Nothing set up yet"
        } else {
            "Not adopted yet"
        };
        vec![
            (captured_by(platform), false),
            (address(platform), false),
            (needs.to_string(), true),
            ("Find it on the app's".to_string(), false),
            ("Devices page".to_string(), false),
        ]
    }

    /// The device's status, shown after a swipe: its name and address, how long it has been up, its heap, and the
    /// features' lines, other than the meters', which are above the graph.
    fn status(&self, platform: &Esp32Platform) -> Vec<Line> {
        let up = unsafe { bruce_esp_idf::sys::esp_timer_get_time() } / 1_000_000;
        let heap = unsafe { bruce_esp_idf::sys::esp_get_free_heap_size() } / 1024;
        let minimum = unsafe { bruce_esp_idf::sys::esp_get_minimum_free_heap_size() } / 1024;
        let mut lines = vec![
            (captured_by(platform), false),
            (address(platform), false),
            (
                format!(
                    "Up {}d {:02}:{:02}  heap {heap}K/{minimum}K",
                    up / 86400,
                    up / 3600 % 24,
                    up / 60 % 60
                ),
                false,
            ),
        ];
        let sections = self.panel.sections.lock().unwrap();
        lines.extend(
            sections
                .iter()
                .filter(|(order, _)| **order != METERS_SECTION)
                .flat_map(|(_, lines)| lines.clone()),
        );
        lines
    }

    /// Listens for swipes, on a board with a touch screen, once the platform is running.
    #[cfg(feature = "touch")]
    fn listen(&mut self, platform: &mut Esp32Platform) {
        if self.swipe.is_none() {
            let swipe: Arc<dyn bruce_embedded_std::listener::Listener<_>> = Arc::new(Swipe::new(self.panel.clone()));
            platform.touch_driver.add_listener(&swipe);
            self.swipe = Some(swipe);
        }
    }

    /// Watches for the board being tipped, on a board with a motion sensor and no touch screen: starts the sensor the
    /// first time, which calibrates it in the background for a few seconds, then, once it has, reads it each time the
    /// screen is drawn, once a second. Its readings before then are zero, which would be taken as where it rests.
    #[cfg(feature = "tip")]
    fn listen(&mut self, platform: &mut Esp32Platform) {
        match self.tip.as_mut() {
            None => match platform.imu_manager.start(Duration::from_millis(50)) {
                Ok(()) => self.tip = Some(Tip::default()),
                Err(error) => {
                    log::error!("Unable to start the motion sensor, so tipping the board does nothing: {error:#}");
                    self.tip = Some(Tip::broken());
                }
            },
            Some(_) if !platform.imu_manager.is_calibrated() => {}
            Some(tip) => {
                let reading = platform.imu_manager.read_tilt();
                if tip.tipped(reading.roll, reading.pitch, Instant::now()) {
                    log::info!("Tipped, showing the status");
                    self.panel.show_status();
                }
            }
        }
    }

    #[cfg(not(any(feature = "touch", feature = "tip")))]
    fn listen(&mut self, _platform: &mut Esp32Platform) {}
}

/// A tip of the board, on one with a motion sensor and no touch screen, which shows the status as a swipe does: tipped
/// more than ANGLE degrees, either way, from how it rests, which is wherever it has been still for SETTLE, as it may
/// stand on its side or lie flat. Edge triggered, so that leaving it tipped shows the status once.
#[cfg(any(feature = "tip", test))]
#[derive(Default)]
struct Tip {
    /// The roll and pitch it rests at, None until the first reading, or if the sensor would not start.
    rest: Option<(f32, f32)>,
    /// Whether it is tipped, and since when it has been, to take a position held for SETTLE as its new rest.
    tipped_since: Option<Instant>,
    /// Whether the sensor would not start, so that it is not read.
    broken: bool,
}

#[cfg(any(feature = "tip", test))]
impl Tip {
    /// How far it must be tipped from its rest, in degrees.
    const ANGLE: f32 = 30.0;
    /// How long it must be held in a new position for that to become its rest.
    const SETTLE: Duration = Duration::from_secs(30);

    #[cfg(feature = "tip")]
    fn broken() -> Tip {
        Tip {
            broken: true,
            ..Tip::default()
        }
    }

    /// Takes a reading, roll and pitch in degrees, and says whether the board has just been tipped.
    fn tipped(&mut self, roll: f32, pitch: f32, now: Instant) -> bool {
        if self.broken {
            return false;
        }
        let Some((rest_roll, rest_pitch)) = self.rest else {
            self.rest = Some((roll, pitch));
            return false;
        };
        let away = angle_between(roll, rest_roll).max(angle_between(pitch, rest_pitch));
        if away <= Self::ANGLE {
            self.tipped_since = None;
            return false;
        }
        match self.tipped_since {
            None => {
                self.tipped_since = Some(now);
                true
            }
            Some(since) => {
                if now.duration_since(since) >= Self::SETTLE {
                    self.rest = Some((roll, pitch));
                    self.tipped_since = None;
                }
                false
            }
        }
    }
}

/// The difference between two angles in degrees, from 0 to 180, so that -179 and 179 are 2 apart.
#[cfg(any(feature = "tip", test))]
fn angle_between(a: f32, b: f32) -> f32 {
    let difference = (a - b).rem_euclid(360.0);
    difference.min(360.0 - difference)
}

/// The device's address on the Wi-Fi, for the screen, as an app is given it, with its port on the simulator.
fn address(platform: &Esp32Platform) -> String {
    platform.address().unwrap_or_else(|| "Not on the Wi-Fi".to_string())
}

/// Draws lines in area, in the console's font, without its margin, which would leave room for fewer numbers.
fn draw_lines<DM: DisplayManager>(manager: &mut DM, area: Rectangle, lines: &[Line]) -> anyhow::Result<()> {
    let background = manager.map_color(&Color::Black);
    let console = manager.console();
    let text = MonoTextStyle::new(console.font, manager.map_color(&Color::White));
    let warning = MonoTextStyle::new(console.font, manager.map_color(&Color::Yellow));
    let display = manager.display();
    area.into_styled(PrimitiveStyle::with_fill(background))
        .draw(display)
        .anyhow()?;
    let line_height = console.line_height() as i32;
    let max_lines = (area.size.height as i32 / line_height) as usize;
    let fit = (area.size.width / console.character_width()) as usize;
    for (index, (line, is_warning)) in lines.iter().take(max_lines).enumerate() {
        let position = area.top_left + Point::new(0, index as i32 * line_height);
        let style = if *is_warning { warning } else { text };
        let line = line
            .char_indices()
            .nth(fit)
            .map_or(line.as_str(), |(end, _)| &line[..end]);
        Text::with_baseline(line, position, style, Baseline::Top)
            .draw(display)
            .anyhow()?;
    }
    Ok(())
}

/// Draws the latest power of each meter, in kW, in its colour on the graph, along the top of area.
fn draw_numbers<DM: DisplayManager>(manager: &mut DM, area: Rectangle, latest: Option<&Sample>) -> anyhow::Result<()> {
    let background = manager.map_color(&Color::Black);
    let colours = graph::SERIES.map(|(_, colour)| manager.map_rgb8(&colour));
    let console = manager.console();
    let display = manager.display();
    // After the clock, each number at the right of an equal share of what is left.
    let left = CLOCK_LEFT + (CLOCK_CHARACTERS * console.character_width()) as i32;
    let right = area.size.width as i32 - graph::RIGHT_MARGIN as i32;
    let share = (right - left) / colours.len() as i32;
    let number_width = (NUMBER_CHARACTERS * console.character_width()) as i32;
    for (index, colour) in colours.into_iter().enumerate() {
        let style = MonoTextStyleBuilder::new()
            .font(console.font)
            .text_color(colour)
            .background_color(background)
            .build();
        let number = latest.map_or_else(
            || "    -".to_string(),
            |sample| format!("{:>5.1}", sample.power[index] / 1000.0),
        );
        let x = left + (index as i32 + 1) * share - number_width;
        let position = area.top_left + Point::new(x, 0);
        Text::with_baseline(&number, position, style, Baseline::Top)
            .draw(display)
            .anyhow()?;
    }
    Ok(())
}

/// Draws a warning, in yellow on black, at the bottom left of area, the graph's, see Panel::set_warning.
fn draw_warning<DM: DisplayManager>(manager: &mut DM, area: Rectangle, warning: &str) -> anyhow::Result<()> {
    let console = manager.console();
    let style = MonoTextStyleBuilder::new()
        .font(console.font)
        .text_color(manager.map_color(&Color::Yellow))
        .background_color(manager.map_color(&Color::Black))
        .build();
    let bottom = area.top_left.y + area.size.height as i32;
    let position = Point::new(area.top_left.x + CLOCK_LEFT, bottom - console.line_height() as i32);
    Text::with_baseline(warning, position, style, Baseline::Top)
        .draw(manager.display())
        .anyhow()?;
    Ok(())
}

/// Draws the time, at the top left of area.
fn draw_clock<DM: DisplayManager>(manager: &mut DM, area: Rectangle, time: &str) -> anyhow::Result<()> {
    let console = manager.console();
    let style = MonoTextStyleBuilder::new()
        .font(console.font)
        .text_color(manager.map_color(&Color::White))
        .background_color(manager.map_color(&Color::Black))
        .build();
    let position = area.top_left + Point::new(CLOCK_LEFT, 0);
    Text::with_baseline(time, position, style, Baseline::Top)
        .draw(manager.display())
        .anyhow()?;
    Ok(())
}

impl ScheduledTask<Esp32Platform> for Screen {
    fn run(&mut self, platform: &mut Esp32Platform) -> anyhow::Result<()> {
        self.listen(platform);
        let generation = self.panel.generation.load(Ordering::SeqCst);
        let adopted = platform.is_adopted();
        let status = self
            .panel
            .status_until
            .lock()
            .unwrap()
            .is_some_and(|until| Instant::now() < until);
        let has_samples = self.panel.history.lock().unwrap().latest().is_some();
        let mode = match (status, crate::board::GRAPH && has_samples) {
            (true, _) => Mode::Status,
            (false, true) => Mode::Graph,
            (false, false) => Mode::Lines,
        };
        let warning = self.panel.warning.lock().unwrap().clone();
        let mut lines = match mode {
            Mode::Status => self.status(platform),
            _ => {
                let lines: Vec<Line> = self
                    .panel
                    .sections
                    .lock()
                    .unwrap()
                    .values()
                    .flatten()
                    .cloned()
                    .collect();
                if lines.is_empty() {
                    Self::unconfigured(platform, adopted)
                } else {
                    lines
                }
            }
        };
        if let Some(warning) = &warning {
            lines.insert(0, (warning.clone(), true));
        }

        let display_manager = platform.display_manager.clone();
        let mut manager = display_manager.lock().unwrap();
        // The status is shown as an overlay, so that the clock, which draws only its hands as they move, draws nothing
        // over it, and draws itself in full once it ends, which clears the screen, see DisplayManager::set_overlay.
        // Another's overlay, e.g. adoption's pairing code, is left alone.
        if mode == Mode::Status && !self.status_overlay {
            if manager.overlay_active() {
                return Ok(());
            }
            manager.set_overlay(true)?;
            self.status_overlay = true;
        } else if mode != Mode::Status && self.status_overlay {
            manager.set_overlay(false)?;
            self.status_overlay = false;
        } else if manager.overlay_active() && !self.status_overlay {
            return Ok(());
        }
        let bounding_box = manager.display().bounding_box();
        let shown = (mode, generation, manager.generation(), adopted);
        let changed = self.shown != Some(shown);
        // Everything is drawn again after the mode changes, or the screen has been cleared.
        let whole = self
            .shown
            .is_none_or(|(mode_shown, _, display_shown, _)| mode_shown != mode || display_shown != shown.2);
        self.shown = Some(shown);

        match mode {
            Mode::Lines if changed => {
                let area = Rectangle::new(
                    Point::new(bounding_box.top_left.x, bounding_box.top_left.y + CLOCK_HEIGHT as i32),
                    Size::new(
                        bounding_box.size.width,
                        bounding_box.size.height.saturating_sub(CLOCK_HEIGHT),
                    ),
                );
                draw_lines(&mut *manager, area, &lines)?;
            }
            Mode::Status if changed => draw_lines(&mut *manager, bounding_box, &lines)?,
            Mode::Graph => {
                if whole {
                    let background = manager.map_color(&Color::Black);
                    bounding_box
                        .into_styled(PrimitiveStyle::with_fill(background))
                        .draw(manager.display())
                        .anyhow()?;
                    self.clock_shown = None;
                }
                if changed {
                    let history = self.panel.history.lock().unwrap();
                    draw_numbers(&mut *manager, bounding_box, history.latest())?;
                    let palette = Palette {
                        background: manager.map_color(&Color::Black),
                        grid: manager.map_rgb8(&RGB8::new(0x50, 0x50, 0x58)),
                        zero: manager.map_rgb8(&RGB8::new(0xb0, 0xb0, 0xb8)),
                        scale: manager.map_rgb8(&RGB8::new(0xe0, 0xe0, 0xe0)),
                        series: graph::SERIES.map(|(_, colour)| manager.map_rgb8(&colour)),
                    };
                    let area = Rectangle::new(
                        bounding_box.top_left + Point::new(0, NUMBERS_HEIGHT as i32),
                        Size::new(
                            bounding_box.size.width,
                            bounding_box.size.height.saturating_sub(NUMBERS_HEIGHT),
                        ),
                    );
                    let font = manager.console().font;
                    graph::draw(manager.display(), &palette, &history, area, font)?;
                    if let Some(warning) = &warning {
                        draw_warning(&mut *manager, area, warning)?;
                    }
                }
                let time = Local::now().format("%H:%M:%S").to_string();
                if self.clock_shown.as_ref() != Some(&time) {
                    draw_clock(&mut *manager, bounding_box, &time)?;
                    self.clock_shown = Some(time);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "Panel"
    }
}

/// Shows the status, see Panel::show_status, when the screen is swiped: touched, and the touch moved far enough,
/// quickly enough, in any direction. The touch driver says only where the screen is touched, as often as it reads it,
/// so a touch which has not moved for a while is taken to be a new one.
#[cfg(feature = "touch")]
struct Swipe {
    panel: Arc<Panel>,
    /// Where and when the touch started, and when it was last moved, and whether it has made a swipe yet.
    touch: Mutex<Option<(i32, i32, Instant, bool)>>,
}

#[cfg(feature = "touch")]
impl Swipe {
    /// How far a touch must move to be a swipe, in pixels, and the longest gap between readings of one touch.
    const DISTANCE: i32 = 50;
    const GAP: Duration = Duration::from_millis(200);

    fn new(panel: Arc<Panel>) -> Swipe {
        Swipe {
            panel,
            touch: Mutex::new(None),
        }
    }
}

#[cfg(feature = "touch")]
impl bruce_embedded_std::listener::Listener<bruce_esp_idf::touch::TouchPoint> for Swipe {
    fn on_event(&self, point: &bruce_esp_idf::touch::TouchPoint) {
        let (x, y) = (point.x as i32, point.y as i32);
        let now = Instant::now();
        let mut touch = self.touch.lock().unwrap();
        match touch.as_mut() {
            Some((start_x, start_y, last, swiped)) if now.duration_since(*last) < Self::GAP => {
                *last = now;
                if !*swiped && (x - *start_x).abs().max((y - *start_y).abs()) >= Self::DISTANCE {
                    *swiped = true;
                    log::info!("Swipe, showing the status");
                    self.panel.show_status();
                }
            }
            _ => *touch = Some((x, y, now, false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Tip resting on its side, as the board stands, at roll 0 and pitch 90.
    fn standing(now: Instant) -> Tip {
        let mut tip = Tip::default();
        assert!(!tip.tipped(0.0, 90.0, now));
        tip
    }

    #[test]
    fn its_rest_is_where_it_starts_not_level() {
        let now = Instant::now();
        let mut tip = standing(now);
        assert!(!tip.tipped(5.0, 85.0, now + Duration::from_secs(1)));
    }

    #[test]
    fn a_tip_shows_the_status_once() {
        let now = Instant::now();
        let mut tip = standing(now);
        assert!(tip.tipped(40.0, 90.0, now + Duration::from_secs(1)));
        assert!(!tip.tipped(45.0, 90.0, now + Duration::from_secs(2)));
        // Back at rest, then tipped again, the other way.
        assert!(!tip.tipped(0.0, 90.0, now + Duration::from_secs(3)));
        assert!(tip.tipped(0.0, 50.0, now + Duration::from_secs(4)));
    }

    #[test]
    fn a_position_held_becomes_its_rest() {
        let now = Instant::now();
        let mut tip = standing(now);
        assert!(tip.tipped(0.0, 0.0, now + Duration::from_secs(1)));
        assert!(!tip.tipped(0.0, 0.0, now + Duration::from_secs(31)));
        // Now resting flat, so lying flat is not a tip, and standing up again is.
        assert!(!tip.tipped(0.0, 0.0, now + Duration::from_secs(32)));
        assert!(tip.tipped(0.0, 90.0, now + Duration::from_secs(33)));
    }

    #[test]
    fn angles_wrap_around() {
        assert_eq!(angle_between(-179.0, 179.0), 2.0);
        assert_eq!(angle_between(10.0, -20.0), 30.0);
    }
}
