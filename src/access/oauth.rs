//! Browser authentication, following Strawberry's Qobuz OAuth exchange.
use super::credentials::Credentials;
use qobuz_models::UserSession;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

pub struct Pending {
    pub url: String,
    task: JoinHandle<Result<UserSession, String>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Pending {
    #[cfg(test)]
    pub fn completed(session: UserSession) -> Self {
        Self {
            url: "https://example.test/authorize".into(),
            task: tokio::spawn(async move { Ok(session) }),
        }
    }
    pub fn finished(&self) -> bool {
        self.task.is_finished()
    }
    pub async fn finish(&mut self) -> Result<UserSession, String> {
        (&mut self.task)
            .await
            .map_err(|_| "Authentication was interrupted")?
    }
}

pub async fn begin(http: reqwest::Client, credentials: Credentials) -> Result<Pending, String> {
    credentials.validate()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Cannot start the authentication callback")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot read callback address")?
        .port();
    let path = format!("/{}", uuid::Uuid::new_v4());
    let redirect = format!("http://127.0.0.1:{port}{path}");
    let mut url = url::Url::parse("https://www.qobuz.com/signin/oauth").unwrap();
    url.query_pairs_mut()
        .append_pair("ext_app_id", &credentials.app_id)
        .append_pair("redirect_url", &redirect);
    let task = tokio::spawn(async move {
        let code = tokio::time::timeout(Duration::from_secs(300), receive(listener, &path))
            .await
            .map_err(|_| "Authentication timed out. Start again.")??;
        exchange(&http, &credentials, &code).await
    });
    Ok(Pending {
        url: url.to_string(),
        task,
    })
}

fn callback_code(request: &str, expected_path: &str) -> Result<String, String> {
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    if parts.next() != Some("GET") {
        return Err("Invalid callback method".into());
    }
    let target = parts.next().ok_or("Missing callback address")?;
    if !target.starts_with('/') || target.starts_with("//") {
        return Err("Invalid callback address".into());
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid callback address")?;
    if url.path() != expected_path {
        return Err("Unexpected callback path".into());
    }
    let codes: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k == "code_autorisation" || k == "code")
        .collect();
    if codes.len() != 1 || codes[0].1.is_empty() || codes[0].1.len() > 4096 {
        return Err("Missing or ambiguous authorization code".into());
    }
    Ok(codes[0].1.to_string())
}

