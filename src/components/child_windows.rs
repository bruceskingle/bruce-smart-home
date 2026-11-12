//! The app's windows other than the main one, e.g. a bill opened from the bill list, see octopus/pages/bills.rs.
//!
//! They are kept above the main window, closed when it is closed, listed in the Window menu so that any window can be
//! brought to the front, and can be arranged side by side in a row across the screen, leaving the main window where it
//! is.
//!
//! The menu bar is the app's own, see menu_bar, rather than Dioxus's default, so that it can have the list of windows.
//! The main window's App calls use_main_window, and the root component of each child window calls use_child_window.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use dioxus::desktop::muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::tao::window::WindowId;
use dioxus::desktop::{Config, DesktopService, LogicalPosition, LogicalSize, WindowBuilder, use_muda_event_handler, use_wry_event_handler, window};
use dioxus::prelude::*;

const ARRANGE_ID: &str = "arrange-windows";
const SHOW_MAIN_ID: &str = "show-window-main";
const SHOW_CHILD_PREFIX: &str = "show-window-";
const CLOSE_CHILD_PREFIX: &str = "close-window-";
const CLOSE_ALL_ID: &str = "close-all-windows";
const MAIN_TITLE: &str = "Bruce's Smart Home";

struct Child {
    /// Identifies the window in its menu item's id.
    number: u64,
    /// What the window shows, e.g. a bill's id, so that opening it again brings the open window to the front.
    key: String,
    title: String,
    /// Until the window has been created, a Weak which never upgrades. Weak so that it does not keep a closed window
    /// open.
    window: Weak<DesktopService>,
    id: Option<WindowId>,
}

/// The Window menu, and the items in it which list the windows, which are replaced whenever a window opens or closes.
struct WindowMenu {
    menu_bar: Menu,
    window_menu: Submenu,
    arrange: MenuItem,
    listed: Vec<MenuItem>,
    /// Lists the child windows, each of which closes that window, then an item which closes them all.
    close_menu: Submenu,
    close_listed: Vec<MenuItem>,
    close_separator: PredefinedMenuItem,
    close_all: MenuItem,
}

#[derive(Default)]
struct State {
    main: Weak<DesktopService>,
    menu: Option<WindowMenu>,
    /// In the order they were opened, which is the order they are arranged in.
    children: Vec<Child>,
    next_number: u64,
    /// How many title bars down and right of the main window the next window is opened, see cascade.
    cascade: u32,
}

thread_local! {
    // Windows, and so this, are only used on the main thread.
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Calls f with the state, unless it is in use, or gone because the app is exiting.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.try_with(|state| state.try_borrow_mut().ok().map(|mut state| f(&mut state))).ok().flatten()
}

/// The app's menu bar, for the main window's Config. It is the same as Dioxus's default, except that the Window menu
/// lists the windows and can arrange them, and on macOS there is an app menu.
pub fn menu_bar() -> Menu {
    let menu_bar = Menu::new();

    let edit_menu = Submenu::new("Edit", true);
    let _ = edit_menu.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::select_all(None),
    ]);

    let arrange = MenuItem::with_id(ARRANGE_ID, "Arrange Bill Windows", false, None);
    // Rather than Close Window, which does not make clear which window it closes.
    let close_menu = Submenu::new("Close", false);
    let window_menu = Submenu::new("Window", true);
    let _ = window_menu.append_items(&[
        &PredefinedMenuItem::minimize(None),
        &PredefinedMenuItem::maximize(None),
        &PredefinedMenuItem::fullscreen(None),
        &PredefinedMenuItem::separator(),
        &arrange,
        &close_menu,
        &PredefinedMenuItem::separator(),
    ]);

    if cfg!(target_os = "macos") {
        let app_menu = Submenu::new(MAIN_TITLE, true);
        let _ = app_menu.append_items(&[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]);
        let _ = menu_bar.append_items(&[&app_menu, &edit_menu, &window_menu]);
    }
    else {
        // The usual order on Windows and Linux, with Exit in the File menu.
        let file_menu = Submenu::new("File", true);
        let _ = file_menu.append(&PredefinedMenuItem::quit(Some("Exit")));
        let _ = menu_bar.append_items(&[&file_menu, &edit_menu, &window_menu]);
    }

    if cfg!(debug_assertions) {
        // Dioxus's own items, which it handles by these ids.
        let help_menu = Submenu::new("Help", true);
        let _ = help_menu.append_items(&[
            &MenuItem::with_id("dioxus-toggle-dev-tools", "Toggle Developer Tools", true, None),
            &MenuItem::with_id("dioxus-float-top", "Float on Top (dev mode only)", true, None),
        ]);
        let _ = menu_bar.append(&help_menu);
        #[cfg(target_os = "macos")]
        help_menu.set_as_help_menu_for_nsapp();
    }

    with_state(|state| {
        state.menu = Some(WindowMenu {
            menu_bar: menu_bar.clone(),
            window_menu,
            arrange,
            listed: Vec::new(),
            close_menu,
            close_listed: Vec::new(),
            close_separator: PredefinedMenuItem::separator(),
            close_all: MenuItem::with_id(CLOSE_ALL_ID, "All Bill Windows", true, None),
        });
        refresh_menu(state);
    });
    menu_bar
}

