mod ordered_map;
pub use ordered_map::OrderedMap;

mod cache_browser;
pub mod capture_device;
pub mod climate;
pub mod components;
mod globird;
mod http;
mod myenergi;
mod octopus;
pub mod period;
pub mod polling;
pub mod power;
mod profile;
mod tesla;
pub mod util;
mod views;

pub mod clock;
pub mod data_home;
pub mod help;
pub mod logging;
mod private_file;
pub mod redact;
pub mod screenshots;
mod secrets;

#[cfg(feature = "desktop")]
pub mod window_state;

mod cache;
pub mod convert;
mod data_set;
/// The kinds capture devices write, shared with the firmware, see device_kinds.rs.
pub mod device_kinds;
pub mod store;
use cache::CacheConfig;

use anyhow::anyhow;
use async_trait::async_trait;
use clap::Parser;
use dioxus::core::Element;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};

use crate::profile::ActiveProfile;

const NONE: &'static str = "None";
const NULL: &'static str = "Null";

/*
 * It is important that the names of all args here do not contain hyphens or underscores
 * as these are used to separate module args from main args.
 *
 * For example, to pass an arg to the octopus module, use --octopus-<arg_name> and in the corresponding Arg struct in the octopus module
 * all field names must begin with octopus_ so octopus_arg_name in this example.
 *
 * Underscores in the names of fields on this struct are translated into hyphens on the command line automatically by clap.
 */
#[derive(Parser, Debug, Clone, PartialEq)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
pub struct AppArgs {
    /// Name of the config profile to use
    #[arg(short, long)]
    profile: Option<String>,
    #[arg(short, long, value_delimiter = ',', num_args = 1..)]
    modules: Vec<String>,
    #[arg(short, long)]
    debug: bool,
    #[arg(short, long)]
    verbose: bool,
    /// The directory holding .bruce-smart-home and .bruce-smart-home-data, instead of the home directory
    #[arg(long)]
    home: Option<PathBuf>,
    /// Fetch the latest data of every module for the last 30 days, without opening a window, then exit, e.g. from a
    /// scheduled task, see docs/ScheduledFetch.md
    #[arg(long)]
    fetch: bool,
    /// With a profile and a new profile's name, copy the profile's data to the new, offline, profile with everything
    /// which identifies you replaced. With docs, redact the documents in unredacted_docs into docs, run in the
    /// repository's directory. Either way without opening a window, then exit, see docs/Redaction.md. Options of the
    /// app itself have no hyphens, which would make them a module's, see Args::extract_module.
    #[arg(long, num_args = 1..=2, value_names = ["PROFILE|docs", "NEW_PROFILE"])]
    redact: Vec<String>,
    /// Take the screenshots for the documentation listed in the file, docs/screenshots.json if it is not given, then
    /// exit. Use with --profile demo, see docs/Screenshots.md
    #[arg(long, num_args = 0..=1, value_name = "SHOT_LIST", default_missing_value = screenshots::DEFAULT_FILE)]
    screenshots: Option<PathBuf>,
    /// For developers: GET a path of the API of one of the profile's capture devices, by its ID, signed with the key the
    /// app keeps for it, which is not shown, print the device's response, then exit, e.g.
    /// --deviceget 441bf6cc7fcc "capture/climate/records?limit=5". No hyphen, see redact.
    #[arg(long, num_args = 2, value_names = ["DEVICE", "PATH"])]
    deviceget: Vec<String>,
    /// For developers: POST a JSON body to a path of the API of one of the profile's capture devices, as --deviceget
    /// does, e.g. --devicepost f85b1b909f50 config '{"features":{"Store":{"enabled":true}}}'. No hyphen, see redact.
    #[arg(long, num_args = 3, value_names = ["DEVICE", "PATH", "JSON"])]
    devicepost: Vec<String>,
    /// Upgrade a profile to the current version, as the Home page does when asked, e.g. converting what one made before
    /// the store kept in the cache directory into its store, leaving the cache as it was, see profile::upgrade_profile,
    /// then exit. No hyphen, see redact.
    #[arg(long, value_name = "PROFILE")]
    upgrade: Option<String>,
    /// Temporary, until 0.5.0 is released: encrypt the secrets of a profile already upgraded to 0.5.0 before they were
    /// encrypted, as the upgrade to 0.5.0 now does, see profile::encrypt_secrets, then exit. No hyphen, see redact.
    #[arg(long, value_name = "PROFILE")]
    encryptsecrets: Option<String>,
}

