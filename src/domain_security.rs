//! The app's side of the domain's Security Contexts, see docs/CaptureDeviceReplication.md, section 1, and bruce-db's
//! design, "Security Contexts and Principals". A domain is a bruce-db domain, whose ID is its store's namespace.
//!
//! - **The user is a user principal of the domain,** whose keys are in their key file, see user_key.rs, one in each
//!   domain they belong to; **this installation of the app is a system principal of the domain,** made for it, which
//!   acts for the user: alongside them while they are present, and, while they are away, e.g. replicating a device,
//!   alone, by the delegation they signed it.
//! - **A domain is made only when the user asks,** see found_domain: from the user's new keys, then the user's
//!   principal in it, its founder, the app's, an SC for the app, `bruce-smart-home`, of which the domain is an Admin,
//!   and under it `control` and `data`, of which the app's SC is an Admin, `data` being the domain's default, with a
//!   delegation to this installation. A domain with a store already is moved to the domain's ID, as a domain's ID is
//!   its namespace. The domain keeps the user's ID, the app's secret and the SCs' private keys, see DomainKeys; the
//!   records, which are public, are in the store.
//! - **A capture device is admitted** to the domain by the user, present, see admit_devices, as only an Admin admits a
//!   principal or adds a member, which a delegation never makes the app; then **moved over,** see move_over, once a
//!   run, by giving it, with the key it was adopted with, the domain's security records.
//! - **The app then signs its requests to it** as this installation, acting for the user, see signer_for, rather than
//!   with the key, which it goes back to if the device refuses a signed request, e.g. as its clock is wrong, see
//!   capture_device.rs.
//! - **Joining** is by files: a co-owner saves an invitation, which gives the domain's ID, see invitation; the joining
//!   user's app makes their principal and its own for that domain and saves a request, see ask_to_join; a co-owner
//!   admits them, see admit, sends the records to the devices, see push, and saves a reply; the joining app makes a
//!   domain for the domain from it, see join.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow, bail};
use bruce_db::security::{
    DelegationRecord, DirectoryRecord, Founding, KeyRecord, MemberRecord, NewPrincipal, PrincipalRecord,
    PrincipalSecret, Role, SYSTEM, ScRecord, Security, USER,
};
use bruce_db::{Id, Item, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The names of the app's SCs, see the module documentation.
pub const APP_SC: &str = "bruce-smart-home";
pub const CONTROL_SC: &str = "control";
pub const DATA_SC: &str = "data";

/// The role to which this installation acts for its user while they are away: Write, the most a delegation gives, so
/// that it may manage and read the devices as far as the user may, but never add or remove a member.
const DELEGATED_ROLE: Role = Role::Write;

/// The directory in the settings directory with the app's principals made for domains it has asked to join, until the
/// reply comes, see ask_to_join and join.
const PENDING_DIR: &str = "joins";

/// What join files say they are, see invitation, ask_to_join and admit.
const INVITATION: &str = "bruce-invitation";
const JOIN_REQUEST: &str = "bruce-join-request";
const JOIN_REPLY: &str = "bruce-join-reply";

/// The domain's place in its domain, kept in the domain, see Domain::security. Fields named key are encrypted when
/// the domain is saved, see secrets.rs.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DomainKeys {
    /// The user principal this domain acts for in the domain, whose key file has their keys, see user_key.rs.
    pub user: String,
    /// This installation's principal in the domain; none in a domain's keys from before domains, 24 November 2026,
    /// which are not used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<ScKey>,
    /// The private keys of the SCs the user made, which are kept until they are wrapped for their members, later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scs: Vec<ScKey>,
}

/// A principal's or an SC's private keys, see DomainKeys.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ScKey {
    pub id: String,
    /// As PrincipalSecret::to_text or ScSecret::to_text gives them.
    pub key: String,
}

/// Debug output is used in verbose mode, so the keys are hidden.
impl std::fmt::Debug for DomainKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomainKeys")
            .field("user", &self.user)
            .field("app", &self.app.as_ref().map(|app| &app.id))
            .field("scs", &self.scs.len())
            .finish()
    }
}

/// What this installation of the app calls itself, e.g. in what a user says of it in a domain.
fn app_name() -> String {
    let host = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| name.trim().trim_end_matches(".local").to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| crate::store::node().unwrap_or_default());
    format!("Bruce's Smart Home on {host}")
}

/// A domain's place in its domain: this installation, the user it acts for, and the domain.
#[derive(Clone)]
pub struct DomainSecurity {
    pub app: Arc<PrincipalSecret>,
    pub user: Id,
    pub domain: Id,
}

impl DomainSecurity {
    /// Who may do what in the domain, from store's security records.
    pub fn load(&self, store: &Store) -> anyhow::Result<Security> {
        Security::load(store)
    }
}

/// The domains' places in their domains, once read, by the domain's ID, so that the domain is read once a run.
static SITES: Mutex<Option<HashMap<String, DomainSecurity>>> = Mutex::new(None);

/// The domain's place in store's domain, None if it has not made or joined one, see found_domain and join.
pub fn ensure(store: &Store) -> anyhow::Result<Option<DomainSecurity>> {
    let site = store.domain().to_string();
    if let Some(security) = SITES.lock().unwrap().get_or_insert_with(HashMap::new).get(&site) {
        return Ok(Some(security.clone()));
    }
    let Some(keys) = crate::domain::domain_keys(&site)? else {
        return Ok(None);
    };
    let Some(app) = &keys.app else {
        return Ok(None);
    };
    let app = PrincipalSecret::from_text(&app.key)?;
    if app.domain() != store.domain() {
        return Ok(None);
    }
    let security = DomainSecurity {
        app: Arc::new(app),
        user: Id::parse(&keys.user)?,
        domain: store.domain().clone(),
    };
    SITES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(site, security.clone());
    Ok(Some(security))
}

