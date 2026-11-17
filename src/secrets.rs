//! The encryption of the secrets kept in the settings directory, see data_home.rs: the logins in the profile file, e.g.
//! an API key or a password, and the modules' token files, so that a slip which shows one of the files, e.g. printing
//! it, or a copy of it, e.g. in a backup, does not give them away.
//!
//! They are encrypted with XChaCha20-Poly1305, with a key kept in the system's credential store, the Keychain on macOS,
//! the Credential Manager on Windows and the Secret Service on Linux, see key, made the first time a secret is saved.
//! An encrypted value is a string of PREFIX and the base64 of its random nonce and its ciphertext, in place of the
//! value, so that the file keeps its shape and which fields it has can still be read. Every value of that form is
//! decrypted when a file is read, wherever it is, and the values of the fields which hold secrets, see is_secret_field,
//! are encrypted when it is saved, whichever module's they are, as are token files whole, see seal.
//!
//! If the key is missing, e.g. on another computer, or the credential store refuses it, or it is not the one any of a
//! file's values were encrypted with, reading the file fails, rather than dropping them, so that they are not lost when
//! the file is next saved. A value which cannot be decrypted when others can, e.g. as the file was edited by hand,
//! cannot be decrypted with any key, so it is dropped, and reported, see decrypt_secrets.

use std::sync::Mutex;

use anyhow::anyhow;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde_json::Value;

/// The start of an encrypted value, whose version says how the rest is to be decrypted.
const PREFIX: &str = "enc:v1:";
/// The length of a nonce, which is random for each value, as XChaCha20's is long enough for that to be safe.
const NONCE_LEN: usize = 24;
/// The name of the key in the credential store, by service and user.
const KEY_SERVICE: &str = "Bruce's Smart Home";
const KEY_USER: &str = "settings-encryption-key";

/// The key, once read from the credential store, or made, so that the store is asked once per run: on macOS each build
/// of the app is asked whether it may have it.
static KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);

/// The name of the credential store the key is kept in, for messages.
pub fn store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Keychain"
    } else if cfg!(windows) {
        "Credential Manager"
    } else {
        "Secret Service"
    }
}

/// The key, from the credential store, or, if it has none and create is true, a new random one, saved there.
fn key(create: bool) -> anyhow::Result<[u8; 32]> {
    let mut cached = KEY.lock().unwrap();
    if let Some(key) = *cached {
        return Ok(key);
    }
    let entry = keyring::Entry::new(KEY_SERVICE, KEY_USER)
        .map_err(|error| anyhow!("Unable to use the {} for the key of the secrets: {error}", store_name()))?;
    let key = match entry.get_secret() {
        Ok(secret) => secret
            .try_into()
            .map_err(|_| anyhow!("The key of the secrets in the {} is not 32 bytes", store_name()))?,
        Err(keyring::Error::NoEntry) if create => {
            let key: [u8; 32] = rand::random();
            entry
                .set_secret(&key)
                .map_err(|error| anyhow!("Unable to save the key of the secrets in the {}: {error}", store_name()))?;
            key
        }
        Err(keyring::Error::NoEntry) => {
            return Err(anyhow!(
                "The settings have encrypted secrets, but there is no key for them in the {}, e.g. as they were made on \
             another computer",
                store_name()
            ));
        }
        Err(error) => {
            return Err(anyhow!(
                "Unable to read the key of the secrets from the {}: {error}",
                store_name()
            ));
        }
    };
    *cached = Some(key);
    Ok(key)
}

/// Uses key instead of the credential store's, so that tests neither need nor change it.
#[cfg(test)]
pub fn use_key_for_tests(key: [u8; 32]) {
    *KEY.lock().unwrap() = Some(key);
}

/// Whether the field holds a secret, which is encrypted when saved: those which bruce_graphql redacts from logs, e.g.
/// apiKey, password or a token, a login's email, and a capture device's key.
fn is_secret_field(name: &str) -> bool {
    bruce_graphql::redact::is_secret_name(name) || name == "email" || name == "key"
}

/// Whether the text is an encrypted value.
pub fn is_encrypted(text: &str) -> bool {
    text.starts_with(PREFIX)
}

/// The text, encrypted.
fn encrypt(text: &str) -> anyhow::Result<String> {
    encrypt_with(&key(true)?, text)
}

/// The text, encrypted with key.
fn encrypt_with(key: &[u8; 32], text: &str) -> anyhow::Result<String> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce: [u8; NONCE_LEN] = rand::random();
    let ciphertext = cipher
        .encrypt(&XNonce::from(nonce), text.as_bytes())
        .map_err(|_| anyhow!("Unable to encrypt a secret"))?;
    let mut bytes = nonce.to_vec();
    bytes.extend(ciphertext);
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// The error of a value which is well formed but does not decrypt with the key, i.e. it was encrypted with another key,
/// or its ciphertext was changed, as distinct from one which is not even well formed, see decrypt_secrets.
#[derive(Debug)]
struct NotThisKey;

