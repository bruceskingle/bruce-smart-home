//! The data home, which holds what the app keeps, in two directories, kept apart: the settings, what the user set and
//! the secrets, and the data, which may be large, copied or deleted, and holds no secrets.
//!
//! ```text
//! .bruce-smart-home/           the settings, see settings_dir
//!     profiles.json            the profiles, their settings and the sources' credentials
//!     tokens/                  each profile's modules' sessions and refresh tokens, <profile>-<module>.json
//!     node                     this installation's node, see store.rs
//!     window-state.json        where the window was
//! .bruce-smart-home-data/      the data, see data_dir
//!     store/<site>/            each profile's store, see store.rs
//!     logs/                    each profile's log, <profile>.log
//! ```
//!
//! Before 0.5.0 the app was Marco Sparko, which kept the profiles in .marco-sparko and everything else in
//! .marco-sparko-cache. Those are copied into the new layout the first time it is needed, and left as they were, see
//! migrate; each profile's data is converted from .marco-sparko-cache into its store when it is upgraded, see
//! profile::upgrade_profile.
//!
//! The data home is the home directory unless it has been moved elsewhere, e.g. to keep the credentials on another
//! drive.

use std::{ffi::OsString, fs, path::{Path, PathBuf}, sync::OnceLock};

use anyhow::anyhow;

/// The environment variable which can give the data home, and its name before 0.5.0, which is still read.
pub const HOME_ENV_VAR: &str = "BRUCE_SMART_HOME_DIR";
const OLD_HOME_ENV_VAR: &str = "MARCO_SPARKO_HOME";
/// The file in the home directory which can give the data home, it contains just the path, and its name before 0.5.0,
/// which is still read.
pub const POINTER_FILE_NAME: &str = ".bruce-smart-home-dir";
const OLD_POINTER_FILE_NAME: &str = ".marco-sparko-home";

/// The directories of the settings and of the data, in the data home, see the module documentation.
const SETTINGS_DIR: &str = ".bruce-smart-home";
const DATA_DIR: &str = ".bruce-smart-home-data";
/// What Marco Sparko kept, before 0.5.0, see migrate.
const OLD_PROFILES: &str = ".marco-sparko";
const OLD_CACHE_DIR: &str = ".marco-sparko-cache";
/// The window's state, in the old cache directory, which is not a module's tokens.
const WINDOW_STATE: &str = "window-state.json";

/// The data directory and where it was set, None for the home directory.
#[derive(Clone, Debug, PartialEq)]
struct DataHome {
    path: PathBuf,
    source: Option<String>,
}

static DATA_HOME: OnceLock<Result<DataHome, String>> = OnceLock::new();

/// The data home, which is the first of
/// 1. the --home command line option
/// 2. the BRUCE_SMART_HOME_DIR environment variable, or MARCO_SPARKO_HOME, its name before 0.5.0
/// 3. the path in the file ~/.bruce-smart-home-dir, or ~/.marco-sparko-home, its name before 0.5.0
/// 4. the home directory
///
/// It is worked out once. A data directory other than the home directory is created if its parent exists. If it does
/// not, e.g. because it is on a drive which is not connected, this is an error rather than falling back to the home
/// directory, so that credentials are never written somewhere the user did not choose.
pub fn data_home() -> anyhow::Result<PathBuf> {
    let data_home = DATA_HOME
        .get_or_init(|| {
            let option = crate::Args::ms_parse().marco_sparko_args.home;
            let env = std::env::var_os(HOME_ENV_VAR).filter(|path| !path.is_empty()).or_else(|| std::env::var_os(OLD_HOME_ENV_VAR));
            resolve(option, env, dirs::home_dir()).map_err(|error| error.to_string())
        })
        .clone()
        .map_err(|error| anyhow!(error))?;

    if let Some(source) = &data_home.source {
        ensure_dir(&data_home.path, source)?;
    }
    Ok(data_home.path)
}

/// The data directory there would be without the --home option, e.g. where the profiles normally are when another
/// data directory is used for screenshots, see screenshots.rs.
pub fn usual_data_home() -> anyhow::Result<PathBuf> {
    let env = std::env::var_os(HOME_ENV_VAR).filter(|path| !path.is_empty()).or_else(|| std::env::var_os(OLD_HOME_ENV_VAR));
    Ok(resolve(None, env, dirs::home_dir())?.path)
}

/// The profiles of a data home, e.g. the usual one, see usual_data_home, from its profiles.json, or, if it has not been
/// copied into the new layout yet, see migrate, from Marco Sparko's .marco-sparko.
pub fn profiles_path_in(home: &Path) -> PathBuf {
    let path = home.join(SETTINGS_DIR).join("profiles.json");
    if path.exists() { path } else { home.join(OLD_PROFILES) }
}

/// Marco Sparko's cache directory in a data home, see old_cache_dir.
pub fn old_cache_dir_in(home: &Path) -> PathBuf {
    home.join(OLD_CACHE_DIR)
}

