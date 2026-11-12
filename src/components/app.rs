use std::sync::Arc;

use crate::{AppContext, Args, InitRequested, ModuleRegistrations, views::*};
use dioxus::prelude::*;

// use crate::views::{Blog, Home, Navbar};

/// The Route enum is used to define the structure of internal routes in our app. All route enums need to derive
/// the [`Routable`] trait, which provides the necessary methods for the router to work.
/// 
/// Each variant represents a different URL pattern that can be matched by the router. If that pattern is matched,
/// the components for that route will be rendered.
#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    // The layout attribute defines a wrapper for all routes under the layout. Layouts are great for wrapping
    // many routes with a common UI like a navbar.
    #[layout(Navbar)]
        // The route attribute defines the URL pattern that a specific route matches. If that pattern matches the URL,
        // the component for that route will be rendered. The component name that is rendered defaults to the variant name.
        #[route("/")]
        Home {},
        // The route attribute can include dynamic parameters that implement [`std::str::FromStr`] and [`std::fmt::Display`] with the `:` syntax.
        // In this case, id will match any integer like `/module/123` or `/module/-456`.
        #[route("/module/:module_id")]
        // Fields of the route variant will be passed to the component as props. In this case, the module component must accept
        // an `id` prop of type `i32`.
        Module { module_id: String },
        // The disk space used by a profile, part of the Home app.
        #[route("/profile/:name")]
        ProfileSize { name: String },
        // The app's settings for the active profile.
        #[route("/settings")]
        Settings {},
}

// We can import assets in dioxus with the `asset!` macro. This macro takes a path to an asset relative to the crate root.
// The macro returns an `Asset` type that will display as the path to the asset in the browser or a local path in desktop bundles.
const FAVICON: Asset = asset!("/icons/icon.ico");
// The asset macro also minifies some assets like CSS and JS to make bundled smaller
pub const MAIN_CSS: Asset = asset!("/assets/styling/main.css");

/// The background of the app, as in main.css, for a window's webview and the page before the stylesheet has loaded,
/// so that a window does not flash white as it opens. The page itself, in Dioxus's root element, main, is hidden until
/// main.css has loaded, which shows it, so that it is not seen without its styles for a moment as the app starts. Every
/// window links main.css, the main one in App and the bills' in BillWindow, or it would stay hidden.
pub const BACKGROUND_COLOR: (u8, u8, u8, u8) = (0x0f, 0x11, 0x16, 0xff);
pub const INITIAL_HEAD: &str =
    "<style>html, body { background-color: #0f1116; color: #ffffff; } #main { visibility: hidden; }</style>";

/// App is the main component of our app. Components are the building blocks of dioxus apps. Each component is a function
/// that takes some props and returns an Element. In this case, App takes no props because it is the root of our app.
///
/// Components should be annotated with `#[component]` to support props, better error messages, and autocomplete
#[component]
pub fn App() -> Element {
    // let context = use_context::<Signal<Args>>();
    // let args = (&*context.read()).clone();

    crate::components::child_windows::use_main_window();
    crate::screenshots::use_screenshots(Args::parse_all().screenshots());
    // The app only ends when the user quits it, by closing the main window or with Quit.
    dioxus::desktop::use_wry_event_handler(|event, _| {
        if let dioxus::desktop::tao::event::Event::LoopDestroyed = event {
            crate::polling::record_quit();
            crate::log_info!("Quit by user");
        }
    });

    let args = Args::parse_all();
    let verbose = args.app_args.verbose;
    let mut init_signal = use_signal(|| InitRequested(true));
    let init = init_signal.read().0;
    let mut context_signal = use_signal::<Option<Arc<AppContext>>>(|| None);

    // Store the init signal in context so it can be accessed and reset from anywhere
    use_context_provider::<Signal<InitRequested>>(move || init_signal);

    use_context_provider::<Signal<Option<Arc<AppContext>>>>(move || context_signal);

    use_context_provider::<ModuleRegistrations>(move || ModuleRegistrations::new(verbose));

    use_context_provider::<crate::polling::PollStatuses>(|| Signal::new(Default::default()));

    // let window_icon =dioxus_desktop::tao::window::Icon::from_file(&std::path::Path::new("icons/icon.ico")).unwrap();
    // window.set_window_icon(window_icon);

    if init {
        let app_context = AppContext::new(args)?;
        context_signal.set(Some(app_context));
        // let x: Signal<Arc<AppContext>>;

        init_signal.set(InitRequested(false));

        return rsx! { "Loading..." };
    }

    // let opt_context = &*context_signal.read();
    // let context = opt_context.as_ref().unwrap();
    // let profile = &context.profile.active_profile;

    // if !profile.version.eq(CURRENT_VERSION) {

    //     let profile_version = if profile.version.len() == 0 {"0.0.0"} else { &profile.version };

    //     return rsx! {
    //         ErrorBoundary {
    //             handle_error: |errors: ErrorContext| {
    //                 rsx! {

    //                     div { "Oops, we encountered an error. Please report this to the developer of this application" }

    //                     pre { "{errors:?}" }
    //                 }
    //             },

    //             // In addition to element and text (which we will see later), rsx can contain other components. In this case,
    //             // we are using the `document::Link` component to add a link to our favicon and main CSS file into the head of our app.
    //             document::Link { rel: "icon", href: FAVICON }
    //             document::Link { rel: "stylesheet", href: MAIN_CSS }

    //             div { class: "dialog",
    //                 p {
    //                     "Profile {profile.name} is version {profile_version} but the current version is {CURRENT_VERSION}"
    //                 }
    //                 p { "Do you want to upgrade it to the current version?" }
    //             }
    //         }
    //     }
    // }

    rsx! {
        ErrorBoundary {
            handle_error: |errors: ErrorContext| {
                rsx! {

                    div { "Oops, we encountered an error. Please report this to the developer of this application" }

                    pre { "{errors:?}" }
                }
            },

            // In addition to element and text (which we will see later), rsx can contain other components. In this case,
            // we are using the `document::Link` component to add a link to our favicon and main CSS file into the head of our app.
            document::Link { rel: "icon", href: FAVICON }
            document::Link { rel: "stylesheet", href: MAIN_CSS }
            crate::components::tooltips::Tooltips {}

            Router::<Route> {}

            // A list of one, keyed by the context, so that a new profile's modules are polled, as Dioxus only uses
            // keys in lists.
            for context in context_signal.read().iter().cloned() {
                crate::polling::Poller {
                    key: "{Arc::as_ptr(&context):p}",
                    context: crate::util::SharedProp(context),
                }
            }
        }
    }
}