/// The domains which are to be asked to make their domain at once, as they were just made, see Home and the Site
/// section of the Devices page.
static NEW_DOMAINS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Notes that the domain was just made, so that the app asks to make its domain at once.
pub fn note_new_domain(domain_name: &str) {
    NEW_DOMAINS.lock().unwrap().insert(domain_name.to_string());
}

/// Whether the domain was just made and has not been asked to make its domain yet, see note_new_domain.
pub fn is_new_domain(domain_name: &str) -> bool {
    NEW_DOMAINS.lock().unwrap().contains(domain_name)
}

/// Forgets that the domain was just made, see note_new_domain, once it has been asked.
pub fn asked_new_domain(domain_name: &str) {
    NEW_DOMAINS.lock().unwrap().remove(domain_name);
}

/// The key file to put the user's new principal in, see found_domain and ask_to_join: one the app has, or a new one,
/// made with the password.
pub enum KeyFile {
    Existing(PathBuf),
    New(PathBuf),
}

impl KeyFile {
    /// Checks the password: that it opens an existing file, or is good for a new one, as confirm says.
    fn check(&self, password: &str, confirm: &str) -> anyhow::Result<()> {
        match self {
            KeyFile::Existing(path) => crate::user_key::check_password(path, password),
            KeyFile::New(_) => crate::user_key::check_new_password(password, confirm),
        }
    }

    /// Adds the principal to the file, made first if it is new.
    fn add(
        &self,
        password: &str,
        name: &str,
        secret: &PrincipalSecret,
        record: &PrincipalRecord,
    ) -> anyhow::Result<()> {
        let path = match self {
            KeyFile::Existing(path) => path,
            KeyFile::New(path) => {
                crate::user_key::create(path, password)?;
                path
            }
        };
        crate::user_key::add(path, password, name, secret, record)?;
        crate::user_key::remember(path)
    }
}

/// Founds the domain named domain_name, as its user asked, see the module documentation, called title, e.g. "7
/// Sadlers Close", or domain_name if it is empty, which its members see, e.g. in an invitation: the user, called name,
/// is its founder, their principal put in key_file, opened with password, or confirm for a new one. A
/// domain with a store already is moved to the domain's ID, after each of devices has been read, so that nothing it
/// took is left behind, and the devices' stores are moved too, then admitted to the domain. Returns what to say.
pub async fn found_domain(
    domain_name: &str,
    key_file: &KeyFile,
    title: &str,
    name: &str,
    password: &str,
    confirm: &str,
    devices: &[crate::capture_device::DeviceProfile],
) -> anyhow::Result<String> {
    if name.trim().is_empty() {
        bail!("Give your name");
    }
    let title = if title.trim().is_empty() {
        domain_name
    } else {
        title.trim()
    };
    key_file.check(password, confirm)?;
    let existing = crate::domain::fetch_active_domain(&Some(domain_name.to_string()))?.active_domain;
    if existing.security.as_ref().is_some_and(|keys| keys.app.is_some()) {
        bail!("{domain_name} has been made a domain already");
    }
    // What its files are named by until now, see domain::files_key, renamed with its ID, below.
    let old_key = crate::domain::files_key(domain_name)?;
    let old = existing
        .id
        .as_deref()
        .map(Id::parse)
        .transpose()?
        .filter(|site| crate::store::store_dir_of(site).is_ok_and(|dir| dir.exists()));
    // Everything the devices took is read first, as their stores move to the domain too.
    if old.is_some() {
        let store = crate::store::for_domain(domain_name)?;
        for device in devices {
            crate::capture_device::replicate(&store, device, &mut |_| {})
                .await
                .with_context(|| {
                    format!(
                        "Unable to read what the capture device {} has first, so that nothing is left behind; make the \
                         domain once it can be reached, or forget it first",
                        device.id
                    )
                })?;
        }
        store.sync()?;
    }
    let founding = Founding::new(title)?;
    let domain = Id::parse(&founding.domain.id)?;
    key_file.add(password, name, &founding.founder.secret, &founding.founder.record)?;
    if let Some(old) = &old {
        let location = format!("file://{}", crate::store::store_dir()?.display());
        let (old, domain) = (old.clone(), domain.clone());
        tokio::task::spawn_blocking(move || Store::copy_domain(&location, &old, &domain)).await??;
    }
    crate::domain::set_id(domain_name, &domain)?;
    if let Some(old_key) = &old_key {
        crate::domain::rename_files(old_key, &domain.hex())?;
    }
    crate::store::forget(domain_name);
    let store = crate::store::for_domain(domain_name)?;
    let (security, keys) = create(&store, &founding, name)?;
    crate::domain::save_domain_keys(domain.as_str(), &keys)?;
    SITES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(domain.to_string(), security.clone());
    crate::log_info!(
        "Made {domain_name} a domain, {domain}, with {} as its founder{}",
        security.user,
        old.as_ref()
            .map(|old| format!(", moved from the site {old}, which is left as it was"))
            .unwrap_or_default()
    );
    // The devices' stores moved to the domain, then, once they are back, admitted to it.
    let mut errors = Vec::new();
    let mut moved = Vec::new();
    for device in devices {
        match crate::capture_device::set_up_store(device, domain.as_str(), true).await {
            Ok(_) => moved.push(device.clone()),
            Err(error) => errors.push(format!("{}: {error:#}", device.id)),
        }
    }
    for device in &moved {
        if let Err(error) = await_store(device, domain.as_str()).await {
            errors.push(format!("{}: {error:#}", device.id));
        }
    }
    let (admitted, more) = admit_devices(&store, &founding.founder.secret, &moved).await?;
    errors.extend(more);
    // A store which was open before is still held by parts of the app, e.g. polling, so it must be opened again;
    // a new domain, which awaited its domain, had none, see Domain::awaiting_founding.
    Ok(format!(
        "Founded this domain, with you as its founder, and added {} of its devices{}{}",
        admitted.len(),
        not_done(&errors),
        if old.is_some() {
            ". Quit the app and open it again, so that every part of it uses the domain's store"
        } else {
            ""
        }
    ))
}

