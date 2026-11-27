use anyhow::{anyhow, bail};
use indexmap::IndexMap;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::{AppContext, private_file};

const DEFAULT_DOMAIN: &str = "default";
/// The version of the domain's format, and of the data kept for it. A change to either which needs a domain's data
/// changing too adds a step to upgrade_domain, and makes this the version it is in.
pub const CURRENT_VERSION: &str = "0.6.0";

/// The last version whose domains kept their data in the old cache directory, for a domain whose data is
/// still made there, e.g. a redacted copy, see save_redacted_domain, which is upgraded to the store when it is opened.
const LAST_CACHE_VERSION: &str = "0.4.3";

/// The version whose domains keep their data in the store, see store.rs, rather than in the cache directory, see
/// upgrade_domain.
const STORE_VERSION: &str = "0.5.0";
/// The version from which a domain's files on this computer, its tokens and its log, are named by its ID, not its name,
/// see files_key and upgrade_domain.
const FILES_BY_ID_VERSION: &str = "0.6.0";

#[derive(PartialEq)]
pub struct ActiveDomain {
    pub all_domains: Vec<String>,
    pub active_domain: Domain,
    pub domain_version_state: DomainVersionState,
}

pub fn domain_names_equal(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// What each domain's files are named by, see files_key, by its name as asked for, read once, and forgotten whenever
/// the domain file is saved, as that may change them.
static FILES_KEYS: std::sync::Mutex<Option<HashMap<String, Option<String>>>> = std::sync::Mutex::new(None);

/// What the domain's files on this computer are named by, its tokens, its modules' notes of refused logins and its log:
/// its ID, see Domain::id, as hex, so that renaming the domain renames none of them; None for a domain which has no ID
/// yet, e.g. one awaiting its founding, which has none of them but its log, see logging::log_path.
pub fn files_key(domain_name: &str) -> anyhow::Result<Option<String>> {
    if let Some(key) = FILES_KEYS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|keys| keys.get(domain_name))
    {
        return Ok(key.clone());
    }
    let domains: DomainFile = read_domain_file()?;
    let key = domains
        .iter()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?
        .id
        .as_deref()
        .map(bruce_db::Id::parse)
        .transpose()?
        .map(|id| id.hex());
    FILES_KEYS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(domain_name.to_string(), key.clone());
    Ok(key)
}

/// The domain's files key, see files_key, made if it has no ID yet, see ensure_id, as a module saving its tokens needs
/// one; an error for a domain awaiting its founding.
pub fn ensure_files_key(domain_name: &str) -> anyhow::Result<String> {
    match files_key(domain_name)? {
        Some(key) => Ok(key),
        None => Ok(ensure_id(domain_name)?.hex()),
    }
}

/// Renames the token files, and the log, named by from, a domain's files key, see files_key, or its name before
/// 0.6.0, as named by to, e.g. as the domain has been moved to its domain's ID, see domain_security::found_domain, or
/// upgraded, see upgrade_domain. Token files already named by to are left as they are; a log is merged, see
/// logging::move_log.
pub fn rename_files(from: &str, to: &str) -> anyhow::Result<()> {
    if from == to {
        return Ok(());
    }
    let tokens = cache_usage_in(&crate::data_home::tokens_dir()?, from)?;
    for path in &tokens.token_files {
        let target = renamed(path, from, to)?;
        if !target.exists() {
            fs::rename(path, target)?;
        }
    }
    crate::logging::move_log(from, to)
}

/// The domains in the domain file, with their secrets decrypted, see secrets.rs.
pub(crate) fn read_domain_file() -> anyhow::Result<DomainFile> {
    read_domain_file_at(&AppContext::get_file_path()?)
}

/// The domains in the domain file, with their secrets decrypted, see secrets.rs, or None if it cannot be opened, e.g.
/// before the first domain is made.
pub fn read_domain_file_if_any() -> anyhow::Result<Option<DomainFile>> {
    let path = AppContext::get_file_path()?;
    if fs::File::open(&path).is_err() {
        return Ok(None);
    }
    Ok(Some(read_domain_file_at(&path)?))
}

/// The domains in the domain file, if there is one, with their secrets left encrypted, for what needs none of them,
/// e.g. agents_guide, so that the Keychain is not asked for the secrets' key, which macOS asks the user to allow for
/// each new build, and the window waits for, as it did for 6 s on 23 November 2026.
pub fn read_domain_file_without_secrets() -> anyhow::Result<Option<DomainFile>> {
    let path = AppContext::get_file_path()?;
    let Ok(file) = fs::File::open(&path) else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_reader(file)?))
}

/// The domains in the domain file at path, e.g. another data home's, with their secrets decrypted, see secrets.rs.
/// A secret which cannot be decrypted is left out, see read_domain_file_reporting.
pub fn read_domain_file_at(path: &Path) -> anyhow::Result<DomainFile> {
    Ok(read_domain_file_with_unreadable(path)?.0)
}

