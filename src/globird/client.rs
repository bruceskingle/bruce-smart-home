//! Requests to the GloBird Energy customer portal, https://myaccount.globirdenergy.com.au, whose API is the one its own
//! web page uses, which GloBird does not document. What is known of it is from the Home Assistant integrations
//! bolagnaise/globird-ha and davidar/globird-stats, and the replies in testdata/globird.
//!
//! Every reply is an envelope, {"data": ..., "message": ..., "success": true}. The session is kept in cookies, which
//! are saved in the module's cache file, see GlobirdClient::session_token, so that the app logs in as rarely as it can: a login is
//! made only when there is no session or the portal no longer accepts it, and a login which fails is not tried again
//! until the user logs in on the login page, so that a wrong password or a captcha cannot lock the account.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::anyhow;
use base64::Engine;
use reqwest::{Method, StatusCode};
use serde::Deserialize;

const BASE: &str = "https://myaccount.globirdenergy.com.au";

/// The envelope of every reply.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub data: Option<serde_json::Value>,
    pub message: Option<String>,
    pub success: Option<bool>,
}

/// The reply to a login.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginData {
    is_login_succeeded: Option<bool>,
    require_retry_capt_cha: Option<bool>,
    #[serde(rename = "requireHCaptcha")]
    require_h_captcha: Option<bool>,
}

pub struct GlobirdClient {
    http: reqwest::Client,
    email: String,
    password: String,
    /// The cookies the portal has set, by name, which hold the session.
    cookies: Mutex<BTreeMap<String, String>>,
    /// Whether a login has failed, after which no other is tried, see login.
    login_failed: AtomicBool,
    /// Whether the session has changed since it was last saved, see take_new_session.
    session_changed: AtomicBool,
    verbose: bool,
}

impl GlobirdClient {
    /// session_token is the session saved before, see session_token, if there is one.
    pub fn new(
        email: String,
        password: String,
        session_token: Option<&str>,
        verbose: bool,
    ) -> anyhow::Result<GlobirdClient> {
        Ok(GlobirdClient {
            http: crate::http::client_builder().build()?,
            email,
            password,
            cookies: Mutex::new(session_token.map(parse_cookie_header).unwrap_or_default()),
            login_failed: AtomicBool::new(false),
            session_changed: AtomicBool::new(false),
            verbose,
        })
    }

