//! Screenshots of the app for the documentation, taken by the app itself: marco-sparko --profile demo --screenshots
//!
//! The shots are listed in a JSON file, docs/screenshots.json by default, see ShotFile. For each one the app goes to
//! the home page, follows the shot's steps, e.g. clicking on what has a given text, as someone using it would, then
//! saves a picture of the window, with its frame, to the shot's file. Then it exits.
//!
//! Use the demo profile, a redacted copy of a real one, see redact.rs, which is offline, so that the pictures do not
//! identify anyone, and are the same each time. The window is the size the file gives, which is not remembered as the
//! size of the app's window, and the log panel is hidden. See docs/Screenshots.md.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail};
use dioxus::desktop::{DesktopService, LogicalSize};
use dioxus::prelude::*;
use serde::Deserialize;

/// The shot list used when --screenshots is not given a file.
pub const DEFAULT_FILE: &str = "docs/screenshots.json";
/// How long the app is given to settle after each step, e.g. for a page to draw.
const SETTLE: Duration = Duration::from_millis(800);
/// How long a wait for some text waits at most.
const WAIT_FOR_LIMIT: Duration = Duration::from_secs(60);

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether screenshots are being taken, so that e.g. the log panel is hidden.
pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Whether a module's list of pages is hidden, while a picture is taken, unless the shot is about it, see Shot::pages.
/// It is shown while the shot's steps are done, as they may click on it. A signal, so that the page is drawn again.
pub static PAGE_LIST_HIDDEN: GlobalSignal<bool> = Signal::global(|| false);