/// A field of a domain file which could not be read: the domain, the module it is in, if one, and the field, e.g.
/// ("Home", Some("octopus"), "apiKey").
type UnreadableField = (String, Option<String>, String);

/// The domains in the domain file at path, as read_domain_file_at, and the secrets which could not be decrypted, by
/// the domain and the module they are in, if one, and the field, e.g. ("Home", Some("octopus"), "apiKey").
fn read_domain_file_with_unreadable(path: &Path) -> anyhow::Result<(DomainFile, Vec<UnreadableField>)> {
    let mut value: serde_json::Value = serde_json::from_reader(fs::File::open(path)?)?;
    let unreadable = crate::secrets::decrypt_secrets(&mut value)?
        .into_iter()
        .map(|secret| {
            let domain = secret
                .path
                .first()
                .and_then(|index| value[index.parse::<usize>().unwrap_or(usize::MAX)]["name"].as_str());
            let module = (secret.path.get(1).map(String::as_str) == Some("modules"))
                .then(|| secret.path.get(2).cloned())
                .flatten();
            let field = secret.path.last().cloned().unwrap_or_default();
            crate::log_error!(
                "Unable to decrypt the saved {field} of domain {}{}, so it is left out, {:#}",
                domain.unwrap_or("?"),
                module
                    .as_ref()
                    .map(|module| format!(", module {module}"))
                    .unwrap_or_default(),
                secret.error
            );
            (domain.unwrap_or_default().to_string(), module, field)
        })
        .collect();
    Ok((serde_json::from_value(value)?, unreadable))
}

/// The domains in the domain file, as read_domain_file_if_any, with each module whose saved login could not be
/// decrypted, e.g. as the file was edited by hand, reported as a refused login, see login.rs, so that its login page
/// says why it asks for the login again. It is used when a domain is opened, rather than every time the file is read,
/// as the file is read and saved again when a module logs in, which would report it again.
fn read_domain_file_reporting() -> anyhow::Result<Option<DomainFile>> {
    let path = AppContext::get_file_path()?;
    if fs::File::open(&path).is_err() {
        return Ok(None);
    }
    let (domains, unreadable) = read_domain_file_with_unreadable(&path)?;
    for (domain, module, field) in unreadable {
        if let Some(module) = module {
            let what = match field.as_str() {
                "apiKey" | "api_key" => "API key".to_string(),
                field => field.to_string(),
            };
            crate::login::unreadable(
                &domain,
                &module,
                format!("The saved {what} could not be decrypted, e.g. as the domain file was edited"),
            );
        }
    }
    Ok(Some(domains))
}

/// Saves the domains to the domain file, with their secrets encrypted, see secrets.rs, whichever version each is.
pub fn write_domain_file(domains: &impl Serialize) -> anyhow::Result<()> {
    write_domain_file_at(&AppContext::get_file_path()?, domains)
}

/// Saves the domains to the domain file at path, e.g. another data home's, with their secrets encrypted, see
/// secrets.rs.
pub fn write_domain_file_at(path: &Path, domains: &impl Serialize) -> anyhow::Result<()> {
    *FILES_KEYS.lock().unwrap() = None;
    let mut value = serde_json::to_value(domains)?;
    crate::secrets::encrypt_secrets(&mut value)?;
    serde_json::to_writer_pretty(private_file::create_private_file(path)?, &value)?;
    Ok(())
}

impl ActiveDomain {
    // Create empty default domain
    fn new() -> ActiveDomain {
        ActiveDomain {
            all_domains: vec![String::from(DEFAULT_DOMAIN)],
            active_domain: Domain::new(),
            domain_version_state: DomainVersionState::Current,
        }
    }
}

pub fn fetch_active_domain(domain_name: &Option<String>) -> anyhow::Result<ActiveDomain> {
    let mut all_domains: Vec<String> = Vec::new();
    let mut set: HashSet<String> = HashSet::new();
    let mut active_domain = None;

    if let Some(domain_file) = read_domain_file_reporting()? {
        for domain in domain_file {
            if all_domains.iter().any(|name| domain_names_equal(name, &domain.name)) {
                return Err(anyhow!("Duplicate domain \"{}\"", &domain.name));
            }
            set.insert(domain.name.clone());

            let got_it = if let Some(name) = domain_name {
                domain_names_equal(name, &domain.name)
            } else {
                active_domain.is_none()
            };

            if got_it {
                all_domains.push(domain.name.clone());
                active_domain = Some(domain)
            } else {
                all_domains.push(domain.name);
            }
        }
    }

    if let Some(name) = domain_name
        && active_domain.is_none()
    {
        return Err(anyhow!("No such domain \"{}\"", name));
    }

    Ok(if let Some(active_domain) = active_domain {
        let domain_version_state = DomainVersionState::from(&active_domain.version)?;
        ActiveDomain {
            all_domains,
            active_domain,
            domain_version_state,
        }
    } else {
        let active_domain = ActiveDomain::new();
        let domains = vec![&active_domain.active_domain];

        write_domain_file(&domains)?;

        active_domain
    })
}