pub struct Args {
    pub app_args: AppArgs,
    pub module_args: HashMap<String, Vec<String>>,
}

impl Args {
    fn extract_module(s: &str) -> Option<&str> {
        // Only the name identifies the module, the value may contain hyphens, e.g. --home=/tmp/ms-test
        let s = s.trim_start_matches('-').split('=').next().unwrap_or_default();
        if let Some((module, _)) = s.split_once('-') {
            if module.trim().is_empty() { None } else { Some(module) }
        } else {
            None
        }
    }

    pub fn parse_all() -> Self {
        let mut main_only_args = Vec::new();
        let mut module_args: HashMap<String, Vec<String>> = HashMap::new();
        let mut module_id: Option<String> = None;

        let mut it = std::env::args();
        let arg0 = it.next().unwrap_or_else(|| "bruce-smart-home".to_string());
        main_only_args.push(arg0.clone());
        for arg in it {
            if arg.starts_with("-") {
                if let Some(module) = Args::extract_module(&arg) {
                    module_id = Some(module.to_string());
                }
            }
            if let Some(module) = &module_id {
                module_args
                    .entry(module.clone())
                    .or_insert_with(|| {
                        let mut v = Vec::new();
                        v.push(arg0.clone());
                        v
                    })
                    .push(arg);
            } else {
                main_only_args.push(arg);
            }
        }

        let app_args = AppArgs::parse_from(main_only_args);

        if app_args.verbose {
            println!("Main args: {:?}", app_args);
            // Only the names of the module args are shown as their values may be secrets, e.g. -o for an API key, which
            // modules show redacted once they are parsed.
            for (module_id, args) in &module_args {
                let names: Vec<&str> = args
                    .iter()
                    .filter(|arg| arg.starts_with('-'))
                    .map(|arg| arg.split('=').next().unwrap_or_default())
                    .collect();
                println!("Module '{}' args: {:?}", module_id, names);
            }
        }

        Args { app_args, module_args }
    }

    pub fn module_args(&self, module_name: &str) -> Option<Vec<String>> {
        self.module_args.get(module_name).cloned()
    }

    /// Whether --fetch was given, to fetch the latest data without a window, see fetch_latest_data.
    /// The device and the path to GET, with --deviceget, see AppArgs.
    pub fn capture_get(&self) -> Option<(&str, &str)> {
        match self.app_args.deviceget.as_slice() {
            [device, path] => Some((device.as_str(), path.as_str())),
            _ => None,
        }
    }

    /// The device, the path and the body to POST, with --devicepost, see AppArgs.
    pub fn capture_post(&self) -> Option<(&str, &str, &str)> {
        match self.app_args.devicepost.as_slice() {
            [device, path, body] => Some((device.as_str(), path.as_str(), body.as_str())),
            _ => None,
        }
    }

    pub fn fetch(&self) -> bool {
        self.app_args.fetch
    }

    /// The shot list, if --screenshots was given, see screenshots.rs.
    pub fn screenshots(&self) -> Option<PathBuf> {
        self.app_args.screenshots.clone()
    }

    /// The profile to upgrade, if --upgrade was given, see profile::upgrade_profile.
    pub fn upgrade(&self) -> Option<&str> {
        self.app_args.upgrade.as_deref()
    }

    /// The profile whose secrets to encrypt, if --encryptsecrets was given, see profile::encrypt_secrets.
    pub fn encrypt_secrets(&self) -> Option<&str> {
        self.app_args.encryptsecrets.as_deref()
    }