impl std::fmt::Display for NotThisKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Unable to decrypt a secret, the key in the {} is not the one it was encrypted with",
            store_name()
        )
    }
}

impl std::error::Error for NotThisKey {}

/// The text of an encrypted value, see is_encrypted.
fn decrypt(value: &str) -> anyhow::Result<String> {
    decrypt_with(&key(false)?, value)
}

/// The text of an encrypted value, decrypted with key.
fn decrypt_with(key: &[u8; 32], value: &str) -> anyhow::Result<String> {
    let encoded = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| anyhow!("The value is not encrypted"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| anyhow!("An encrypted secret is not base64: {error}"))?;
    if bytes.len() < NONCE_LEN {
        return Err(anyhow!("An encrypted secret is too short"));
    }
    let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let text = cipher
        .decrypt(&XNonce::from(nonce), ciphertext)
        .map_err(|_| anyhow::Error::new(NotThisKey))?;
    Ok(String::from_utf8(text)?)
}

/// Encrypts the value of every field which holds a secret, at any depth, see is_secret_field, which is a string and
/// not already encrypted. A null, which says there is none, is left as it is.
pub fn encrypt_secrets(value: &mut Value) -> anyhow::Result<()> {
    match value {
        Value::Object(map) => {
            for (name, field) in map.iter_mut() {
                match field {
                    Value::String(text) if is_secret_field(name) && !is_encrypted(text) => *text = encrypt(text)?,
                    _ => encrypt_secrets(field)?,
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                encrypt_secrets(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// An encrypted value which could not be decrypted, see decrypt_secrets, by where it is in the file, e.g.
/// ["0", "modules", "octopus", "apiKey"], and why.
#[derive(Debug)]
pub struct UnreadableSecret {
    pub path: Vec<String>,
    pub error: anyhow::Error,
}

/// Decrypts every encrypted value, at any depth, whichever field it is in. A value which cannot be decrypted is made
/// null, as if there were none, and returned, so that it does not stop the rest of the file being read, e.g. a module's
/// login, which it then asks for again. But if none can be decrypted with the key, and one at least is well formed, the
/// key is taken to be the wrong one, and it is an error, so that the values are kept for the right key.
pub fn decrypt_secrets(value: &mut Value) -> anyhow::Result<Vec<UnreadableSecret>> {
    let mut paths = Vec::new();
    encrypted_paths(value, &mut Vec::new(), &mut paths);
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    decrypt_secrets_with(&key(false)?, value, paths)
}

/// Decrypts the values at paths with key, see decrypt_secrets.
fn decrypt_secrets_with(
    key: &[u8; 32],
    value: &mut Value,
    paths: Vec<Vec<String>>,
) -> anyhow::Result<Vec<UnreadableSecret>> {
    let mut decrypted = 0;
    let mut unreadable = Vec::new();
    for path in paths {
        let Some(slot) = at_path(value, &path) else {
            continue;
        };
        match decrypt_with(key, slot.as_str().unwrap_or_default()) {
            Ok(text) => {
                *slot = Value::String(text);
                decrypted += 1;
            }
            Err(error) => unreadable.push(UnreadableSecret { path, error }),
        }
    }
    if decrypted == 0
        && let Some(index) = unreadable.iter().position(|secret| secret.error.is::<NotThisKey>())
    {
        return Err(unreadable.swap_remove(index).error);
    }
    for secret in &unreadable {
        if let Some(slot) = at_path(value, &secret.path) {
            *slot = Value::Null;
        }
    }
    Ok(unreadable)
}

/// Adds the path of every encrypted value in value, which is at path, to paths.
fn encrypted_paths(value: &Value, path: &mut Vec<String>, paths: &mut Vec<Vec<String>>) {
    match value {
        Value::String(text) if is_encrypted(text) => paths.push(path.clone()),
        Value::Object(map) => {
            for (name, field) in map {
                path.push(name.clone());
                encrypted_paths(field, path, paths);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                encrypted_paths(item, path, paths);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The value at path in value, see encrypted_paths.
fn at_path<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    path.iter().try_fold(value, |value, step| match value {
        Value::Object(map) => map.get_mut(step),
        Value::Array(items) => items.get_mut(step.parse::<usize>().ok()?),
        _ => None,
    })
}

/// The whole of value encrypted, as an encrypted string, for a file which is all secret, e.g. a module's tokens, or
/// value as it is if it is already, so that sealing twice does not encrypt it twice.
pub fn seal(value: &Value) -> anyhow::Result<Value> {
    match value {
        Value::String(text) if is_encrypted(text) => Ok(value.clone()),
        value => Ok(Value::String(encrypt(&value.to_string())?)),
    }
}

/// The value which seal encrypted, or the value as it is if it is not encrypted, e.g. a file saved before secrets were.
pub fn unseal(value: Value) -> anyhow::Result<Value> {
    match value {
        Value::String(text) if is_encrypted(&text) => Ok(serde_json::from_str(&decrypt(&text)?)?),
        value => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn secrets_are_encrypted_in_place_and_decrypted_back() {
        use_key_for_tests([7; 32]);
        let original = json!([{
            "name": "Home",
            "modules": {
                "octopus": { "apiKey": "sk_live_x", "billingTimezone": "Europe/London", "accountNumber": null },
                "globird": { "email": "someone@example.com", "password": "hunter2" },
            },
            "settings": { "captureDevices": [{ "id": "f4650ba9e5c4", "address": "192.168.4.97", "key": "c492" }] },
        }]);
        let mut value = original.clone();
        encrypt_secrets(&mut value).unwrap();

        let octopus = &value[0]["modules"]["octopus"];
        assert!(is_encrypted(octopus["apiKey"].as_str().unwrap()));
        assert_eq!(octopus["billingTimezone"], "Europe/London");
        assert!(octopus["accountNumber"].is_null());
        assert!(is_encrypted(value[0]["modules"]["globird"]["email"].as_str().unwrap()));
        assert!(is_encrypted(
            value[0]["settings"]["captureDevices"][0]["key"].as_str().unwrap()
        ));
        assert_eq!(value[0]["settings"]["captureDevices"][0]["address"], "192.168.4.97");
        assert!(!value.to_string().contains("hunter2"));

        // Saving again leaves what is encrypted as it is.
        let once = value.clone();
        encrypt_secrets(&mut value).unwrap();
        assert_eq!(value, once);

        decrypt_secrets(&mut value).unwrap();
        assert_eq!(value, original);
    }

    #[test]
    fn each_value_has_its_own_nonce() {
        use_key_for_tests([7; 32]);
        assert_ne!(encrypt("same").unwrap(), encrypt("same").unwrap());
    }

    #[test]
    fn a_sealed_file_is_unsealed_and_a_plain_one_left_as_it_is() {
        use_key_for_tests([7; 32]);
        let tokens = json!({ "token": "abc", "expires": 1793874512 });
        let sealed = seal(&tokens).unwrap();
        assert!(is_encrypted(sealed.as_str().unwrap()));
        assert_eq!(unseal(sealed).unwrap(), tokens);
        assert_eq!(unseal(tokens.clone()).unwrap(), tokens);
    }

    #[test]
    fn a_damaged_value_is_dropped_and_reported_when_others_decrypt() {
        let key = [9; 32];
        let mut value = json!([{
            "modules": {
                "octopus": { "apiKey": encrypt_with(&key, "sk_live_x").unwrap() },
                "globird": { "password": "enc:v1:garbage!" },
            },
        }]);
        let damaged = {
            let mut text = encrypt_with(&key, "hunter2").unwrap();
            text.pop();
            text.push(if text.ends_with('A') { 'B' } else { 'A' });
            text
        };
        value[0]["modules"]["tesla"] = json!({ "password": damaged });
        let mut paths = Vec::new();
        encrypted_paths(&value, &mut Vec::new(), &mut paths);

        let unreadable = decrypt_secrets_with(&key, &mut value, paths).unwrap();
        let mut found: Vec<_> = unreadable.iter().map(|secret| secret.path.join("/")).collect();
        found.sort();
        assert_eq!(found, ["0/modules/globird/password", "0/modules/tesla/password"]);
        assert_eq!(value[0]["modules"]["octopus"]["apiKey"], "sk_live_x");
        assert!(value[0]["modules"]["globird"]["password"].is_null());
        assert!(value[0]["modules"]["tesla"]["password"].is_null());
    }

    #[test]
    fn values_of_another_key_are_an_error_and_kept() {
        let mut value = json!({ "apiKey": encrypt_with(&[1; 32], "sk_live_x").unwrap(), "password": "enc:v1:!" });
        let original = value.clone();
        let mut paths = Vec::new();
        encrypted_paths(&value, &mut Vec::new(), &mut paths);

        let error = decrypt_secrets_with(&[2; 32], &mut value, paths).unwrap_err();
        assert!(error.is::<NotThisKey>(), "{error:#}");
        assert_eq!(value, original);
    }

    #[test]
    fn a_tampered_value_is_an_error() {
        use_key_for_tests([7; 32]);
        let mut value = encrypt("secret").unwrap();
        value.pop();
        value.push(if value.ends_with('A') { 'B' } else { 'A' });
        assert!(decrypt(&value).is_err());
    }
}