pub fn set_active_domain(domain_name: &String) -> anyhow::Result<ActiveDomain> {
    let mut all_domains: Vec<String> = Vec::new();
    let mut map: IndexMap<String, Domain> = IndexMap::new();

    if let Some(domain_file) = read_domain_file_if_any()? {
        for domain in domain_file {
            if map.keys().any(|name| domain_names_equal(name, &domain.name)) {
                return Err(anyhow!("Duplicate domain \"{}\"", &domain.name));
            }
            all_domains.push(domain.name.clone());
            map.insert(domain.name.clone(), domain);
        }
        let active_domain =
            if let Some(existing_name) = map.keys().find(|name| domain_names_equal(name, domain_name)).cloned() {
                map.shift_remove(&existing_name).unwrap()
            } else {
                return Err(anyhow!("No such domain \"{}\"", domain_name));
            };

        let mut domains = Vec::new();

        domains.push(&active_domain);
        domains.extend(map.values());

        write_domain_file(&domains)?;
        let domain_version_state = DomainVersionState::from(&active_domain.version)?;

        Ok(ActiveDomain {
            all_domains,
            active_domain,
            domain_version_state,
        })
    } else {
        Err(anyhow!(
            "No such domain \"{}\" (no domains at all, in fact)",
            domain_name
        ))
    }
}

/// Upgrades the domain from its version to CURRENT_VERSION, doing each step its version needs, then saves it as the
/// current version and makes it the active domain, which it returns. A step for each version which changed what is
/// kept, in order:
///
/// - 0.5.0: the data kept in the cache directory is converted into the domain's store, see convert.rs, and its
///   secrets are encrypted, see encrypt_secrets.
/// - 0.6.0: its token files and log are named by its ID, not its name, see files_key, so that renaming it renames
///   none of them.
///
/// Called when the user agrees to upgrade a domain on the Home page, and for --upgrade, saying how far it is with
/// progress. If a step fails, or progress is cancelled, the domain is left at its version, its old data as it was, so
/// that it can be upgraded again.
pub fn upgrade_domain(
    domain_name: &str,
    progress: &mut dyn bruce_db::progress::Progress,
) -> anyhow::Result<ActiveDomain> {
    let domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    let name = domain.name.clone();
    // A domain with no version is from before they had one, so older than any.
    let version = Version::parse(domain.version.trim()).unwrap_or_else(|_| Version::new(0, 0, 0));
    if version < Version::parse(STORE_VERSION)? {
        crate::convert::convert_domain(&name, progress)?;
        encrypt_secrets(&name)?;
    }
    if version < Version::parse(FILES_BY_ID_VERSION)? && !domain.awaiting_founding {
        progress.report("Naming the domain's files by its ID");
        let key = ensure_id(&name)?.hex();
        rename_files(&name, &key)?;
    }

    // Read again, as a step may have changed it, e.g. given it a site.
    let mut domain_file: DomainFile = read_domain_file()?;
    if let Some(domain) = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, &name))
    {
        domain.version = CURRENT_VERSION.to_string();
    }
    write_domain_file(&domain_file)?;
    set_active_domain(&name)
}

/// Encrypts the domain's secrets, see secrets.rs: its logins in the domain file, with every other domain's, as the
/// file is saved with them encrypted whichever domain they are, afresh, with new nonces, each time, and its modules'
/// token files, which are encrypted whole, those already encrypted being left as they are. A step of the upgrade to
/// 0.5.0, see upgrade_domain, and, until 0.5.0 is released, of --encryptsecrets, for the domains upgraded to 0.5.0
/// before secrets were encrypted.
pub fn encrypt_secrets(domain_name: &str) -> anyhow::Result<()> {
    let domain_file = read_domain_file()?;
    if !domain_file
        .iter()
        .any(|domain| domain_names_equal(&domain.name, domain_name))
    {
        return Err(anyhow!("No such domain \"{}\"", domain_name));
    }
    write_domain_file(&domain_file)?;
    for path in cache_usage_in(&crate::data_home::tokens_dir()?, domain_name)?.token_files {
        let value: serde_json::Value = serde_json::from_reader(fs::File::open(&path)?)
            .map_err(|error| anyhow!("{} is not JSON: {error}", path.display()))?;
        serde_json::to_writer_pretty(
            private_file::create_private_file(&path)?,
            &crate::secrets::seal(&value)?,
        )?;
    }
    Ok(())
}

/// Whether a domain may have the name, which is letters, digits and underscores, so that the hyphen in the name of a
/// cache directory, e.g. n2-octopus, ends the domain's name, see cache_usage_in.
pub fn is_valid_domain_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