/// Lists the windows in the Window menu, after the items which are always there.
fn refresh_menu(state: &mut State) {
    let Some(menu) = state.menu.as_mut() else {
        return;
    };
    for item in menu.listed.drain(..) {
        let _ = menu.window_menu.remove(&item);
    }
    menu.listed.push(MenuItem::with_id(SHOW_MAIN_ID, MAIN_TITLE, true, None));
    for child in &state.children {
        menu.listed.push(MenuItem::with_id(format!("{SHOW_CHILD_PREFIX}{}", child.number), &child.title, true, None));
    }
    for item in &menu.listed {
        let _ = menu.window_menu.append(item);
    }
    menu.arrange.set_enabled(!state.children.is_empty());

    for item in menu.close_listed.drain(..) {
        let _ = menu.close_menu.remove(&item);
    }
    let _ = menu.close_menu.remove(&menu.close_separator);
    let _ = menu.close_menu.remove(&menu.close_all);
    for child in &state.children {
        menu.close_listed.push(MenuItem::with_id(format!("{CLOSE_CHILD_PREFIX}{}", child.number), &child.title, true, None));
    }
    for item in &menu.close_listed {
        let _ = menu.close_menu.append(item);
    }
    let _ = menu.close_menu.append_items(&[&menu.close_separator, &menu.close_all]);
    menu.close_menu.set_enabled(!state.children.is_empty());
}

/// The child windows which are open, all of them or the one numbered number.
fn child_windows(number: Option<u64>) -> Vec<Rc<DesktopService>> {
    with_state(|state| state.children.iter()
        .filter(|child| number.is_none_or(|number| child.number == number))
        .filter_map(|child| child.window.upgrade())
        .collect())
        .unwrap_or_default()
}

/// Called by the main window's App, to handle the Window menu, and close the other windows when it closes.
pub fn use_main_window() {
    use_hook(|| with_state(|state| state.main = Rc::downgrade(&window())));

    use_muda_event_handler(|event| {
        let id = event.id().0.as_str();
        if id == ARRANGE_ID {
            arrange();
        }
        else if id == SHOW_MAIN_ID {
            if let Some(main) = with_state(|state| state.main.upgrade()).flatten() {
                bring_to_front(&main);
            }
        }
        else if id == CLOSE_ALL_ID {
            for child in child_windows(None) {
                child.close();
            }
        }
        else if let Some(number) = id.strip_prefix(SHOW_CHILD_PREFIX).and_then(|number| number.parse::<u64>().ok()) {
            for child in child_windows(Some(number)) {
                bring_to_front(&child);
            }
        }
        else if let Some(number) = id.strip_prefix(CLOSE_CHILD_PREFIX).and_then(|number| number.parse::<u64>().ok()) {
            for child in child_windows(Some(number)) {
                child.close();
            }
        }
    });

    use_wry_event_handler(|event, _| {
        if let Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } = event {
            let is_main = with_state(|state| state.main.upgrade().is_some_and(|main| main.window.id() == *window_id));
            if is_main == Some(true) {
                for child in child_windows(None) {
                    child.close();
                }
            }
        }
    });
}

