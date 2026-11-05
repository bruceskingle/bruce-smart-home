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
pub const CURRENT_VERSION: &str = "0.4.3";

#[derive(PartialEq)]
pub struct ActiveProfile {
    pub all_profiles:   Vec<String>,
    pub active_profile: Profile,
    pub profile_version_state: ProfileVersionState,
}

pub fn profile_names_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
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

    if let Ok(file)= fs::File::open(&MarcoSparkoContext::get_file_path()?) {
        let profile_file: ProfileFile = serde_json::from_reader(file)?;

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

            let f = private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?;
            serde_json::to_writer_pretty(f, &profiles)?;

            active_profile
        }
    )
}

pub fn set_active_profile(profile_name: &String) -> anyhow::Result<ActiveProfile> {

    let mut all_profiles: Vec<String> = Vec::new();
    let mut map: IndexMap<String, Profile> = IndexMap::new();
    

    if let Ok(file)= fs::File::open(&MarcoSparkoContext::get_file_path()?) {
        let profile_file: ProfileFile = serde_json::from_reader(file)?;

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

        serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profiles)?;
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

    let mut profiles: ProfileFile = if let Ok(file) = fs::File::open(&MarcoSparkoContext::get_file_path()?) {
        serde_json::from_reader(file)?
    } else {
        Vec::new()
    };

    if profiles.iter().any(|profile| profile_names_equal(&profile.name, profile_name)) {
        return Err(anyhow!("A profile named \"{}\" already exists", profile_name));
    }

    let mut active_profile = Profile::new();
    active_profile.name = profile_name.to_string();
    profiles.insert(0, active_profile.clone());

    serde_json::to_writer_pretty(
        private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?,
        &profiles,
    )?;

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
    if let Ok(file)= fs::File::open(&MarcoSparkoContext::get_file_path()?) {
        let mut profile_file: ProfileFile = serde_json::from_reader(file)?;


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
    
        serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profile_file)?;

        Ok(())
    }
    else {
        Err(anyhow!("No such profile \"{}\" (no profiles at all, in fact)", profile_name))
    }
}

/// Saves the profile name, offline, as a redacted copy of source, see redact.rs, replacing any previous copy. Its
/// modules are set up as the source's are, without their logins. as_of is when its data was last fetched.
pub fn save_redacted_profile(source: &Profile, name: &str, as_of: Option<i64>) -> anyhow::Result<()> {
    let mut profiles: ProfileFile = serde_json::from_reader(fs::File::open(&MarcoSparkoContext::get_file_path()?)?)?;
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
        version: source.version.clone(),
        modules,
        // Not the capture devices, whose keys would let anyone with the copy manage them.
        settings: Settings { capture_devices: Vec::new(), ..source.settings.clone() },
        offline: true,
        as_of,
    });
    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profiles)?;
    Ok(())
}

/// The cache files of a profile, for each plugin a <profile>-<plugin> directory of cached data and a
/// <profile>-<plugin>.json file of tokens.
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

/// The cache files of the given profile and the disk space they use.
pub fn profile_cache_usage(profile_name: &str) -> anyhow::Result<ProfileCacheUsage> {
    cache_usage_in(&MarcoSparkoContext::get_cache_dir_path()?, profile_name)
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

    let file = fs::File::open(&MarcoSparkoContext::get_file_path()?)?;
    let mut profiles: ProfileFile = serde_json::from_reader(file)?;
    let count = profiles.len();
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
    let trashed: Vec<PathBuf> = usage.data_dirs.iter().cloned().chain(logs.into_iter().filter(|log| log.exists())).collect();
    if !trashed.is_empty() {
        trash::delete_all(&trashed)?;
    }

    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profiles)?;
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
}

/// Saves the settings of the profile.
pub fn update_settings(profile_name: &str, settings: &Settings) -> anyhow::Result<()> {
    let file = fs::File::open(&MarcoSparkoContext::get_file_path()?)?;
    let mut profile_file: ProfileFile = serde_json::from_reader(file)?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    // Whether the help is shown is saved by update_help_settings, from any window, the capture devices by
    // update_capture_devices, and the topology by update_topology, so they are not overwritten here.
    let help = profile.settings.help.clone();
    let capture_devices = std::mem::take(&mut profile.settings.capture_devices);
    let topology = profile.settings.topology.take();
    profile.settings = settings.clone();
    profile.settings.help = help;
    profile.settings.capture_devices = capture_devices;
    profile.settings.topology = topology;
    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profile_file)?;
    Ok(())
}

/// Saves the capture devices the profile has adopted, see capture_device.rs.
pub fn update_capture_devices(profile_name: &str, devices: &[crate::capture_device::DeviceProfile]) -> anyhow::Result<()> {
    let file = fs::File::open(&MarcoSparkoContext::get_file_path()?)?;
    let mut profile_file: ProfileFile = serde_json::from_reader(file)?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.capture_devices = devices.to_vec();
    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profile_file)?;
    Ok(())
}

/// Saves the site's topology, see power::topology, or clears it if it is None.
pub fn update_topology(profile_name: &str, topology: Option<&crate::power::topology::Circuit>) -> anyhow::Result<()> {
    let file = fs::File::open(&MarcoSparkoContext::get_file_path()?)?;
    let mut profile_file: ProfileFile = serde_json::from_reader(file)?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.topology = topology.cloned();
    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profile_file)?;
    Ok(())
}

/// Saves the help settings of the profile, see help::set_shown.
pub fn update_help_settings(profile_name: &str, help: &crate::help::HelpSettings) -> anyhow::Result<()> {
    let file = fs::File::open(&MarcoSparkoContext::get_file_path()?)?;
    let mut profile_file: ProfileFile = serde_json::from_reader(file)?;
    let profile = profile_file.iter_mut()
        .find(|profile| profile_names_equal(&profile.name, profile_name))
        .ok_or_else(|| anyhow!("No such profile \"{}\"", profile_name))?;
    profile.settings.help = help.clone();
    serde_json::to_writer_pretty(private_file::create_private_file(&MarcoSparkoContext::get_file_path()?)?, &profile_file)?;
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
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{cache_usage_in, format_size};

    #[test]
    fn cache_usage_is_split_by_plugin_and_leaves_out_other_profiles() {
        let dir = std::env::temp_dir().join(format!("marco-sparko-test-{}", std::process::id()));
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
    fn sizes_are_formatted_in_decimal_units() {
        assert_eq!(format_size(999), "999 bytes");
        assert_eq!(format_size(12_345_678), "12.3 MB");
    }
}