pub fn create_domain(domain_name: &str) -> anyhow::Result<ActiveDomain> {
    let domain_name = domain_name.trim();
    if domain_name.is_empty() {
        return Err(anyhow!("Domain name cannot be empty"));
    }
    if !is_valid_domain_name(domain_name) {
        return Err(anyhow!("Domain name must contain only letters, digits and underscores"));
    }

    let mut domains: DomainFile = read_domain_file_if_any()?.unwrap_or_default();

    if domains
        .iter()
        .any(|domain| domain_names_equal(&domain.name, domain_name))
    {
        return Err(anyhow!("A domain named \"{}\" already exists", domain_name));
    }

    let mut active_domain = Domain::new();
    active_domain.name = domain_name.to_string();
    domains.insert(0, active_domain.clone());

    write_domain_file(&domains)?;

    let all_domains = domains.into_iter().map(|domain| domain.name).collect();
    let domain_version_state = DomainVersionState::from(&active_domain.version)?;

    Ok(ActiveDomain {
        all_domains,
        active_domain,
        domain_version_state,
    })
}

pub fn update_domain<T>(domain_name: &String, module_id: &str, module_profile: &T) -> anyhow::Result<()>
where
    T: Serialize,
{
    if let Some(mut domain_file) = read_domain_file_if_any()? {
        let mut found = false;

        for domain in domain_file.iter_mut() {
            if domain_names_equal(&domain.name, domain_name) {
                domain
                    .modules
                    .insert(module_id.to_string(), serde_json::to_value(module_profile)?);
                found = true;
            }
        }
        if !found {
            return Err(anyhow!("No such domain \"{}\"", domain_name));
        };

        write_domain_file(&domain_file)?;

        Ok(())
    } else {
        Err(anyhow!(
            "No such domain \"{}\" (no domains at all, in fact)",
            domain_name
        ))
    }
}

/// Saves the domain name, offline, as a redacted copy of source, see redact.rs, replacing any previous copy. Its
/// modules are set up as the source's are, without their logins. as_of is when its data was last fetched.
pub fn save_redacted_domain(source: &Domain, name: &str, as_of: Option<i64>) -> anyhow::Result<()> {
    let mut domains: DomainFile = read_domain_file()?;
    domains.retain(|domain| !domain_names_equal(&domain.name, name));
    let modules = source
        .modules
        .iter()
        .filter(|(module_id, _)| crate::redact::is_redactable(module_id))
        .map(|(module_id, module)| {
            let mut module = module.clone();
            if let Some(module) = module.as_object_mut() {
                module.remove("apiKey");
            }
            (module_id.clone(), module)
        })
        .collect();
    domains.push(Domain {
        name: name.to_string(),
        // Its data is in the old layout, see redact.rs, so it is converted into its store when it is upgraded.
        version: String::from(LAST_CACHE_VERSION),
        modules,
        // Not the capture devices, whose keys would let anyone with the copy manage them.
        settings: Settings {
            capture_devices: Vec::new(),
            ..source.settings.clone()
        },
        offline: true,
        as_of,
        // A site of its own, made when it is first used.
        id: None,
        // Not the app's keys in the source's site, which are no use offline, and would let anyone with the copy act as
        // the app in it.
        security: None,
        awaiting_founding: false,
    });
    write_domain_file(&domains)?;
    Ok(())
}

/// The files of a domain, for each plugin a <domain>-<plugin> directory of data, in the old cache directory,
/// and a <domain>-<plugin>.json file of tokens, in the tokens directory, see data_home.rs, and its store.
#[derive(Debug, Default, PartialEq)]
pub struct DomainCacheUsage {
    /// The bytes used by each plugin, by plugin id.
    pub plugins: BTreeMap<String, u64>,
    /// The cached data directories, which are moved to the trash when the domain is deleted.
    pub data_dirs: Vec<PathBuf>,
    /// The token files, which are deleted immediately when the domain is deleted.
    pub token_files: Vec<PathBuf>,
}

impl DomainCacheUsage {
    pub fn total(&self) -> u64 {
        self.plugins.values().sum()
    }
}

/// The cache files of the given domain and the disk space they use, with its store, see store.rs, as "store",
/// which is not one of its data_dirs, as it is named by the site, not the domain, so it is kept when the domain is
/// renamed.
pub fn domain_cache_usage(domain_name: &str) -> anyhow::Result<DomainCacheUsage> {
    // Its data in the old cache directory, and old copies of its tokens there, see data_home.rs, and its tokens.
    let mut usage = cache_usage_in(&AppContext::get_cache_dir_path()?, domain_name)?;
    let tokens = match files_key(domain_name)? {
        Some(key) => cache_usage_in(&crate::data_home::tokens_dir()?, &key)?,
        None => DomainCacheUsage::default(),
    };
    usage.token_files.extend(tokens.token_files);
    for (module, bytes) in tokens.plugins {
        *usage.plugins.entry(module).or_default() += bytes;
    }
    let domains: DomainFile = read_domain_file()?;
    let site = domains
        .iter()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .and_then(|domain| domain.id.as_deref())
        .and_then(|site| bruce_db::Id::parse(site).ok());
    if let Some(dir) = site
        .map(|site| crate::store::store_dir_of(&site))
        .transpose()?
        .filter(|dir| dir.exists())
    {
        usage.plugins.insert(String::from("store"), disk_usage(&dir)?);
    }
    Ok(usage)
}

