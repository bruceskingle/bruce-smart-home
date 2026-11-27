//! The user's key file: their user principals' private keys, one in each domain they belong to, see bruce-db's design,
//! "Principals", protected by one password, so that the user, not the app, holds their authority in each domain's
//! Security Contexts, see domain_security.rs. It may be kept anywhere, e.g. on a USB stick, and the app remembers
//! where, see remembered.
//!
//! The file is JSON, which says what it is: each principal's record, public, with the name the user gave themselves in
//! its domain, so that the app can say whose it is without the password; and the secrets, by principal ID, encrypted
//! together with XChaCha20-Poly1305 by a key derived from the password with Argon2id, with its parameters and salt, so
//! that they can be made stronger in a later file and still read. The password is never kept. A principal is added
//! to the file when the user founds or joins a domain, which needs the password, see add.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use base64::Engine;
use bruce_db::Id;
use bruce_db::security::{PrincipalRecord, PrincipalSecret};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

/// What a key file says it is, see KeyFile::format.
const FORMAT: &str = "bruce-key-file";
/// The version of the files this app makes: 2, with a principal in each domain; 1, from 24 November 2026, held one.
const VERSION: u32 = 2;
/// Argon2id's parameters for a new file: OWASP's first recommendation, 19 MiB, 2 passes, 1 lane, about 50 ms on a Mac.
const MEMORY_KIB: u32 = 19 * 1024;
const ITERATIONS: u32 = 2;
const PARALLELISM: u32 = 1;
/// The shortest password a new file is given.
pub const MIN_PASSWORD: usize = 8;
/// The file in the settings directory which says where the user's key file is, see remembered.
const REMEMBERED_FILE: &str = "user-key.json";

/// A key file, as it is kept.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyFile {
    format: String,
    version: u32,
    /// The principals' records, public, with their names.
    #[serde(default)]
    principals: Vec<Held>,
    kdf: Kdf,
    /// The secrets, by principal ID, as PrincipalSecret::to_text gives them, as JSON, encrypted, and the nonce, in
    /// base64.
    nonce: String,
    ciphertext: String,
}

/// One of the principals a key file holds, see KeyFile::principals.
#[derive(Clone, Serialize, Deserialize)]
pub struct Held {
    /// What the user called themselves in its domain.
    pub name: String,
    pub record: PrincipalRecord,
}

/// How the key is derived from the password.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Kdf {
    alg: String,
    salt: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unbase64(text: &str) -> anyhow::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|error| anyhow!("The key file is damaged, {error}"))
}

/// The key the password gives with kdf's parameters.
fn derive(password: &str, kdf: &Kdf) -> anyhow::Result<[u8; 32]> {
    if kdf.alg != "argon2id" {
        bail!(
            "The key file's key is derived with {}, which this app does not know",
            kdf.alg
        );
    }
    let params = argon2::Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32))
        .map_err(|error| anyhow!("The key file's parameters are not right, {error}"))?;
    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), &unbase64(&kdf.salt)?, &mut key)
        .map_err(|error| anyhow!("Unable to derive the key file's key, {error}"))?;
    Ok(key)
}

/// A new kdf, with a new salt, for a new file.
fn new_kdf() -> Kdf {
    Kdf {
        alg: "argon2id".to_string(),
        salt: base64(&rand::random::<[u8; 16]>()),
        memory_kib: MEMORY_KIB,
        iterations: ITERATIONS,
        parallelism: PARALLELISM,
    }
}

/// The secrets of file, by principal ID, if password is right.
fn open(file: &KeyFile, password: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let key = derive(password, &file.kdf)?;
    let nonce: [u8; 24] = unbase64(&file.nonce)?
        .try_into()
        .map_err(|_| anyhow!("The key file is damaged"))?;
    let text = XChaCha20Poly1305::new((&key).into())
        .decrypt(&XNonce::from(nonce), unbase64(&file.ciphertext)?.as_slice())
        .map_err(|_| anyhow!("The password is not right"))?;
    serde_json::from_slice(&text).context("The key file is damaged")
}

