use marco_sparko::components::app::App;

fn main() {
    // With --fetch the latest data is fetched without a window, e.g. from a scheduled task, then the app exits.
    let args = marco_sparko::Args::ms_parse();
    // With --redact a profile's data is copied to a redacted, offline, profile, then the app exits.
    if let Some((from, to)) = args.redact() {
        if let Err(error) = marco_sparko::redact::redact_profile(from, to) {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if let Some(profile) = args.upgrade() {
        if let Err(error) = marco_sparko::convert::upgrade(profile) {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if args.redact_docs() {
        if let Err(error) = marco_sparko::redact::redact_docs() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if let Some((device, path)) = args.capture_get() {
        if let Err(error) = marco_sparko::capture_device::cli_get(&args, device, path) {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if args.fetch() {
        std::process::exit(if marco_sparko::fetch_latest_data(args) { 0 } else { 1 });
    }

    // For screenshots the window is the size the shot list gives, see screenshots.rs.
    let screenshot_size = match args.screenshots() {
        Some(path) => match marco_sparko::screenshots::read_shot_file(&path) {
            Ok(shot_file) => {
                if let Some(fresh) = &shot_file.fresh_data_directory
                    && let Err(error) = marco_sparko::screenshots::prepare_data_directory(fresh)
                {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
                Some(shot_file.window)
            },
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            },
        },
        None => None,
    };

    #[cfg(feature = "desktop")]
    fn launch_app(screenshot_size: Option<marco_sparko::screenshots::WindowSize>) {
        // The size is the default for the first run, after that the size from the last run is used.
        let window = dioxus::desktop::tao::window::WindowBuilder::new()
            .with_title("Marco Sparko")
            .with_inner_size(dioxus::desktop::LogicalSize::new(1400, 768))
            .with_resizable(true);

        let config = dioxus::desktop::Config::new()
            // The app's own menu bar, whose Window menu lists the windows.
            .with_menu(marco_sparko::components::child_windows::menu_bar())
            .with_background_color(marco_sparko::components::app::BACKGROUND_COLOR)
            .with_custom_head(marco_sparko::components::app::INITIAL_HEAD.to_string())
            // The webview's context menu (Reload, Inspect Element...) is only offered in debug builds, its
            // Reload would restart the app in a broken state.
            .with_disable_context_menu(!cfg!(debug_assertions));

        let config = match screenshot_size {
            // Not remembered as the size of the app's window.
            Some(size) => config.with_window(window.with_inner_size(dioxus::desktop::LogicalSize::new(size.width, size.height))),
            None => marco_sparko::window_state::remember_window_state(config, window),
        };
        dioxus::LaunchBuilder::new()
            .with_cfg(config)
            .launch(App);
    }

    #[cfg(not(feature = "desktop"))]
    fn launch_app(_screenshot_size: Option<marco_sparko::screenshots::WindowSize>) {
        dioxus::launch(App);
    }

    launch_app(screenshot_size);
}