fn cache_usage_in(cache_dir: &Path, domain_name: &str) -> anyhow::Result<DomainCacheUsage> {
    // Domain names are letters, digits and underscores, so the hyphen ends the domain name.
    let prefix = format!("{}-", domain_name);
    let mut usage = DomainCacheUsage::default();
    if let Ok(read_dir) = fs::read_dir(cache_dir) {
        for entry in read_dir {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let Some(rest) = file_name.strip_prefix(&prefix) else {
                continue;
            };
            let path = entry.path();
            let plugin = if entry.file_type()?.is_dir() {
                usage.data_dirs.push(path.clone());
                rest
            } else if let Some(plugin) = rest.strip_suffix(".json") {
                usage.token_files.push(path.clone());
                plugin
            } else {
                continue;
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
    } else {
        Ok(metadata.len())
    }
}

/// Deletes the token files of the given domain, moves its cached data to the trash and removes it from the domain
/// file. The active domain cannot be deleted.
pub fn delete_domain(domain_name: &str, active_domain: &str) -> anyhow::Result<()> {
    if domain_names_equal(domain_name, active_domain) {
        return Err(anyhow!("The active domain \"{}\" cannot be deleted", domain_name));
    }

    let mut domains: DomainFile = read_domain_file()?;
    let count = domains.len();
    // Its store goes to the trash with its other data, see store.rs.
    let store_dir_of = domains
        .iter()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .and_then(|domain| domain.id.as_deref())
        .map(|site| bruce_db::Id::parse(site).and_then(|site| crate::store::store_dir_of(&site)))
        .transpose()?
        .filter(|dir| dir.exists());
    domains.retain(|domain| !domain_names_equal(&domain.name, domain_name));
    if domains.len() == count {
        return Err(anyhow!("No such domain \"{}\"", domain_name));
    }

    // The tokens are not kept in the trash.
    let usage = domain_cache_usage(domain_name)?;
    for path in &usage.token_files {
        fs::remove_file(path)?;
    }
    // The log files go to the trash with the data, unless it has none of its own, see logging::log_path.
    let logs = match files_key(domain_name)? {
        Some(_) => crate::logging::log_files(domain_name)?.to_vec(),
        None => Vec::new(),
    };
    let trashed: Vec<PathBuf> = usage
        .data_dirs
        .iter()
        .cloned()
        .chain(logs.into_iter().filter(|log| log.exists()))
        .chain(store_dir_of)
        .collect();
    if !trashed.is_empty() {
        trash::delete_all(&trashed)?;
    }

    write_domain_file(&domains)?;
    Ok(())
}

/// The name a new domain is to have, trimmed, if it is valid and no domain has it.
fn new_domain_name<'a>(domains: &DomainFile, name: &'a str) -> anyhow::Result<&'a str> {
    let name = name.trim();
    if !is_valid_domain_name(name) {
        return Err(anyhow!("A domain's name must be letters, digits and underscores"));
    }
    if domains.iter().any(|domain| domain_names_equal(&domain.name, name)) {
        return Err(anyhow!("A domain named \"{}\" already exists", name));
    }
    Ok(name)
}

/// The path of one of a domain's files, path, as another domain's, e.g. n2-octopus as n3-octopus, as the domain's
/// name ends at the first hyphen, see is_valid_domain_name, or its log, e.g. n2.log as n3.log.
fn renamed(path: &Path, from: &str, to: &str) -> anyhow::Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("{} has no name", path.display()))?;
    let rest = file_name
        .strip_prefix(from)
        .ok_or_else(|| anyhow!("{} is not of domain {from}", path.display()))?;
    Ok(path.with_file_name(format!("{to}{rest}")))
}

/// Renames a domain, which is not the active one, as the app has its files open: its entry in the domain file, its
/// cached data and logins, and its log.
pub fn rename_domain(from: &str, to: &str, active_domain: &str) -> anyhow::Result<()> {
    if domain_names_equal(from, active_domain) {
        return Err(anyhow!(
            "The active domain \"{}\" cannot be renamed, change to another one first",
            from
        ));
    }
    let mut domains: DomainFile = read_domain_file()?;
    let to = new_domain_name(&domains, to)?;
    let domain = domains
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, from))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", from))?;
    // As it is saved, as the match is not case sensitive. Its tokens and log are named by its ID, see files_key, so
    // only its data in the old cache directory, from before 0.5.0, is named by its name.
    let from = domain.name.clone();
    let usage = cache_usage_in(&AppContext::get_cache_dir_path()?, &from)?;
    for path in &usage.data_dirs {
        fs::rename(path, renamed(path, &from, to)?)?;
    }
    domain.name = to.to_string();
    write_domain_file(&domains)?;
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

