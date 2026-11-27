//! Who may do what on the device, from its domain's Security Contexts, see bruce-smart-home's
//! docs/CaptureDeviceReplication.md, section 1, and bruce-db's design, "Security Contexts and Principals". The domain
//! is the one the device's store keeps, as a domain's ID is its namespace, its store's site.
//!
//! - **The device is a principal of its domain,** made, with its private keys, when its store first has that domain,
//!   kept in NVS, never sent, and whose record and first key, signed by itself, /api/info gives, as `security`, for a
//!   co-owner's app to admit it to the domain, see bruce-smart-home's domain_security::admit_devices.
//! - **The domain's security records** are given to the device by an app which manages it, `POST /api/store/security`,
//!   as a new device's store has none, see post, and are kept in its store, as any others.
//! - **A signed request** may then do what its subject's role allows, see DomainAccess::allows, or, if its actor acts
//!   for the subject by a delegation, the lower of that and the delegation's, see bruce-db's "Subjects and actors":
//!   read the store if the subject is a member of the app's `data` or `control`, manage the device if it is a Write
//!   member of `control`, as an admin is, or an Admin of the domain, as a co-owner is, and through the domain of
//!   `control` too. The adopting app's key goes on working alongside, see bruce-esp-idf's access.rs.
//! - **The store's security records** are given, in its change feed, only to the adopting app's key, or to a request
//!   whose subject is an Admin of the domain, see sees_security_records, so that a reader is never given a list of
//!   members, see bruce-db's "Servers and clients".

use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow, bail};
use bruce_db::security::{self, KeyRecord, NewPrincipal, PrincipalRecord, PrincipalSecret, Requester, Role, Security};
use bruce_db::{Id, Item, Micros, Store, Version};
use bruce_esp_idf::access::{Access, Authoriser, Forbidden, SignedRequest};
use bruce_esp_idf::svc::http::server::{EspHttpConnection, Request};
use bruce_esp_idf::svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::{info, warn};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db::Db;

/// The NVS namespace the device's principal is kept in, and its domain, its secret, as PrincipalSecret::to_text gives,
/// and its record and first key, as JSON.
const NAMESPACE: &str = "domain";
const DOMAIN_KEY: &str = "domain";
const SECRET_KEY: &str = "secret";
const RECORD_KEY: &str = "record";
const FIRST_KEY: &str = "key";

/// The app's SC, and under it the one whose members manage the devices, and the one their records are in, see the
/// module documentation.
const APP_SC: &str = "bruce-smart-home";
const CONTROL_SC: &str = "control";
const DATA_SC: &str = "data";

/// The stack of the threads the security work runs on, see on_big_stack.
const SECURITY_STACK: usize = 24 * 1024;

/// Runs work on a thread of its own with a stack of SECURITY_STACK, and waits for it, as working out who may do what,
/// and checking a signature, need more stack than the HTTP servers' tasks have, 6 to 7 KB, which overflowed on the
/// Guitron on 25 November 2026, the first time it was given its domain's records.
fn on_big_stack<T: Send>(work: impl FnOnce() -> anyhow::Result<T> + Send) -> anyhow::Result<T> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(bruce_embedded_std::thread_stack(SECURITY_STACK))
            .spawn_scoped(scope, work)
            .context("Unable to start the security check's thread")?
            .join()
            .map_err(|_| anyhow!("the security check panicked"))?
    })
}

/// The device as a principal of its domain, see DomainAccess::principal.
struct Principal {
    domain: Id,
    secret: PrincipalSecret,
    record: PrincipalRecord,
    key: KeyRecord,
}

/// The device as one of its domain's principals, and who may do what on it, given to the platform as its authoriser.
pub struct DomainAccess {
    db: Arc<Db>,
    nvs: Mutex<EspNvs<NvsDefault>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The device's principal in its store's domain, once it has one, see reload.
    principal: Option<Arc<Principal>>,
    /// Who may do what, worked out from the store's security records, see reload, None until there are a store and the
    /// domain's records, or if they could not be read.
    security: Option<Arc<Security>>,
    /// Why there is no security, if there should be.
    problem: Option<String>,
}

impl DomainAccess {
    /// The device's principals' store, and who may do what, from the store, if it has its domain's records.
    pub fn load(nvs_partition: EspNvsPartition<NvsDefault>, db: Arc<Db>) -> anyhow::Result<Arc<Self>> {
        let access = Arc::new(DomainAccess {
            db,
            nvs: Mutex::new(EspNvs::new(nvs_partition, NAMESPACE, true)?),
            state: Mutex::new(State::default()),
        });
        access.reload();
        Ok(access)
    }

