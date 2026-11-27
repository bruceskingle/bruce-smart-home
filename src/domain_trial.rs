//! A trial of the domain's Security Contexts against virtual capture devices, see docs/CaptureDeviceReplication.md,
//! section 1, run by hand, not by `cargo test`: each test is a step, run in a process of its own, as each app has its
//! own data home, in TRIAL_DIR, with the devices' addresses and the keys they were adopted with in TRIAL_DEVICES, as
//! `<address>=<key>,…`:
//!
//! 1. `trial_1_owner`, Bruce's app: adopts the devices, sets up their stores, reads them with their keys, then makes
//!    the domain, which moves its store and theirs to the domain's ID and admits them, reads them signed,
//!    checks a stranger is refused, and saves an invitation;
//! 2. `trial_2_request`, Jeff's app: makes Jeff's key file and his request to join, from the invitation;
//! 3. `trial_3_admit`, Bruce's app: admits Jeff as a reader, and tells the devices;
//! 4. `trial_4_join`, Jeff's app: joins with the reply, reads the devices signed as his app for him, without their
//!    security records, and is refused their settings.

use std::path::PathBuf;
use std::time::Duration;

use crate::capture_device::{self, DeviceClient, DeviceProfile};
use crate::domain_security::{self, JoinAs, KeyFile};
use crate::user_key;

const PASSWORD: &str = "trial password";
const OWNER: &str = "Owner";
const JOINED: &str = "Bruce_home";

/// The trial's folder, with the data home of the app named home in it, used from now on.
fn setup(home: &str) -> PathBuf {
    let dir = PathBuf::from(std::env::var("TRIAL_DIR").expect("TRIAL_DIR, the trial's folder"));
    let home = dir.join(home);
    std::fs::create_dir_all(&home).unwrap();
    crate::data_home::use_for_tests(&home);
    crate::secrets::use_key_for_tests([42u8; 32]);
    dir
}

/// The devices' addresses and keys, see the module documentation.
fn devices() -> Vec<(String, String)> {
    std::env::var("TRIAL_DEVICES")
        .expect("TRIAL_DEVICES, <address>=<key>,…")
        .split(',')
        .map(|device| {
            let (address, key) = device.split_once('=').expect("<address>=<key>");
            (address.to_string(), key.to_string())
        })
        .collect()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

/// Waits until the device's store keeps site, after it restarted to take it.
async fn await_store(device: &DeviceProfile, site: &str) {
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(client) = DeviceClient::for_device(device)
            && let Ok(info) = client.get_query("store/info", &[]).await
            && info["site"] == site
        {
            return;
        }
    }
    panic!("The device {} did not come back with the domain's store", device.id);
}

/// Whether a request was refused as its signer may not do it, 403.
fn forbidden(result: anyhow::Result<serde_json::Value>) -> bool {
    result.is_err_and(|error| {
        error
            .downcast_ref::<capture_device::StatusError>()
            .is_some_and(|error| error.status == 403)
    })
}