/// Copies a domain, its settings and its cached data, but not its log, as a new domain to. An offline copy only shows
/// what was copied, as the source was when it was copied, see Domain::as_of, without fetching anything, so it has
/// neither the source's logins, the token files, nor its capture devices, which it would otherwise manage as the
/// source does; another copy has both, and fetches as the source does.
pub fn copy_domain(from: &str, to: &str, offline: bool) -> anyhow::Result<()> {
    let mut domains: DomainFile = read_domain_file()?;
    let to = new_domain_name(&domains, to)?;
    let source = domains
        .iter()
        .find(|domain| domain_names_equal(&domain.name, from))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", from))?
        .clone();
    // An ID of its own, which its files are named by, see files_key, with a copy of the source's store, so that each
    // is deleted alone, see delete_domain.
    let site = crate::store::new_id(to)?;
    let usage = cache_usage_in(&AppContext::get_cache_dir_path()?, &source.name)?;
    for dir in &usage.data_dirs {
        copy_dir(dir, &renamed(dir, &source.name, to)?)?;
    }
    if !offline && let Some(key) = files_key(&source.name)? {
        for file in cache_usage_in(&crate::data_home::tokens_dir()?, &key)?.token_files {
            fs::copy(&file, renamed(&file, &key, &site.hex())?)?;
        }
    }
    if let Some(from) = source.id.as_deref().map(bruce_db::Id::parse).transpose()?
        && crate::store::store_dir_of(&from)?.exists()
    {
        let location = format!("file://{}", crate::store::store_dir()?.display());
        bruce_db::Store::copy_domain(&location, &from, &site)?;
    }
    let site = Some(site.to_string());
    // A new site has none of the source's principals or SCs, so the app is made a principal of it afresh.
    let mut copy = Domain {
        name: to.to_string(),
        id: site,
        security: None,
        ..source
    };
    if offline {
        copy.offline = true;
        copy.as_of = copy.as_of.or_else(|| Some(crate::clock::now_utc().unix_timestamp()));
        copy.settings.capture_devices = Vec::new();
    }
    domains.push(copy);
    write_domain_file(&domains)?;
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
pub type DomainFile = Vec<Domain>;

#[derive(PartialEq)]
pub enum DomainVersionState {
    OlderThanCurrent,
    Current,
    NewerThanCurrent,
    Invalid,
}

impl DomainVersionState {
    pub fn from(version: &str) -> anyhow::Result<DomainVersionState> {
        let version = version.trim();

        if version.is_empty() {
            return Ok(DomainVersionState::OlderThanCurrent);
        }

        let current_version = Version::parse(CURRENT_VERSION).unwrap();
        Ok(if let Ok(version) = Version::parse(version) {
            match version.cmp(&current_version) {
                std::cmp::Ordering::Less => Self::OlderThanCurrent,
                std::cmp::Ordering::Equal => Self::Current,
                std::cmp::Ordering::Greater => Self::NewerThanCurrent,
            }
        } else {
            Self::Invalid
        })
    }
}

fn default_domain_version() -> String {
    "0.0.0".to_string()
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Domain {
    pub name: String,
    #[serde(default = "default_domain_version")]
    pub version: String,
    pub modules: ModuleProfiles,
    /// The app's own settings for the domain, which domains from before there were any do not have.
    #[serde(default)]
    pub settings: Settings,
    /// A redacted copy of another domain's data, see redact.rs, which only shows what it has, without logging in or
    /// fetching anything.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub offline: bool,
    /// For an offline domain, when its data was last fetched, as a unix timestamp. The app behaves as if it is then,
    /// see clock.rs, so that its pages show the latest data.
    #[serde(default, rename = "asOf", skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    /// The domain's ID, which never changes, see store.rs: its store's namespace, and what its files on this computer
    /// are named by, so that renaming the domain renames nothing. A domain made before 25 November 2026 has its site's
    /// ID, made the first time it was needed, until it is moved to its domain, see domain_security::found_domain. Kept
    /// as `site` before then.
    #[serde(default, alias = "site", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The domain's place in its site, a bruce-db domain: the user it acts for, this installation's private keys as a
    /// principal of the domain, and its SCs', see domain_security.rs, made when the user founds the domain or
    /// joins another's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<crate::domain_security::DomainKeys>,
    /// Whether the domain, just made, has no store until its domain is made, see await_founding, so that its store is
    /// opened only once, under the domain's ID, which is its namespace, and nothing has to be moved, nor the app
    /// restarted.
    #[serde(default, rename = "awaitingFounding", skip_serializing_if = "std::ops::Not::not")]
    pub awaiting_founding: bool,
}

/// The settings of a domain which are the app's rather than a module's, set on the Settings page.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub log: crate::logging::LogSettings,
    #[serde(default)]
    pub help: crate::help::HelpSettings,
    /// The capture devices this domain has adopted, see capture_device.rs, saved by update_capture_devices.
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
        crate::power::topology::BUILT_IN_CATEGORIES
            .iter()
            .map(|category| category.to_string())
            .chain(self.categories.iter().cloned())
            .collect()
    }
}

