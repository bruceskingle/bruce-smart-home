//! A page showing a module's cache files as they are, newest first, to see what an API returns while the module is
//! being developed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dioxus::prelude::*;

/// Every file under dir, with when it was last modified, newest first.
fn files(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                found.push((path, modified));
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found
}

/// The file's contents, pretty printed if it is JSON.
fn contents(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => serde_json::to_string_pretty(&json).unwrap_or(text),
            Err(_) => text,
        },
        Err(error) => format!("Unable to read {}: {error}", path.display()),
    }
}

/// The files in dir, and the contents of the one selected. generation is changed to read the files again, e.g. after a
/// fetch.
#[component]
pub fn CacheBrowser(dir: PathBuf, generation: u32) -> Element {
    // Only a prop so that a new generation renders this again.
    let _ = generation;
    let mut selected = use_signal(|| None::<PathBuf>);
    let files = files(&dir);
    let shown = selected
        .read()
        .clone()
        .or_else(|| files.first().map(|(path, _)| path.clone()));

    rsx! {
        div { class: "cache-browser",
            if files.is_empty() {
                p { "Nothing has been fetched yet." }
            }
            table { class: "display",
                for (path , _) in files.iter().cloned() {
                    tr {
                        class: if Some(&path) == shown.as_ref() { "selected" } else { "" },
                        onclick: {
                            let path = path.clone();
                            move |_| selected.set(Some(path.clone()))
                        },
                        td { {path.strip_prefix(&dir).unwrap_or(&path).display().to_string()} }
                    }
                }
            }
            if let Some(path) = shown {
                h3 { {path.strip_prefix(&dir).unwrap_or(&path).display().to_string()} }
                pre { {contents(&path)} }
            }
        }
    }
}