/// What to add to a message about the devices which could not be done, if any.
pub fn not_done(errors: &[String]) -> String {
    if errors.is_empty() {
        String::new()
    } else {
        format!("; not {}", errors.join("; "))
    }
}

/// Waits until the device's store keeps site, after it restarted to take it.
async fn await_store(device: &crate::capture_device::DeviceProfile, site: &str) -> anyhow::Result<()> {
    for _ in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Ok(client) = crate::capture_device::DeviceClient::for_device(device)
            && let Ok(info) = client.get_query("store/info", &[]).await
            && info["site"] == site
        {
            return Ok(());
        }
    }
    bail!("it did not come back with the domain's store")
}

/// Puts items in store.
fn put_all(store: &Store, items: impl IntoIterator<Item = anyhow::Result<Item>>) -> anyhow::Result<()> {
    for item in items {
        store.put(&item?)?;
    }
    Ok(())
}

/// Puts a principal's records in store: its record, admitted, its key, and its name.
fn put_principal(
    store: &Store,
    record: &PrincipalRecord,
    key: &KeyRecord,
    name: Option<&DirectoryRecord>,
) -> anyhow::Result<()> {
    let origin = crate::store::origin();
    for item in record.items(store, &origin)? {
        store.put(&item)?;
    }
    store.put(&key.item(store, &origin)?)?;
    if let Some(name) = name {
        store.put(&name.item(store, &origin)?)?;
    }
    Ok(())
}

/// Puts the domain's first records in store, see found_domain: its founder, called name, this installation, admitted by
/// the founder, the app's SCs, and the founder's delegation to this installation. Returns the domain's keys.
fn create(store: &Store, founding: &Founding, name: &str) -> anyhow::Result<(DomainSecurity, DomainKeys)> {
    let founder = &founding.founder.secret;
    let domain = Id::parse(&founding.domain.id)?;
    if store.domain() != &domain {
        bail!("The store is not the domain's");
    }
    let origin = crate::store::origin();
    put_principal(
        store,
        &founding.founder.record,
        &founding.founder.key,
        Some(&DirectoryRecord::name(founder.id(), &domain, name.trim(), founder)?),
    )?;
    store.put(&founding.domain.item(store, &origin)?)?;
    // The founder is the domain's Admin by name, not only as its creator, so that they stay one once others join.
    store.put(&MemberRecord::new(&domain, founder.id(), Role::Admin, founder)?.item(store, &origin)?)?;
    let app = NewPrincipal::new(&domain, SYSTEM)?;
    put_principal(
        store,
        &app.record.admitted_by(founder)?,
        &app.key,
        Some(&DirectoryRecord::name(app.secret.id(), &domain, &app_name(), founder)?),
    )?;
    store.put(&DelegationRecord::new(founder, app.secret.id(), DELEGATED_ROLE)?.item(store, &origin)?)?;
    let (app_key, app_sc) = ScRecord::new(APP_SC, founder)?;
    let (control_key, control) = ScRecord::new(CONTROL_SC, founder)?;
    let (data_key, data) = ScRecord::new(DATA_SC, founder)?;
    let (app_id, control_id, data_id) = (Id::parse(&app_sc.id)?, Id::parse(&control.id)?, Id::parse(&data.id)?);
    put_all(store, [&app_sc, &control, &data].map(|sc| sc.item(store, &origin)))?;
    put_all(
        store,
        [
            MemberRecord::new(&app_id, &domain, Role::Admin, founder),
            MemberRecord::new(&control_id, &app_id, Role::Admin, founder),
            MemberRecord::new(&data_id, &app_id, Role::Admin, founder),
        ]
        .map(|member| member.and_then(|member| member.item(store, &origin))),
    )?;
    let mut record = founding.domain.clone();
    record.default = Some(data.id.clone());
    record.sign(founder)?;
    store.put(&record.item(store, &origin)?)?;
    let keys = DomainKeys {
        user: founder.id().to_string(),
        app: Some(ScKey {
            id: app.secret.id().to_string(),
            key: app.secret.to_text(),
        }),
        scs: [
            (&founding.domain.id, founding.domain_secret.to_text()),
            (&app_sc.id, app_key.to_text()),
            (&control.id, control_key.to_text()),
            (&data.id, data_key.to_text()),
        ]
        .into_iter()
        .map(|(id, key)| ScKey { id: id.clone(), key })
        .collect(),
    };
    Ok((
        DomainSecurity {
            app: Arc::new(app.secret),
            user: founder.id().clone(),
            domain,
        },
        keys,
    ))
}

/// The SC at the path of names under the app's SC, e.g. `[DATA_SC]`.
fn app_sc(security: &Security, name: &str) -> anyhow::Result<Id> {
    Id::parse(
        &security
            .sc_path(&[APP_SC, name])
            .ok_or_else(|| anyhow!("The domain has no {APP_SC}/{name} SC"))?
            .id,
    )
}

/// An invitation to join a domain, see invitation: its ID and name, and who invited, none of it secret.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub format: String,
    pub domain: String,
    pub name: String,
    pub by: String,
}