/// Saves the settings of the domain.
pub fn update_settings(domain_name: &str, settings: &Settings) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    // Whether the help is shown is saved by update_help_settings, from any window, the capture devices by
    // update_capture_devices, the topology by update_topology, and the categories and colours by update_settings_with,
    // so they are not overwritten here.
    let help = domain.settings.help.clone();
    let capture_devices = std::mem::take(&mut domain.settings.capture_devices);
    let topology = domain.settings.topology.take();
    let categories = std::mem::take(&mut domain.settings.categories);
    let colours = std::mem::take(&mut domain.settings.colours);
    domain.settings = settings.clone();
    domain.settings.help = help;
    domain.settings.capture_devices = capture_devices;
    domain.settings.topology = topology;
    domain.settings.categories = categories;
    domain.settings.colours = colours;
    write_domain_file(&domain_file)?;
    Ok(())
}

/// Saves the capture devices the domain has adopted, see capture_device.rs.
pub fn update_capture_devices(
    domain_name: &str,
    devices: &[crate::capture_device::DeviceProfile],
) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    domain.settings.capture_devices = devices.to_vec();
    write_domain_file(&domain_file)?;
    Ok(())
}

/// Changes the domain's settings as saved with change, and saves them, e.g. a colour chosen, leaving the rest as they
/// are saved, which may be newer than the settings the page has, returning them as they are now.
pub fn update_settings_with(domain_name: &str, change: impl FnOnce(&mut Settings)) -> anyhow::Result<Settings> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    change(&mut domain.settings);
    let settings = domain.settings.clone();
    write_domain_file(&domain_file)?;
    Ok(settings)
}

/// Saves the site's topology, see power::topology, or clears it if it is None.
pub fn update_topology(domain_name: &str, topology: Option<&crate::power::topology::Circuit>) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    domain.settings.topology = topology.cloned();
    write_domain_file(&domain_file)?;
    Ok(())
}

/// The domain's ID, see Domain::id, made and saved if it has none yet, unless the domain is awaiting
/// its domain, see await_founding, whose ID is made when it is founded.
pub fn ensure_id(domain_name: &str) -> anyhow::Result<bruce_db::Id> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    if let Some(id) = &domain.id {
        return bruce_db::Id::parse(id);
    }
    if domain.awaiting_founding {
        bail!("The domain {domain_name} has not been founded yet: found it on the Devices page");
    }
    let id = crate::store::new_id(&domain.name)?;
    domain.id = Some(id.to_string());
    write_domain_file(&domain_file)?;
    Ok(id)
}

/// Notes that the domain, just made, is to have no store until its domain is made, see Domain::awaiting_founding.
pub fn await_founding(domain_name: &str) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    if domain.id.is_none() {
        domain.awaiting_founding = true;
        write_domain_file(&domain_file)?;
    }
    Ok(())
}

/// Makes id the domain's ID, see domain_security::found_domain, its store having been copied to it, so that the
/// domain's data goes with it, and forgets its keys of the site before, which are not the domain's.
pub fn set_id(domain_name: &str, id: &bruce_db::Id) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    domain.id = Some(id.to_string());
    domain.security = None;
    domain.awaiting_founding = false;
    write_domain_file(&domain_file)?;
    Ok(())
}

/// The app's keys in the site, see Domain::security, from the domain whose site it is, None if it has none yet.
pub fn domain_keys(site: &str) -> anyhow::Result<Option<crate::domain_security::DomainKeys>> {
    Ok(read_domain_file()?
        .into_iter()
        .find(|domain| domain.id.as_deref() == Some(site))
        .ok_or_else(|| anyhow!("No domain has the site {site}"))?
        .security)
}

/// Saves the app's keys in the site, see Domain::security, in the domain whose site it is.
pub fn save_domain_keys(site: &str, keys: &crate::domain_security::DomainKeys) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain.id.as_deref() == Some(site))
        .ok_or_else(|| anyhow!("No domain has the site {site}"))?;
    domain.security = Some(keys.clone());
    write_domain_file(&domain_file)?;
    Ok(())
}