/// The shot list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShotFile {
    /// The size of the app's window, its content, in logical pixels.
    pub window: WindowSize,
    pub shots: Vec<Shot>,
    /// Profiles the shots create, e.g. to show a new profile's first run, which are deleted before the shots, in case
    /// an earlier run stopped part way, and afterwards, when the profile the app started with is made the active one
    /// again. Any profile with one of these names is deleted, so they must not be ones in use.
    #[serde(default)]
    pub new_profiles: Vec<String>,
    /// A data directory whose profiles and data are deleted before the shots, so that the first shots show what
    /// someone sees the first time they run the app, e.g. /Users/yourname. It must be the data directory given with
    /// --home, see prepare_data_directory, so that no other is ever cleared.
    #[serde(default)]
    pub fresh_data_directory: Option<PathBuf>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shot {
    /// Where the picture is saved, relative to the directory the app is run in, e.g. docs/readMe/Screenshot6.png
    file: String,
    /// Whether the help panel is shown, it is hidden unless the shot is of the help.
    #[serde(default)]
    help: bool,
    /// Whether a module's list of pages on the left is shown, it is hidden, as it takes up space, unless the shot is
    /// about it.
    #[serde(default)]
    pages: bool,
    /// Whose window is pictured, the main window, or the newest bill's.
    #[serde(default)]
    window: ShotWindow,
    /// What is done before the picture is taken, starting from the home page.
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ShotWindow {
    #[default]
    Main,
    /// The newest of the windows opened from the main one, e.g. a bill's.
    Newest,
}

/// One step of a shot, as in the file, e.g. {"click": "Meters"}.
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Step {
    /// Clicks on the innermost visible element whose text is this, e.g. a button, link or page in the list.
    Click { click: String },
    /// Clicks on the index'th element the CSS selector selects, the first if index is not given.
    Select {
        select: String,
        #[serde(default)]
        index: usize,
    },
    /// Clicks on the element the CSS selector selects in the first table row which has this text, ignoring case, e.g.
    /// the link of the import meter, {"row": "import", "select": ".link"}.
    Row { row: String, select: String },
    /// Types value into the index'th element the CSS selector selects, e.g. a date in the date picker.
    Type {
        input: String,
        #[serde(default)]
        index: usize,
        value: String,
    },
    /// Scrolls the shot's window, e.g. a bill's, so that the innermost element whose text is this is at its top, or with
    /// bottom at its bottom, e.g. {"scroll": "Consumption Analysis", "bottom": true}.
    Scroll {
        scroll: String,
        #[serde(default)]
        bottom: bool,
    },
    /// Presses a key in what has the focus, e.g. {"key": "Enter"} in a field which has been typed into.
    Key { key: String },
    /// Copies the profile of this name from the usual data directory, e.g. the redacted demo profile, see redact.rs,
    /// into the active profile, offline and without logins, and starts the app again with it, e.g. after the shots of
    /// the first run, {"loadProfile": "demo"}.
    LoadProfile {
        #[serde(rename = "loadProfile")]
        load_profile: String,
    },
    /// Waits for this many milliseconds.
    Wait { wait: u64 },
    /// Waits until this text is on the page, e.g. once some data has been loaded.
    WaitFor {
        #[serde(rename = "waitFor")]
        wait_for: String,
    },
}

/// Deletes the profiles and data in the data directory, for a shot list with fresh_data_directory, so that the app
/// starts as it does the first time. Called before the window opens. The data directory must be the one the shot list
/// gives, and not the usual one, so that the real profiles are never deleted.
pub fn prepare_data_directory(fresh: &Path) -> anyhow::Result<()> {
    let data_home = crate::data_home::data_home()?;
    if data_home != std::path::absolute(fresh)? || data_home == crate::data_home::usual_data_home()? {
        bail!("The shot list starts with a fresh data directory {}, which deletes its profiles and data, so give it with \
            --home, it is not the data directory in use, {}", fresh.display(), data_home.display());
    }
    crate::data_home::clear(&data_home)?;
    println!("Cleared the data directory {}", data_home.display());
    Ok(())
}

/// Copies the profile from, in the usual data directory, into the active profile, see Step::LoadProfile: its cached
/// data, replacing the active profile's, and its modules without their logins, settings and when its data is from.
fn load_profile_data(from: &str) -> anyhow::Result<()> {
    use crate::profile::{Profile, ProfileFile, profile_names_equal};

    let usual = crate::data_home::usual_data_home()?;
    let usual_profiles: ProfileFile = serde_json::from_reader(std::fs::File::open(crate::data_home::profiles_path_in(&usual))?)?;
    let source = usual_profiles.iter().find(|profile| profile_names_equal(&profile.name, from))
        .ok_or_else(|| anyhow!("there is no profile {from} in {}", usual.display()))?;
    let active = crate::profile::fetch_active_profile(&None)?.active_profile.name;

    // A redacted profile's data is in the old layout, see redact.rs, which is converted into the copy's store below.
    let usual_cache = crate::data_home::old_cache_dir_in(&usual);
    let cache = crate::MarcoSparkoContext::get_cache_dir_path()?;
    // Only the modules which a redacted profile can have, see redact::is_redactable.
    for module_id in source.modules.keys().filter(|module_id| crate::redact::is_redactable(module_id)) {
        let target = cache.join(format!("{active}-{module_id}"));
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        copy_dir(&usual_cache.join(format!("{from}-{module_id}")), &target)?;
    }

    let path = crate::MarcoSparkoContext::get_file_path()?;
    let mut profiles: ProfileFile = std::fs::File::open(&path).ok()
        .and_then(|file| serde_json::from_reader(file).ok())
        .unwrap_or_default();
    profiles.retain(|profile| !profile_names_equal(&profile.name, &active));
    let modules = source.modules.iter()
        .filter(|(module_id, _)| crate::redact::is_redactable(module_id))
        .map(|(module_id, module)| {
            let mut module = module.clone();
            if let Some(module) = module.as_object_mut() {
                module.remove("apiKey");
            }
            (module_id.clone(), module)
        })
        .collect();
    profiles.insert(0, Profile {
        name: active,
        version: source.version.clone(),
        modules,
        settings: source.settings.clone(),
        offline: true,
        as_of: source.as_of,
        // A site of its own, in this data home, into which its data is converted, as it is in the old layout.
        site: None,
    });
    serde_json::to_writer_pretty(crate::private_file::create_private_file(&path)?, &profiles)?;
    let active = profiles[0].name.clone();
    if crate::profile::ProfileVersionState::from(&profiles[0].version)? == crate::profile::ProfileVersionState::OlderThanCurrent {
        crate::profile::upgrade_profile(&active, &mut bruce_db::progress::Silent)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    crate::private_file::create_private_dir(to)?;
    for entry in std::fs::read_dir(from).map_err(|error| anyhow!("unable to read {}: {error}", from.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()))?;
        }
        else {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Reads the shot list, for its window size, before the window opens.
pub fn read_shot_file(path: &Path) -> anyhow::Result<ShotFile> {
    let text = std::fs::read_to_string(path).map_err(|error| anyhow!("Unable to read the shot list {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| anyhow!("The shot list {} is not valid: {error}", path.display()))
}

/// Takes the screenshots in the shot list at path, then exits the app. Called by App, in the main window, when
/// --screenshots is given.
pub fn use_screenshots(path: Option<PathBuf>) {
    use_future(move || {
        let path = path.clone();
        async move {
            let Some(path) = path else {
                return;
            };
            RUNNING.store(true, Ordering::SeqCst);
            let result = take_all(&path).await;
            let status = match result {
                Ok(count) => {
                    println!("Took {count} screenshots from {}", path.display());
                    0
                },
                Err(error) => {
                    eprintln!("Screenshots failed: {error:#}");
                    1
                },
            };
            std::process::exit(status);
        }
    });
}

async fn take_all(path: &Path) -> anyhow::Result<usize> {
    let shot_file = read_shot_file(path)?;
    check_permission()?;
    keep_display_awake();
    let main = dioxus::desktop::window();
    main.window.set_inner_size(LogicalSize::new(shot_file.window.width, shot_file.window.height));
    // The app is ready once it shows its navigation bar, and has had time to draw it in front.
    wait_for_selector(".nav", WAIT_FOR_LIMIT).await?;
    to_front(&main.window);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let help_was_shown = crate::help::is_shown();
    // The profile the app started with, to go back to after the shots which create profiles.
    let started_with = crate::profile::fetch_active_profile(&None).map(|profile| profile.active_profile.name).ok();
    remove_new_profiles(&shot_file.new_profiles, started_with.as_deref());
    let mut taken = 0;
    for (index, shot) in shot_file.shots.iter().enumerate() {
        println!("Shot {} of {}: {}", index + 1, shot_file.shots.len(), shot.file);
        // Once more if it fails, as a click can be missed while a page is being drawn.
        let mut result = take(shot, &main).await;
        if result.is_err() {
            crate::components::child_windows::close_all();
            tokio::time::sleep(SETTLE).await;
            result = take(shot, &main).await;
        }
        match result {
            Ok(()) => taken += 1,
            Err(error) => eprintln!("  Not taken: {error:#}"),
        }
        crate::components::child_windows::close_all();
        tokio::time::sleep(SETTLE).await;
    }
    crate::help::show_for_now(help_was_shown);
    remove_new_profiles(&shot_file.new_profiles, started_with.as_deref());
    Ok(taken)
}

/// Deletes the profiles the shots created, after making the profile the app started with the active one again, so that
/// it is the one the app opens with next time.
fn remove_new_profiles(new_profiles: &[String], started_with: Option<&str>) {
    let Some(started_with) = started_with else {
        return;
    };
    if new_profiles.is_empty() {
        return;
    }
    if let Err(error) = crate::profile::set_active_profile(&started_with.to_string()) {
        eprintln!("Unable to make {started_with} the active profile again: {error:#}");
        return;
    }
    let existing = crate::profile::fetch_active_profile(&None).map(|profile| profile.all_profiles).unwrap_or_default();
    for profile in new_profiles.iter().filter(|profile| existing.iter().any(|name| crate::profile::profile_names_equal(name, profile))) {
        match crate::profile::delete_profile(profile, started_with) {
            Ok(()) => println!("Deleted the profile {profile} the shots created"),
            Err(error) => eprintln!("Unable to delete the profile {profile} the shots created: {error:#}"),
        }
    }
}

async fn take(shot: &Shot, main: &DesktopService) -> anyhow::Result<()> {
    crate::help::show_for_now(shot.help);
    // Closes anything the last shot left open, e.g. the date picker or the new profile's name.
    let _ = document::eval("const element = document.activeElement || document.body;
        element.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));").await;
    click("Home").await?;
    tokio::time::sleep(SETTLE).await;
    for step in &shot.steps {
        match step {
            Step::Click { click: text } => click(text).await?,
            Step::Select { select, index } => run(&format!(
                "const element = document.querySelectorAll({})[{index}]; if (!element) {{ return false; }} element.click(); return true;",
                json(select),
            ), &format!("nothing is selected by {select} at {index}")).await?,
            Step::Row { row, select } => run(&format!(
                "const want = {}.toLowerCase();
                 const rows = [...document.querySelectorAll('tr')].filter((row) => row.innerText.toLowerCase().includes(want));
                 // The innermost, not e.g. a row of a table the rows are in.
                 const row = rows.find((row) => !rows.some((other) => other !== row && row.contains(other)));
                 const element = row && row.querySelector({});
                 if (!element) {{ return false; }} element.click(); return true;",
                json(row), json(select),
            ), &format!("there is no {select} in a row with the text {row:?}")).await?,
            Step::Type { input, index, value } => run(&format!(
                "const element = document.querySelectorAll({})[{index}]; if (!element) {{ return false; }}
                 element.value = {}; element.dispatchEvent(new Event('input', {{ bubbles: true }}));
                 element.dispatchEvent(new Event('change', {{ bubbles: true }})); return true;",
                json(input), json(value),
            ), &format!("nothing is selected by {input} at {index}")).await?,
            Step::Scroll { scroll, bottom } => {
                let script = format!(
                    "const want = {};
                     const matches = (element) => (element.innerText || '').trim() === want;
                     const element = [...document.querySelectorAll('body *')].filter(matches).find((element) => ![...element.children].some(matches));
                     if (element) {{ element.scrollIntoView({{ block: {} }}); }}",
                    json(scroll), if *bottom { "'end'" } else { "'start'" },
                );
                match shot.window {
                    ShotWindow::Main => { let _ = document::eval(&script).await; },
                    ShotWindow::Newest => {
                        let window = crate::components::child_windows::newest().ok_or_else(|| anyhow!("there is no window to scroll"))?;
                        window.webview.evaluate_script(&script).map_err(|error| anyhow!("unable to scroll: {error}"))?;
                    },
                }
            },
            Step::Key { key } => run(&format!(
                "const element = document.activeElement || document.body;
                 for (const type of ['keydown', 'keyup']) {{
                     element.dispatchEvent(new KeyboardEvent(type, {{ key: {}, bubbles: true }}));
                 }}
                 return true;",
                json(key),
            ), "unable to press the key").await?,
            Step::LoadProfile { load_profile } => {
                load_profile_data(load_profile)?;
                let mut init = consume_context::<Signal<crate::InitRequested>>();
                init.set(crate::InitRequested(true));
                wait_for_selector(".nav", WAIT_FOR_LIMIT).await?;
            },
            Step::Wait { wait } => tokio::time::sleep(Duration::from_millis(*wait)).await,
            Step::WaitFor { wait_for } => wait_for_text(wait_for, WAIT_FOR_LIMIT).await?,
        }
        tokio::time::sleep(SETTLE).await;
    }
    // Only while the picture is taken, as the steps may have clicked on the page list. The help again, as e.g.
    // choosing a new profile shows or hides it as the profile's settings say.
    *PAGE_LIST_HIDDEN.write() = !shot.pages;
    crate::help::show_for_now(shot.help);
    tokio::time::sleep(SETTLE).await;
    let result = picture(shot, main).await;
    *PAGE_LIST_HIDDEN.write() = false;
    result
}

/// Takes the picture of the shot's window. A window opened from the main one is pictured on its own, the size of the
/// main window, see Alone.
async fn picture(shot: &Shot, main: &DesktopService) -> anyhow::Result<()> {
    match shot.window {
        ShotWindow::Main => {
            to_front(&main.window);
            tokio::time::sleep(SETTLE).await;
            capture(&main.window, Path::new(&shot.file))
        },
        ShotWindow::Newest => {
            let child = crate::components::child_windows::newest()
                .ok_or_else(|| anyhow!("there is no window opened from the main one"))?;
            let _alone = Alone::new(&child.window, &main.window);
            to_front(&child.window);
            tokio::time::sleep(SETTLE).await;
            capture(&child.window, Path::new(&shot.file))
        },
    }
}

/// Brings the window in front of all the others, even when the app is not the active one, e.g. when it was started from
/// VS Code, as WebKit does not draw a window which is covered by another, which would be pictured blank.
#[cfg(target_os = "macos")]
fn to_front(window: &dioxus::desktop::tao::window::Window) {
    use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
    // SAFETY: tao's window is an NSWindow, which lives as long as the window.
    unsafe { &*(window.ns_window() as *const objc2_app_kit::NSWindow) }.orderFrontRegardless();
}

#[cfg(not(target_os = "macos"))]
fn to_front(window: &dioxus::desktop::tao::window::Window) {
    window.set_focus();
}

/// A window opened from the main one, detached from it, as macOS pictures a window with those attached to it, and
/// given its frame, while it is pictured. It is attached again when this is dropped.
struct Alone {
    #[cfg(target_os = "macos")]
    windows: (*mut std::ffi::c_void, *mut std::ffi::c_void),
}

impl Alone {
    #[cfg(target_os = "macos")]
    fn new(child: &dioxus::desktop::tao::window::Window, main: &dioxus::desktop::tao::window::Window) -> Self {
        use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
        let windows = (child.ns_window(), main.ns_window());
        let (child_window, main_window) = Self::ns_windows(windows);
        main_window.removeChildWindow(child_window);
        child_window.setFrame_display(main_window.frame(), true);
        Alone { windows }
    }

    #[cfg(not(target_os = "macos"))]
    fn new(_child: &dioxus::desktop::tao::window::Window, _main: &dioxus::desktop::tao::window::Window) -> Self {
        Alone {}
    }

    #[cfg(target_os = "macos")]
    fn ns_windows<'a>(windows: (*mut std::ffi::c_void, *mut std::ffi::c_void)) -> (&'a objc2_app_kit::NSWindow, &'a objc2_app_kit::NSWindow) {
        // SAFETY: tao's windows are NSWindows, which live as long as the windows, longer than an Alone.
        unsafe { (&*(windows.0 as *const objc2_app_kit::NSWindow), &*(windows.1 as *const objc2_app_kit::NSWindow)) }
    }
}

#[cfg(target_os = "macos")]
impl Drop for Alone {
    fn drop(&mut self) {
        let (child_window, main_window) = Self::ns_windows(self.windows);
        // SAFETY: both are windows, and the child is not attached to another.
        unsafe { main_window.addChildWindow_ordered(child_window, objc2_app_kit::NSWindowOrderingMode::Above) };
    }
}

fn json(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Runs a script in the main window which returns whether it did what it should, failing with problem if it has not
/// within CLICK_LIMIT, e.g. as what it clicks on has not appeared yet.
async fn run(script: &str, problem: &str) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    loop {
        match run_once(script, problem).await {
            Err(_) if started.elapsed() < CLICK_LIMIT => tokio::time::sleep(Duration::from_millis(250)).await,
            result => return result,
        }
    }
}

async fn run_once(script: &str, problem: &str) -> anyhow::Result<()> {
    match document::eval(script).join::<bool>().await {
        Ok(true) => Ok(()),
        Ok(false) => bail!("{problem}"),
        Err(error) => bail!("{problem}: {error:?}"),
    }
}

/// How long a click waits for what it clicks on to appear, e.g. while a page loads.
const CLICK_LIMIT: Duration = Duration::from_secs(15);

/// Clicks on the innermost visible element whose text is text, once there is one, see run.
async fn click(text: &str) -> anyhow::Result<()> {
    run(&format!(
        "const want = {};
         const matches = (element) => element.offsetParent !== null && (element.innerText || '').trim() === want;
         const element = [...document.querySelectorAll('body *')]
             .filter(matches)
             .find((element) => ![...element.children].some(matches));
         if (!element) {{ return false; }}
         element.click();
         return true;",
        json(text),
    ), &format!("there is nothing to click on with the text {text:?}")).await
}

async fn wait_for_text(text: &str, limit: Duration) -> anyhow::Result<()> {
    wait_until(&format!("return document.body.innerText.includes({});", json(text)), limit, &format!("the text {text:?} did not appear")).await
}

async fn wait_for_selector(selector: &str, limit: Duration) -> anyhow::Result<()> {
    wait_until(&format!("return document.querySelector({}) !== null;", json(selector)), limit, &format!("{selector} did not appear")).await
}

async fn wait_until(script: &str, limit: Duration, problem: &str) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < limit {
        if let Ok(true) = document::eval(script).join::<bool>().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("{problem} within {} seconds", limit.as_secs())
}

/// Keeps the display awake, and so the screen unlocked, until the app exits, as nothing is drawn while the display is
/// asleep or the screen is locked, so the pictures would be blank. It does not stop the screen being locked by hand.
#[cfg(target_os = "macos")]
fn keep_display_awake() {
    if let Err(error) = std::process::Command::new("caffeinate")
        .args(["-d", "-i", "-w", &std::process::id().to_string()])
        .spawn()
    {
        eprintln!("Unable to keep the display awake, the screen must not lock while the screenshots are taken: {error}");
    }
}

#[cfg(not(target_os = "macos"))]
fn keep_display_awake() {}

/// Checks that the app may take pictures of its windows, asking macOS for the Screen Recording permission if not, which
/// shows its prompt, or adds the app to the list in System Settings, Privacy & Security, Screen & System Audio Recording.
#[cfg(target_os = "macos")]
fn check_permission() -> anyhow::Result<()> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    // SAFETY: neither takes arguments, and both may be called at any time.
    if unsafe { CGPreflightScreenCaptureAccess() } || unsafe { CGRequestScreenCaptureAccess() } {
        return Ok(());
    }
    bail!("macOS has not given the Screen Recording permission, which is needed to take the pictures. Turn it on for \
        what started the app, e.g. Visual Studio Code or Terminal, in System Settings, Privacy & Security, Screen & System \
        Audio Recording, adding it with + if it is not listed, then quit and restart it and try again")
}

#[cfg(not(target_os = "macos"))]
fn check_permission() -> anyhow::Result<()> {
    Ok(())
}

/// Saves a picture of the window, with its frame and shadow, to path, with macOS's screencapture, which needs the
/// Screen Recording permission for whatever started the app, e.g. VS Code or Terminal.
#[cfg(target_os = "macos")]
fn capture(window: &dioxus::desktop::tao::window::Window, path: &Path) -> anyhow::Result<()> {
    use dioxus::desktop::tao::platform::macos::WindowExtMacOS;
    use objc2_app_kit::NSWindow;

    // SAFETY: tao's window is an NSWindow, which lives as long as the window.
    let number = unsafe { &*(window.ns_window() as *const NSWindow) }.windowNumber();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let status = std::process::Command::new("screencapture")
        .args(["-x", "-l", &number.to_string()])
        .arg(path)
        .status()?;
    if !status.success() || !path.exists() {
        bail!("screencapture failed, it needs the Screen Recording permission, in System Settings, Privacy & Security, for what started the app");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn capture(_window: &dioxus::desktop::tao::window::Window, _path: &Path) -> anyhow::Result<()> {
    bail!("screenshots are only taken on macOS")
}
