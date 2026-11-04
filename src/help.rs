//! The help shown beside each screen, see views/help_panel.rs.
//!
//! The help is Markdown in docs/help, so that it is also part of the documentation, and is built into the app. Each
//! screen has a page, see the Navbar and Module::help_topic, which can link to other pages for more detail. Links to
//! other documents open in the browser, in the documentation on GitHub Pages if they are part of it, see book.toml,
//! otherwise on GitHub. See docs/WritingHelp.md for how to add and change pages.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Where the documentation is published, for links to documents in it other than the help pages.
const BOOK_URL: &str = "https://bruceskingle.github.io/marco-sparko/";
/// The contents of the documentation, which says which documents are in it.
const BOOK_SUMMARY: &str = include_str!("../docs/SUMMARY.md");
/// Where the repository is on GitHub, for links to anything else in it.
const REPO_URL: &str = "https://github.com/bruceskingle/marco-sparko/blob/main/";
/// Where the documentation is, relative to the repository.
const DOCS_DIR: &str = "docs/";
/// Where the help pages are, relative to the repository.
const HELP_DIR: &str = "docs/help/";

pub struct HelpPage {
    /// Relative to docs/help, e.g. octopus/meters.md
    pub path: &'static str,
    pub markdown: &'static str,
}

macro_rules! help_pages {
    ($($path:literal),* $(,)?) => {
        &[$(HelpPage { path: $path, markdown: include_str!(concat!("../docs/help/", $path)) }),*]
    };
}

/// All the help pages. A page added to docs/help must be added here too.
static PAGES: &[HelpPage] = help_pages![
    "index.md",
    "home.md",
    "profile-size.md",
    "settings.md",
    "devices.md",
    "climate.md",
    "log.md",
    "date-range.md",
    "graphs.md",
    "octopus/index.md",
    "octopus/login.md",
    "octopus/user.md",
    "octopus/account.md",
    "octopus/bills.md",
    "octopus/bill.md",
    "octopus/properties.md",
    "octopus/property.md",
    "octopus/meters.md",
    "octopus/meter.md",
    "octopus/home-mini.md",
    "octopus/electricity.md",
    "octopus/ev-charging.md",
    "octopus/charging-sessions.md",
    "myenergi/index.md",
    "myenergi/login.md",
    "tesla/index.md",
    "tesla/login.md",
];

macro_rules! help_icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_str!(concat!("../docs/help/icons/", $name, ".svg")))),*]
    };
}

/// The icons of the app's buttons, which the help refers to as images, e.g. ![Help](icons/help.svg), and are drawn
/// inline in the app, by their path relative to docs/help.
static ICONS: &[(&str, &str)] = help_icons![
    "help", "log", "settings", "profile", "close", "back", "hide-pages", "show-pages", "reload", "export", "categories",
    "calendar", "external-link", "previous", "next",
];

/// The help page at path, relative to docs/help.
pub fn page(path: &str) -> Option<&'static HelpPage> {
    PAGES.iter().find(|page| page.path == path)
}

/// Where a link on a help page goes.
#[derive(Debug, PartialEq)]
pub enum HelpLink {
    /// Another help page, by its path relative to docs/help.
    Page(String),
    /// A web page, e.g. another document on GitHub.
    Web(String),
    /// Somewhere on the same page, which is not followed.
    None,
}

/// Where href, a link on the help page at from, goes.
pub fn follow(from: &str, href: &str) -> HelpLink {
    if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:") {
        return HelpLink::Web(href.to_string());
    }
    let (href, fragment) = href.split_once('#').map_or((href, String::new()), |(href, fragment)| (href, format!("#{fragment}")));
    if href.is_empty() {
        return HelpLink::None;
    }
    let path = resolve(from, href);
    if let Some(help_path) = path.strip_prefix(HELP_DIR)
        && page(help_path).is_some()
    {
        return HelpLink::Page(help_path.to_string());
    }
    // A document in the book is on its page there, e.g. docs/ScheduledFetch.md at ScheduledFetch.html
    match path.strip_prefix(DOCS_DIR).filter(|doc| BOOK_SUMMARY.contains(&format!("]({doc})"))).and_then(|doc| doc.strip_suffix(".md")) {
        Some(doc) => HelpLink::Web(format!("{BOOK_URL}{doc}.html{fragment}")),
        None => HelpLink::Web(format!("{REPO_URL}{path}{fragment}")),
    }
}