/// What store's domain is called, see found_domain, if it has been founded.
pub fn title(store: &Store) -> anyhow::Result<Option<String>> {
    let Some(site) = ensure(store)? else {
        return Ok(None);
    };
    Ok(Some(site.load(store)?.domain().name.clone()))
}

/// Renames store's domain to title, see found_domain, signed by by, an Admin of the domain, as its record is changed,
/// present, their key file unlocked. The devices are then told, see push.
pub fn rename(store: &Store, by: &PrincipalSecret, title: &str) -> anyhow::Result<()> {
    let title = title.trim();
    if title.is_empty() {
        bail!("Give the domain's name");
    }
    let site = ensure(store)?.ok_or_else(|| anyhow!("This domain has not been founded"))?;
    let security = site.load(store)?;
    if !security.allows(by.id(), &site.domain, Role::Admin) {
        bail!("Only a co-owner of the domain may rename it");
    }
    let mut record = security.domain().clone();
    record.name = title.to_string();
    record.sign(by)?;
    store.put(&record.item(store, &crate::store::origin())?)?;
    if site.load(store)?.domain().name != title {
        bail!("The domain's new name was put, but does not count, see the log");
    }
    crate::log_info!("Renamed the domain {} to {title}", site.domain);
    Ok(())
}

/// An invitation to store's domain from the domain's user, see Invitation.
pub fn invitation(store: &Store) -> anyhow::Result<Invitation> {
    let site =
        ensure(store)?.ok_or_else(|| anyhow!("This domain has not been founded, so no one can be invited to it"))?;
    let security = site.load(store)?;
    Ok(Invitation {
        format: INVITATION.to_string(),
        domain: site.domain.to_string(),
        name: security.domain().name.clone(),
        by: security
            .display_name(&site.user)
            .map(str::to_string)
            .unwrap_or_else(|| site.user.to_string()),
    })
}

/// Reads an invitation, checking that it is one.
pub fn read_invitation(bytes: &[u8]) -> anyhow::Result<Invitation> {
    let invitation: Invitation = serde_json::from_slice(bytes).context("It is not an invitation to join a domain")?;
    if invitation.format != INVITATION {
        bail!("It is not an invitation to join a domain");
    }
    Id::parse(&invitation.domain)?;
    Ok(invitation)
}

/// A request to join a domain, see ask_to_join: the joining user's principal and this installation's, made for the
/// domain, the user's delegation to it, which gives nothing until the user is a member, and what the user calls
/// themselves and the app, none of it secret.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub format: String,
    pub domain: String,
    pub user: PrincipalRecord,
    pub user_key: KeyRecord,
    pub app: PrincipalRecord,
    pub app_key: KeyRecord,
    pub delegation: DelegationRecord,
    pub names: Vec<DirectoryRecord>,
}

impl JoinRequest {
    /// What the joining user calls themselves.
    pub fn user_name(&self) -> String {
        self.names
            .iter()
            .find(|said| said.subject == self.user.id)
            .and_then(|said| said.attributes.get(bruce_db::security::DISPLAY_NAME))
            .and_then(Value::as_str)
            .unwrap_or(&self.user.id)
            .to_string()
    }
}

/// What a user may be made when they join, see admit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinAs {
    /// An Admin of the domain, an equal of the other co-owners.
    CoOwner,
    /// Write in `control`: manages the devices, but adds no one.
    Admin,
    /// Read in `data`.
    Reader,
}

impl JoinAs {
    pub const ALL: [JoinAs; 3] = [JoinAs::CoOwner, JoinAs::Admin, JoinAs::Reader];

    pub fn label(self) -> &'static str {
        match self {
            JoinAs::CoOwner => "Co-owner",
            JoinAs::Admin => "Admin",
            JoinAs::Reader => "Reader",
        }
    }
}

/// A capture device, as a joining app is told of it, see JoinReply.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinDevice {
    pub id: String,
    pub address: String,
    pub fingerprint: String,
}

/// The reply to a join request, see admit: the domain, the user, what they were made, who admitted them, and the
/// devices, none of it secret.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinReply {
    pub format: String,
    pub domain: String,
    pub user: String,
    pub role: JoinAs,
    /// The name of the co-owner who admitted them, e.g. to name the domain.
    pub by: String,
    pub devices: Vec<JoinDevice>,
}

/// How this installation's principal made for a domain it asked to join is kept, until the reply comes, see
/// ask_to_join, its secret encrypted as the settings' secrets are.
#[derive(Serialize, Deserialize)]
struct Pending {
    key: String,
}

fn pending_path(domain: &Id) -> anyhow::Result<PathBuf> {
    Ok(crate::data_home::settings_dir()?
        .join(PENDING_DIR)
        .join(format!("{}.json", domain.hex())))
}

/// A request to join the domain of invitation, see JoinRequest: the user's principal for it, called name, put in
/// key_file, opened with password, or confirm for a new one, and this installation's, kept until the reply comes.
pub fn ask_to_join(
    invitation: &Invitation,
    key_file: &KeyFile,
    name: &str,
    password: &str,
    confirm: &str,
) -> anyhow::Result<JoinRequest> {
    if name.trim().is_empty() {
        bail!("Give your name");
    }
    key_file.check(password, confirm)?;
    let domain = Id::parse(&invitation.domain)?;
    let user = NewPrincipal::new(&domain, USER)?;
    let app = NewPrincipal::new(&domain, SYSTEM)?;
    let request = JoinRequest {
        format: JOIN_REQUEST.to_string(),
        domain: domain.to_string(),
        user: user.record.clone(),
        user_key: user.key.clone(),
        app: app.record.clone(),
        app_key: app.key.clone(),
        delegation: DelegationRecord::new(&user.secret, app.secret.id(), DELEGATED_ROLE)?,
        names: vec![
            DirectoryRecord::name(user.secret.id(), &domain, name.trim(), &user.secret)?,
            DirectoryRecord::name(app.secret.id(), &domain, &app_name(), &user.secret)?,
        ],
    };
    key_file.add(password, name, &user.secret, &user.record)?;
    let path = pending_path(&domain)?;
    crate::private_file::create_private_dir(path.parent().unwrap_or(Path::new(".")))?;
    let mut value = serde_json::to_value(Pending {
        key: app.secret.to_text(),
    })?;
    crate::secrets::encrypt_secrets(&mut value)?;
    let mut file = crate::private_file::create_private_file(&path)?;
    std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&value)?)?;
    crate::log_info!(
        "Asked to join the domain {domain} as {}, from the app's principal {}",
        user.secret.id(),
        app.secret.id()
    );
    Ok(request)
}