    /// The session's cookies as a Cookie header, to save.
    pub fn session_token(&self) -> String {
        self.cookies
            .lock()
            .unwrap()
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The session, if it has changed since this was last called, e.g. after a login, to save.
    pub fn take_new_session(&self) -> Option<String> {
        self.session_changed
            .swap(false, Ordering::SeqCst)
            .then(|| self.session_token())
    }

    /// One request, see send_bytes, whose reply is text.
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<(StatusCode, String)> {
        let (status, bytes) = self.send_bytes(method, path, body).await?;
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// One request, sending the session's cookies and keeping those the portal sets. Every cookie is sent whatever its
    /// domain, as the portal's sticky Azure ARRAffinity cookies are set for its backend's host rather than this one.
    async fn send_bytes(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<(StatusCode, Vec<u8>)> {
        crate::log_info!("GloBird {method} {path}");
        let mut request = self
            .http
            .request(method, format!("{BASE}{path}"))
            .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
            .header(reqwest::header::ORIGIN, BASE)
            .header(reqwest::header::REFERER, format!("{BASE}/"));
        let cookie = self.session_token();
        if !cookie.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let mut changed = false;
        {
            let mut cookies = self.cookies.lock().unwrap();
            for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Some((name, value)) = header.to_str().ok().and_then(parse_set_cookie) {
                    if cookies.get(&name) != Some(&value) {
                        cookies.insert(name, value);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.session_changed.store(true, Ordering::SeqCst);
        }
        Ok((status, response.bytes().await?.to_vec()))
    }

    /// A request's reply, logging in first if there is no session, and once more if the portal no longer accepts the
    /// session.
    async fn call_bytes(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<(StatusCode, Vec<u8>)> {
        if self.cookies.lock().unwrap().is_empty() {
            self.login().await?;
        }
        let (status, bytes) = self.send_bytes(method.clone(), path, body.as_ref()).await?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            crate::log_info!("The GloBird session has expired, logging in again");
            self.cookies.lock().unwrap().clear();
            self.login().await?;
            return self.send_bytes(method, path, body.as_ref()).await;
        }
        Ok((status, bytes))
    }

    /// The data of a request's reply, see call_bytes.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let (status, bytes) = self.call_bytes(method, path, body).await?;
        let data = envelope_data(status, &String::from_utf8_lossy(&bytes), path)?;
        crate::http::print_response(self.verbose, &format!("GloBird {path}"), &data);
        Ok(data)
    }

    /// Logs in as the portal's web page does, with the password encrypted to the portal's public key. Stops at a
    /// captcha or a refusal, without trying again, and no other login is tried by this client after one has failed.
    pub async fn login(&self) -> anyhow::Result<()> {
        if self.login_failed.load(Ordering::SeqCst) {
            return Err(anyhow!(
                "An earlier login to GloBird failed, so the app will not try again until you log in on the GloBird page"
            ));
        }
        let result = self.try_login().await;
        if result.is_err() {
            self.login_failed.store(true, Ordering::SeqCst);
        }
        result
    }

    async fn try_login(&self) -> anyhow::Result<()> {
        let (status, text) = self.send(Method::GET, "/api/account/publicjwk", None).await?;
        let jwk: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            anyhow!(
                "GloBird's public key is not JSON, {error}: {}",
                crate::http::summary(&text)
            )
        })?;
        if !status.is_success() {
            return Err(anyhow!("GloBird responded {status} when asked for its public key"));
        }
        let password = encrypt_password(&jwk, &self.password)?;
        let body = serde_json::json!({ "emailAddress": self.email, "password": password, "rememberMe": false });
        let (status, text) = self.send(Method::POST, "/api/account/login", Some(&body)).await?;
        let envelope: Envelope = serde_json::from_str(&text).map_err(|error| {
            anyhow!(
                "GloBird's reply to the login is not JSON, {error}: {}",
                crate::http::summary(&text)
            )
        })?;
        let login: LoginData = envelope
            .data
            .clone()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();
        if login.require_retry_capt_cha == Some(true) || login.require_h_captcha == Some(true) {
            return Err(anyhow!(
                "GloBird asked for a captcha. Log in once on the GloBird web site, then try again later"
            ));
        }
        if !status.is_success() || envelope.success != Some(true) || login.is_login_succeeded == Some(false) {
            return Err(anyhow!(
                "GloBird refused the login ({status}){}",
                envelope
                    .message
                    .map(|message| format!(", {message}"))
                    .unwrap_or_default()
            ));
        }
        crate::log_info!("Logged in to GloBird");
        Ok(())
    }

    /// The user's accounts and their services, power and gas, each with its accountServiceId and siteIdentifier, the
    /// NMI of a power service.
    pub async fn current_user(&self) -> anyhow::Result<serde_json::Value> {
        self.call(Method::GET, "/api/account/currentuser", None).await
    }

    /// The meters of a service, e.g. an energised smart meter and one removed.
    pub async fn read_meters(&self, account_service_id: i64) -> anyhow::Result<serde_json::Value> {
        self.call(
            Method::GET,
            &format!("/api/site/readmeters?accountServiceId={account_service_id}"),
            None,
        )
        .await
    }

    /// The smart meter's 5 minute readings of each NEM day from from to to, see super::fetch::UsageRow.
    pub async fn usage(
        &self,
        account_service_id: i64,
        nmi: &str,
        serial: &str,
        from: time::Date,
        to: time::Date,
    ) -> anyhow::Result<serde_json::Value> {
        let format = time::macros::format_description!("[year]/[month]/[day]");
        self.call(Method::POST, &format!("/api/site/accountservicetimezonesmartmeterread?accountServiceId={account_service_id}"),
            Some(serde_json::json!({
                "identifier": nmi, "serialNumber": serial, "fromDate": from.format(&format)?, "toDate": to.format(&format)?,
                "isSmart": true, "isAcrossAccount": false,
            }))).await
    }

    /// A page of the account's correspondence, its emails and letters, newest first, from offset, see
    /// super::bills::ListItem.
    pub async fn correspondence_list(
        &self,
        account_id: i64,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<serde_json::Value> {
        self.call(
            Method::POST,
            &format!("/api/correspondence/getcorrespondencelist?accountId={account_id}"),
            Some(serde_json::json!({ "offset": offset, "limit": limit })),
        )
        .await
    }

    /// An email of the correspondence, with the documents attached to it, see super::bills::Detail.
    pub async fn correspondence_detail(&self, account_id: i64, email_id: i64) -> anyhow::Result<serde_json::Value> {
        self.call(
            Method::GET,
            &format!("/api/correspondence/detail?accountId={account_id}&emailId={email_id}"),
            None,
        )
        .await
    }

    /// A document of the correspondence, e.g. a bill, a PDF, which is sent as it is, not in an envelope.
    pub async fn document(&self, account_id: i64, document_id: i64) -> anyhow::Result<Vec<u8>> {
        let path = format!("/api/correspondence/document?accountId={account_id}&documentId={document_id}");
        let (status, bytes) = self.call_bytes(Method::GET, &path, None).await?;
        if !status.is_success() || !bytes.starts_with(b"%PDF") {
            return Err(anyhow!(
                "GloBird responded {status} to {path} with something which is not a PDF: {}",
                crate::http::summary(&String::from_utf8_lossy(&bytes[..bytes.len().min(500)]))
            ));
        }
        Ok(bytes)
    }

    /// The cost of each NEM day from from to to by category, see super::fetch::CostRow.
    pub async fn cost(
        &self,
        account_service_id: i64,
        nmi: &str,
        from: time::Date,
        to: time::Date,
    ) -> anyhow::Result<serde_json::Value> {
        let format = time::macros::format_description!("[year]-[month]-[day]");
        self.call(
            Method::POST,
            "/api/transaction/CostDetail",
            Some(serde_json::json!({
                "accountServiceId": account_service_id, "identifier": nmi, "from": from.format(&format)?,
                "to": to.format(&format)?, "isSmart": true,
            })),
        )
        .await
    }
}

/// The data of a reply, or an error saying why there is none, e.g. the envelope's message.
fn envelope_data(status: StatusCode, text: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let envelope: Envelope = serde_json::from_str(text).map_err(|error| {
        anyhow!(
            "GloBird responded {status} to {path} with something which is not JSON, {error}: {}",
            crate::http::summary(text)
        )
    })?;
    if !status.is_success() || envelope.success != Some(true) {
        return Err(anyhow!(
            "GloBird responded {status} to {path}{}",
            envelope
                .message
                .map(|message| format!(", {message}"))
                .unwrap_or_default()
        ));
    }
    Ok(envelope.data.unwrap_or(serde_json::Value::Null))
}

/// The password encrypted with RSA-OAEP-SHA256 to the public key jwk, a JSON web key {"n": ..., "e": ...}, which may be
/// in an envelope's data, in base64, as the portal's login expects.
fn encrypt_password(jwk: &serde_json::Value, password: &str) -> anyhow::Result<String> {
    let jwk = if jwk.get("n").is_some() {
        jwk
    } else {
        jwk.get("data").unwrap_or(jwk)
    };
    let number = |name: &str| -> anyhow::Result<rsa::BigUint> {
        let text = jwk
            .get(name)
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("GloBird's public key has no {name}"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text.trim_end_matches('='))?;
        Ok(rsa::BigUint::from_bytes_be(&bytes))
    };
    let key = rsa::RsaPublicKey::new(number("n")?, number("e")?)?;
    let encrypted = key.encrypt(&mut rand_core(), rsa::Oaep::new::<sha2::Sha256>(), password.as_bytes())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

/// The random numbers for the encryption's padding, from the operating system.
fn rand_core() -> rsa::rand_core::OsRng {
    rsa::rand_core::OsRng
}

/// The name and value of a Set-Cookie header, without its attributes, e.g. ("session", "abc") from
/// "session=abc; path=/; HttpOnly".
fn parse_set_cookie(header: &str) -> Option<(String, String)> {
    let (name, rest) = header.split_once('=')?;
    let value = rest.split(';').next().unwrap_or_default();
    let name = name.trim();
    (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
}

/// The cookies in a Cookie header, e.g. "a=1; b=2", as saved by GlobirdClient::session_token.
fn parse_cookie_header(header: &str) -> BTreeMap<String, String> {
    header
        .split(';')
        .filter_map(|cookie| {
            let (name, value) = cookie.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::PublicKeyParts;

    #[test]
    fn the_password_is_encrypted_so_that_the_private_key_decrypts_it() {
        let private = rsa::RsaPrivateKey::new(&mut rand_core(), 1024).unwrap();
        let encode =
            |number: &rsa::BigUint| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(number.to_bytes_be());
        let jwk = serde_json::json!({"data": {"kty": "RSA", "n": encode(private.n()), "e": encode(private.e())}, "success": true});
        let encrypted = base64::engine::general_purpose::STANDARD
            .decode(encrypt_password(&jwk, "a password").unwrap())
            .unwrap();
        let decrypted = private.decrypt(rsa::Oaep::new::<sha2::Sha256>(), &encrypted).unwrap();
        assert_eq!(decrypted, b"a password");
    }

    #[test]
    fn cookies_are_kept_without_their_attributes_and_read_back() {
        assert_eq!(
            parse_set_cookie("ARRAffinity=abc123; Path=/; HttpOnly; Domain=x.azurewebsites.net"),
            Some(("ARRAffinity".to_string(), "abc123".to_string()))
        );
        assert_eq!(parse_set_cookie("=nothing"), None);
        let cookies = parse_cookie_header("a=1; b=x=y");
        assert_eq!(cookies.get("a").map(String::as_str), Some("1"));
        assert_eq!(cookies.get("b").map(String::as_str), Some("x=y"));
    }

    #[test]
    fn an_envelope_which_is_not_a_success_is_an_error_with_its_message() {
        let error = envelope_data(
            StatusCode::OK,
            r#"{"data": null, "message": "No access", "success": false}"#,
            "/x",
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("No access"));
        let data = envelope_data(
            StatusCode::OK,
            r#"{"data": {"a": 1}, "message": null, "success": true}"#,
            "/x",
        )
        .unwrap();
        assert_eq!(data, serde_json::json!({"a": 1}));
    }
}
