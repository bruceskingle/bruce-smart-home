use std::collections::{BTreeMap, HashSet};
use std::{collections::HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use indexmap::IndexMap;
use semver::Version;
use serde::{Deserialize, Serialize};
use anyhow::anyhow;

use crate::{MarcoSparkoContext, private_file};

const DEFAULT_PROFILE: &str = "default";
/// The version of the profile's format, and of the data kept for it. A change to either which needs a profile's data
/// changing too adds a step to upgrade_profile, and makes this the version it is in.
pub const CURRENT_VERSION: &str = "0.5.0";

/// The last version whose profiles kept their data in Marco Sparko's cache directory, for a profile whose data is
/// still made there, e.g. a redacted copy, see save_redacted_profile, which is upgraded to the store when it is opened.
const LAST_CACHE_VERSION: &str = "0.4.3";

/// The version whose profiles keep their data in the store, see store.rs, rather than in the cache directory, see
/// upgrade_profile.
const STORE_VERSION: &str = "0.5.0";

#[derive(PartialEq)]
pub struct ActiveProfile {
    pub all_profiles:   Vec<String>,
    pub active_profile: Profile,
    pub profile_version_state: ProfileVersionState,
}

pub fn profile_names_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// The profiles in the profile file, with their secrets decrypted, see secrets.rs.
fn read_profile_file() -> anyhow::Result<ProfileFile> {
    read_profile_file_at(&MarcoSparkoContext::get_file_path()?)
}

/// The profiles in the profile file, with their secrets decrypted, see secrets.rs, or None if it cannot be opened, e.g.
/// before the first profile is made.
pub fn read_profile_file_if_any() -> anyhow::Result<Option<ProfileFile>> {
    let path = MarcoSparkoContext::get_file_path()?;
    if fs::File::open(&path).is_err() {
        return Ok(None);
    }
    Ok(Some(read_profile_file_at(&path)?))
}

/// The profiles in the profile file at path, e.g. another data home's, with their secrets decrypted, see secrets.rs.
pub fn read_profile_file_at(path: &Path) -> anyhow::Result<ProfileFile> {
    let mut value: serde_json::Value = serde_json::from_reader(fs::File::open(path)?)?;
    crate::secrets::decrypt_secrets(&mut value)?;
    Ok(serde_json::from_value(value)?)
}

/// Saves the profiles to the profile file, with their secrets encrypted, see secrets.rs, whichever version each is.
pub fn write_profile_file(profiles: &impl Serialize) -> anyhow::Result<()> {
    write_profile_file_at(&MarcoSparkoContext::get_file_path()?, profiles)
}

/// Saves the profiles to the profile file at path, e.g. another data home's, with their secrets encrypted, see
/// secrets.rs.
pub fn write_profile_file_at(path: &Path, profiles: &impl Serialize) -> anyhow::Result<()> {
    let mut value = serde_json::to_value(profiles)?;
    crate::secrets::encrypt_secrets(&mut value)?;
    serde_json::to_writer_pretty(private_file::create_private_file(path)?, &value)?;
    Ok(())
}

impl ActiveProfile {
    // Create empty default profile
    fn new() -> ActiveProfile {
        ActiveProfile {
            all_profiles: vec!(String::from(DEFAULT_PROFILE)),
            active_profile: Profile::new(),
            profile_version_state: ProfileVersionState::Current,
        }
    }
}

pub fn fetch_active_profile(profile_name: &Option<String>) -> anyhow::Result<ActiveProfile> {
    let mut all_profiles: Vec<String> = Vec::new();
    let mut set: HashSet<String> = HashSet::new();
    let mut active_profile = None;

    if let Some(profile_file) = read_profile_file_if_any()? {

        for profile in profile_file {
            if all_profiles.iter().any(|name| profile_names_equal(name, &profile.name)) {
                return Err(anyhow!("Duplicate profile \"{}\"", &profile.name));
            }
            set.insert(profile.name.clone());
            
            let got_it = if let Some(name) = profile_name {
                profile_names_equal(name, &profile.name)
            }
            else {
                active_profile.is_none()
            };

            if got_it {
                all_profiles.push(profile.name.clone());
                active_profile = Some(profile)
            }
            else {
                all_profiles.push(profile.name);
            }
        }
    }

    if let Some(name) = profile_name {
        if active_profile.is_none() {
            return Err(anyhow!("No such profile \"{}\"", name));
        }
    }

    Ok(
        
        if let Some(active_profile) = active_profile
        {
            let profile_version_state = ProfileVersionState::from(&active_profile.version)?;
            ActiveProfile {
                all_profiles,
                active_profile: active_profile,
                profile_version_state
            }
        }
        else
        {
            let active_profile = ActiveProfile::new();
            let mut profiles = Vec::new();

            profiles.push(&active_profile.active_profile);

            write_profile_file(&profiles)?;

            active_profile
        }
    )
}

pub fn set_active_profile(profile_name: &String) -> anyhow::Result<ActiveProfile> {

    let mut all_profiles: Vec<String> = Vec::new();
    let mut map: IndexMap<String, Profile> = IndexMap::new();
    

    if let Some(profile_file) = read_profile_file_if_any()? {

        for profile in profile_file {
            if map.keys().any(|name| profile_names_equal(name, &profile.name)) {
                return Err(anyhow!("Duplicate profile \"{}\"", &profile.name));
            }
            all_profiles.push(profile.name.clone());
            map.insert(profile.name.clone(), profile);
        }
        let active_profile = if let Some(existing_name) = map.keys().find(|name| profile_names_equal(name, profile_name)).cloned() {
            map.shift_remove(&existing_name).unwrap()
        }
        else {
            return Err(anyhow!("No such profile \"{}\"", profile_name)); 
        };

        let mut profiles = Vec::new();

        profiles.push(&active_profile);
        profiles.extend(map.values());

        write_profile_file(&profiles)?;
        let profile_version_state = ProfileVersionState::from(&active_profile.version)?;

        Ok(ActiveProfile {
            all_profiles,
            active_profile,
            profile_version_state,
        })
    }
    else {
        Err(anyhow!("No such profile \"{}\" (no profiles at all, in fact)", profile_name))
    }
}

/// Upgrades the profile from its version to CURRENT_VERSION, doing each step its version needs, then saves it as the
/// current version and makes it the active profile, which it returns. A step for each version which changed what is
/// kept, in order:
///
/// - 0.5.0: the data kept in the cache directory is converted into the profile's site's store, see convert.rs, and its
///   secrets are encrypted, see encrypt_secrets.
///
/// Called when the user agrees to upgrade a profile on the Home page, and for --upgrade, saying how far it is with
/// progress. If a step fails, or progress is cancelled, the profile is left at its version, its old data as it was, so
/// that it can be upgraded again.
pub fn upgrade_profile(profile_name: &str, progress: &mut dyn bruce_db::progress::Progress) -> anyhow::Result<ActiveProfile> {
    let profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    let name = profile.name.clone();
    // A profile with no version is from before they had one, so older than any.
    let version = Version::parse(profile.version.trim()).unwrap_or_else(|_| Version::new(0, 0, 0));
    if version < Version::parse(STORE_VERSION)? {
        crate::convert::convert_profile(&name, progress)?;
        encrypt_secrets(&name)?;
    }

    // Read again, as a step may have changed it, e.g. given it a site.
    let mut profile_file: ProfileFile = read_profile_file()?;
    if let Some(profile) = profile_file.iter_mut().find(|profile| profile_names_equal(&profile.name, &name)) {
        profile.version = CURRENT_VERSION.to_string();
    }
    write_profile_file(&profile_file)?;
    set_active_profile(&name)
}

/// Encrypts the profile's secrets, see secrets.rs: its logins in the profile file, with every other profile's, as the
/// file is saved with them encrypted whichever profile they are, afresh, with new nonces, each time, and its modules'
/// token files, which are encrypted whole, those already encrypted being left as they are. A step of the upgrade to
/// 0.5.0, see upgrade_profile, and, until 0.5.0 is released, of --encryptsecrets, for the profiles upgraded to 0.5.0
/// before secrets were encrypted.
pub fn encrypt_secrets(profile_name: &str) -> anyhow::Result<()> {
    let profile_file = read_profile_file()?;
    if !profile_file.iter().any(|profile| profile_names_equal(&profile.name, profile_name)) {
        return Err(anyhow!("No such profile \"{}\"", profile_name));
    }
    write_profile_file(&profile_file)?;
    for path in cache_usage_in(&crate::data_home::tokens_dir()?, profile_name)?.token_files {
        let value: serde_json::Value = serde_json::from_reader(fs::File::open(&path)?)
            .map_err(|error| anyhow!("{} is not JSON: {error}", path.display()))?;
        serde_json::to_writer_pretty(private_file::create_private_file(&path)?, &crate::secrets::seal(&value)?)?;
    }
    Ok(())
}

/// Whether a profile may have the name, which is letters, digits and underscores, so that the hyphen in the name of a
/// cache directory, e.g. n2-octopus, ends the profile's name, see cache_usage_in.
pub fn is_valid_profile_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

pub fn create_profile(profile_name: &str) -> anyhow::Result<ActiveProfile> {
    let profile_name = profile_name.trim();
    if profile_name.is_empty() {
        return Err(anyhow!("Profile name cannot be empty"));
    }
    if !is_valid_profile_name(profile_name) {
        return Err(anyhow!("Profile name must contain only letters, digits and underscores"));
    }

    let mut profiles: ProfileFile = read_profile_file_if_any()?.unwrap_or_default();

    if profiles.iter().any(|profile| profile_names_equal(&profile.name, profile_name)) {
        return Err(anyhow!("A profile named \"{}\" already exists", profile_name));
    }

    let mut active_profile = Profile::new();
    active_profile.name = profile_name.to_string();
    profiles.insert(0, active_profile.clone());

    write_profile_file(&profiles)?;

    let all_profiles = profiles.into_iter().map(|profile| profile.name).collect();
    let profile_version_state = ProfileVersionState::from(&active_profile.version)?;

    Ok(ActiveProfile {
        all_profiles,
        active_profile,
        profile_version_state,
    })
}

pub fn update_profile<T>(profile_name: &String, module_id: &str, module_profile: &T) -> anyhow::Result<()>
    where
    T: Serialize
{
    if let Some(mut profile_file) = read_profile_file_if_any()? {


        let mut found = false;

        for profile in profile_file.iter_mut() {
            if profile_names_equal(&profile.name, profile_name) {
                profile.modules.insert(module_id.to_string(), serde_json::to_value(module_profile)?);
                found = true;

            }
        }
        if ! found{
            return Err(anyhow!("No such profile \"{}\"", profile_name)); 
        };
    
        write_profile_file(&profile_file)?;

        Ok(())
    }
    else {
        Err(anyhow!("No such profile \"{}\" (no profiles at all, in fact)", profile_name))
    }
}

/// Saves the profile name, offline, as a redacted copy of source, see redact.rs, replacing any previous copy. Its
/// modules are set up as the source's are, without their logins. as_of is when its data was last fetched.
pub fn save_redacted_profile(source: &Profile, name: &str, as_of: Option<i64>) -> anyhow::Result<()> {
    let mut profiles: ProfileFile = read_profile_file()?;
    profiles.retain(|profile| !profile_names_equal(&profile.name, name));
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
    profiles.push(Profile {
        name: name.to_string(),
        // Its data is in Marco Sparko's layout, see redact.rs, so it is converted into its store when it is upgraded.
        version: String::from(LAST_CACHE_VERSION),
        modules,
        // Not the capture devices, whose keys would let anyone with the copy manage them.
        settings: Settings { capture_devices: Vec::new(), ..source.settings.clone() },
        offline: true,
        as_of,
        // A site of its own, made when it is first used.
        site: None,
    });
    write_profile_file(&profiles)?;
    Ok(())
}

/// The files of a profile, for each plugin a <profile>-<plugin> directory of data, in Marco Sparko's cache directory,
/// and a <profile>-<plugin>.json file of tokens, in the tokens directory, see data_home.rs, and its store.
#[derive(Debug, Default, PartialEq)]
pub struct ProfileCacheUsage {
    /// The bytes used by each plugin, by plugin id.
    pub plugins: BTreeMap<String, u64>,
    /// The cached data directories, which are moved to the trash when the profile is deleted.
    pub data_dirs: Vec<PathBuf>,
    /// The token files, which are deleted immediately when the profile is deleted.
    pub token_files: Vec<PathBuf>,
}

impl ProfileCacheUsage {
    pub fn total(&self) -> u64 {
        self.plugins.values().sum()
    }
}

/// The cache files of the given profile and the disk space they use, with its site's store, see store.rs, as "store",
/// which is not one of its data_dirs, as it is named by the site, not the profile, so it is kept when the profile is
/// renamed.
pub fn profile_cache_usage(profile_name: &str) -> anyhow::Result<ProfileCacheUsage> {
    // Its data in Marco Sparko's cache directory, and old copies of its tokens there, see data_home.rs, and its tokens.
    let mut usage = cache_usage_in(&MarcoSparkoContext::get_cache_dir_path()?, profile_name)?;
    let tokens = cache_usage_in(&crate::data_home::tokens_dir()?, profile_name)?;
    usage.token_files.extend(tokens.token_files);
    for (module, bytes) in tokens.plugins {
        *usage.plugins.entry(module).or_default() += bytes;
    }
    let profiles: ProfileFile = read_profile_file()?;
    let site = profiles.iter()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .and_then(|profile| profile.site.as_deref())
        .and_then(|site| bruce_db::Id::parse(site).ok());
    if let Some(dir) = site.map(|site| crate::store::site_dir(&site)).transpose()?.filter(|dir| dir.exists()) {
        usage.plugins.insert(String::from("store"), disk_usage(&dir)?);
    }
    Ok(usage)
}

fn cache_usage_in(cache_dir: &Path, profile_name: &str) -> anyhow::Result<ProfileCacheUsage> {
    // Profile names are letters, digits and underscores, so the hyphen ends the profile name.
    let prefix = format!("{}-", profile_name);
    let mut usage = ProfileCacheUsage::default();
    if let Ok(read_dir) = fs::read_dir(cache_dir) {
        for entry in read_dir {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let Some(rest) = file_name.strip_prefix(&prefix) else { continue };
            let path = entry.path();
            let plugin = if entry.file_type()?.is_dir() {
                usage.data_dirs.push(path.clone());
                rest
            }
            else if let Some(plugin) = rest.strip_suffix(".json") {
                usage.token_files.push(path.clone());
                plugin
            }
            else {
                continue
            };
            *usage.plugins.entry(plugin.to_string()).or_default() += disk_usage(&path)?;
        }
    }
    Ok(usage)
}

/// The total size of a file, or of all the files in a directory, symbolic links are not followed.
fn disk_usage(path: &Path) -> anyhow::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            total += disk_usage(&entry?.path())?;
        }
        Ok(total)
    }
    else {
        Ok(metadata.len())
    }
}