/// Called by the root component of a window opened by open, so that it is removed from the Window menu when it closes.
pub fn use_child_window() {
    let id = use_hook(|| window().window.id());
    use_drop(move || {
        with_state(|state| {
            state.children.retain(|child| child.id != Some(id));
            if state.children.is_empty() {
                state.cascade = 0;
            }
            refresh_menu(state);
        });
    });
}

/// The child window opened last which is still open, e.g. for a screenshot of it.
pub fn newest() -> Option<Rc<DesktopService>> {
    child_windows(None).pop()
}

/// Closes all the child windows.
pub fn close_all() {
    for child in child_windows(None) {
        child.close();
    }
}

fn bring_to_front(window: &DesktopService) {
    window.window.set_visible(true);
    window.window.set_minimized(false);
    window.window.set_focus();
}

/// The height of a window's title bar, in logical pixels: on macOS its standard height, as tao reports the wrong inner
/// size there, see window_state.rs, elsewhere what the frame adds to the height, which includes the menu bar.
fn title_bar_height(window: &dioxus::desktop::tao::window::Window) -> f64 {
    if cfg!(target_os = "macos") {
        crate::window_state::MACOS_TITLE_BAR_HEIGHT as f64
    }
    else {
        let scale = window.scale_factor();
        (window.outer_size().to_logical::<f64>(scale).height - window.inner_size().to_logical::<f64>(scale).height).max(0.0)
    }
}

/// Where the next window opens, and its size, in logical pixels. The first opens at anchor, a point in the main
/// window's content, if there is one, otherwise a title bar down and right of the main window, and goes to a title bar
/// short of the main window's bottom right corner. Each one after that is the same size, a title bar further down and
/// right, so that they overlap with their title bars showing. They may go past the main window, and off the screen,
/// which is allowed. The cascade starts again when the windows are arranged, or the last is closed.
fn cascade(main: &DesktopService, anchor: Option<LogicalPosition<f64>>, state: &mut State) -> Option<(LogicalPosition<f64>, LogicalSize<f64>)> {
    let window = &main.window;
    let scale = window.scale_factor();
    let step = title_bar_height(window);
    let outer = window.outer_position().ok()?.to_logical::<f64>(scale);
    let outer_size = window.outer_size().to_logical::<f64>(scale);
    // Where the main window's content is on the screen, below its title bar, see title_bar_height.
    let (content, content_size) = if cfg!(target_os = "macos") {
        (LogicalPosition::new(outer.x, outer.y + step), LogicalSize::new(outer_size.width, outer_size.height - step))
    }
    else {
        (window.inner_position().ok()?.to_logical::<f64>(scale), window.inner_size().to_logical::<f64>(scale))
    };
    let first = match anchor {
        Some(anchor) => LogicalPosition::new(content.x + anchor.x, content.y + anchor.y),
        None => LogicalPosition::new(outer.x + step, outer.y + step),
    };
    // The first window's outer bottom right is a title bar up and left of the main window's content's, and its content
    // is a title bar less high than its outer size.
    let size = LogicalSize::new(
        (content.x + content_size.width - step - first.x).max(MIN_WIDTH),
        (content.y + content_size.height - step - first.y - step).max(MIN_HEIGHT),
    );
    let offset = step * state.cascade as f64;
    state.cascade += 1;
    Some((LogicalPosition::new(first.x + offset, first.y + offset), size))
}

/// The smallest a window is opened, however small the main window.
const MIN_WIDTH: f64 = 400.0;
const MIN_HEIGHT: f64 = 300.0;