/// Seals secrets into file, with password and a new nonce.
fn seal(file: &mut KeyFile, password: &str, secrets: &BTreeMap<String, String>) -> anyhow::Result<()> {
    let key = derive(password, &file.kdf)?;
    let nonce: [u8; 24] = rand::random();
    let ciphertext = XChaCha20Poly1305::new((&key).into())
        .encrypt(&XNonce::from(nonce), serde_json::to_vec(secrets)?.as_slice())
        .map_err(|_| anyhow!("Unable to encrypt the key file"))?;
    file.nonce = base64(&nonce);
    file.ciphertext = base64(&ciphertext);
    Ok(())
}

/// Checks a new password is long enough, and the same as confirm.
pub fn check_new_password(password: &str, confirm: &str) -> anyhow::Result<()> {
    if password != confirm {
        bail!("The passwords are not the same");
    }
    if password.chars().count() < MIN_PASSWORD {
        bail!("The password must be at least {MIN_PASSWORD} characters");
    }
    Ok(())
}

/// Makes a new key file at path, holding no principals yet, protected by password, and remembers it.
pub fn create(path: &Path, password: &str) -> anyhow::Result<()> {
    if password.chars().count() < MIN_PASSWORD {
        bail!("The password must be at least {MIN_PASSWORD} characters");
    }
    let mut file = KeyFile {
        format: FORMAT.to_string(),
        version: VERSION,
        principals: Vec::new(),
        kdf: new_kdf(),
        nonce: String::new(),
        ciphertext: String::new(),
    };
    seal(&mut file, password, &BTreeMap::new())?;
    let mut out = crate::private_file::create_private_file(path)
        .with_context(|| format!("Unable to create the key file {}", path.display()))?;
    std::io::Write::write_all(&mut out, &serde_json::to_vec_pretty(&file)?)?;
    remember(path)?;
    crate::log_info!("Made the key file {}", path.display());
    Ok(())
}

/// Adds the principal of secret, whose record is record, called name in its domain, to the key file at path, which
/// password opens.
pub fn add(
    path: &Path,
    password: &str,
    name: &str,
    secret: &PrincipalSecret,
    record: &PrincipalRecord,
) -> anyhow::Result<()> {
    if record.id != secret.id().as_str() {
        bail!("The record is not the secret's principal's");
    }
    let mut file = read(path)?;
    let mut secrets = open(&file, password)?;
    secrets.insert(record.id.clone(), secret.to_text());
    file.principals.retain(|held| held.record.id != record.id);
    file.principals.push(Held {
        name: name.trim().to_string(),
        record: record.clone(),
    });
    seal(&mut file, password, &secrets)?;
    // Written beside it, then put in its place, so that a crash leaves the old file or the new, never half of one.
    let new = path.with_extension("bruce-key-new");
    let mut out = crate::private_file::create_private_file(&new)
        .with_context(|| format!("Unable to save the key file {}", path.display()))?;
    std::io::Write::write_all(&mut out, &serde_json::to_vec_pretty(&file)?)?;
    out.sync_all()?;
    std::fs::rename(&new, path).with_context(|| format!("Unable to save the key file {}", path.display()))?;
    crate::log_info!(
        "Added the principal {} of the domain {} to the key file {}",
        record.id,
        record.domain,
        path.display()
    );
    Ok(())
}

fn read(path: &Path) -> anyhow::Result<KeyFile> {
    let bytes = std::fs::read(path).with_context(|| format!("Unable to read the key file {}", path.display()))?;
    let file: KeyFile = serde_json::from_slice(&bytes).context("It is not a key file")?;
    if file.format != FORMAT {
        bail!("It is not a key file");
    }
    if file.version == 1 {
        bail!("The key file was made before domains, on 24 November 2026; make a new one");
    }
    if file.version != VERSION {
        bail!(
            "The key file is of version {}, which this app does not know",
            file.version
        );
    }
    Ok(file)
}

/// Checks that path is a key file this app can read.
pub fn check(path: &Path) -> anyhow::Result<()> {
    read(path).map(|_| ())
}