/// Deletes the token files of the given profile, moves its cached data to the trash and removes it from the profile
/// file. The active profile cannot be deleted.
pub fn delete_profile(profile_name: &str, active_profile: &str) -> anyhow::Result<()> {
    if profile_names_equal(profile_name, active_profile) {
        return Err(anyhow!("The active profile \"{}\" cannot be deleted", profile_name));
    }

    let mut profiles: ProfileFile = read_profile_file()?;
    let count = profiles.len();
    // Its site's store goes to the trash with its other data, see store.rs.
    let site_dir = profiles.iter()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .and_then(|profile| profile.site.as_deref())
        .map(|site| bruce_db::Id::parse(site).and_then(|site| crate::store::site_dir(&site)))
        .transpose()?
        .filter(|dir| dir.exists());
    profiles.retain(|profile| !profile_names_equal(&profile.name, profile_name));
    if profiles.len() == count {
        return Err(anyhow!("No such profile \"{}\"", profile_name));
    }

    // The tokens are not kept in the trash.
    let usage = profile_cache_usage(profile_name)?;
    for path in &usage.token_files {
        fs::remove_file(path)?;
    }
    // The log files go to the trash with the data.
    let logs = crate::logging::log_files(profile_name)?;
    let trashed: Vec<PathBuf> = usage.data_dirs.iter().cloned().chain(logs.into_iter().filter(|log| log.exists())).chain(site_dir).collect();
    if !trashed.is_empty() {
        trash::delete_all(&trashed)?;
    }

    write_profile_file(&profiles)?;
    Ok(())
}