/// Reads a join request, checking that it is one, and that its records are as they should be.
pub fn read_join_request(bytes: &[u8]) -> anyhow::Result<JoinRequest> {
    let request: JoinRequest = serde_json::from_slice(bytes).context("It is not a request to join a domain")?;
    if request.format != JOIN_REQUEST {
        bail!("It is not a request to join a domain");
    }
    request.user.verify_own().context("The joining user's record")?;
    request.app.verify_own().context("The joining app's record")?;
    if !request.user.is_user() {
        bail!("Only a user principal joins a domain, not a {}", request.user.kind);
    }
    if request.user.domain != request.domain || request.app.domain != request.domain {
        bail!("The request's principals are not of its domain");
    }
    if request.user_key.principal != request.user.id || request.app_key.principal != request.app.id {
        bail!("The request's keys are not its principals'");
    }
    if request.delegation.subject != request.user.id || request.delegation.actor != request.app.id {
        bail!("The request's delegation is not from its user to its app");
    }
    Ok(request)
}

/// Admits the user of request to store's domain as role, signed by by, a co-owner, present, their key file unlocked,
/// putting their records in the store, and returns the reply for them, with devices, the domain's. The records are
/// then sent to the devices, see push.
pub fn admit(
    store: &Store,
    request: &JoinRequest,
    role: JoinAs,
    by: &PrincipalSecret,
    devices: &[crate::capture_device::DeviceProfile],
) -> anyhow::Result<JoinReply> {
    let site =
        ensure(store)?.ok_or_else(|| anyhow!("This domain has not been founded, so no one can be admitted to it"))?;
    if request.domain != site.domain.as_str() {
        bail!("The request is to join another domain than this domain's");
    }
    let security = site.load(store)?;
    if !security.allows(by.id(), &site.domain, Role::Admin) {
        bail!("Only a co-owner of the domain may admit someone");
    }
    let user = Id::parse(&request.user.id)?;
    let (member_sc, member_role) = match role {
        JoinAs::CoOwner => (site.domain.clone(), Role::Admin),
        JoinAs::Admin => (app_sc(&security, CONTROL_SC)?, Role::Write),
        JoinAs::Reader => (app_sc(&security, DATA_SC)?, Role::Read),
    };
    let origin = crate::store::origin();
    put_principal(store, &request.user.admitted_by(by)?, &request.user_key, None)?;
    put_principal(store, &request.app.admitted_by(by)?, &request.app_key, None)?;
    for said in &request.names {
        store.put(&said.item(store, &origin)?)?;
    }
    store.put(&request.delegation.item(store, &origin)?)?;
    store.put(&MemberRecord::new(&member_sc, &user, member_role, by)?.item(store, &origin)?)?;
    // Checked as the devices will, so that a reply is not sent for a membership which does not count.
    let security = site.load(store)?;
    if !security.allows(&user, &member_sc, member_role) {
        bail!("The membership was put, but does not count, see the log");
    }
    let name = request.user_name();
    crate::log_info!("Admitted {name} to the domain {} as {}", store.domain(), role.label());
    Ok(JoinReply {
        format: JOIN_REPLY.to_string(),
        domain: store.domain().to_string(),
        user: request.user.id.clone(),
        role,
        by: security
            .display_name(by.id())
            .map(str::to_string)
            .unwrap_or_else(|| by.id().to_string()),
        devices: devices
            .iter()
            .map(|device| JoinDevice {
                id: device.id.clone(),
                address: device.address.clone(),
                fingerprint: device.fingerprint.clone(),
            })
            .collect(),
    })
}

/// Reads a join reply, checking that it is one.
pub fn read_join_reply(bytes: &[u8]) -> anyhow::Result<JoinReply> {
    let reply: JoinReply = serde_json::from_slice(bytes).context("It is not a reply to a request to join a domain")?;
    if reply.format != JOIN_REPLY {
        bail!("It is not a reply to a request to join a domain");
    }
    Id::parse(&reply.domain)?;
    Id::parse(&reply.user)?;
    Ok(reply)
}