/// The principals the key file at path holds, which needs no password.
pub fn principals(path: &Path) -> anyhow::Result<Vec<Held>> {
    Ok(read(path)?.principals)
}

/// The key file's principal in domain, if it holds one.
pub fn principal_for(path: &Path, domain: &Id) -> anyhow::Result<Option<Held>> {
    Ok(principals(path)?
        .into_iter()
        .find(|held| held.record.domain == domain.as_str()))
}

/// The secret of the principal in the key file at path, if password is right.
pub fn unlock(path: &Path, password: &str, principal: &Id) -> anyhow::Result<PrincipalSecret> {
    let file = read(path)?;
    let secrets = open(&file, password)?;
    let text = secrets
        .get(principal.as_str())
        .ok_or_else(|| anyhow!("The key file holds no principal {principal}"))?;
    let secret = PrincipalSecret::from_text(text)?;
    if secret.id() != principal {
        bail!("The key file's secret is not its principal's");
    }
    Ok(secret)
}

/// Checks password opens the key file at path, e.g. before a domain is made, so that a wrong password is found before
/// anything is changed.
pub fn check_password(path: &Path, password: &str) -> anyhow::Result<()> {
    open(&read(path)?, password).map(|_| ())
}

/// Where the user's key file is, see the module documentation.
#[derive(Serialize, Deserialize)]
struct Remembered {
    path: PathBuf,
}

/// Notes that the user's key file is at path, so that the app asks for its password, not for the file, next time.
pub fn remember(path: &Path) -> anyhow::Result<()> {
    let file = crate::data_home::settings_dir()?.join(REMEMBERED_FILE);
    std::fs::write(
        file,
        serde_json::to_vec(&Remembered {
            path: path.to_path_buf(),
        })?,
    )?;
    Ok(())
}

/// The user's key file, where the app last made or opened it, if it is still there and this app can read it.
pub fn remembered() -> Option<PathBuf> {
    let file = crate::data_home::settings_dir().ok()?.join(REMEMBERED_FILE);
    let remembered: Remembered = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    check(&remembered.path).ok()?;
    Some(remembered.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_file_holds_a_principal_in_each_domain_and_opens_only_with_its_password() {
        let dir = std::env::temp_dir().join(format!("bruce-smart-home-key-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Bruce.bruce-key");
        // Not create, which remembers it in the real settings directory, and with light parameters, for speed.
        let mut file = KeyFile {
            format: FORMAT.to_string(),
            version: VERSION,
            principals: Vec::new(),
            kdf: Kdf {
                alg: "argon2id".to_string(),
                salt: base64(&[7u8; 16]),
                memory_kib: 64,
                iterations: 1,
                parallelism: 1,
            },
            nonce: String::new(),
            ciphertext: String::new(),
        };
        seal(&mut file, "correct horse", &BTreeMap::new()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let home = bruce_db::security::Founding::new("Home").unwrap();
        let away = Id::of(&serde_json::json!({ "domain": "away" })).unwrap();
        let joined = bruce_db::security::NewPrincipal::new(&away, bruce_db::security::USER).unwrap();
        let wrong = add(
            &path,
            "wrong horse",
            "Bruce",
            &home.founder.secret,
            &home.founder.record,
        );
        add(
            &path,
            "correct horse",
            "Bruce",
            &home.founder.secret,
            &home.founder.record,
        )
        .unwrap();
        add(&path, "correct horse", "B", &joined.secret, &joined.record).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let unlocked = unlock(&path, "correct horse", home.founder.secret.id());
        let refused = unlock(&path, "wrong horse", home.founder.secret.id());
        let held = principal_for(&path, &away).unwrap();
        let count = principals(&path).unwrap().len();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(format!("{:#}", wrong.err().unwrap()), "The password is not right");
        assert_eq!(unlocked.unwrap().id(), home.founder.secret.id());
        assert_eq!(format!("{:#}", refused.err().unwrap()), "The password is not right");
        assert_eq!(held.unwrap().name, "B");
        assert_eq!(count, 2);
        // The secrets are not in the file as they are.
        assert!(!text.contains(&home.founder.secret.to_text()));
    }
}
