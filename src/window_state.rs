//! Remembers the size and position of the main window between runs.
//!
//! On macOS positions and sizes are saved in logical units and the size is derived from the outer size, because tao
//! reports the wrong inner size there (https://github.com/tauri-apps/tao/issues/889). On other platforms physical units
//! and the inner size are used. This is the same approach dioxus uses for its debug only window state.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use dioxus::desktop::{
    Config,
    tao::{
        dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Position},
        event::{Event, WindowEvent},
        window::{Window, WindowBuilder},
    },
};
use serde::{Deserialize, Serialize};

use crate::private_file::create_private_dir;

const STATE_FILE_NAME: &str = "window-state.json";

/// The smallest size that will be restored, in case a tiny window was saved.
const MIN_SIZE: u32 = 200;

/// The height of the macOS title bar, which is included in the outer size.
pub(crate) const MACOS_TITLE_BAR_HEIGHT: u32 = 28;

#[derive(Serialize, Deserialize, Clone, Copy)]
struct WindowState {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
}

impl WindowState {
    /// Read the state of the window, while it is maximized or minimized the previous position and size are kept so that
    /// they can be restored when it returns to normal.
    fn capture(window: &Window, previous: Option<WindowState>) -> Option<WindowState> {
        if window.is_minimized() {
            return previous;
        }

        let maximized = window.is_maximized();
        if maximized {
            return previous.map(|previous| WindowState { maximized, ..previous });
        }

        let position = window.outer_position().ok()?;
        let (x, y, width, height) = if cfg!(target_os = "macos") {
            let position = position.to_logical::<i32>(window.scale_factor());
            let size = window.outer_size().to_logical::<u32>(window.scale_factor());
            let title_bar = if window.is_decorated() { MACOS_TITLE_BAR_HEIGHT } else { 0 };
            (position.x, position.y, size.width, size.height.saturating_sub(title_bar))
        }
        else {
            let size = window.inner_size();
            (position.x, position.y, size.width, size.height)
        };

        Some(WindowState { x, y, width, height, maximized })
    }

    fn position(&self) -> Position {
        if cfg!(target_os = "macos") {
            LogicalPosition::new(self.x, self.y).into()
        }
        else {
            PhysicalPosition::new(self.x, self.y).into()
        }
    }

    /// The position is also set here so the window is created on the right monitor, but on macOS tao treats it as
    /// the top of the content area rather than the title bar, so it is set again by restore_position once the window
    /// exists.
    fn apply_to(&self, window: WindowBuilder) -> WindowBuilder {
        let width = self.width.max(MIN_SIZE);
        let height = self.height.max(MIN_SIZE);
        let window = if cfg!(target_os = "macos") {
            window.with_inner_size(LogicalSize::new(width, height))
        }
        else {
            window.with_inner_size(PhysicalSize::new(width, height))
        };
        window
            .with_position(self.position())
            .with_maximized(self.maximized)
    }

    /// Move the window to the saved position, unless it would not be on any connected monitor (e.g. an external
    /// display was unplugged) in which case centre it on the primary monitor.
    fn restore_position(&self, window: &Window) {
        let position = self.position().to_physical::<i32>(window.scale_factor());
        if is_on_screen(window, position) {
            window.set_outer_position(position);
        }
        else {
            centre_on_primary_monitor(window);
        }
    }

    fn load() -> Option<WindowState> {
        let json = std::fs::read_to_string(state_file()?).ok()?;
        serde_json::from_str(&json).ok()
    }

    fn save(&self) {
        let Some(path) = state_file() else {
            return;
        };
        if let Some(dir) = path.parent() {
            _ = create_private_dir(dir);
        }
        match serde_json::to_string(self) {
            Ok(json) => {
                if let Err(error) = std::fs::write(&path, json) {
                    crate::log_error!("Failed to save the window's size and position to {}: {error}", path.display());
                }
            },
            Err(error) => crate::log_error!("Failed to save the window's size and position: {error}"),
        }
    }
}

fn state_file() -> Option<PathBuf> {
    Some(crate::data_home::settings_dir().ok()?.join(STATE_FILE_NAME))
}

/// Whether a window with its top left corner at position would have its title bar on a connected monitor, which is
/// needed for the user to be able to move it.
fn is_on_screen(window: &Window, position: PhysicalPosition<i32>) -> bool {
    let size = window.outer_size();

    // A point near the top left of the window, in the title bar.
    let grab_x = position.x + (size.width / 2).min(100) as i32;
    let grab_y = position.y + 10;

    window.available_monitors().any(|monitor| {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        grab_x >= monitor_position.x
            && grab_x < monitor_position.x + monitor_size.width as i32
            && grab_y >= monitor_position.y
            && grab_y < monitor_position.y + monitor_size.height as i32
    })
}

fn centre_on_primary_monitor(window: &Window) {
    if let Some(monitor) = window.primary_monitor().or_else(|| window.available_monitors().next()) {
        let size = window.outer_size();
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let x = monitor_position.x + (monitor_size.width as i32 - size.width as i32).max(0) / 2;
        let y = monitor_position.y + (monitor_size.height as i32 - size.height as i32).max(0) / 2;
        window.set_outer_position(PhysicalPosition::new(x, y));
    }
}

/// Configure the main window to open with the size and position it had when the app last closed, and to save them
/// when they change.
pub fn remember_window_state(config: Config, window: WindowBuilder) -> Config {
    let saved = WindowState::load();
    let window = match &saved {
        Some(state) => state.apply_to(window),
        None => window,
    };

    // The main window, once it has been created, and its latest state.
    let main_window: Rc<RefCell<Option<Arc<Window>>>> = Rc::new(RefCell::new(None));
    let state = Rc::new(RefCell::new(saved));

    config
        .with_window(window)
        .with_on_window({
            let main_window = main_window.clone();
            move |window, _dom| {
                // This is called for every window, only the first is the main window.
                // The window is not visible yet, dioxus shows it once the page has loaded.
                if main_window.borrow().is_none() {
                    if let Some(saved) = &saved {
                        saved.restore_position(&window);
                    }
                    *main_window.borrow_mut() = Some(window);
                }
            }
        })
        .with_custom_event_handler(move |event, _target| {
            let Some(window) = main_window.borrow().clone() else {
                return;
            };

            match event {
                Event::WindowEvent { window_id, event, .. } if *window_id == window.id() => match event {
                    WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                        let previous = *state.borrow();
                        *state.borrow_mut() = WindowState::capture(&window, previous);
                    },
                    WindowEvent::CloseRequested => {
                        let previous = *state.borrow();
                        if let Some(current) = WindowState::capture(&window, previous) {
                            current.save();
                        }
                    },
                    _ => {},
                },
                // Quitting from the menu (e.g. Cmd+Q on macOS) ends the event loop without a close request.
                Event::LoopDestroyed => {
                    if let Some(current) = *state.borrow() {
                        current.save();
                    }
                },
                _ => {},
            }
        })
}