/// Makes a domain, named domain_name, for the domain of reply, whose user must be in key_file, with this
/// installation's principal made when it asked to join, see ask_to_join and domain::create_joined_domain.
pub fn join(reply: &JoinReply, key_file: &Path, domain_name: &str) -> anyhow::Result<()> {
    let domain = Id::parse(&reply.domain)?;
    let held = crate::user_key::principal_for(key_file, &domain)?
        .ok_or_else(|| anyhow!("The key file holds no principal of the domain; open the one you asked to join with"))?;
    if held.record.id != reply.user {
        bail!(
            "The reply is for another user than {}, whose key file this app has",
            held.name
        );
    }
    let path = pending_path(&domain)?;
    let mut value: Value = serde_json::from_slice(
        &std::fs::read(&path).context("This app did not ask to join the domain, or has joined it already")?,
    )?;
    crate::secrets::decrypt_secrets(&mut value)?;
    let pending: Pending = serde_json::from_value(value)?;
    let app = PrincipalSecret::from_text(&pending.key)?;
    let devices = reply
        .devices
        .iter()
        .map(|device| crate::capture_device::DeviceProfile {
            id: device.id.clone(),
            address: device.address.clone(),
            fingerprint: device.fingerprint.clone(),
            key: String::new(),
        })
        .collect();
    let keys = DomainKeys {
        user: reply.user.clone(),
        app: Some(ScKey {
            id: app.id().to_string(),
            key: app.to_text(),
        }),
        scs: Vec::new(),
    };
    crate::domain::create_joined_domain(domain_name, &reply.domain, keys, devices)?;
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Sends store's security records to each of devices, see move_over, so that a change, e.g. a member admitted, reaches
/// them at once, rather than by replication. Returns how many took them, and the errors of those which did not.
pub async fn push(store: &Arc<Store>, devices: &[crate::capture_device::DeviceProfile]) -> (usize, Vec<String>) {
    let mut sent = 0;
    let mut errors = Vec::new();
    for device in devices {
        let result = async {
            let body = {
                let store = store.clone();
                tokio::task::spawn_blocking(move || records(&store)).await??
            };
            post_records(&crate::capture_device::DeviceClient::for_device(device)?, body).await
        }
        .await;
        match result {
            Ok(_) => sent += 1,
            Err(error) => errors.push(format!("{}: {error:#}", device.id)),
        }
    }
    (sent, errors)
}

/// The most of a request's body a device takes, see bruce-esp-idf's api.rs, less a margin, so that the records are
/// sent in batches which fit, see post_records.
const BATCH_BYTES: usize = 6 * 1024;

/// Sends records, as records gives them, to a device with client, in batches small enough for it to take, see
/// BATCH_BYTES, returning its answer to the last, which says whether it is ready.
async fn post_records(client: &crate::capture_device::DeviceClient, records: Value) -> anyhow::Result<Value> {
    let items = records["items"].as_array().cloned().unwrap_or_default();
    let mut batches: Vec<Vec<Value>> = vec![Vec::new()];
    let mut size = 0;
    for item in items {
        let bytes = item.to_string().len();
        if size + bytes > BATCH_BYTES && !batches.last().is_some_and(Vec::is_empty) {
            batches.push(Vec::new());
            size = 0;
        }
        size += bytes + 1;
        batches.last_mut().unwrap_or_else(|| unreachable!()).push(item);
    }
    let mut answer = Value::Null;
    for batch in batches {
        answer = client.post("store/security", &json!({ "items": batch })).await?;
    }
    Ok(answer)
}

/// What a device says of itself in its domain, /api/info's `security`, see the capture firmware's site.rs; firmware
/// from before has none.
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct DeviceSecurity {
    /// The device's principal's record, signed by itself, and its first key, for its store's domain, once it has one.
    pub principal: Option<PrincipalRecord>,
    pub key: Option<KeyRecord>,
    /// The domain of its store.
    pub domain: Option<String>,
    /// Whether it has the domain's security records, and so checks signed requests.
    #[serde(default)]
    pub ready: bool,
    /// Why it can not work out who may use it, if it can not.
    pub problem: Option<String>,
}

/// Who signs requests to a device which has been moved over: this installation, acting for user.
#[derive(Clone)]
pub struct Signer {
    pub app: Arc<PrincipalSecret>,
    pub user: Id,
}

/// The signer of the requests to each device moved over, by the device's ID, see signer_for.
static SIGNERS: Mutex<Option<HashMap<String, Signer>>> = Mutex::new(None);

/// The devices move_over has looked at this run, so that it asks each once, unless it failed.
static CHECKED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Who to sign requests to the device as, if it has been moved over, see move_over.
pub fn signer_for(device_id: &str) -> Option<Signer> {
    SIGNERS.lock().unwrap().as_ref()?.get(device_id).cloned()
}

/// Stops signing requests to the device, e.g. as it refused one, so that the key is used, until move_over looks at it
/// again, at the next replication.
pub fn stop_signing(device_id: &str) {
    if let Some(signers) = SIGNERS.lock().unwrap().as_mut() {
        signers.remove(device_id);
    }
    CHECKED.lock().unwrap().remove(device_id);
}

fn start_signing(device_id: &str, site: &DomainSecurity) {
    SIGNERS.lock().unwrap().get_or_insert_with(HashMap::new).insert(
        device_id.to_string(),
        Signer {
            app: site.app.clone(),
            user: site.user.clone(),
        },
    );
}

/// Moves the device over to the domain's Security Contexts, once a run, if the domain has been founded, the device's
/// store is the domain's and its firmware has them, see the module documentation, giving it the domain's security
/// records with client, which uses its key. A device which has them already, e.g. for a domain which joined the domain,
/// needs nothing more. From then on, the app signs its requests to it. Returns whether it has been moved over.
pub async fn move_over(
    store: &Arc<Store>,
    client: &crate::capture_device::DeviceClient,
    device: &crate::capture_device::DeviceProfile,
) -> anyhow::Result<bool> {
    if !CHECKED.lock().unwrap().insert(device.id.clone()) {
        return Ok(signer_for(&device.id).is_some());
    }
    let result = try_move_over(store, client, device).await;
    if result.is_err() {
        CHECKED.lock().unwrap().remove(&device.id);
    }
    result
}

async fn try_move_over(
    store: &Arc<Store>,
    client: &crate::capture_device::DeviceClient,
    device: &crate::capture_device::DeviceProfile,
) -> anyhow::Result<bool> {
    let Some(site) = ensure(store)? else {
        return Ok(false);
    };
    let Some(said) = client.info().await?.security else {
        return Ok(false);
    };
    if said.domain.as_deref() != Some(site.domain.as_str()) {
        return Ok(false);
    }
    let Some(principal) = said.principal else {
        return Ok(false);
    };
    if said.ready && said.problem.is_none() {
        start_signing(&device.id, &site);
        return Ok(true);
    }
    if device.key.is_empty() {
        bail!(
            "The capture device {} has not been given the domain's records yet, which only a co-owner's app can do",
            device.id
        );
    }
    let device_principal = Id::parse(&principal.id)?;
    let body = {
        let (store, site, id) = (store.clone(), site.clone(), device.id.clone());
        tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
            let security = site.load(&store)?;
            if !security.allows(&device_principal, &app_sc(&security, DATA_SC)?, Role::Write) {
                bail!(
                    "The capture device {id} is not a member of the domain yet: Add Devices to the Domain, on the \
                     Devices page"
                );
            }
            records(&store)
        })
        .await??
    };
    let answer: DeviceSecurity = serde_json::from_value(post_records(client, body).await?)?;
    if !answer.ready {
        bail!(
            "The capture device {} did not take the domain's records, {}",
            device.id,
            answer.problem.as_deref().unwrap_or("it did not say why")
        );
    }
    crate::log_info!(
        "Moved the capture device {} over to the domain's Security Contexts",
        device.id
    );
    start_signing(&device.id, &site);
    Ok(true)
}

