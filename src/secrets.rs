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
//! If the key is missing, e.g. on another computer, or the credential store refuses it, reading a file with encrypted
//! values fails, rather than dropping them, so that they are not lost when the file is next saved.

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
fn store_name() -> &'static str {
    if cfg!(target_os = "macos") { "Keychain" } else if cfg!(windows) { "Credential Manager" } else { "Secret Service" }
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
        Ok(secret) => secret.try_into()
            .map_err(|_| anyhow!("The key of the secrets in the {} is not 32 bytes", store_name()))?,
        Err(keyring::Error::NoEntry) if create => {
            let key: [u8; 32] = rand::random();
            entry.set_secret(&key)
                .map_err(|error| anyhow!("Unable to save the key of the secrets in the {}: {error}", store_name()))?;
            key
        },
        Err(keyring::Error::NoEntry) => return Err(anyhow!(
            "The settings have encrypted secrets, but there is no key for them in the {}, e.g. as they were made on \
             another computer", store_name())),
        Err(error) => return Err(anyhow!("Unable to read the key of the secrets from the {}: {error}", store_name())),
    };
    *cached = Some(key);
    Ok(key)
}

/// Uses key instead of the credential store's, so that tests neither need nor change it.
#[cfg(test)]
pub fn use_key_for_tests(key: [u8; 32]) {
    *KEY.lock().unwrap() = Some(key);
}

/// Whether the field holds a secret, which is encrypted when saved: those which sparko_graphql redacts from logs, e.g.
/// apiKey, password or a token, a login's email, and a capture device's key.
fn is_secret_field(name: &str) -> bool {
    sparko_graphql::redact::is_secret_name(name) || name == "email" || name == "key"
}

/// Whether the text is an encrypted value.
pub fn is_encrypted(text: &str) -> bool {
    text.starts_with(PREFIX)
}

/// The text, encrypted.
fn encrypt(text: &str) -> anyhow::Result<String> {
    let cipher = XChaCha20Poly1305::new(&key(true)?.into());
    let nonce: [u8; NONCE_LEN] = rand::random();
    let ciphertext = cipher.encrypt(&XNonce::from(nonce), text.as_bytes())
        .map_err(|_| anyhow!("Unable to encrypt a secret"))?;
    let mut bytes = nonce.to_vec();
    bytes.extend(ciphertext);
    Ok(format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// The text of an encrypted value, see is_encrypted.
fn decrypt(value: &str) -> anyhow::Result<String> {
    let encoded = value.strip_prefix(PREFIX).ok_or_else(|| anyhow!("The value is not encrypted"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)
        .map_err(|error| anyhow!("An encrypted secret is not base64: {error}"))?;
    if bytes.len() < NONCE_LEN {
        return Err(anyhow!("An encrypted secret is too short"));
    }
    let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into()?;
    let cipher = XChaCha20Poly1305::new(&key(false)?.into());
    let text = cipher.decrypt(&XNonce::from(nonce), ciphertext)
        .map_err(|_| anyhow!("Unable to decrypt a secret, the key in the {} is not the one it was encrypted with", store_name()))?;
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
        },
        Value::Array(items) => {
            for item in items {
                encrypt_secrets(item)?;
            }
        },
        _ => {},
    }
    Ok(())
}

/// Decrypts every encrypted value, at any depth, whichever field it is in.
pub fn decrypt_secrets(value: &mut Value) -> anyhow::Result<()> {
    match value {
        Value::String(text) if is_encrypted(text) => *text = decrypt(text)?,
        Value::Object(map) => {
            for field in map.values_mut() {
                decrypt_secrets(field)?;
            }
        },
        Value::Array(items) => {
            for item in items {
                decrypt_secrets(item)?;
            }
        },
        _ => {},
    }
    Ok(())
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
        assert!(is_encrypted(value[0]["settings"]["captureDevices"][0]["key"].as_str().unwrap()));
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
    fn a_tampered_value_is_an_error() {
        use_key_for_tests([7; 32]);
        let mut value = encrypt("secret").unwrap();
        value.pop();
        value.push(if value.ends_with('A') { 'B' } else { 'A' });
        assert!(decrypt(&value).is_err());
    }
}