/// Makes a domain named domain_name for a domain the user has joined, see domain_security::join: its site
/// and its place in it, keys, and its devices, which it holds no keys for, as every request to them is signed, see
/// domain_security.rs. It is added after the others, so that the active domain stays as it is.
pub fn create_joined_domain(
    domain_name: &str,
    site: &str,
    keys: crate::domain_security::DomainKeys,
    devices: Vec<crate::capture_device::DeviceProfile>,
) -> anyhow::Result<()> {
    let domain_name = domain_name.trim();
    if !is_valid_domain_name(domain_name) {
        return Err(anyhow!("Domain name must contain only letters, digits and underscores"));
    }
    let mut domains: DomainFile = read_domain_file_if_any()?.unwrap_or_default();
    if domains
        .iter()
        .any(|domain| domain_names_equal(&domain.name, domain_name))
    {
        return Err(anyhow!("A domain named \"{}\" already exists", domain_name));
    }
    if domains.iter().any(|domain| domain.id.as_deref() == Some(site)) {
        return Err(anyhow!("A domain has the site already"));
    }
    let mut domain = Domain::new();
    domain.name = domain_name.to_string();
    domain.id = Some(site.to_string());
    domain.security = Some(keys);
    domain.settings.capture_devices = devices;
    domains.push(domain);
    write_domain_file(&domains)?;
    Ok(())
}

/// Saves the help settings of the domain, see help::set_shown.
pub fn update_help_settings(domain_name: &str, help: &crate::help::HelpSettings) -> anyhow::Result<()> {
    let mut domain_file: DomainFile = read_domain_file()?;
    let domain = domain_file
        .iter_mut()
        .find(|domain| domain_names_equal(&domain.name, domain_name))
        .ok_or_else(|| anyhow!("No such domain \"{}\"", domain_name))?;
    domain.settings.help = help.clone();
    write_domain_file(&domain_file)?;
    Ok(())
}

impl Domain {
    pub fn new() -> Domain {
        Domain {
            name: DEFAULT_DOMAIN.to_string(),
            version: CURRENT_VERSION.to_string(),
            modules: ModuleProfiles::new(),
            settings: Settings::default(),
            offline: false,
            as_of: None,
            id: None,
            security: None,
            awaiting_founding: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::path::Path;

    use super::{cache_usage_in, copy_dir, format_size, renamed};

    #[test]
    fn cache_usage_is_split_by_plugin_and_leaves_out_other_domains() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-cache-usage-{}", std::process::id()));
        fs::create_dir_all(dir.join("p-octopus/sub")).unwrap();
        fs::create_dir_all(dir.join("p2-octopus")).unwrap();
        fs::write(dir.join("p-octopus/sub/data"), [0u8; 100]).unwrap();
        fs::write(dir.join("p-octopus.json"), [0u8; 10]).unwrap();
        fs::write(dir.join("p-other.json"), [0u8; 1]).unwrap();
        fs::write(dir.join("p-history.txt"), [0u8; 5]).unwrap();
        fs::write(dir.join("p2-octopus.json"), [0u8; 1000]).unwrap();

        let usage = cache_usage_in(&dir, "p").unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            usage.plugins.into_iter().collect::<Vec<_>>(),
            [(String::from("octopus"), 110), (String::from("other"), 1)]
        );
        assert_eq!(usage.data_dirs, [dir.join("p-octopus")]);
        let mut token_files = usage.token_files;
        token_files.sort();
        assert_eq!(token_files, [dir.join("p-octopus.json"), dir.join("p-other.json")]);
    }

    #[test]
    fn a_domains_files_are_renamed_from_the_start_of_their_names() {
        let dir = Path::new("/cache");
        assert_eq!(
            renamed(&dir.join("n2-octopus"), "n2", "home").unwrap(),
            dir.join("home-octopus")
        );
        assert_eq!(
            renamed(&dir.join("n2.log.old"), "n2", "home").unwrap(),
            dir.join("home.log.old")
        );
        assert!(renamed(&dir.join("other-octopus"), "n2", "home").is_err());
    }

    #[test]
    fn a_directory_is_copied_with_everything_in_it() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-copy-{}", std::process::id()));
        fs::create_dir_all(dir.join("from/2026/November")).unwrap();
        fs::write(dir.join("from/2026/November/3#snapshots"), "data").unwrap();
        fs::write(dir.join("from/top"), "top").unwrap();
        copy_dir(&dir.join("from"), &dir.join("to")).unwrap();
        let copied = (
            fs::read_to_string(dir.join("to/2026/November/3#snapshots")).unwrap(),
            fs::read_to_string(dir.join("to/top")).unwrap(),
        );
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(copied, (String::from("data"), String::from("top")));
    }

    #[test]
    fn sizes_are_formatted_in_decimal_units() {
        assert_eq!(format_size(999), "999 bytes");
        assert_eq!(format_size(12_345_678), "12.3 MB");
    }
}