/// Where href, relative to the help page at from, is in the repository, so that links out of docs/help can go to
/// GitHub.
fn resolve(from: &str, href: &str) -> String {
    let from_path = format!("{HELP_DIR}{from}");
    let mut parts: Vec<&str> = from_path.split('/').collect();
    // The directory the page is in.
    parts.pop();
    for part in href.split('/') {
        match part {
            "" | "." => {},
            ".." => {
                parts.pop();
            },
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The help page at path as HTML, to show in the help panel, with the icons of buttons drawn inline.
pub fn to_html(path: &str, markdown: &str) -> String {
    use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};

    let mut events = Vec::new();
    // The icon of the image being read, and its alt text, which comes as text before its end.
    let mut icon: Option<(&str, String)> = None;
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match (&mut icon, event) {
            (None, Event::Start(Tag::Image { dest_url, .. })) => {
                let icon_path = resolve(path, &dest_url);
                match icon_path.strip_prefix(HELP_DIR).and_then(|icon_path| ICONS.iter().find(|(path, _)| *path == icon_path)) {
                    Some((_, svg)) => icon = Some((svg, String::new())),
                    None => events.push(Event::Start(Tag::Image {
                        link_type: pulldown_cmark::LinkType::Inline, dest_url, title: CowStr::Borrowed(""), id: CowStr::Borrowed(""),
                    })),
                }
            },
            (Some((_, alt)), Event::Text(text)) => alt.push_str(&text),
            (Some((svg, alt)), Event::End(TagEnd::Image)) => {
                events.push(Event::InlineHtml(CowStr::from(format!("<span class=\"help-icon\" title=\"{alt}\">{}</span>", svg.trim()))));
                icon = None;
            },
            (Some(_), _) => {},
            (None, event) => events.push(event),
        }
    }
    let mut html = String::new();
    html::push_html(&mut html, events.into_iter());
    html
}

/// The help page of the screen being shown in a module, set by the module's view with use_help_topic, see the Navbar.
#[derive(Clone, Copy)]
pub struct ModuleHelpTopic(pub Signal<Option<String>>);

/// The help page of the Home page being shown, e.g. devices.md, set by Home, see the Navbar.
#[derive(Clone, Copy)]
pub struct HomeHelpTopic(pub Signal<&'static str>);

/// Sets the help page of the module screen being shown, if it has one. Does nothing outside the main window.
pub fn use_help_topic(topic: Option<String>) {
    let help = try_use_context::<ModuleHelpTopic>();
    use_effect(use_reactive((&topic,), move |(topic,)| {
        if let Some(mut help) = help
            && topic.is_some()
            && *help.0.peek() != topic
        {
            help.0.set(topic);
        }
    }));
}

/// Whether the help is shown, in every window, see use_help_shown: the active profile's setting, see start.
static SHOWN: AtomicBool = AtomicBool::new(true);
/// The active profile, whose setting it is.
static PROFILE: Mutex<String> = Mutex::new(String::new());
/// How often a window checks whether another has shown or hidden the help.
const POLL: std::time::Duration = std::time::Duration::from_millis(300);

/// Starts showing or hiding the help as the profile's settings say. Called when the profile is chosen.
pub fn start(profile_name: &str, settings: &HelpSettings) {
    *PROFILE.lock().unwrap() = profile_name.to_string();
    SHOWN.store(settings.show, Ordering::SeqCst);
}

pub fn is_shown() -> bool {
    SHOWN.load(Ordering::SeqCst)
}

/// Shows or hides the help in every window, and saves it in the profile so that it stays that way.
pub fn set_shown(show: bool) {
    SHOWN.store(show, Ordering::SeqCst);
    let profile_name = PROFILE.lock().unwrap().clone();
    if let Err(error) = crate::profile::update_help_settings(&profile_name, &HelpSettings { show }) {
        crate::log_error!("Failed to save whether the help is shown: {error}");
    }
}

/// Shows or hides the help in every window, without saving it, e.g. for a screenshot.
pub fn show_for_now(show: bool) {
    SHOWN.store(show, Ordering::SeqCst);
}

/// Whether the help is shown, for a window, which follows the help being shown or hidden in another, e.g. a bill's.
pub fn use_help_shown() -> Signal<bool> {
    let mut shown = use_signal(is_shown);
    use_future(move || async move {
        loop {
            tokio::time::sleep(POLL).await;
            if *shown.peek() != is_shown() {
                shown.set(is_shown());
            }
        }
    });
    shown
}

/// The help settings of a profile, see Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelpSettings {
    /// Whether the help panel is shown.
    #[serde(default = "shown")]
    pub show: bool,
}

fn shown() -> bool {
    true
}

impl Default for HelpSettings {
    fn default() -> Self {
        HelpSettings { show: true }
    }
}

#[cfg(test)]
mod tests {
    use super::{HelpLink, PAGES, follow};

    #[test]
    fn links_go_to_help_pages_documents_or_web_sites() {
        assert_eq!(follow("octopus/meters.md", "home-mini.md"), HelpLink::Page(String::from("octopus/home-mini.md")));
        assert_eq!(follow("octopus/meters.md", "../graphs.md#zooming"), HelpLink::Page(String::from("graphs.md")));
        assert_eq!(follow("home.md", "octopus/index.md"), HelpLink::Page(String::from("octopus/index.md")));
        assert_eq!(follow("home.md", "../ScheduledFetch.md"),
            HelpLink::Web(String::from("https://bruceskingle.github.io/marco-sparko/ScheduledFetch.html")));
        assert_eq!(follow("octopus/index.md", "../../gettingStarted/profiles.md#moving-the-data-directory"),
            HelpLink::Web(String::from("https://bruceskingle.github.io/marco-sparko/gettingStarted/profiles.html#moving-the-data-directory")));
        // Not in the book, so on GitHub.
        assert_eq!(follow("home.md", "../Release.md"),
            HelpLink::Web(String::from("https://github.com/bruceskingle/marco-sparko/blob/main/docs/Release.md")));
        assert_eq!(follow("home.md", "https://octopus.energy"), HelpLink::Web(String::from("https://octopus.energy")));
        assert_eq!(follow("home.md", "#top"), HelpLink::None);
    }

    #[test]
    fn icons_are_drawn_inline() {
        let html = super::to_html("octopus/meters.md", "Click ![Help](../icons/help.svg) to show it");
        assert!(html.starts_with("<p>Click <span class=\"help-icon\" title=\"Help\"><svg "), "{html}");
        assert!(html.ends_with("</svg></span> to show it</p>\n"), "{html}");
    }

    /// Every link on every help page goes somewhere which exists, as far as can be checked here.
    #[test]
    fn links_between_help_pages_are_not_broken() {
        for page in PAGES {
            for href in markdown_links(page.markdown) {
                let HelpLink::Web(url) = follow(page.path, &href) else { continue };
                let url = url.split('#').next().unwrap_or_default();
                let path = match (url.strip_prefix(super::BOOK_URL), url.strip_prefix(super::REPO_URL)) {
                    (Some(doc), _) => format!("docs/{}", doc.replace(".html", ".md")),
                    (_, Some(path)) => path.to_string(),
                    _ => continue,
                };
                assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(&path).exists(), "{} links to {href} which does not exist", page.path);
            }
        }
    }

    /// Every help page is in the book too, see docs/SUMMARY.md.
    #[test]
    fn help_pages_are_in_the_book() {
        for page in PAGES {
            assert!(super::BOOK_SUMMARY.contains(&format!("](help/{})", page.path)), "{} is not in docs/SUMMARY.md", page.path);
        }
    }

    /// The targets of the Markdown links in text.
    fn markdown_links(text: &str) -> Vec<String> {
        text.match_indices("](").filter_map(|(index, _)| {
            let rest = &text[index + 2..];
            rest.find(')').map(|end| rest[..end].to_string())
        }).collect()
    }
}
