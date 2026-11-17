//! The log panel along the bottom of the window, which shows the log's entries, see logging.rs, with errors in red and
//! warnings in yellow, and scrolls to the latest as they are added. The entries are read from the log file, so those
//! of other processes, e.g. a scheduled fetch, are shown too. It can be resized by dragging its top edge, and hidden,
//! see Navbar.

use std::time::Duration;

use dioxus::prelude::*;
use dioxus_free_icons::{Icon, icons::ld_icons::LdX};
use time::macros::format_description;
use time_tz::OffsetDateTimeExt;

use crate::logging::{self, Level, LogEntry, LogReader};

/// The most entries shown, the rest are in the log file.
const MAX_SHOWN: usize = 2000;
/// How often the log file is checked for new entries. Checking is only looking at its size, which is cheap.
const POLL: Duration = Duration::from_secs(1);
const DEFAULT_HEIGHT: f64 = 180.0;
const MIN_HEIGHT: f64 = 60.0;

#[component]
pub fn LogPanel(on_close: EventHandler<()>) -> Element {
    let mut entries = use_signal(Vec::<LogEntry>::new);
    let mut height = use_signal(|| DEFAULT_HEIGHT);
    // Where a drag of the top edge started, its y and the height then, while it is dragged.
    let mut dragging = use_signal(|| None::<(f64, f64)>);

    // The log is written by other processes too, so the file is checked for new entries rather than them being
    // signalled.
    use_future(move || async move {
        let mut reader: Option<LogReader> = None;
        let mut unwritten = 0;
        loop {
            let path = logging::current_path();
            // Starting again when the profile, and so the log, changes.
            if reader.as_ref().map(|reader| reader.path()) != path.as_deref() {
                reader = path.map(LogReader::new);
                unwritten = 0;
                entries.write().clear();
            }
            let mut added = reader.as_mut().and_then(LogReader::read_new).unwrap_or_default();
            // Any entries of this process which could not be written to the file.
            let failed = logging::unwritten();
            let newly_failed = failed.len().saturating_sub(unwritten);
            added.extend(failed.into_iter().skip(unwritten));
            unwritten += newly_failed;
            if !added.is_empty() {
                let mut shown = entries.write();
                shown.extend(added);
                let excess = shown.len().saturating_sub(MAX_SHOWN);
                shown.drain(..excess);
            }
            tokio::time::sleep(POLL).await;
        }
    });

    // Scrolls to the latest entry once new ones are shown.
    use_effect(move || {
        let _ = entries.read().len();
        document::eval(
            "const log = document.getElementById('log-entries'); if (log) { log.scrollTop = log.scrollHeight; }",
        );
    });

    let time_format = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    // In the computer's time zone, or UTC if it can not be found.
    let local = time_tz::system::get_timezone().ok();
    let shown_time = |time: time::OffsetDateTime| match local {
        Some(tz) => time.to_timezone(tz).format(&time_format).unwrap_or_default(),
        None => time.format(&time_format).unwrap_or_default(),
    };

    rsx! {
        div { class: "log-panel", style: "height: {height}px;",
            div {
                class: "log-resize",
                title: "Drag to resize the log",
                onmousedown: move |event: MouseEvent| dragging.set(Some((event.client_coordinates().y, height()))),
            }
            div { class: "log-header",
                span { "Log" }
                button {
                    r#type: "button",
                    class: "log-close",
                    title: "Hide the log",
                    onclick: move |_| on_close.call(()),
                    Icon { icon: LdX, width: 14, height: 14 }
                }
            }
            div { id: "log-entries", class: "log-entries",
                for (index, entry) in entries.read().iter().enumerate() {
                    div {
                        key: "{index}",
                        class: match entry.level {
                            Level::Error => "log-entry log-error",
                            Level::Warning => "log-entry log-warning",
                            Level::Info => "log-entry log-info",
                        },
                        span { class: "log-time",
                            {shown_time(entry.time)}
                        }
                        span { class: "log-level",
                            match entry.level {
                                Level::Error => "ERROR",
                                Level::Warning => "WARNING",
                                Level::Info => "INFO",
                            }
                        }
                        span { class: "log-source",
                            {entry.source.clone().unwrap_or_default()}
                        }
                        span { class: "log-message", "{entry.message}" }
                    }
                }
            }
        }
        // While the top edge is dragged, the whole window follows the mouse, so that it is not lost over other elements.
        if let Some((start_y, start_height)) = dragging() {
            div {
                class: "log-drag-overlay",
                onmousemove: move |event: MouseEvent| {
                    let new_height = start_height + start_y - event.client_coordinates().y;
                    height.set(new_height.max(MIN_HEIGHT));
                },
                onmouseup: move |_| dragging.set(None),
                onmouseleave: move |_| dragging.set(None),
            }
        }
    }
}