    /// Whether --redact docs was given, see redact::redact_docs.
    pub fn redact_docs(&self) -> bool {
        matches!(self.app_args.redact.as_slice(), [docs] if docs == "docs")
    }

    /// The profile to redact and the name of the redacted copy, if --redact was given, see redact.rs.
    pub fn redact(&self) -> Option<(&str, &str)> {
        match self.app_args.redact.as_slice() {
            [from, to] => Some((from, to)),
            _ => None,
        }
    }
}

/// Fetches the latest data of each module of the profile, see Module::fetch_latest_data, without a window, for --fetch.
/// A module which is not logged in, or whose fetch fails, is reported and the others are still fetched. Returns whether
/// they all succeeded, for the exit status.
pub fn fetch_latest_data(args: Args) -> bool {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Unable to start: {error}");
            return false;
        }
    };
    println!(
        "Bruce's Smart Home {} fetching the latest data at {} UTC",
        env!("CARGO_PKG_VERSION"),
        time::OffsetDateTime::now_utc()
    );
    let verbose = args.app_args.verbose;
    let result = runtime.block_on(async move {
        let context = AppContext::new(args)?;
        let registrations = ModuleRegistrations::new(verbose);
        let module_ids: Vec<String> = context.profile.active_profile.modules.keys().cloned().collect();
        let devices = capture_device::adopted_devices(&context.profile.active_profile.name);
        if module_ids.is_empty() && devices.is_empty() {
            return Err(anyhow!(
                "Profile {} has no modules set up, open the app and log in first",
                context.profile.active_profile.name
            ));
        }
        let mut all_ok = true;
        // What the capture devices on the store have captured, first, so that the modules see it, see
        // capture_device::replicate. Those with older firmware are read by the modules, as they were.
        if !context.profile.active_profile.offline && !devices.is_empty() {
            let store = context.store()?;
            for device in &devices {
                let progress = &mut |message: String| println!("{message}");
                match capture_device::replicate(&store, device, progress).await {
                    Ok(Some(_)) => println!("Capture device {}: done", device.id),
                    Ok(None) => println!("Capture device {}: its firmware is from before the store", device.id),
                    Err(error) => {
                        println!("Capture device {}: FAILED: {error:#}", device.id);
                        log_error!("Replicating the capture device {} failed: {error:#}", device.id);
                        all_ok = false;
                    }
                }
            }
        }
        for module_id in module_ids {
            println!("Module {module_id}");
            log_info!("Fetching the latest data of {module_id}");
            let fetched = async {
                let factory = registrations.module_factory(&module_id, &context).await?;
                if !factory.is_ready().await? {
                    return Err(anyhow!("Not logged in, open the app and log in first"));
                }
                factory.build().await?.fetch_latest_data().await
            }
            .await;
            match fetched {
                Ok(()) => {
                    println!("Module {module_id}: done");
                    log_info!("Fetched the latest data of {module_id}");
                }
                Err(error) => {
                    println!("Module {module_id}: FAILED: {error:#}");
                    log_error!("Fetching the latest data of {module_id} failed: {error:#}");
                    all_ok = false;
                }
            }
        }
        anyhow::Ok(all_ok)
    });
    let all_ok = match result {
        Ok(all_ok) => {
            println!("Finished{}", if all_ok { "" } else { " with errors" });
            all_ok
        }
        Err(error) => {
            println!("FAILED: {error:#}");
            log_error!("Fetching the latest data failed: {error:#}");
            false
        }
    };
    // Any errors have been logged already, this says the fetch ended rather than crashed.
    log_info!("Terminating normally");
    all_ok
}

#[derive(Clone, PartialEq)]
pub struct PageInfo {
    label: &'static str,
    path: &'static str,
}

//  let x: fn(ModuleProps) -> std::result::Result<VNode, RenderError> = Module;
/// How many days back --fetch fetches, see Module::fetch_latest_data.
pub const FETCH_DAYS: i64 = 30;

