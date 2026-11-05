//! A rolling graph of the gateway's power readings, see tesla.rs, one column of pixels for each 10 second sample, the
//! latest at the right, drawn by the screen on a board which has one, see screen.rs and main.rs's board.
//!
//! Each column is drawn in one go, see draw, from a buffer of its pixels, so that the graph changes without the
//! flicker of clearing it first, and without the RAM of a buffer for the whole of it.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use rgb::RGB8;
use sparko_embedded_std::graphics::DrawTargetResultExt;

/// The gateway's meters which are graphed, in the order of Sample::power, with the colour of each: the grid, the solar,
/// the battery and the house. Bright, as softer colours are too dark against the black on a small screen. The house is
/// the app's green, as on its graphs, e.g. Home on the Electricity page.
pub const SERIES: [(&str, RGB8); 4] = [
    ("site", RGB8::new(0xff, 0x50, 0x50)),
    ("solar", RGB8::new(0xff, 0xff, 0x00)),
    ("battery", RGB8::new(0xff, 0x40, 0xff)),
    ("load", RGB8::new(0x46, 0xee, 0x43)),
];
/// Drawn in this order, so that the house, which most often moves, is on top.
const DRAWING_ORDER: [usize; 4] = [0, 1, 2, 3];

/// How far apart the samples are, the Tesla poll's default schedule, see tesla.rs.
const STEP_SECONDS: i64 = 10;
/// The most samples kept, more than fit across any screen.
const CAPACITY: usize = 480;
/// How far back the samples kept go, which tesla.rs reads back from the SD card when the device starts, so that the graph
/// is not empty after a restart.
pub const HISTORY: chrono::TimeDelta = chrono::TimeDelta::seconds(CAPACITY as i64 * STEP_SECONDS);
/// The scale at the right: the most characters of a label, e.g. "-10", in kW, the gap before them, and the space left
/// at the right, where the corners of the Waveshare's screen are hidden, in pixels.
const SCALE_CHARACTERS: u32 = 3;
const SCALE_GAP: u32 = 4;
pub const RIGHT_MARGIN: u32 = 10;
/// How many samples each point is the median of, see smooth, as the gateway's readings, each of a moment, are spiky,
/// e.g. with a hot tap's element switching on and off, which the meters are read at slightly different moments of.
const SMOOTHING: usize = 5;
/// The width of the lines, in pixels.
const LINE_WIDTH: usize = 2;
/// The scale's steps, in whole kW, so the labels are whole numbers, the smallest which makes no more than MAX_STEPS of
/// them.
const STEPS: [f32; 5] = [1.0, 2.0, 5.0, 10.0, 20.0];
const MAX_STEPS: f32 = 4.0;

/// The power of each of the SERIES at a time, in W, as the gateway says it: the grid is positive when importing, the
/// battery when discharging.
#[derive(Clone, Copy)]
pub struct Sample {
    pub time: DateTime<Utc>,
    pub power: [f32; 4],
}

/// The latest samples, oldest first.
#[derive(Default)]
pub struct History {
    samples: VecDeque<Sample>,
}

impl History {
    /// Adds a sample, replacing one for the same time.
    pub fn add(&mut self, sample: Sample) {
        if self.samples.back().is_some_and(|last| last.time >= sample.time) {
            self.samples.pop_back();
        }
        self.samples.push_back(sample);
        while self.samples.len() > CAPACITY {
            self.samples.pop_front();
        }
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back()
    }

    /// The samples which fit across width columns, each in the column of its time, None where there is none, e.g. as
    /// the poll failed.
    fn columns(&self, width: usize) -> Vec<Option<[f32; 4]>> {
        let mut columns = vec![None; width];
        if let Some(latest) = self.samples.back() {
            for sample in &self.samples {
                let age = ((latest.time - sample.time).num_seconds() + STEP_SECONDS / 2) / STEP_SECONDS;
                if let Ok(age) = usize::try_from(age)
                    && age < width
                {
                    columns[width - 1 - age] = Some(sample.power);
                }
            }
        }
        smooth(&columns)
    }
}

/// Each of columns, if it has a sample, as the median of each series over the SMOOTHING columns around it which have.
fn smooth(columns: &[Option<[f32; 4]>]) -> Vec<Option<[f32; 4]>> {
    let half = SMOOTHING / 2;
    (0..columns.len()).map(|index| {
        columns[index]?;
        let around = &columns[index.saturating_sub(half)..(index + half + 1).min(columns.len())];
        Some(std::array::from_fn(|series| {
            let mut values: Vec<f32> = around.iter().flatten().map(|power| power[series]).collect();
            values.sort_by(f32::total_cmp);
            values[values.len() / 2]
        }))
    }).collect()
}