/// Deletes the settings and data of the data home, and what Marco Sparko kept, for the screenshots' fresh data
/// directory, see screenshots::prepare_data_directory, which checks it is not the usual one.
pub fn clear(home: &Path) -> anyhow::Result<()> {
    for dir in [SETTINGS_DIR, DATA_DIR, OLD_CACHE_DIR] {
        if home.join(dir).exists() {
            fs::remove_dir_all(home.join(dir))?;
        }
    }
    if home.join(OLD_PROFILES).exists() {
        fs::remove_file(home.join(OLD_PROFILES))?;
    }
    Ok(())
}

/// Where the data directory was set, for display, e.g. "the MARCO_SPARKO_HOME environment variable", or None if it is
/// the home directory.
pub fn data_home_source() -> Option<String> {
    DATA_HOME.get()?.as_ref().ok()?.source.clone()
}

fn resolve(option: Option<PathBuf>, env: Option<OsString>, home: Option<PathBuf>) -> anyhow::Result<DataHome> {
    if let Some(path) = option {
        return Ok(DataHome { path: std::path::absolute(path)?, source: Some(String::from("the --home option")) });
    }

    if let Some(path) = env.filter(|path| !path.is_empty()) {
        return Ok(DataHome { path: std::path::absolute(path)?, source: Some(format!("the {} environment variable", HOME_ENV_VAR)) });
    }

    let home = home.ok_or(anyhow!("Unable to locate home directory"))?;
    let pointer_file = [POINTER_FILE_NAME, OLD_POINTER_FILE_NAME].iter()
        .map(|name| home.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| home.join(POINTER_FILE_NAME));
    match fs::read_to_string(&pointer_file) {
        Ok(contents) => {
            let source = format!("the file {}", pointer_file.display());
            let line = contents.trim();
            // ~/ at the start is the home directory.
            let path = match line.strip_prefix("~/").or_else(|| line.strip_prefix("~\\")) {
                Some(rest) => home.join(rest),
                None => PathBuf::from(line),
            };
            if line.is_empty() || !path.is_absolute() {
                return Err(anyhow!("{} must contain the full path of the data directory, but it contains \"{}\"", source, line));
            }
            Ok(DataHome { path, source: Some(source) })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DataHome { path: home, source: None }),
        Err(error) => Err(anyhow!("Unable to read {}: {}", pointer_file.display(), error)),
    }
}

/// The settings directory, see the module documentation, made if it is not there, having copied what Marco Sparko kept
/// into the new layout the first time, see migrate.
pub fn settings_dir() -> anyhow::Result<PathBuf> {
    static MIGRATED: OnceLock<Result<(), String>> = OnceLock::new();
    let home = data_home()?;
    MIGRATED.get_or_init(|| migrate(&home).map_err(|error| format!("{error:#}"))).clone().map_err(|error| anyhow!(error))?;
    let dir = home.join(SETTINGS_DIR);
    crate::private_file::create_private_dir(&dir)?;
    Ok(dir)
}

/// The data directory, see the module documentation, made if it is not there.
pub fn data_dir() -> anyhow::Result<PathBuf> {
    // The settings first, so that what Marco Sparko kept has been copied before anything is written here.
    settings_dir()?;
    let dir = data_home()?.join(DATA_DIR);
    crate::private_file::create_private_dir(&dir)?;
    Ok(dir)
}

/// The file of the profiles.
pub fn profiles_path() -> anyhow::Result<PathBuf> {
    Ok(settings_dir()?.join("profiles.json"))
}

/// The directory of the modules' sessions and tokens, <profile>-<module>.json, made if it is not there.
pub fn tokens_dir() -> anyhow::Result<PathBuf> {
    let dir = settings_dir()?.join("tokens");
    crate::private_file::create_private_dir(&dir)?;
    Ok(dir)
}

/// The directory of the profiles' logs, made if it is not there.
pub fn logs_dir() -> anyhow::Result<PathBuf> {
    let dir = data_dir()?.join("logs");
    crate::private_file::create_private_dir(&dir)?;
    Ok(dir)
}

/// The directory of the profiles' stores, a directory of sites, see store.rs.
pub fn store_dir() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("store"))
}

/// Marco Sparko's cache directory, from before 0.5.0, which is only read, to convert a profile's data into its store
/// when it is upgraded, see convert.rs, and is not made if it is not there.
pub fn old_cache_dir() -> anyhow::Result<PathBuf> {
    Ok(data_home()?.join(OLD_CACHE_DIR))
}

