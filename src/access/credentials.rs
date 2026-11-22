//! Credential discovery following Strawberry's optional web-player fetcher.
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Credentials {
    pub app_id: String,
    pub app_secret: String,
    pub private_key: String,
}

impl Credentials {
    pub fn validate(&self) -> Result<(), String> {
        if self.app_id.is_empty()
            || self.app_id.len() > 32
            || !self.app_id.bytes().all(|c| c.is_ascii_digit())
        {
            return Err("Enter a valid App ID".into());
        }
        if self.app_secret.len() != 32 || !self.app_secret.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Enter a valid App Secret".into());
        }
        if self.private_key.is_empty()
            || self.private_key.len() > 256
            || !self.private_key.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("Enter a valid Private Key".into());
        }
        Ok(())
    }
}

fn capture(pattern: &str, text: &str) -> Result<String, String> {
    Regex::new(pattern)
        .expect("constant expression")
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|s| s.as_str().to_owned())
        .ok_or_else(|| "The web player credential format has changed".into())
}

pub fn bundle_url(html: &str) -> Result<url::Url, String> {
    let path = capture(
        r#"<script\b[^>]*\bsrc=["'](/resources/[\d.]+-[a-z]\d+/bundle\.js)["']"#,
        html,
    )?;
    url::Url::parse("https://play.qobuz.com")
        .unwrap()
        .join(&path)
        .map_err(|_| "Invalid web player address".into())
}

pub fn extract(bundle: &str) -> Result<Credentials, String> {
    let app_id = capture(r#"production:\{api:\{appId:"(\d+)""#, bundle)?;
    let private_key = capture(r#"privateKey:"([A-Za-z0-9]+)""#, bundle)?;
    let seeds =
        Regex::new(r#"[a-z]\.initialSeed\("([\w=]+)",window\.utimezone\.([a-z]+)\)"#).unwrap();
    let parts = Regex::new(r#"name:"\w+/(\w+)",info:"([\w=]+)",extras:"([\w=]+)""#).unwrap();
    let mut candidates: Vec<_> = seeds.captures_iter(bundle).collect();
    candidates.sort_by_key(|c| match &c[2] {
        "berlin" => 0,
        "london" => 1,
        "abidjan" => 2,
        _ => 3,
    });
    for seed in candidates {
        for part in parts.captures_iter(bundle) {
            if !part[1].eq_ignore_ascii_case(&seed[2]) {
                continue;
            }
            let combined = zeroize::Zeroizing::new(format!("{}{}{}", &seed[1], &part[2], &part[3]));
            let Some(length) = combined.len().checked_sub(44) else {
                continue;
            };
            let Ok(decoded) = STANDARD.decode(&combined[..length]) else {
                continue;
            };
            let Ok(secret) = String::from_utf8(decoded) else {
                continue;
            };
            let credentials = Credentials {
                app_id: app_id.clone(),
                app_secret: secret,
                private_key: private_key.clone(),
            };
            if credentials.validate().is_ok() {
                return Ok(credentials);
            }
        }
    }
    Err("No usable app credentials were found in the web player".into())
}

async fn text(http: &reqwest::Client, url: &str, limit: usize) -> Result<String, String> {
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|_| "Cannot reach the web player")?;
    if !response.status().is_success() {
        return Err(format!(
            "Web player returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cannot read the web player")?
    {
        if bytes.len() + chunk.len() > limit {
            return Err("Web player response is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "Invalid web player response".into())
}

pub async fn fetch(http: &reqwest::Client) -> Result<Credentials, String> {
    let html = text(http, "https://play.qobuz.com/login", 2 * 1024 * 1024).await?;
    let url = bundle_url(&html)?;
    let bundle = zeroize::Zeroizing::new(text(http, url.as_str(), 24 * 1024 * 1024).await?);
    extract(&bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_web_player_resource_is_accepted() {
        assert!(bundle_url(r#"<script src="https://other.example/bundle.js"></script>"#).is_err());
        assert_eq!(
            bundle_url(r#"<script src="/resources/8.2.0-b034/bundle.js"></script>"#)
                .unwrap()
                .host_str(),
            Some("play.qobuz.com")
        );
    }
    #[test]
    fn extraction_uses_matching_timezone_and_rejects_invalid_material() {
        let secret = "0123456789abcdef0123456789abcdef";
        let encoded = STANDARD.encode(secret);
        let bundle = format!(
            r#"production:{{api:{{appId:"123456789"}}}} privateKey:"testkey" a.initialSeed("{}",window.utimezone.berlin) name:"Europe/Berlin",info:"{}",extras:"{}""#,
            &encoded[..20],
            &encoded[20..],
            "x".repeat(44)
        );
        assert_eq!(extract(&bundle).unwrap().app_secret, secret);
        assert!(extract(&bundle.replace("Europe/Berlin", "Europe/Paris")).is_err());
    }
}