/// Admits each of devices to the domain and makes it a Write member of `data`, signed by user, a co-owner, present,
/// their key file unlocked, from its principal in its /api/info, for the devices whose firmware has one for the
/// domain and which are not members yet, so that move_over can then give them the domain's records, at the next
/// replication. Returns the IDs of those made members, and the errors of those which could not be.
pub async fn admit_devices(
    store: &Arc<Store>,
    user: &PrincipalSecret,
    devices: &[crate::capture_device::DeviceProfile],
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let site = ensure(store)?.ok_or_else(|| anyhow!("This domain has not been founded"))?;
    let (mut admitted, mut errors) = (Vec::new(), Vec::new());
    for device in devices {
        let result = async {
            let info = crate::capture_device::DeviceClient::for_device(device)?.info().await?;
            let name = info
                .hostname
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| device.id.clone());
            let Some(said) = info.security else {
                bail!("its firmware is from before the domain's Security Contexts");
            };
            if said.domain.as_deref() != Some(site.domain.as_str()) {
                bail!("its store is not the domain's yet");
            }
            let (Some(principal), Some(key)) = (said.principal, said.key) else {
                bail!("it has no principal in the domain yet");
            };
            principal.verify_own().context("Its principal record")?;
            if principal.domain != site.domain.as_str() || key.principal != principal.id {
                bail!("its principal is not of the domain");
            }
            let (store, site, user) = (store.clone(), site.clone(), user.clone());
            tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
                let security = site.load(&store)?;
                let data = app_sc(&security, DATA_SC)?;
                let member = Id::parse(&principal.id)?;
                if security.allows(&member, &data, Role::Write) {
                    return Ok(false);
                }
                if security.principal(&member).is_none() {
                    put_principal(
                        &store,
                        &principal.admitted_by(&user)?,
                        &key,
                        Some(&DirectoryRecord::name(&member, &site.domain, &name, &user)?),
                    )?;
                }
                let origin = crate::store::origin();
                store.put(&MemberRecord::new(&data, &member, Role::Write, &user)?.item(&store, &origin)?)?;
                Ok(true)
            })
            .await?
        }
        .await;
        match result {
            Ok(made) => {
                if made {
                    crate::log_info!(
                        "Made the capture device {} a member of the domain's {DATA_SC}",
                        device.id
                    );
                    admitted.push(device.id.clone());
                }
                // Looked at again at the next replication, now that it is a member.
                CHECKED.lock().unwrap().remove(&device.id);
            }
            Err(error) => errors.push(format!("{}: {error:#}", device.id)),
        }
    }
    Ok((admitted, errors))
}