/// Copies what Marco Sparko kept in home, the data home, into the new layout, see the module documentation, if it has
/// not been, i.e. there is no profiles.json yet, and there is a .marco-sparko: the profiles, the modules' tokens, the
/// window's state and the logs. They are copied, not moved, and left as they were, so that a build from before 0.5.0
/// still runs from them; the profiles' data is converted when each profile is upgraded.
fn migrate(home: &Path) -> anyhow::Result<()> {
    let settings = home.join(SETTINGS_DIR);
    let old_profiles = home.join(OLD_PROFILES);
    if settings.join("profiles.json").exists() || !old_profiles.is_file() {
        return Ok(());
    }
    crate::private_file::create_private_dir(&settings)?;
    let tokens = settings.join("tokens");
    crate::private_file::create_private_dir(&tokens)?;
    let data = home.join(DATA_DIR);
    crate::private_file::create_private_dir(&data)?;
    let logs = data.join("logs");
    crate::private_file::create_private_dir(&logs)?;
    let copy = |from: &Path, to: &Path| -> anyhow::Result<()> {
        use std::io::Write;
        crate::private_file::create_private_file(to)?.write_all(&fs::read(from)?)?;
        Ok(())
    };
    let mut copied = 0;
    for entry in fs::read_dir(home.join(OLD_CACHE_DIR)).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_string) else { continue };
        if !path.is_file() {
            continue;
        }
        let target = if name == WINDOW_STATE {
            settings.join(WINDOW_STATE)
        }
        // A module's tokens, <profile>-<module>.json, profile names being letters, digits and underscores.
        else if name.ends_with(".json") && name.contains('-') {
            tokens.join(&name)
        }
        else if name.ends_with(".log") || name.ends_with(".log.old") {
            logs.join(&name)
        }
        else {
            continue;
        };
        copy(&path, &target)?;
        copied += 1;
    }
    // The profiles last, as their being there says the copy is done.
    copy(&old_profiles, &settings.join("profiles.json"))?;
    crate::log_info!("Copied Marco Sparko's profiles and {copied} other files into {} and {}", settings.display(), data.display());
    Ok(())
}

/// Creates the data directory if its parent exists, and warns if it can be read by others.
fn ensure_dir(path: &Path, source: &str) -> anyhow::Result<()> {
    if !path.is_dir() {
        if path.parent().is_some_and(Path::is_dir) {
            crate::private_file::create_private_dir(path)?;
        }
        else {
            return Err(anyhow!(
                "The data directory {} (set by {}) does not exist. If it is on a drive which is not connected, connect it and restart Marco Sparko.",
                path.display(), source));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            crate::log_warning!("The data directory {} can be accessed by other users, it should only be accessible by its owner (chmod 700)", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{POINTER_FILE_NAME, resolve};

    #[test]
    fn the_option_then_the_environment_then_the_pointer_file_then_the_home_directory_is_used() {
        let home = std::env::temp_dir().join(format!("marco-sparko-home-test-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        // Full paths on every platform, Windows ones need a drive.
        let option = home.join("option");
        let env = home.join("env");
        let pointer = home.join("pointer");

        let path = |option: Option<PathBuf>, env: Option<PathBuf>| resolve(option, env.map(Into::into), Some(home.clone())).map(|data_home| data_home.path);

        assert_eq!(path(None, None).unwrap(), home);

        fs::write(home.join(POINTER_FILE_NAME), format!("  {}\n", pointer.display())).unwrap();
        assert_eq!(path(None, None).unwrap(), pointer);
        assert_eq!(path(None, Some(env.clone())).unwrap(), env);
        assert_eq!(path(Some(option.clone()), Some(env)).unwrap(), option);

        fs::write(home.join(POINTER_FILE_NAME), "~/elsewhere").unwrap();
        assert_eq!(path(None, None).unwrap(), home.join("elsewhere"));

        fs::write(home.join(POINTER_FILE_NAME), "relative").unwrap();
        assert!(path(None, None).is_err());

        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn marco_sparkos_profiles_tokens_window_and_logs_are_copied_once_and_left_as_they_were() {
        let home = std::env::temp_dir().join(format!("marco-sparko-migrate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".marco-sparko-cache/n2-octopus")).unwrap();
        fs::write(home.join(".marco-sparko"), "[profiles]").unwrap();
        for name in ["n2-octopus.json", "window-state.json", "n2.log", "n2.log.old", "n2.log.lock", "n2-octopus/data"] {
            fs::write(home.join(".marco-sparko-cache").join(name), name).unwrap();
        }
        super::migrate(&home).unwrap();
        let settings = home.join(".bruce-smart-home");
        let data = home.join(".bruce-smart-home-data");
        assert_eq!(fs::read_to_string(settings.join("profiles.json")).unwrap(), "[profiles]");
        assert_eq!(fs::read_to_string(settings.join("tokens/n2-octopus.json")).unwrap(), "n2-octopus.json");
        assert_eq!(fs::read_to_string(settings.join("window-state.json")).unwrap(), "window-state.json");
        assert!(data.join("logs/n2.log").exists() && data.join("logs/n2.log.old").exists());
        assert!(!data.join("logs/n2.log.lock").exists());
        assert!(!settings.join("tokens/window-state.json").exists());
        assert!(home.join(".marco-sparko").exists() && home.join(".marco-sparko-cache/n2-octopus.json").exists(), "left as they were");

        // Once: a profile changed since is not overwritten.
        fs::write(settings.join("profiles.json"), "[changed]").unwrap();
        super::migrate(&home).unwrap();
        assert_eq!(fs::read_to_string(settings.join("profiles.json")).unwrap(), "[changed]");
        fs::remove_dir_all(&home).unwrap();
    }
}