#[test]
#[ignore]
fn trial_1_owner() {
    let dir = setup("bruce");
    runtime().block_on(async {
        crate::domain::create_domain(OWNER).unwrap();
        let store = crate::store::for_domain(OWNER).unwrap();
        let site = store.domain().to_string();

        // Adopted with the keys seeded into the devices, as the Devices page would with their pairing codes.
        let mut adopted = Vec::new();
        for (address, key) in devices() {
            let found = capture_device::find(&address).await.unwrap();
            adopted.push(DeviceProfile {
                id: found.info.id.clone(),
                address,
                fingerprint: found.fingerprint.clone(),
                key,
            });
        }
        crate::domain::update_capture_devices(OWNER, &adopted).unwrap();
        for device in &adopted {
            capture_device::set_up_store(device, &site, true).await.unwrap();
        }
        for device in &adopted {
            await_store(device, &site).await;
        }
        // Before a domain, the keys are used.
        for device in &adopted {
            let info = capture_device::replicate(&store, device, &mut |_| {}).await.unwrap();
            assert!(info.is_some());
            assert!(domain_security::signer_for(&device.id).is_none());
        }
        let before = capture_device::device_statuses()
            .into_iter()
            .map(|status| status.saved)
            .sum::<u64>();
        println!(
            "TRIAL: {} devices adopted, their stores for the site {site}, read with their keys, {before} changes",
            adopted.len()
        );

        // What the store has, which goes with it to the domain.
        let data_sets = |store: &bruce_db::Store| -> Vec<String> {
            let site = store.domain().to_string();
            store
                .data_sets(&format!("{site}/"))
                .unwrap()
                .into_iter()
                .filter(|hk| !bruce_db::security::is_security_data_set(hk))
                .map(|hk| hk[site.len()..].to_string())
                .collect()
        };
        let kept = data_sets(&store);
        assert!(!kept.is_empty());

        // The domain, made by Bruce, with a new key file: the store moved to it, and the devices too.
        tokio::time::sleep(Duration::from_secs(6)).await;
        let path = dir.join("Bruce.bruce-key");
        let message = domain_security::found_domain(
            OWNER,
            &KeyFile::New(path.clone()),
            "7 Sadlers Close",
            "Bruce",
            PASSWORD,
            PASSWORD,
            &adopted,
        )
        .await
        .unwrap();
        println!("TRIAL: {message}");
        let store = crate::store::for_domain(OWNER).unwrap();
        let domain = store.domain().to_string();
        assert_ne!(domain, site);
        assert!(
            message.contains(&format!("added {} of its devices", adopted.len())),
            "{message}"
        );
        assert!(user_key::unlock(&path, "not the password", &store.domain().clone()).is_err());
        let moved = data_sets(&store);
        assert!(kept.iter().all(|hk| moved.contains(hk)), "{kept:?} {moved:?}");
        println!(
            "TRIAL: the domain was founded, its ID now {domain}, with its {} DataSets",
            kept.len()
        );

        // Moved over, then read signed as Bruce's app for Bruce, once the replication above is no longer taken as
        // current, see capture_device's REPLICATED_FOR.
        tokio::time::sleep(Duration::from_secs(6)).await;
        for device in &adopted {
            let info = capture_device::replicate(&store, device, &mut |_| {}).await.unwrap();
            assert!(info.is_some());
            assert!(
                domain_security::signer_for(&device.id).is_some(),
                "{} not moved over",
                device.id
            );
            let client = DeviceClient::for_device(device).unwrap();
            let said = client.info().await.unwrap().security.unwrap();
            assert_eq!(said.domain.as_deref(), Some(domain.as_str()));
            assert!(said.ready, "{:?}", said.problem);
            // Managing it, which the app may do for Bruce while he is away: its settings.
            client.config().await.unwrap();
            // A signed read of its store.
            let info = client.get_query("store/info", &[]).await.unwrap();
            assert_eq!(info["site"], domain.as_str());
        }
        println!("TRIAL: the devices were moved over, and took signed requests, reads and settings");

        // A stranger, signing as themselves, is refused.
        let stranger = bruce_db::security::NewPrincipal::new(store.domain(), bruce_db::security::SYSTEM).unwrap();
        for device in &adopted {
            let time = time::OffsetDateTime::now_utc().unix_timestamp();
            let header = bruce_db::security::sign_request(&stranger.secret, "GET", "/api/store/info", b"", time);
            let response = reqwest::Client::new()
                .get(format!("http://{}/api/store/info", device.address))
                .header(reqwest::header::AUTHORIZATION, header)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), 401, "{}", device.id);
        }
        println!("TRIAL: a stranger's signed request was refused");

        let invitation = domain_security::invitation(&store).unwrap();
        std::fs::write(
            dir.join("invitation.json"),
            serde_json::to_vec_pretty(&invitation).unwrap(),
        )
        .unwrap();
        println!(
            "TRIAL: saved an invitation from {} to {}",
            invitation.by, invitation.name
        );
    });
}

