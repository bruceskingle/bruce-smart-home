//! HTTP digest authentication (RFC 7616, MD5 with qop auth), which the myenergi API uses.

use md5::{Digest, Md5};

/// What the server sent in its WWW-Authenticate header to ask for digest authentication.
#[derive(Debug, Clone, PartialEq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub qop: Option<String>,
}

/// The challenge in a WWW-Authenticate header, e.g. Digest realm="MyEnergi Telemetry", nonce="...", qop="auth", or None
/// if it is not for digest authentication or has no nonce.
pub fn parse_challenge(header: &str) -> Option<Challenge> {
    let params = header.trim().strip_prefix("Digest")?;
    let mut realm = None;
    let mut nonce = None;
    let mut opaque = None;
    let mut qop = None;
    for (name, value) in parse_params(params) {
        match name.to_ascii_lowercase().as_str() {
            "realm" => realm = Some(value),
            "nonce" => nonce = Some(value),
            "opaque" => opaque = Some(value),
            "qop" => qop = Some(value),
            _ => {}
        }
    }
    Some(Challenge {
        realm: realm.unwrap_or_default(),
        nonce: nonce?,
        opaque,
        qop,
    })
}

/// The name=value pairs of a header, separated by commas, where values may be quoted and contain commas.
fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        let name: String = std::iter::from_fn(|| chars.next_if(|c| *c != '=')).collect();
        if chars.next().is_none() {
            break;
        }
        let value: String = if chars.next_if_eq(&'"').is_some() {
            let value = std::iter::from_fn(|| chars.next_if(|c| *c != '"')).collect();
            chars.next();
            value
        } else {
            std::iter::from_fn(|| chars.next_if(|c| *c != ',')).collect()
        };
        params.push((name.trim().to_string(), value.trim().to_string()));
    }
    params
}

fn md5_hex(text: &str) -> String {
    format!("{:x}", Md5::digest(text.as_bytes()))
}

/// The Authorization header answering challenge for a request with method to uri, the path and query of its URL.
/// nc counts the requests made with the challenge's nonce, from 1, and cnonce is a random string chosen by the client.
pub fn authorization(
    challenge: &Challenge,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let ha1 = md5_hex(&format!("{username}:{}:{password}", challenge.realm));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    // Only auth is supported, which is all that servers which offer a choice are known to need.
    let qop = challenge.qop.as_deref().map(|qop| {
        qop.split(',')
            .map(str::trim)
            .find(|qop| *qop == "auth")
            .unwrap_or("auth")
    });
    let mut header = format!(
        "Digest username=\"{username}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\"",
        challenge.realm, challenge.nonce
    );
    match qop {
        Some(qop) => {
            let response = md5_hex(&format!("{ha1}:{}:{nc:08x}:{cnonce}:{qop}:{ha2}", challenge.nonce));
            header.push_str(&format!(
                ", qop={qop}, nc={nc:08x}, cnonce=\"{cnonce}\", response=\"{response}\""
            ));
        }
        None => {
            let response = md5_hex(&format!("{ha1}:{}:{ha2}", challenge.nonce));
            header.push_str(&format!(", response=\"{response}\""));
        }
    }
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    header.push_str(", algorithm=MD5");
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_challenge_is_parsed_with_quoted_values_containing_commas() {
        let challenge =
            parse_challenge(r#"Digest realm="MyEnergi Telemetry", qop="auth,auth-int", nonce="abc,def", opaque="xyz""#);
        assert_eq!(
            challenge,
            Some(Challenge {
                realm: "MyEnergi Telemetry".to_string(),
                nonce: "abc,def".to_string(),
                opaque: Some("xyz".to_string()),
                qop: Some("auth,auth-int".to_string()),
            })
        );
        assert_eq!(parse_challenge("Basic realm=\"x\""), None);
    }

    /// The example in RFC 2617 section 3.5.
    #[test]
    fn the_response_is_the_rfc_example() {
        let challenge = Challenge {
            realm: "testrealm@host.com".to_string(),
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string(),
            opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".to_string()),
            qop: Some("auth,auth-int".to_string()),
        };
        let header = authorization(
            &challenge,
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            1,
            "0a4f113b",
        );
        assert!(
            header.contains("response=\"6629fae49393a05397450978507c4ef1\""),
            "{header}"
        );
        assert!(header.contains("nc=00000001"), "{header}");
    }
}
