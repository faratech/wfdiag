use crate::{
    API_BASE, AUTHORIZE_URL, AuthorizationUrl, ChatGptPorts, DISCOVERY_URL, SCOPES, Secret,
    TOKEN_URL,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
pub(crate) struct Identity {
    pub sub: String,
    pub email: Option<String>,
    pub nonce: String,
    aud: Value,
    #[serde(default)]
    azp: Option<String>,
}
pub(crate) struct Authorization {
    pub client_id: String,
    pub identity: Identity,
    pub tokens: Value,
}
pub(crate) struct AuthorizationCode {
    pub client_id: String,
    code: Secret,
    verifier: Secret,
    nonce: String,
    redirect: String,
}
fn random() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Secure random generation is unavailable")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn callback(
    target: &str,
    expected_state: &str,
    saved_client: Option<&str>,
) -> Result<(String, String), String> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid authorization callback")?;
    if url.path() != "/auth/callback" {
        return Err("Invalid authorization callback path".into());
    }
    let mut values = HashMap::new();
    for (key, value) in url.query_pairs() {
        if values
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("Duplicate authorization callback fields".into());
        }
    }
    if values.get("state").map(String::as_str) != Some(expected_state) {
        return Err("Authorization callback state did not match".into());
    }
    if values.contains_key("error") {
        return Err("ChatGPT authorization was declined or interrupted".into());
    }
    let client = if let Some(saved) = saved_client {
        if values.get("client_id").is_some_and(|c| c != saved) {
            return Err("Authorization returned a different registration".into());
        }
        saved.to_owned()
    } else {
        values
            .get("client_id")
            .filter(|c| !c.is_empty() && c.as_str() != "dynamic_agent_client")
            .cloned()
            .ok_or("ChatGPT registration was incomplete")?
    };
    let code = values
        .get("code")
        .filter(|c| !c.is_empty())
        .cloned()
        .ok_or("Authorization code was missing")?;
    Ok((client, code))
}

// Listener lifetime and PKCE values must be owned by the same authorization attempt.
#[allow(clippy::too_many_lines)]
pub(crate) async fn authorize(
    ports: &ChatGptPorts,
    host: &str,
    saved: Option<(&str, &str, Option<&Secret>)>,
    consent: bool,
    cancel: &CancellationToken,
) -> Result<AuthorizationCode, String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Could not start the browser callback listener")?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener
            .local_addr()
            .map_err(|_| "Callback listener unavailable")?
            .port()
    );
    let state = random()?;
    let nonce = random()?;
    let verifier = Secret::new(random()?);
    let mut url = url::Url::parse(AUTHORIZE_URL).map_err(|_| "Invalid authorization endpoint")?;
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            ("client_id", saved.map_or("dynamic_agent_client", |a| a.0)),
            ("ext_agent_host_id", host),
            ("response_type", "code"),
            ("redirect_uri", &redirect),
            ("scope", SCOPES),
            ("resource", API_BASE),
            ("state", &state),
            ("nonce", &nonce),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes())),
            ),
        ]);
        if let Some((_, email, hint)) = saved {
            if !email.is_empty() {
                query.append_pair("login_hint", email);
            }
            if let Some(hint) = hint {
                query.append_pair("id_token_hint", hint.expose());
            }
        } else {
            query.append_pair("agent_name_hint", "WindowsForum Diagnostics");
        }
        if consent {
            query.append_pair("prompt", "consent");
        }
    }
    ports
        .browser
        .open(&AuthorizationUrl::new(url)?)
        .map_err(|_| "Could not open the browser for ChatGPT sign-in")?;
    let receive = async {
        loop {
            let (mut stream, peer) = listener
                .accept()
                .await
                .map_err(|_| "Callback listener failed")?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let mut request = Vec::new();
            let read = async {
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let n = stream
                        .read(&mut chunk)
                        .await
                        .map_err(|_| "Callback read failed")?;
                    if n == 0 || request.len() + n > 16_384 {
                        return Err("Invalid callback request");
                    }
                    request.extend_from_slice(&chunk[..n]);
                }
                Ok(())
            };
            if tokio::time::timeout(std::time::Duration::from_secs(3), read).await != Ok(Ok(())) {
                continue;
            }
            let Ok(request_text) = std::str::from_utf8(&request) else {
                continue;
            };
            let line = request_text.lines().next().unwrap_or_default();
            let mut request_fields = line.split_whitespace();
            if request_fields.next() != Some("GET") {
                continue;
            }
            let target = request_fields.next().unwrap_or_default();
            let result = callback(target, &state, saved.map(|a| a.0));
            let body = if result.is_ok() {
                "ChatGPT authorization received. You can return to WindowsForum Diagnostics."
            } else {
                "Authorization callback was not accepted. Return to WindowsForum Diagnostics."
            };
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                if result.is_ok() {
                    "200 OK"
                } else {
                    "400 Bad Request"
                },
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            // Ignore unrelated/forged callbacks; consume a correctly bound denial.
            if result.is_err() {
                let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).ok();
                if !parsed.is_some_and(|u| u.query_pairs().any(|(k, v)| k == "state" && v == state))
                {
                    continue;
                }
            }
            return result;
        }
    };
    let (client_id, code) = tokio::select! {
        () = cancel.cancelled() => return Err("ChatGPT sign-in cancelled".into()),
        result = tokio::time::timeout(std::time::Duration::from_secs(600), receive) => result.map_err(|_| "ChatGPT sign-in timed out")??,
    };
    drop(listener);
    Ok(AuthorizationCode {
        client_id,
        code: Secret::new(code),
        verifier,
        nonce,
        redirect,
    })
}