/// The labels the modules give energy, e.g. Home Low Rate, with their default colours, in the order they are shown,
/// each module's own, then the app's, see components::category_filter.
pub fn known_labels() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    octopus::LABELS
        .iter()
        .chain(&power::combine::LABELS)
        .chain(&globird::LABELS)
        .chain(&power::LABELS)
        .chain(&power::flows::LABELS)
}

// Not Send, as fetch_latest_data is run on one thread, see fetch_latest_data, and the data sets' futures are not Send.
#[async_trait(?Send)]
pub trait Module {
    fn get_page_list(&self) -> Vec<PageInfo>;
    fn module_id(&self) -> &'static str;
    fn get_component(&self, page_id: &str, path: Vec<String>) -> Element;
    /// The help page of the page page_id showing path, relative to docs/help, see help.rs, e.g. octopus/meters.md
    fn help_topic(&self, page_id: &str, path: &[String]) -> String;
    fn cli_debug(&self) -> anyhow::Result<()>;
    /// The site's electricity, for Home's Electricity page, see views/electricity.rs, if the module has it, e.g. a
    /// supplier's meters, or a device which measures the grid, None if it does not.
    fn electricity_source(&self) -> Option<std::sync::Arc<dyn power::ElectricitySource>> {
        None
    }
    /// What the module's devices measured of the site's use, which splits a supplier's electricity by use, see
    /// power::measured, None if it has none.
    fn measurement_source(&self) -> Option<std::sync::Arc<dyn power::measured::MeasurementSource>> {
        None
    }
    /// The places the module measures, which the site's topology places, see power::topology, e.g. the billing
    /// meter, or a gateway's meters.
    fn channels(&self) -> Vec<power::topology::Channel> {
        Vec::new()
    }
    /// Fetches the latest data of every page, as if each was shown with the last FETCH_DAYS days, printing its progress,
    /// for --fetch. Data which is already cached and up to date is not fetched again.
    async fn fetch_latest_data(&self) -> anyhow::Result<()>;
    /// How often the app polls the module while it is open, see polling.rs, None if it is not polled.
    fn poll_interval(&self) -> Option<std::time::Duration> {
        None
    }
    /// Polls once, e.g. a snapshot of a device which keeps no history of its own, returning a short summary of what it
    /// found, for the Polling page and the log. slot is the time the poll is scheduled for, which the sample is saved
    /// with, see polling::next_slot, or None if it is not a scheduled poll, e.g. the first.
    async fn poll(&self, _slot: Option<time::OffsetDateTime>) -> anyhow::Result<String> {
        Err(anyhow!("{} is not polled", self.module_id()))
    }
    /// When to poll next, after now, if not on the next slot of poll_interval, e.g. soon after a capture device which
    /// does the polling has polled, see capture_device::CaptureReader::next_read.
    fn next_poll(&self, _now: time::OffsetDateTime) -> Option<time::OffsetDateTime> {
        None
    }
}

#[async_trait]
pub trait ModuleFactory: Send {
    async fn is_ready(&self) -> anyhow::Result<bool>;
    fn init_page(&self) -> Element;
    async fn build(&self) -> anyhow::Result<Box<dyn Module + Send>>;
}

pub type ModuleFactoryConstructor =
    dyn Fn(Arc<AppContext>, Option<serde_json::Value>) -> anyhow::Result<Arc<dyn ModuleFactory>>;

pub struct ModuleRegistration {
    pub module_id: String,
    pub constructor: Arc<ModuleFactoryConstructor>,
    /// Converts what the module kept in its directory of the cache before the store into the profile's store, for
    /// --convert, see convert.rs, None if it has nothing to convert.
    pub convert: Option<convert::Converter>,
}

#[derive(Clone, Default)]
pub struct ModuleRegistrations(Arc<HashMap<String, ModuleRegistration>>);

