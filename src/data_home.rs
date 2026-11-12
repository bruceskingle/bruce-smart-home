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
//! A profile's data from before 0.5.0, in the old cache directory, see old_cache_dir, is converted into its store when
//! it is upgraded, see profile::upgrade_profile.
//!
//! The data home is the home directory unless it has been moved elsewhere, e.g. to keep the credentials on another
//! drive.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::anyhow;

/// The environment variable which can give the data home.
pub const HOME_ENV_VAR: &str = "BRUCE_SMART_HOME_DIR";
/// The file in the home directory which can give the data home, it contains just the path.
pub const POINTER_FILE_NAME: &str = ".bruce-smart-home-dir";

/// The directories of the settings and of the data, in the data home, see the module documentation.
const SETTINGS_DIR: &str = ".bruce-smart-home";
const DATA_DIR: &str = ".bruce-smart-home-data";
/// The cache directory of the app before 0.5.0, in the data directory, see old_cache_dir.
const OLD_CACHE_DIR: &str = "old-cache";

/// The data directory and where it was set, None for the home directory.
#[derive(Clone, Debug, PartialEq)]
struct DataHome {
    path: PathBuf,
    source: Option<String>,
}

static DATA_HOME: OnceLock<Result<DataHome, String>> = OnceLock::new();

/// The data home, which is the first of
/// 1. the --home command line option
/// 2. the BRUCE_SMART_HOME_DIR environment variable
/// 3. the path in the file ~/.bruce-smart-home-dir
/// 4. the home directory
///
/// It is worked out once. A data directory other than the home directory is created if its parent exists. If it does
/// not, e.g. because it is on a drive which is not connected, this is an error rather than falling back to the home
/// directory, so that credentials are never written somewhere the user did not choose.
pub fn data_home() -> anyhow::Result<PathBuf> {
    let data_home = DATA_HOME
        .get_or_init(|| {
            let option = crate::Args::parse_all().app_args.home;
            let env = std::env::var_os(HOME_ENV_VAR);
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
    Ok(resolve(None, std::env::var_os(HOME_ENV_VAR), dirs::home_dir())?.path)
}

/// The profiles of a data home, e.g. the usual one, see usual_data_home.
pub fn profiles_path_in(home: &Path) -> PathBuf {
    home.join(SETTINGS_DIR).join("profiles.json")
}

/// The old cache directory in a data home, see old_cache_dir.
pub fn old_cache_dir_in(home: &Path) -> PathBuf {
    home.join(DATA_DIR).join(OLD_CACHE_DIR)
}

/// Deletes the settings and data of the data home, with its old cache directory, for the screenshots' fresh data
/// directory, see screenshots::prepare_data_directory, which checks it is not the usual one.
pub fn clear(home: &Path) -> anyhow::Result<()> {
    for dir in [SETTINGS_DIR, DATA_DIR] {
        if home.join(dir).exists() {
            fs::remove_dir_all(home.join(dir))?;
        }
    }
    Ok(())
}

/// Where the data directory was set, for display, e.g. "the BRUCE_SMART_HOME_DIR environment variable", or None if it is
/// the home directory.
pub fn data_home_source() -> Option<String> {
    DATA_HOME.get()?.as_ref().ok()?.source.clone()
}

fn resolve(option: Option<PathBuf>, env: Option<OsString>, home: Option<PathBuf>) -> anyhow::Result<DataHome> {
    if let Some(path) = option {
        return Ok(DataHome {
            path: std::path::absolute(path)?,
            source: Some(String::from("the --home option")),
        });
    }

    if let Some(path) = env.filter(|path| !path.is_empty()) {
        return Ok(DataHome {
            path: std::path::absolute(path)?,
            source: Some(format!("the {} environment variable", HOME_ENV_VAR)),
        });
    }

    let home = home.ok_or(anyhow!("Unable to locate home directory"))?;
    let pointer_file = home.join(POINTER_FILE_NAME);
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
                return Err(anyhow!(
                    "{} must contain the full path of the data directory, but it contains \"{}\"",
                    source,
                    line
                ));
            }
            Ok(DataHome {
                path,
                source: Some(source),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DataHome {
            path: home,
            source: None,
        }),
        Err(error) => Err(anyhow!("Unable to read {}: {}", pointer_file.display(), error)),
    }
}

/// The settings directory, see the module documentation, made if it is not there.
pub fn settings_dir() -> anyhow::Result<PathBuf> {
    let dir = data_home()?.join(SETTINGS_DIR);
    crate::private_file::create_private_dir(&dir)?;
    Ok(dir)
}

/// The data directory, see the module documentation, made if it is not there.
pub fn data_dir() -> anyhow::Result<PathBuf> {
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

/// The cache directory of the app before 0.5.0, which is only read, to convert a profile's data into its store
/// when it is upgraded, see convert.rs, and is not made if it is not there.
pub fn old_cache_dir() -> anyhow::Result<PathBuf> {
    Ok(old_cache_dir_in(&data_home()?))
}

/// Creates the data directory if its parent exists, and warns if it can be read by others.
fn ensure_dir(path: &Path, source: &str) -> anyhow::Result<()> {
    if !path.is_dir() {
        if path.parent().is_some_and(Path::is_dir) {
            crate::private_file::create_private_dir(path)?;
        } else {
            return Err(anyhow!(
                "The data directory {} (set by {}) does not exist. If it is on a drive which is not connected, connect it and restart the app.",
                path.display(),
                source
            ));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            crate::log_warning!(
                "The data directory {} can be accessed by other users, it should only be accessible by its owner (chmod 700)",
                path.display()
            );
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
        let home = std::env::temp_dir().join(format!("bruce-smart-home-test-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        // Full paths on every platform, Windows ones need a drive.
        let option = home.join("option");
        let env = home.join("env");
        let pointer = home.join("pointer");

        let path = |option: Option<PathBuf>, env: Option<PathBuf>| {
            resolve(option, env.map(Into::into), Some(home.clone())).map(|data_home| data_home.path)
        };

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
}