/// The name a new profile is to have, trimmed, if it is valid and no profile has it.
fn new_profile_name<'a>(profiles: &ProfileFile, name: &'a str) -> anyhow::Result<&'a str> {
    let name = name.trim();
    if !is_valid_profile_name(name) {
        return Err(anyhow!("A profile's name must be letters, digits and underscores"));
    }
    if profiles.iter().any(|profile| profile_names_equal(&profile.name, name)) {
        return Err(anyhow!("A profile named \"{}\" already exists", name));
    }
    Ok(name)
}

/// The path of one of a profile's files, path, as another profile's, e.g. n2-octopus as n3-octopus, as the profile's
/// name ends at the first hyphen, see is_valid_profile_name, or its log, e.g. n2.log as n3.log.
fn renamed(path: &Path, from: &str, to: &str) -> anyhow::Result<PathBuf> {
    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| anyhow!("{} has no name", path.display()))?;
    let rest = file_name.strip_prefix(from).ok_or_else(|| anyhow!("{} is not of profile {from}", path.display()))?;
    Ok(path.with_file_name(format!("{to}{rest}")))
}

/// Renames a profile, which is not the active one, as the app has its files open: its entry in the profile file, its
/// cached data and logins, and its log.
pub fn rename_profile(from: &str, to: &str, active_profile: &str) -> anyhow::Result<()> {
    if profile_names_equal(from, active_profile) {
        return Err(anyhow!("The active profile \"{}\" cannot be renamed, change to another one first", from));
    }
    let mut profiles: ProfileFile = read_profile_file()?;
    let to = new_profile_name(&profiles, to)?;
    let profile = profiles.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, from))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", from))?;
    // As it is saved, as the match is not case sensitive.
    let from = profile.name.clone();
    let usage = profile_cache_usage(&from)?;
    let logs = crate::logging::log_files(&from)?;
    for path in usage.data_dirs.iter().chain(&usage.token_files).chain(logs.iter().filter(|log| log.exists())) {
        fs::rename(path, renamed(path, &from, to)?)?;
    }
    profile.name = to.to_string();
    write_profile_file(&profiles)?;
    Ok(())
}