/// Every security record in store's domain, as POST /api/store/security takes them: each item of the domain's
/// security DataSets, with its hash key, sort key and version, so that the device puts them as they are.
fn records(store: &Store) -> anyhow::Result<Value> {
    let mut items: Vec<Item> = Vec::new();
    let principals = store.all(&PrincipalRecord::list_hash_key(store)?)?;
    let scs = store.all(&ScRecord::hash_key(store)?)?;
    for listed in &principals {
        let id = Id::parse(&listed.sk)?;
        items.extend(store.all(&PrincipalRecord::hash_key(store, &id)?)?);
        items.extend(store.all(&DelegationRecord::hash_key(store, &id)?)?);
        items.extend(store.all(&DirectoryRecord::hash_key(store, &id, store.domain())?)?);
    }
    for sc in &scs {
        items.extend(store.all(&MemberRecord::hash_key(store, &Id::parse(&sc.sk)?)?)?);
    }
    items.extend(principals);
    items.extend(scs);
    let items: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "hk": item.hk,
                "sk": item.sk,
                "t": item.version.time.to_text(),
                "o": item.version.origin,
                "a": item.attributes,
            })
        })
        .collect();
    Ok(json!({ "items": items }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store of a new domain in a directory of its own, removed when the test ends.
    struct TestStore {
        dir: std::path::PathBuf,
        store: Store,
    }

    impl Drop for TestStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn test_store(name: &str, domain: &Id) -> TestStore {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-{name}-{}", std::process::id()));
        let store = Store::open(&format!("file://{}", dir.display()), domain).unwrap();
        TestStore { dir, store }
    }

    #[test]
    fn a_domain_made_by_its_user_gives_them_admin_and_their_app_write_by_delegation() {
        let founding = Founding::new("Home").unwrap();
        let domain = Id::parse(&founding.domain.id).unwrap();
        let test = test_store("site-security", &domain);
        let store = &test.store;
        let (site, keys) = create(store, &founding, "Bruce").unwrap();
        let security = site.load(store).unwrap();
        let user = founding.founder.secret.id();

        // The user is an Admin of the domain, and through it of the app's SC, control and data.
        assert!(security.allows(user, &domain, Role::Admin));
        let control = app_sc(&security, CONTROL_SC).unwrap();
        let data = app_sc(&security, DATA_SC).unwrap();
        for sc in [&control, &data] {
            assert!(security.allows(user, sc, Role::Admin));
        }
        assert_eq!(security.domain().default.as_deref(), Some(data.as_str()));
        assert_eq!(security.display_name(user), Some("Bruce"));
        // The app has no role of its own.
        assert_eq!(security.role(site.app.id(), &data), None);
        assert_eq!(keys.user, user.to_string());
        assert_eq!(keys.scs.len(), 4);

        // What a device is given: every record, with its version.
        let given = records(store).unwrap();
        let items = given["items"].as_array().unwrap();
        assert!(items.len() >= 12, "{}", items.len());

        // The app acting for the user while they are away manages and reads the devices, but adds no member.
        let now = 1_800_000_000;
        let uri = "/api/store/changes?since=0";
        let header = bruce_db::security::sign_request_for(
            bruce_db::security::Subject::Away(user),
            &site.app,
            "GET",
            uri,
            b"",
            now,
        );
        let requester = security.check_request(Some(&header), "GET", uri, b"", now).unwrap();
        assert!(requester.delegated);
        assert_eq!(security.role_for(&requester, &control), Some(Role::Write));
        assert_eq!(security.role_for(&requester, &data), Some(Role::Write));
        assert!(!security.allows_request(&requester, &domain, Role::Admin));
    }

    /// A co-owner renames the domain, which its record then says; a reader may not.
    #[test]
    fn a_co_owner_renames_the_domain() {
        let founding = Founding::new("Bruce").unwrap();
        let domain = Id::parse(&founding.domain.id).unwrap();
        let test = test_store("site-rename", &domain);
        let store = &test.store;
        let (site, _) = create(store, &founding, "Bruce").unwrap();
        SITES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(domain.to_string(), site.clone());
        assert_eq!(title(store).unwrap().as_deref(), Some("Bruce"));
        std::thread::sleep(std::time::Duration::from_millis(2));
        rename(store, &founding.founder.secret, " 7 Sadlers Close ").unwrap();
        assert_eq!(title(store).unwrap().as_deref(), Some("7 Sadlers Close"));
        let stranger = NewPrincipal::new(&domain, USER).unwrap();
        assert!(rename(store, &stranger.secret, "Not theirs").is_err());
        assert!(rename(store, &founding.founder.secret, "  ").is_err());
        SITES.lock().unwrap().as_mut().unwrap().remove(domain.as_str());
    }

    /// A reader joins: their request, read back, gives their app, by its delegation, Read in data and nothing more.
    #[test]
    fn a_reader_joins_and_their_app_reads_for_them() {
        let founding = Founding::new("Home").unwrap();
        let domain = Id::parse(&founding.domain.id).unwrap();
        let test = test_store("site-join", &domain);
        let store = &test.store;
        let (site, _) = create(store, &founding, "Bruce").unwrap();
        let owner = &founding.founder.secret;
        // As found_domain does, without the domain.
        SITES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(domain.to_string(), site.clone());

        // As ask_to_join does, without the key file.
        let jeff = NewPrincipal::new(&domain, USER).unwrap();
        let jeffs_app = NewPrincipal::new(&domain, SYSTEM).unwrap();
        let request = JoinRequest {
            format: JOIN_REQUEST.to_string(),
            domain: domain.to_string(),
            user: jeff.record.clone(),
            user_key: jeff.key.clone(),
            app: jeffs_app.record.clone(),
            app_key: jeffs_app.key.clone(),
            delegation: DelegationRecord::new(&jeff.secret, jeffs_app.secret.id(), DELEGATED_ROLE).unwrap(),
            names: vec![DirectoryRecord::name(jeff.secret.id(), &domain, "Jeff", &jeff.secret).unwrap()],
        };
        let request = read_join_request(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(request.user_name(), "Jeff");
        // Only a co-owner may admit.
        assert!(admit(store, &request, JoinAs::Reader, &jeff.secret, &[]).is_err());
        let reply = admit(store, &request, JoinAs::Reader, owner, &[]).unwrap();
        let reply = read_join_reply(&serde_json::to_vec(&reply).unwrap()).unwrap();
        assert_eq!(
            (reply.domain, reply.role, reply.by),
            (domain.to_string(), JoinAs::Reader, "Bruce".to_string())
        );

        let security = site.load(store).unwrap();
        let data = app_sc(&security, DATA_SC).unwrap();
        let control = app_sc(&security, CONTROL_SC).unwrap();
        let now = 1_800_000_000;
        let header = bruce_db::security::sign_request_for(
            bruce_db::security::Subject::Away(jeff.secret.id()),
            &jeffs_app.secret,
            "GET",
            "/api/store/info",
            b"",
            now,
        );
        let requester = security
            .check_request(Some(&header), "GET", "/api/store/info", b"", now)
            .unwrap();
        assert_eq!(security.role_for(&requester, &data), Some(Role::Read));
        assert_eq!(security.role_for(&requester, &control), None);
        assert!(!security.sees_security_records(&requester));
        SITES.lock().unwrap().as_mut().unwrap().remove(domain.as_str());
    }
}