    /// The device's principal in domain, made, and kept in NVS, if it has none there yet, e.g. as its store has just
    /// been given another domain.
    fn principal_in(&self, domain: &Id) -> anyhow::Result<Principal> {
        let nvs = self.nvs.lock().unwrap();
        let mut domain_buffer = vec![0u8; 128];
        let mut secret_buffer = vec![0u8; 512];
        let mut record_buffer = vec![0u8; 1024];
        let mut key_buffer = vec![0u8; 1024];
        if nvs.get_str(DOMAIN_KEY, &mut domain_buffer)? == Some(domain.as_str())
            && let (Some(secret), Some(record), Some(key)) = (
                nvs.get_str(SECRET_KEY, &mut secret_buffer)?,
                nvs.get_str(RECORD_KEY, &mut record_buffer)?,
                nvs.get_str(FIRST_KEY, &mut key_buffer)?,
            )
        {
            return Ok(Principal {
                domain: domain.clone(),
                secret: PrincipalSecret::from_text(secret)?,
                record: serde_json::from_str(record).context("The device's principal record")?,
                key: serde_json::from_str(key).context("The device's key record")?,
            });
        }
        let new = NewPrincipal::new(domain, security::SYSTEM)?;
        nvs.set_str(SECRET_KEY, &new.secret.to_text())?;
        nvs.set_str(RECORD_KEY, &serde_json::to_string(&new.record)?)?;
        nvs.set_str(FIRST_KEY, &serde_json::to_string(&new.key)?)?;
        nvs.set_str(DOMAIN_KEY, domain.as_str())?;
        info!("Made the device's principal {} in the domain {domain}", new.secret.id());
        Ok(Principal {
            domain: domain.clone(),
            secret: new.secret,
            record: new.record,
            key: new.key,
        })
    }

    /// The device's principal's secret, for signing its requests to its peers.
    #[allow(dead_code)]
    pub fn secret(&self) -> Option<PrincipalSecret> {
        let state = self.state.lock().unwrap();
        state.principal.as_ref().map(|principal| principal.secret.clone())
    }

    /// Works out who may do what again, from the store's security records, e.g. after some have arrived, the device's
    /// principal made for its store's domain if it has none.
    pub fn reload(&self) {
        let result = on_big_stack(|| -> anyhow::Result<(Option<Arc<Principal>>, Option<Arc<Security>>)> {
            let Some(store) = self.db.store() else {
                return Ok((None, None));
            };
            let domain = store.domain().clone();
            let current = self.state.lock().unwrap().principal.clone();
            let principal = match current.filter(|principal| principal.domain == domain) {
                Some(principal) => principal,
                None => Arc::new(self.principal_in(&domain)?),
            };
            // The domain's records may not have been given yet, which is not a problem.
            if store
                .get(&store.hash_key(security::SC_KIND, &[])?, domain.as_str())?
                .is_none()
            {
                return Ok((Some(principal), None));
            }
            Ok((Some(principal), Some(Arc::new(Security::load(&store)?))))
        });
        let mut state = self.state.lock().unwrap();
        match result {
            Ok((principal, security)) => {
                state.principal = principal;
                state.security = security;
                state.problem = None;
            }
            Err(error) => {
                warn!("Unable to work out who may use the device, {error:#}");
                state.security = None;
                state.problem = Some(format!("{error:#}"));
            }
        }
    }

    /// Who made a request, its subject and actor, if it is signed as it should be by principals the device knows.
    fn signer(&self, request: &SignedRequest) -> anyhow::Result<(Arc<Security>, Requester)> {
        let security = self
            .state
            .lock()
            .unwrap()
            .security
            .clone()
            .ok_or_else(|| anyhow!("the device has not been given its domain's security yet"))?;
        let requester = on_big_stack(|| {
            security.check_request(
                Some(request.authorization),
                request.method,
                request.uri,
                request.body,
                chrono::Utc::now().timestamp(),
            )
        })?;
        Ok((security, requester))
    }

    /// Whether a request, which the platform has let read the store, may be given its security records too, see the
    /// module documentation: one with the adopting app's key, or signed for an Admin of the domain.
    pub fn sees_security_records(&self, method: &str, uri: &str, authorization: Option<&str>) -> bool {
        let Some(authorization) = authorization.filter(|header| header.starts_with(security::SCHEME)) else {
            return true;
        };
        self.signer(&SignedRequest {
            method,
            uri,
            authorization,
            body: &[],
        })
        .is_ok_and(|(security, requester)| security.sees_security_records(&requester))
    }