async fn receive(listener: TcpListener, path: &str) -> Result<String, String> {
    loop {
        let (mut stream, address) = listener
            .accept()
            .await
            .map_err(|_| "Cannot receive authentication callback")?;
        if !address.ip().is_loopback() {
            continue;
        }
        let read = async {
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            while bytes.len() < 8192 && !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                let count = stream
                    .read(&mut buffer)
                    .await
                    .map_err(|_| "Cannot read callback")?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            if bytes.len() >= 8192 {
                return Err("Callback is too large".into());
            }
            let text = std::str::from_utf8(&bytes).map_err(|_| "Invalid callback encoding")?;
            callback_code(text, path)
        };
        let result = tokio::time::timeout(Duration::from_secs(2), read).await;
        let code = match result {
            Ok(Ok(code)) => code,
            _ => {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
        };
        let body = "Authorization received. Return to your player and press Check sign-in.";
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        return Ok(code);
    }
}

fn user_id(value: &serde_json::Value) -> Option<u64> {
    [
        value.get("userId"),
        value.get("user_id"),
        value.get("id"),
        value.pointer("/user/id"),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| {
        v.as_u64()
            .or_else(|| v.as_str()?.parse().ok())
            .filter(|id| *id > 0)
    })
}

fn session(value: &serde_json::Value) -> Result<UserSession, String> {
    let token = value
        .get("token")
        .or_else(|| value.get("user_auth_token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("Authentication response has no token")?;
    reqwest::header::HeaderValue::from_str(token)
        .map_err(|_| "Authentication response has an invalid token")?;
    // OAuth can return only the token; resolve identity before saving the session.
    let id = user_id(value).unwrap_or_default();
    Ok(UserSession {
        user_auth_token: token.to_owned(),
        user_id: id,
        display_name: format!("Account {id}"),
        ..Default::default()
    })
}

async fn exchange(
    http: &reqwest::Client,
    credentials: &Credentials,
    code: &str,
) -> Result<UserSession, String> {
    let response = http
        .get("https://www.qobuz.com/api.json/0.2/oauth/callback")
        .header("X-App-Id", &credentials.app_id)
        .query(&[
            ("code", code),
            ("private_key", credentials.private_key.as_str()),
        ])
        .send()
        .await
        .map_err(|_| "Cannot exchange the authorization code")?;
    if !response.status().is_success() {
        return Err(format!(
            "Authentication returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid authentication response")?;
    complete_session(
        http,
        &credentials.app_id,
        &value,
        "https://www.qobuz.com/api.json/0.2/user/get",
    )
    .await
}

async fn complete_session(
    http: &reqwest::Client,
    app_id: &str,
    value: &serde_json::Value,
    profile_url: &str,
) -> Result<UserSession, String> {
    let mut session = session(value)?;
    if session.user_id == 0 {
        let response = http
            .get(profile_url)
            .header("X-App-Id", app_id)
            .header("X-User-Auth-Token", &session.user_auth_token)
            .send()
            .await
            .map_err(|_| "Cannot retrieve the signed-in account. Sign in again.")?;
        if !response.status().is_success() {
            return Err(format!(
                "Account verification returned HTTP {}. Sign in again.",
                response.status().as_u16()
            ));
        }
        let profile: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "Invalid account profile response")?;
        session.user_id = user_id(&profile).ok_or("The account profile has no valid user ID")?;
        session.display_name = format!("Account {}", session.user_id);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_are_bound_to_the_login_attempt() {
        assert_eq!(
            callback_code(
                "GET /nonce?code_autorisation=abc%2B123 HTTP/1.1\r\n",
                "/nonce"
            )
            .unwrap(),
            "abc+123"
        );
        assert!(callback_code("GET /other?code=abc HTTP/1.1", "/nonce").is_err());
        assert!(callback_code("GET /nonce?code=a&code_autorisation=b HTTP/1.1", "/nonce").is_err());
        assert!(callback_code("POST /nonce?code=a HTTP/1.1", "/nonce").is_err());
    }
    #[test]
    fn token_spellings_and_optional_oauth_identity_are_supported() {
        for key in ["token", "user_auth_token"] {
            let s = session(&serde_json::json!({key: "fixture-token", "userId": "42"})).unwrap();
            assert_eq!(s.user_id, 42);
        }
        assert_eq!(
            session(&serde_json::json!({"token":"fixture-token"}))
                .unwrap()
                .user_id,
            0
        );
        assert!(session(&serde_json::json!({"userId":42})).is_err());
        for value in [
            serde_json::json!({"id":42}),
            serde_json::json!({"user_id":"42"}),
            serde_json::json!({"user":{"id":42}}),
        ] {
            assert_eq!(user_id(&value), Some(42));
        }
        for value in [
            serde_json::json!({"id":0}),
            serde_json::json!({"id":-1}),
            serde_json::json!({"id":"invalid"}),
        ] {
            assert_eq!(user_id(&value), None);
        }
    }
}

#[cfg(test)]
mod listener_tests {
    use super::*;
    #[tokio::test]
    async fn token_only_callback_resolves_identity_and_rejects_invalid_profiles() {
        for (status, profile, expected) in [
            (200, r#"{"id":42}"#, Some(42)),
            (200, r#"{"user":{"id":"43"}}"#, Some(43)),
            (200, r#"{"id":0}"#, None),
            (401, r#"{"message":"fixture-private-value"}"#, None),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/user/get", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with("get /user/get "));
                assert!(request.contains("x-app-id: 123"));
                assert!(request.contains("x-user-auth-token: fixture-token"));
                let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{profile}", profile.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result = complete_session(
                &reqwest::Client::new(),
                "123",
                &serde_json::json!({"token":"fixture-token"}),
                &url,
            )
            .await;
            server.await.unwrap();
            match expected {
                Some(id) => {
                    let session = result.unwrap();
                    assert_eq!(session.user_id, id);
                    assert_eq!(session.user_auth_token, "fixture-token");
                }
                None => {
                    let error = result.unwrap_err();
                    assert!(!error.contains("fixture-private-value"));
                }
            }
        }
        let result = complete_session(
            &reqwest::Client::new(),
            "123",
            &serde_json::json!({"token":"fixture-token", "userId":42}),
            "http://127.0.0.1:0",
        )
        .await
        .unwrap();
        assert_eq!(result.user_id, 42);
    }
    #[tokio::test]
    async fn loopback_listener_rejects_unrelated_requests_and_receives_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { receive(listener, "/attempt").await });
        let mut wrong = tokio::net::TcpStream::connect(address).await.unwrap();
        wrong
            .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        wrong.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        let mut right = tokio::net::TcpStream::connect(address).await.unwrap();
        right
            .write_all(
                b"GET /attempt?code_autorisation=fixture-code HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap(), "fixture-code");
    }
}