impl std::fmt::Debug for ModuleRegistrations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for k in self.0.keys() {
            write!(f, "{}, ", k)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

impl PartialEq for ModuleRegistrations {
    fn eq(&self, other: &Self) -> bool {
        let s = self.0.keys();
        let o = other.0.keys();
        let mut it = o.into_iter();

        for k in s {
            if let Some(other_key) = it.next() {
                if k != other_key {
                    return false;
                }
            }
        }

        it.next().is_none()
    }
}

impl ModuleRegistrations {
    /// Every module's registration.
    fn all() -> Vec<ModuleRegistration> {
        vec![
            octopus::OctopusModule::registration(),
            myenergi::MyenergiModule::registration(),
            tesla::TeslaModule::registration(),
            globird::GlobirdModule::registration(),
        ]
    }

    /// Each module's converter, by its ID, see ModuleRegistration::convert.
    pub fn converters() -> Vec<(String, convert::Converter)> {
        Self::all()
            .into_iter()
            .filter_map(|registration| Some((registration.module_id, registration.convert?)))
            .collect()
    }

    pub async fn module_factory(
        &self,
        module_id: &str,
        context: &Arc<AppContext>,
    ) -> anyhow::Result<Arc<dyn ModuleFactory + Send>> {
        if let Some(module_registration) = self.0.get(module_id) {
            let constructor = module_registration.constructor.as_ref();
            let profile = if let Some(value) = context.profile.active_profile.modules.get(module_id) {
                Some(value.clone())
            } else {
                None
            };

            if context.args.app_args.verbose {
                let mut redacted_profile = profile.clone();
                if let Some(value) = &mut redacted_profile {
                    bruce_graphql::redact::redact_secrets(value);
                }
                println!(
                    "Initializing module '{}' with profile '{:?}'",
                    module_id, redacted_profile
                );
            }
            let builder = constructor(context.clone(), profile)?;

            Ok(builder)
        } else {
            return Err(anyhow!(format!("Unknown module \"{}\"", module_id)));
        }
    }

    fn new(verbose: bool) -> ModuleRegistrations {
        let dir = std::env::current_dir().unwrap();

        if verbose {
            println!("Current directory is {}", dir.display());
        }
        //assert!(cfg!(debug_assertions));

        let mut module_registrations = HashMap::new();

        for registration in Self::all() {
            Self::load_module(&mut module_registrations, registration, verbose);
        }

        if verbose {
            println!("Loaded {} modules", module_registrations.len());

            for (k, _v) in &module_registrations {
                println!(" Module {}", k);
            }
        }
        ModuleRegistrations(Arc::new(module_registrations))
    }

    fn load_module(
        module_registrations: &mut HashMap<String, ModuleRegistration>,
        registration: ModuleRegistration,
        verbose: bool,
    ) {
        if verbose {
            println!("Load module {}", &registration.module_id);
        }
        module_registrations.insert(registration.module_id.clone(), registration);
    }
}

pub struct AppContext {
    pub args: Args,
    pub profile: ActiveProfile,
    /// The profile's site's store, opened the first time it is needed, see store.
    pub store: std::sync::OnceLock<Arc<bruce_db::Store>>,
}

#[derive(Clone, Copy, Default)]
pub struct UpgradeRequested(pub bool);

#[derive(Clone, Copy, Default)]
pub struct InitRequested(pub bool);

// impl PartialEq for AppContext {
//     fn eq(&self, other: &Self) -> bool {
//         self.args == other.args && self.profile == other.profile
//     }
// }

impl AppContext {
    /// The profile file, see data_home.
    pub fn get_file_path() -> anyhow::Result<PathBuf> {
        data_home::profiles_path()
    }

    /// The old cache directory, from before 0.5.0, holding each profile's modules' data, which is only read, to
    /// convert a profile's data into its store, see data_home::old_cache_dir.
    pub fn get_cache_dir_path() -> anyhow::Result<PathBuf> {
        data_home::old_cache_dir()
    }

    pub fn new(args: Args) -> anyhow::Result<Arc<AppContext>> {
        let profile = crate::profile::fetch_active_profile(&args.app_args.profile)?;
        Ok(Self::for_profile(args, profile))
    }