/// Copies a directory and everything in it, keeping the files' permissions, as the cached data is private.
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copies a profile, its settings and its cached data, but not its log, as a new profile to. An offline copy only shows
/// what was copied, as the source was when it was copied, see Profile::as_of, without fetching anything, so it has
/// neither the source's logins, the token files, nor its capture devices, which it would otherwise manage as the
/// source does; another copy has both, and fetches as the source does.
pub fn copy_profile(from: &str, to: &str, offline: bool) -> anyhow::Result<()> {
    let mut profiles: ProfileFile = read_profile_file()?;
    let to = new_profile_name(&profiles, to)?;
    let source = profiles.iter()
        .find(|profile| profile_names_equal(&profile.name, from))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", from))?
        .clone();
    let usage = profile_cache_usage(&source.name)?;
    for dir in &usage.data_dirs {
        copy_dir(dir, &renamed(dir, &source.name, to)?)?;
    }
    if !offline {
        for file in &usage.token_files {
            fs::copy(file, renamed(file, &source.name, to)?)?;
        }
    }
    // A site of its own, with a copy of the source's store, so that each is deleted alone, see delete_profile.
    let site = match source.site.as_deref().map(bruce_db::Id::parse).transpose()? {
        Some(from) if crate::store::site_dir(&from)?.exists() => {
            let site = crate::store::new_site(to)?;
            let location = format!("file://{}", crate::store::store_dir()?.display());
            bruce_db::Store::copy_site(&location, &from, &site)?;
            Some(site.to_string())
        },
        _ => None,
    };
    let mut copy = Profile { name: to.to_string(), site, ..source };
    if offline {
        copy.offline = true;
        copy.as_of = copy.as_of.or_else(|| Some(crate::clock::now_utc().unix_timestamp()));
        copy.settings.capture_devices = Vec::new();
    }
    profiles.push(copy);
    write_profile_file(&profiles)?;
    Ok(())
}