/// The colours the graph is drawn in, as the display has them, see DisplayManager::map_rgb8.
pub struct Palette<C> {
    pub background: C,
    pub grid: C,
    pub zero: C,
    pub scale: C,
    pub series: [C; 4],
}

/// The range of the scale, low and high, in kW, which are whole steps, and the step.
fn scale(columns: &[Option<[f32; 4]>]) -> (f32, f32, f32) {
    let (mut low, mut high) = (0.0_f32, 0.0_f32);
    for power in columns.iter().flatten().flatten() {
        low = low.min(power / 1000.0);
        high = high.max(power / 1000.0);
    }
    // At least a kW, so that a quiet house is not all noise.
    high = high.max(low + 1.0);
    let step = STEPS.into_iter().find(|step| (high - low) / step <= MAX_STEPS).unwrap_or(STEPS[STEPS.len() - 1]);
    ((low / step).floor() * step, (high / step).ceil() * step, step)
}

/// Draws the graph of history in area, the scale at its right.
/// Draws the graph of history in area, the scale at its right, its labels in font.
pub fn draw<D: DrawTarget>(display: &mut D, palette: &Palette<D::Color>, history: &History, area: Rectangle,
    font: &MonoFont)
    -> anyhow::Result<()> {
    let scale_width = SCALE_GAP + SCALE_CHARACTERS * (font.character_size.width + font.character_spacing) + RIGHT_MARGIN;
    let width = area.size.width.saturating_sub(scale_width) as usize;
    let height = area.size.height as usize;
    if width < 2 || height < 2 {
        return Ok(());
    }
    let columns = history.columns(width);
    let (low, high, step) = scale(&columns);
    let y = |kw: f32| (((high - kw) / (high - low)) * (height - 1) as f32).round().clamp(0.0, (height - 1) as f32) as usize;
    let lines: Vec<(f32, usize)> = (0..=((high - low) / step).round() as i32).map(|index| low + index as f32 * step)
        .map(|kw| (kw, y(kw))).collect();

    let mut pixels = vec![palette.background; height];
    for x in 0..width {
        pixels.fill(palette.background);
        for (kw, line) in &lines {
            if kw.abs() < step / 2.0 {
                pixels[*line] = palette.zero;
            } else if x % 3 == 0 {
                pixels[*line] = palette.grid;
            }
        }
        if let Some(power) = columns[x] {
            let previous = if x > 0 { columns[x - 1] } else { None };
            for series in DRAWING_ORDER {
                let now = y(power[series] / 1000.0);
                let before = previous.map_or(now, |previous| y(previous[series] / 1000.0));
                let end = (now.max(before) + LINE_WIDTH - 1).min(height - 1);
                for pixel in &mut pixels[now.min(before)..=end] {
                    *pixel = palette.series[series];
                }
            }
        }
        let column = Rectangle::new(area.top_left + Point::new(x as i32, 0), Size::new(1, height as u32));
        display.fill_contiguous(&column, pixels.iter().copied()).anyhow()?;
    }

    // The scale, a label at each line.
    let scale_area = Rectangle::new(area.top_left + Point::new(width as i32, 0), Size::new(scale_width, height as u32));
    scale_area.into_styled(PrimitiveStyle::with_fill(palette.background)).draw(display).anyhow()?;
    let style = MonoTextStyle::new(font, palette.scale);
    let text_style = TextStyleBuilder::new().alignment(Alignment::Right).baseline(Baseline::Middle).build();
    let right = scale_area.top_left.x + (scale_width - RIGHT_MARGIN) as i32;
    for (kw, line) in &lines {
        let label = format!("{kw:.0}");
        // Kept inside the area, the top and bottom labels being half a line in.
        let half = font.character_size.height as i32 / 2;
        let line = (*line as i32).clamp(half, height as i32 - half);
        Text::with_text_style(&label, Point::new(right, area.top_left.y + line), style, text_style).draw(display).anyhow()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_removes_a_spike() {
        let columns = [Some([0.0, 0.0, 0.0, 500.0]), Some([0.0, 0.0, 0.0, 2500.0]), Some([0.0, 0.0, 0.0, 600.0]), None];
        let smoothed = smooth(&columns);
        assert_eq!(smoothed[1].unwrap()[3], 600.0);
        assert!(smoothed[3].is_none());
    }

    #[test]
    fn scale_is_whole_steps_including_zero() {
        let columns = [Some([1500.0, 0.0, -2300.0, 800.0]), None];
        assert_eq!(scale(&columns), (-3.0, 2.0, 1.0));
        assert_eq!(scale(&[None]), (0.0, 1.0, 1.0));
    }
}