#[test]
#[ignore]
fn trial_2_request() {
    let dir = setup("jeff");
    let invitation = domain_security::read_invitation(&std::fs::read(dir.join("invitation.json")).unwrap()).unwrap();
    let request = domain_security::ask_to_join(
        &invitation,
        &KeyFile::New(dir.join("Jeff.bruce-key")),
        "Jeff",
        PASSWORD,
        PASSWORD,
    )
    .unwrap();
    std::fs::write(dir.join("request.json"), serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    println!(
        "TRIAL: Jeff {} asks to join {} from his app {}",
        request.user.id, invitation.name, request.app.id
    );
}

#[test]
#[ignore]
fn trial_3_admit() {
    let dir = setup("bruce");
    runtime().block_on(async {
        let store = crate::store::for_domain(OWNER).unwrap();
        let adopted = capture_device::adopted_devices(OWNER);
        let site = domain_security::ensure(&store).unwrap().unwrap();
        let secret = user_key::unlock(&dir.join("Bruce.bruce-key"), PASSWORD, &site.user).unwrap();
        let request = domain_security::read_join_request(&std::fs::read(dir.join("request.json")).unwrap()).unwrap();
        let reply = domain_security::admit(&store, &request, JoinAs::Reader, &secret, &adopted).unwrap();
        let (sent, errors) = domain_security::push(&store, &adopted).await;
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(sent, adopted.len());
        std::fs::write(dir.join("reply.json"), serde_json::to_vec_pretty(&reply).unwrap()).unwrap();
        println!(
            "TRIAL: admitted {} as a reader, and told {sent} devices",
            request.user_name()
        );
    });
}

#[test]
#[ignore]
fn trial_4_join() {
    let dir = setup("jeff");
    runtime().block_on(async {
        let reply = domain_security::read_join_reply(&std::fs::read(dir.join("reply.json")).unwrap()).unwrap();
        let path = user_key::remembered().expect("Jeff's key file, from trial_2_request");
        domain_security::join(&reply, &path, JOINED).unwrap();
        let store = crate::store::for_domain(JOINED).unwrap();
        assert_eq!(store.domain().to_string(), reply.domain);
        println!("TRIAL: Jeff joined {}'s domain as a {}", reply.by, reply.role.label());

        // Read signed as Jeff's app for Jeff, with no keys, and given none of the domain's security records.
        for device in capture_device::adopted_devices(JOINED) {
            let info = capture_device::replicate(&store, &device, &mut |_| {}).await.unwrap();
            assert!(info.is_some());
            assert!(
                domain_security::signer_for(&device.id).is_some(),
                "{} not signed for",
                device.id
            );
            let status = capture_device::device_statuses()
                .into_iter()
                .find(|status| status.id == device.id)
                .unwrap();
            assert!(status.saved > 0, "nothing read from {}", device.id);
            println!("TRIAL: Jeff's app read {} changes from {}", status.saved, device.id);
            // A reader may not manage it.
            let client = DeviceClient::for_device(&device).unwrap();
            assert!(
                forbidden(client.config().await),
                "{} let a reader see its settings",
                device.id
            );
        }
        let listed = store
            .all(&bruce_db::security::PrincipalRecord::list_hash_key(&store).unwrap())
            .unwrap();
        assert!(listed.is_empty(), "a reader was given the list of principals");
        println!("TRIAL: Jeff read the devices, was given no security records, and was refused their settings");
    });
}

/// A domain just made, as the Domain menu makes it: it has no store until its domain is made, which is then opened
/// under the domain's ID, so that nothing is moved and the app need not be restarted. Run with TRIAL_DIR alone.
#[test]
#[ignore]
fn trial_new_domain() {
    let dir = setup("fresh");
    runtime().block_on(async {
        crate::domain::create_domain("Fresh").unwrap();
        crate::domain::await_founding("Fresh").unwrap();
        assert!(crate::store::for_domain("Fresh").is_err());
        let message = domain_security::found_domain(
            "Fresh",
            &KeyFile::New(dir.join("Fresh.bruce-key")),
            "",
            "Bruce",
            PASSWORD,
            PASSWORD,
            &[],
        )
        .await
        .unwrap();
        assert!(!message.contains("Quit the app"), "{message}");
        let store = crate::store::for_domain("Fresh").unwrap();
        let site = domain_security::ensure(&store).unwrap().unwrap();
        assert_eq!(&site.domain, store.domain());
        println!(
            "TRIAL: {message}; the new domain's store is the domain's, {}",
            store.domain()
        );
    });
}

/// A domain from 0.5.0, whose token files and log are named by its name, upgraded: they are then named by its ID, so
/// that renaming it renames none of them. Run with TRIAL_DIR alone.
#[test]
#[ignore]
fn trial_upgrade_names_files_by_id() {
    setup("upgrade");
    crate::domain::create_domain("Old").unwrap();
    let mut domains = crate::domain::read_domain_file().unwrap();
    domains
        .iter_mut()
        .for_each(|domain| domain.version = "0.5.0".to_string());
    crate::domain::write_domain_file(&domains).unwrap();
    let tokens = crate::data_home::tokens_dir().unwrap();
    let logs = crate::data_home::logs_dir().unwrap();
    crate::private_file::create_private_dir(&tokens).unwrap();
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(tokens.join("Old-octopus.json"), "{}").unwrap();
    std::fs::write(logs.join("Old.log"), "a line\n").unwrap();
    // As the app logs, under the domain's ID, before it is upgraded.
    let key = crate::domain::ensure_id("Old").unwrap().hex();
    std::fs::write(logs.join(format!("{key}.log")), "a later line\n").unwrap();

    crate::domain::upgrade_domain("Old", &mut bruce_db::progress::Silent).unwrap();
    assert_eq!(crate::domain::files_key("Old").unwrap().unwrap(), key);
    assert_eq!(
        std::fs::read_to_string(logs.join(format!("{key}.log"))).unwrap(),
        "a line\na later line\n"
    );
    assert!(!logs.join("Old.log").exists());
    assert!(tokens.join(format!("{key}-octopus.json")).exists());
    assert!(!tokens.join("Old-octopus.json").exists());
    assert_eq!(
        crate::logging::log_path("Old").unwrap(),
        logs.join(format!("{key}.log"))
    );
    assert!(logs.join(format!("{key}.log")).exists());

    // Renamed, its files are not.
    crate::domain::create_domain("Other").unwrap();
    crate::domain::rename_domain("Old", "Renamed", "Other").unwrap();
    assert_eq!(crate::domain::files_key("Renamed").unwrap().unwrap(), key);
    assert!(tokens.join(format!("{key}-octopus.json")).exists());
    println!("TRIAL: upgraded, the domain's files are named by its ID, {key}, and stay so when it is renamed");
}