/// A number of bytes for display, e.g. 12.3 MB
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{} bytes", bytes);
    }
    let mut size = bytes as f64 / 1000.0;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}

pub type ModuleProfiles = HashMap<String, serde_json::Value>;
pub type ProfileFile = Vec<Profile>;

#[derive(PartialEq)]
pub enum ProfileVersionState {
    OlderThanCurrent,
    Current,
    NewerThanCurrent,
    Invalid
}

impl ProfileVersionState {
    pub fn from(version: &str) -> anyhow::Result<ProfileVersionState> {
        let version = version.trim();

        if version.len() == 0 {
            return Ok(ProfileVersionState::OlderThanCurrent);
        }


        let current_version = Version::parse(CURRENT_VERSION).unwrap();
        Ok(if let Ok(version) = Version::parse(version) {
            match version.cmp(&current_version) {
                std::cmp::Ordering::Less => Self::OlderThanCurrent,
                std::cmp::Ordering::Equal => Self::Current,
                std::cmp::Ordering::Greater => Self::NewerThanCurrent,
            }
        }
        else {
            Self::Invalid
        })
    }
}

fn default_profile_version() ->String {
    "0.0.0".to_string()
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    #[serde(default = "default_profile_version")]
    pub version: String,
    pub modules: ModuleProfiles,
    /// The app's own settings for the profile, which profiles from before there were any do not have.
    #[serde(default)]
    pub settings: Settings,
    /// A redacted copy of another profile's data, see redact.rs, which only shows what it has, without logging in or
    /// fetching anything.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offline: bool,
    /// For an offline profile, when its data was last fetched, as a unix timestamp. The app behaves as if it is then,
    /// see clock.rs, so that its pages show the latest data.
    #[serde(default, rename = "asOf", skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    /// The ID of the profile's site in the store, see store.rs, made the first time it is needed, and kept when the
    /// profile is renamed, so that its data goes with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

/// The settings of a profile which are the app's rather than a module's, set on the Settings page.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub log: crate::logging::LogSettings,
    #[serde(default)]
    pub help: crate::help::HelpSettings,
    /// The capture devices this profile has adopted, see capture_device.rs, saved by update_capture_devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capture_devices: Vec<crate::capture_device::DeviceProfile>,
    /// Where in the site the modules' channels measure, set on the Site page and saved by update_topology, see
    /// power::topology. Until it is set, the grid's import is not split by use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<crate::power::topology::Circuit>,
    /// The categories of loads added on the Site page, besides the built-in ones, see power::topology::Use::Load, saved
    /// by update_settings_with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// The colours chosen for graphs' labels, e.g. "EV Low Rate", and for categories, e.g. "Hot water", which are the
    /// defaults of their labels, as CSS colours, saved by update_settings_with, see category_filter.rs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colours: BTreeMap<String, String>,
}