pub(crate) async fn exchange(
    ports: &ChatGptPorts,
    authorization: AuthorizationCode,
) -> Result<Authorization, String> {
    let AuthorizationCode {
        client_id,
        code,
        verifier,
        nonce,
        redirect,
    } = authorization;
    let tokens = ports
        .http
        .post_form(
            TOKEN_URL,
            vec![
                ("grant_type", "authorization_code".into()),
                ("client_id", client_id.clone()),
                ("code", code.expose().into()),
                ("code_verifier", verifier.expose().into()),
                ("redirect_uri", redirect),
                ("resource", API_BASE.into()),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
    let identity = validate_identity(ports, &tokens, &client_id, &nonce).await?;
    Ok(Authorization {
        client_id,
        identity,
        tokens,
    })
}

pub(crate) async fn validate_identity(
    ports: &ChatGptPorts,
    tokens: &Value,
    client: &str,
    nonce: &str,
) -> Result<Identity, String> {
    let token = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .ok_or("ChatGPT returned no identity token")?;
    let discovery = ports
        .http
        .get(DISCOVERY_URL, None)
        .await
        .map_err(|e| e.to_string())?;
    let jwks_url = discovery
        .get("jwks_uri")
        .and_then(Value::as_str)
        .ok_or("Identity key discovery failed")?;
    let url = url::Url::parse(jwks_url).map_err(|_| "Invalid identity key endpoint")?;
    if discovery.get("issuer").and_then(Value::as_str) != Some("https://auth.openai.com")
        || url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Invalid identity discovery response".into());
    }
    let json = ports
        .http
        .get(jwks_url, None)
        .await
        .map_err(|e| e.to_string())?;
    let keys: JwkSet = serde_json::from_value(json).map_err(|_| "Invalid identity keys")?;
    let header = decode_header(token).map_err(|_| "Invalid identity token")?;
    if header.alg != Algorithm::RS256 {
        return Err("Unsupported identity signature".into());
    }
    let key = keys
        .find(header.kid.as_deref().ok_or("Identity key ID is missing")?)
        .ok_or("Identity signing key was not found")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.validate_nbf = true;
    validation.leeway = 15;
    validation.set_audience(&[client]);
    validation.set_issuer(&["https://auth.openai.com"]);
    let identity = decode::<Identity>(
        token,
        &DecodingKey::from_jwk(key).map_err(|_| "Invalid identity key")?,
        &validation,
    )
    .map_err(|_| "ChatGPT identity validation failed")?
    .claims;
    if identity.nonce != nonce
        || identity.sub.is_empty()
        || identity.azp.as_ref().is_some_and(|azp| azp != client)
        || (identity.aud.as_array().is_some_and(|aud| aud.len() > 1)
            && identity.azp.as_deref() != Some(client))
    {
        return Err("ChatGPT identity did not match this sign-in".into());
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_forgery_and_registration_mixing() {
        assert!(callback("/auth/callback?state=wrong&code=a&client_id=b", "s", None).is_err());
        assert!(
            callback(
                "/auth/callback?state=s&code=a&client_id=b",
                "s",
                Some("old")
            )
            .is_err()
        );
        assert!(
            callback(
                "/auth/callback?state=s&state=s&code=a&client_id=b",
                "s",
                None
            )
            .is_err()
        );
        assert!(callback("/auth/callback?state=s&code=a", "s", None).is_err());
        assert!(callback("/auth/callback?state=s&error=access_denied", "s", None).is_err());
        assert_eq!(
            callback("/auth/callback?state=s&code=a", "s", Some("old")).unwrap(),
            ("old".into(), "a".into())
        );
    }
    #[test]
    fn authorization_url_is_closed_and_redacted() {
        let url = AuthorizationUrl::new(
            url::Url::parse(&format!("{AUTHORIZE_URL}?id_token_hint=secret")).unwrap(),
        )
        .unwrap();
        assert!(!format!("{url:?}").contains("secret"));
        assert!(
            AuthorizationUrl::new(url::Url::parse("https://attacker.example/authorize").unwrap())
                .is_err()
        );
    }
}