/// Opens a window showing what key identifies, titled title, whose content is the VirtualDom from make_dom, whose root
/// component must call use_child_window. If one with the same key is open, it is brought to the front instead. It is
/// placed and sized by cascade, from anchor, a point in the main window's content, if there is one.
pub fn open(key: &str, title: String, anchor: Option<LogicalPosition<f64>>, make_dom: impl FnOnce() -> VirtualDom) {
    open_shaped(key, title, anchor, None, make_dom);
}

/// Opens a window as open does, but as wide as width_for_height says for the height cascade gives it, if it is given,
/// e.g. the width at which a page shown whole down the window also fills it across, see globird/bill_pages.rs.
pub fn open_shaped(key: &str, title: String, anchor: Option<LogicalPosition<f64>>, width_for_height: Option<fn(f64) -> f64>,
    make_dom: impl FnOnce() -> VirtualDom)
{
    let Some((existing, main, menu_bar)) = with_state(|state| (
        state.children.iter().find(|child| child.key == key).map(|child| child.window.clone()),
        state.main.upgrade(),
        state.menu.as_ref().map(|menu| menu.menu_bar.clone()),
    ))
    else {
        return;
    };
    if let Some(existing) = existing {
        // Unless it is still being created.
        if let Some(existing) = existing.upgrade() {
            bring_to_front(&existing);
        }
        return;
    }
    let Some(main) = main else {
        return;
    };

    let placement = with_state(|state| cascade(&main, anchor, state)).flatten()
        .map(|(position, size)| match width_for_height {
            Some(width_for_height) => (position, LogicalSize::new(width_for_height(size.height).max(MIN_WIDTH), size.height)),
            None => (position, size),
        });
    #[allow(unused_mut)]
    let mut window_builder = WindowBuilder::new()
        .with_title(&title)
        .with_resizable(true);
    if let Some((position, size)) = placement {
        window_builder = window_builder.with_inner_size(size).with_position(position);
    }
    // Kept above the main window. On macOS it is a child window, which also moves with the main window, on Windows an
    // owned window, which is also hidden when the main window is minimized.
    #[cfg(target_os = "macos")]
    {
        use dioxus::desktop::tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
        window_builder = window_builder.with_parent_window(main.window.ns_window());
    }
    #[cfg(target_os = "windows")]
    {
        use dioxus::desktop::tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
        window_builder = window_builder.with_owner_window(main.window.hwnd());
    }
    let config = Config::new()
        .with_window(window_builder)
        // On Windows each window has its own menu bar, on macOS this is the app's menu bar already.
        .with_menu(menu_bar)
        .with_background_color(crate::components::app::BACKGROUND_COLOR)
        .with_custom_head(crate::components::app::INITIAL_HEAD.to_string())
        .with_disable_context_menu(!cfg!(debug_assertions));

    let pending = main.new_window(make_dom(), config);
    let number = with_state(|state| {
        let number = state.next_number;
        state.next_number += 1;
        state.children.push(Child { number, key: key.to_string(), title, window: Weak::new(), id: None });
        refresh_menu(state);
        number
    });
    spawn(async move {
        let window = pending.await;
        // Again, as on macOS a new window's position is taken as that of its content, below the title bar.
        if let Some((position, _)) = placement {
            window.window.set_outer_position(position);
        }
        with_state(|state| {
            if let Some(child) = state.children.iter_mut().find(|child| Some(child.number) == number) {
                child.window = Rc::downgrade(&window);
                child.id = Some(window.window.id());
            }
        });
    });
}

