//! The views module contains the components for all Layouts and Routes for our app. Each layout and route in our [`Route`]
//! enum will render one of these components.
//!
//!
//! The [`Home`] and [`Blog`] components will be rendered when the current route is [`Route::Home`] or [`Route::Blog`] respectively.
//!
//!
//! The [`Navbar`] component will be rendered on all pages of our app since every page is under the layout. The layout defines
//! a common wrapper around all child routes.

mod home;
pub use home::Home;

mod module;
pub use module::Module;

mod sidebar;

mod polling;

mod navbar;
pub use navbar::Navbar;

mod profile_size;
pub use profile_size::ProfileSize;

mod settings;
pub use settings::Settings;

mod devices;
pub use devices::Devices;

mod climate;
pub use climate::ClimatePage;

pub mod log_panel;
pub mod help_panel;