    /// Puts the security records given, unless the store has them already, or later versions of them. Only records of
    /// the store's domain's security DataSets are taken, and they count only if signed as they should be, see
    /// bruce-db's Security::load.
    fn put_records(&self, given: &Given) -> anyhow::Result<usize> {
        let store = self
            .db
            .store()
            .ok_or_else(|| anyhow!("the device has no store, so can not keep its domain's security"))?;
        let mut put = 0;
        for record in &given.items {
            let item = record.item(&store)?;
            if store
                .get(&item.hk, &item.sk)?
                .is_some_and(|kept| kept.version >= item.version)
            {
                continue;
            }
            store.put(&item)?;
            put += 1;
        }
        Ok(put)
    }

    /// What /api/info gives as `security`, see Authoriser::info.
    fn said(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({
            "principal": state.principal.as_ref().map(|principal| &principal.record),
            "key": state.principal.as_ref().map(|principal| &principal.key),
            "domain": state.principal.as_ref().map(|principal| principal.domain.to_string()),
            "ready": state.security.is_some(),
            "problem": state.problem,
        })
    }
}

impl Authoriser for DomainAccess {
    fn scheme(&self) -> &str {
        security::SCHEME
    }

    fn allows(&self, request: &SignedRequest, access: Access) -> anyhow::Result<()> {
        let (security, requester) = self.signer(request)?;
        let in_sc = |name: &str, role: Role| {
            security
                .sc_path(&[APP_SC, name])
                .and_then(|sc| Id::parse(&sc.id).ok())
                .is_some_and(|sc| security.allows_request(&requester, &sc, role))
        };
        // A co-owner's app acting for them while they are away is Write in control, through the domain.
        let manages =
            security.allows_request(&requester, security.domain_id(), Role::Admin) || in_sc(CONTROL_SC, Role::Write);
        let allowed = match access {
            Access::Manage => manages,
            Access::Read => manages || in_sc(CONTROL_SC, Role::Read) || in_sc(DATA_SC, Role::Read),
        };
        if !allowed {
            // Signed as it should be, by someone who may not, which the device answers 403.
            return Err(Forbidden(format!(
                "{} may not {}",
                requester.subject,
                match access {
                    Access::Read => "read the device",
                    Access::Manage => "manage the device",
                }
            ))
            .into());
        }
        Ok(())
    }

    fn info(&self) -> Value {
        self.said()
    }

    fn released(&self) {
        // Its principal in the domain is kept, unused, until its store is given another domain, see principal_in.
        self.reload();
    }
}

/// One of the records an app gives, see post, as the app's store keeps it: its hash key, sort key, version and
/// attributes.
#[derive(Deserialize)]
struct GivenItem {
    hk: String,
    sk: String,
    /// Its version's time and origin.
    t: String,
    o: String,
    a: Value,
}

impl GivenItem {
    /// It as an item of store, if it is one of the store's domain's security records.
    fn item(&self, store: &Store) -> anyhow::Result<Item> {
        let site = self.hk.split('/').next().unwrap_or_default();
        if site != store.domain().as_str() || !security::is_security_data_set(&self.hk) {
            bail!("{} is not one of the domain's security DataSets", self.hk);
        }
        Ok(Item::new(
            &self.hk,
            &self.sk,
            Version::new(Micros::parse(&self.t)?, &self.o),
            self.a.clone(),
        ))
    }
}

/// What an app gives, see post.
#[derive(Deserialize)]
struct Given {
    #[serde(default)]
    items: Vec<GivenItem>,
}

/// POST /api/store/security {"items": [{"hk", "sk", "t", "o", "a"}]}, from an app which manages the device: puts the
/// domain's security records given in the store, which are only counted if they are signed as they should be, see
/// bruce-db's Security::load. As a new device's store has none, an app gives, when it moves the device over, every
/// security record it has, the device's principal and its membership of `data` among them, so that the device can check
/// who signs; the rest come later, e.g. when a member is added or removed. Answers with the device's security, as
/// /api/info's.
pub fn post(request: Request<&mut EspHttpConnection<'_>>, body: Vec<u8>, site: &DomainAccess) -> anyhow::Result<()> {
    let result = (|| -> anyhow::Result<()> {
        let given: Given = serde_json::from_slice(&body)?;
        let put = site.put_records(&given)?;
        info!("Put {put} of the domain's security records");
        site.reload();
        Ok(())
    })();
    match result {
        Ok(()) => bruce_esp_idf::api::respond(request, 200, &site.said()),
        Err(error) => {
            warn!("Unable to take the domain's security, {error:#}");
            bruce_esp_idf::api::respond(request, 400, &json!({ "error": format!("{error:#}") }))
        }
    }
}