/// Arranges the child windows side by side in a row, in the order they were opened, filling the screen the main window
/// is on, apart from the menu bar, dock or task bar. The main window is left where it is.
fn arrange() {
    let Some(main) = with_state(|state| {
        state.cascade = 0;
        state.main.upgrade()
    }).flatten() else {
        return;
    };
    let children = child_windows(None);
    if children.is_empty() {
        return;
    }
    for child in &children {
        child.window.set_minimized(false);
        child.window.set_maximized(false);
    }
    platform::arrange(&main, &children);
    for child in &children {
        child.window.set_focus();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use dioxus::desktop::DesktopService;
    use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    fn ns_window(window: &DesktopService) -> &NSWindow {
        // SAFETY: tao's window is an NSWindow, which lives as long as the DesktopService.
        unsafe { &*(window.window.ns_window() as *const NSWindow) }
    }

    /// AppKit's own frames are used, which include the title bar, as tao's inner size is wrong on macOS, see
    /// window_state.rs.
    pub fn arrange(main: &DesktopService, children: &[std::rc::Rc<DesktopService>]) {
        let Some(screen) = ns_window(main).screen() else {
            return;
        };
        let visible = screen.visibleFrame();
        let width = visible.size.width / children.len() as f64;
        for (index, child) in children.iter().enumerate() {
            let frame = NSRect::new(
                NSPoint::new(visible.origin.x + width * index as f64, visible.origin.y),
                NSSize::new(width, visible.size.height),
            );
            ns_window(child).setFrame_display(frame, true);
        }
    }
}

/// Windows positions a window by its frame including its invisible resize borders, so the borders are allowed for to
/// leave no gaps between the windows.
#[cfg(target_os = "windows")]
mod platform {
    use dioxus::desktop::DesktopService;
    use dioxus::desktop::tao::platform::windows::WindowExtWindows;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

    /// The part of the screen the window is on which is not the task bar, in physical pixels.
    fn work_area(hwnd: HWND) -> Option<RECT> {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        // SAFETY: the window handle is valid while the window is, and info is the size it says it is.
        let found = unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() };
        found.then_some(info.rcWork)
    }

    /// How far the window's frame extends beyond what is visible of it, on each side, as left, top, right and bottom.
    fn invisible_borders(hwnd: HWND) -> (i32, i32, i32, i32) {
        let mut frame = RECT::default();
        let mut visible = RECT::default();
        // SAFETY: the window handle is valid while the window is, and visible is the size it says it is.
        let found = unsafe {
            GetWindowRect(hwnd, &mut frame).is_ok()
                && DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut visible as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32).is_ok()
        };
        if !found {
            return (0, 0, 0, 0);
        }
        (visible.left - frame.left, visible.top - frame.top, frame.right - visible.right, frame.bottom - visible.bottom)
    }

    pub fn arrange(main: &DesktopService, children: &[std::rc::Rc<DesktopService>]) {
        let Some(area) = work_area(HWND(main.window.hwnd())) else {
            return;
        };
        let count = children.len() as i32;
        let width = (area.right - area.left) / count;
        for (index, child) in children.iter().enumerate() {
            let hwnd = HWND(child.window.hwnd());
            let (left, top, right, bottom) = invisible_borders(hwnd);
            let x = area.left + width * index as i32;
            // The last one takes up what is left after rounding.
            let visible_width = if index as i32 == count - 1 { area.right - x } else { width };
            // SAFETY: the window handle is valid while the window is.
            let _ = unsafe {
                SetWindowPos(hwnd, HWND(0), x - left, area.top - top, visible_width + left + right,
                    area.bottom - area.top + top + bottom, SWP_NOZORDER | SWP_NOACTIVATE)
            };
        }
    }
}

/// tao does not know where panels are, so the whole screen is used.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use dioxus::desktop::DesktopService;
    use dioxus::desktop::tao::dpi::{PhysicalPosition, PhysicalSize};

    pub fn arrange(main: &DesktopService, children: &[std::rc::Rc<DesktopService>]) {
        let Some(monitor) = main.window.current_monitor() else {
            return;
        };
        let (position, size) = (monitor.position(), monitor.size());
        let width = size.width / children.len() as u32;
        for (index, child) in children.iter().enumerate() {
            let window = &child.window;
            // The position is of the frame, the size of the content inside it.
            let outer = window.outer_size();
            let inner = window.inner_size();
            window.set_outer_position(PhysicalPosition::new(position.x + (width * index as u32) as i32, position.y));
            window.set_inner_size(PhysicalSize::new(
                width.saturating_sub(outer.width.saturating_sub(inner.width)),
                size.height.saturating_sub(outer.height.saturating_sub(inner.height)),
            ));
        }
    }
}