    /// The context with the given profile active, starting its log, see logging.rs.
    pub fn for_profile(args: Args, profile: ActiveProfile) -> Arc<AppContext> {
        // An offline profile is shown as it was when its data was last fetched.
        clock::set_as_of(profile.active_profile.as_of.filter(|_| profile.active_profile.offline));
        help::start(&profile.active_profile.name, &profile.active_profile.settings.help);
        let process = if args.fetch() {
            logging::Process::Fetch
        } else {
            logging::Process::App
        };
        logging::start(
            &profile.active_profile.name,
            &profile.active_profile.settings.log,
            process,
        );
        Arc::new(AppContext {
            args,
            profile,
            store: Default::default(),
        })
    }

    // pub fn with_profile(&self, profile_name: &String) -> anyhow::Result<Arc<AppContext>> {
    //     Ok(Arc::new(AppContext {
    //         args: self.args.clone(),
    //         profile: crate::profile::set_active_profile(profile_name)?,
    //    }))
    // }

    /// The profile's site's store, see store.rs, opened, and the site made, the first time it is needed.
    pub fn store(&self) -> anyhow::Result<Arc<bruce_db::Store>> {
        if let Some(store) = self.store.get() {
            return Ok(store.clone());
        }
        let store = store::for_profile(&self.profile.active_profile.name)?;
        Ok(self.store.get_or_init(|| store).clone())
    }

    fn get_cache_file_path(&self, module_id: &str) -> anyhow::Result<PathBuf> {
        let profile_name = &self.profile.active_profile.name;
        Ok(data_home::tokens_dir()?.join(format!("{}-{}.json", profile_name, module_id)))
    }

    fn get_cache_data_dir_path(&self, module_id: &str) -> anyhow::Result<PathBuf> {
        let profile_name = &self.profile.active_profile.name;
        Ok(Self::get_cache_dir_path()?.join(format!("{}-{}", profile_name, module_id)))
    }

    fn create_cache_config(&self, module_id: &str) -> anyhow::Result<Arc<CacheConfig>> {
        // Only read, see get_cache_dir_path, so not made if it is not there.
        let dir_path = self.get_cache_data_dir_path(module_id)?;

        Ok(Arc::new(
            CacheConfig::new(dir_path, self.profile.active_profile.offline).with_store(self.store()?),
        ))
    }

    /// The module's tokens, decrypted, see secrets::unseal, or None if it has none, or they cannot be read, when it logs
    /// in again.
    pub fn read_cache<T>(&self, module_id: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        if let Ok(path) = self.get_cache_file_path(module_id) {
            if let Ok(reader) = fs::File::open(path) {
                let result = serde_json::from_reader(reader)
                    .map_err(anyhow::Error::from)
                    .and_then(secrets::unseal)
                    .and_then(|value| Ok(serde_json::from_value(value)?));
                match result {
                    Ok(result) => return Some(result),
                    Err(error) => println!("ERROR reading cached token: {error:#}"),
                }
            }
        }
        return None;
    }

    /// Saves the module's tokens, encrypted whole, see secrets::seal, as they are all secret.
    pub fn update_cache<T>(&self, module_id: &str, profile: &T) -> anyhow::Result<()>
    where
        T: Serialize,
    {
        let path = self.get_cache_file_path(module_id)?;

        serde_json::to_writer_pretty(
            private_file::create_private_file(path)?,
            &secrets::seal(&serde_json::to_value(profile)?)?,
        )?;

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::Args;

    #[test]
    fn the_module_comes_from_the_name_of_an_arg_not_its_value() {
        assert_eq!(Args::extract_module("--octopus-api-key=abc"), Some("octopus"));
        assert_eq!(Args::extract_module("--home=/tmp/ms-test"), None);
        assert_eq!(Args::extract_module("--profile"), None);
        assert_eq!(Args::extract_module("-v"), None);
    }
}