impl Settings {
    /// The categories a load can be, the built-in ones, then those added, see categories.
    pub fn all_categories(&self) -> Vec<String> {
        crate::power::topology::BUILT_IN_CATEGORIES.iter().map(|category| category.to_string())
            .chain(self.categories.iter().cloned())
            .collect()
    }
}

/// Saves the settings of the profile.
pub fn update_settings(profile_name: &str, settings: &Settings) -> anyhow::Result<()> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    // Whether the help is shown is saved by update_help_settings, from any window, the capture devices by
    // update_capture_devices, the topology by update_topology, and the categories and colours by update_settings_with,
    // so they are not overwritten here.
    let help = profile.settings.help.clone();
    let capture_devices = std::mem::take(&mut profile.settings.capture_devices);
    let topology = profile.settings.topology.take();
    let categories = std::mem::take(&mut profile.settings.categories);
    let colours = std::mem::take(&mut profile.settings.colours);
    profile.settings = settings.clone();
    profile.settings.help = help;
    profile.settings.capture_devices = capture_devices;
    profile.settings.topology = topology;
    profile.settings.categories = categories;
    profile.settings.colours = colours;
    write_profile_file(&profile_file)?;
    Ok(())
}

/// Saves the capture devices the profile has adopted, see capture_device.rs.
pub fn update_capture_devices(profile_name: &str, devices: &[crate::capture_device::DeviceProfile]) -> anyhow::Result<()> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.capture_devices = devices.to_vec();
    write_profile_file(&profile_file)?;
    Ok(())
}

/// Changes the profile's settings as saved with change, and saves them, e.g. a colour chosen, leaving the rest as they
/// are saved, which may be newer than the settings the page has, returning them as they are now.
pub fn update_settings_with(profile_name: &str, change: impl FnOnce(&mut Settings)) -> anyhow::Result<Settings> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    change(&mut profile.settings);
    let settings = profile.settings.clone();
    write_profile_file(&profile_file)?;
    Ok(settings)
}

/// Saves the site's topology, see power::topology, or clears it if it is None.
pub fn update_topology(profile_name: &str, topology: Option<&crate::power::topology::Circuit>) -> anyhow::Result<()> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.topology = topology.cloned();
    write_profile_file(&profile_file)?;
    Ok(())
}

/// The ID of the profile's site, see Profile::site, made and saved if it has none yet.
pub fn ensure_site(profile_name: &str) -> anyhow::Result<bruce_db::Id> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    if let Some(site) = &profile.site {
        return bruce_db::Id::parse(site);
    }
    let site = crate::store::new_site(&profile.name)?;
    profile.site = Some(site.to_string());
    write_profile_file(&profile_file)?;
    Ok(site)
}

/// Saves the help settings of the profile, see help::set_shown.
pub fn update_help_settings(profile_name: &str, help: &crate::help::HelpSettings) -> anyhow::Result<()> {
    let mut profile_file: ProfileFile = read_profile_file()?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.help = help.clone();
    write_profile_file(&profile_file)?;
    Ok(())
}

impl Profile {
    pub fn new() -> Profile {
        Profile {
            name: DEFAULT_PROFILE.to_string(),
            version: CURRENT_VERSION.to_string(),
            modules: ModuleProfiles::new(),
            settings: Settings::default(),
            offline: false,
            as_of: None,
            site: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::path::Path;

    use super::{cache_usage_in, copy_dir, format_size, renamed};

    #[test]
    fn cache_usage_is_split_by_plugin_and_leaves_out_other_profiles() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("p-octopus/sub")).unwrap();
        fs::create_dir_all(dir.join("p2-octopus")).unwrap();
        fs::write(dir.join("p-octopus/sub/data"), [0u8; 100]).unwrap();
        fs::write(dir.join("p-octopus.json"), [0u8; 10]).unwrap();
        fs::write(dir.join("p-other.json"), [0u8; 1]).unwrap();
        fs::write(dir.join("p-history.txt"), [0u8; 5]).unwrap();
        fs::write(dir.join("p2-octopus.json"), [0u8; 1000]).unwrap();

        let usage = cache_usage_in(&dir, "p").unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(usage.plugins.into_iter().collect::<Vec<_>>(), [(String::from("octopus"), 110), (String::from("other"), 1)]);
        assert_eq!(usage.data_dirs, [dir.join("p-octopus")]);
        let mut token_files = usage.token_files;
        token_files.sort();
        assert_eq!(token_files, [dir.join("p-octopus.json"), dir.join("p-other.json")]);
    }

    #[test]
    fn a_profiles_files_are_renamed_from_the_start_of_their_names() {
        let dir = Path::new("/cache");
        assert_eq!(renamed(&dir.join("n2-octopus"), "n2", "home").unwrap(), dir.join("home-octopus"));
        assert_eq!(renamed(&dir.join("n2.log.old"), "n2", "home").unwrap(), dir.join("home.log.old"));
        assert!(renamed(&dir.join("other-octopus"), "n2", "home").is_err());
    }

    #[test]
    fn a_directory_is_copied_with_everything_in_it() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-copy-{}", std::process::id()));
        fs::create_dir_all(dir.join("from/2026/November")).unwrap();
        fs::write(dir.join("from/2026/November/3#snapshots"), "data").unwrap();
        fs::write(dir.join("from/top"), "top").unwrap();
        copy_dir(&dir.join("from"), &dir.join("to")).unwrap();
        let copied = (fs::read_to_string(dir.join("to/2026/November/3#snapshots")).unwrap(), fs::read_to_string(dir.join("to/top")).unwrap());
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(copied, (String::from("data"), String::from("top")));
    }

    #[test]
    fn sizes_are_formatted_in_decimal_units() {
        assert_eq!(format_size(999), "999 bytes");
        assert_eq!(format_size(12_345_678), "12.3 MB");
    }
}
