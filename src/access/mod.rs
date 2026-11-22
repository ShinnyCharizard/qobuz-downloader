//! Account lifecycle and settings, owned entirely by the plugin.
mod credentials;
mod oauth;
mod settings;

use self::credentials::Credentials;
use crate::files;
use qobuz_api::QobuzClient;
use qobuz_models::UserSession;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::sync::Mutex;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    credentials: Credentials,
    session: Option<UserSession>,
}
#[derive(Default)]
struct State {
    saved: Saved,
    pending: Option<oauth::Pending>,
    shown: [bool; 3],
    revision: u64,
    notice: Option<String>,
}
pub struct Account {
    path: PathBuf,
    state: Mutex<State>,
    http: reqwest::Client,
}
impl Account {
    pub fn new(data: &std::path::Path) -> Self {
        Self {
            path: data.join("account.json"),
            state: Mutex::new(State::default()),
            http: reqwest::Client::builder()
                .user_agent("Qobuz-Downloader/1.0")
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(25))
                .build()
                .expect("HTTP client"),
        }
    }
    pub async fn restore(&self, client: &QobuzClient) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let saved = match files::read::<Saved>(&self.path) {
            Ok(Some(saved)) => saved,
            Ok(None) => return Ok(()),
            Err(error) => {
                state.notice = Some(error);
                return Ok(());
            }
        };
        state.saved = saved;
        if state.saved.credentials.app_id.is_empty() && state.saved.session.is_none() {
            return Ok(());
        }
        if let Err(error) = state.saved.credentials.validate() {
            state.saved.session = None;
            state.notice = Some(error);
            return Ok(());
        }
        client
            .configure(
                state.saved.credentials.app_id.clone(),
                state.saved.credentials.app_secret.clone(),
            )
            .await
            .map_err(|_| "Invalid saved credentials")?;
        if let Some(session) = &state.saved.session {
            if session.user_id == 0
                || session.user_auth_token.is_empty()
                || reqwest::header::HeaderValue::from_str(&session.user_auth_token).is_err()
            {
                state.saved.session = None;
                state.notice = Some("The saved session is invalid. Sign in again.".into());
            } else {
                client.set_session(session.clone()).await;
            }
        }
        Ok(())
    }
    async fn complete(&self, state: &mut State, client: &QobuzClient) {
        if !state.pending.as_ref().is_some_and(|p| p.finished()) {
            return;
        }
        let mut pending = state.pending.take().unwrap();
        match pending.finish().await {
            Ok(session) => {
                state.saved.session = Some(session.clone());
                match files::save(&self.path, &state.saved) {
                    Ok(()) => {
                        client.set_session(session).await;
                        state.notice = None;
                    }
                    Err(error) => {
                        state.saved.session = None;
                        state.notice = Some(error);
                    }
                }
            }
            Err(error) => state.notice = Some(error),
        }
        state.shown = [false; 3];
    }
    pub async fn status(&self, client: &QobuzClient) -> Value {
        let mut state = self.state.lock().await;
        self.complete(&mut state, client).await;
        json!({"signed_in": state.saved.session.is_some(), "user": state.saved.session.as_ref().map(|s| &s.display_name),
            "pending": state.pending.is_some(), "notice": state.notice})
    }
    pub async fn logout(&self, client: &QobuzClient) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.pending = None;
        state.saved.session = None;
        state.shown = [false; 3];
        client.logout().await;
        files::save(&self.path, &state.saved)
    }
    pub async fn open(&self, client: &QobuzClient, language: &str) -> Value {
        let mut state = self.state.lock().await;
        self.complete(&mut state, client).await;
        state.shown = [false; 3];
        Self::document(&mut state, language)
    }
    fn document(state: &mut State, language: &str) -> Value {
        state.revision += 1;
        settings::document(
            language,
            state.revision,
            &state.saved.credentials,
            state.shown,
            state
                .saved
                .session
                .as_ref()
                .map(|s| s.display_name.as_str()),
            state.pending.as_ref().map(|p| p.url.as_str()),
            state.notice.as_deref(),
        )
    }
    pub async fn submit(&self, client: &QobuzClient, language: &str, params: Value) -> Value {
        let mut state = self.state.lock().await;
        // The host already echoes a revision; reject a stale visibility/login action.
        if params.get("revision").and_then(Value::as_u64) != Some(state.revision) {
            state.shown = [false; 3];
            state.notice = Some("This settings page changed. Try again.".into());
            return Self::document(&mut state, language);
        }
        state.notice = None;
        self.complete(&mut state, client).await;
        let action = params.get("action").and_then(Value::as_str).unwrap_or("");
        let result = self
            .act(&mut state, client, action, &params["values"])
            .await;
        if let Err(error) = result {
            state.notice = Some(error);
            state.shown = [false; 3];
        }
        Self::document(&mut state, language)
    }
    async fn act(
        &self,
        state: &mut State,
        client: &QobuzClient,
        action: &str,
        values: &Value,
    ) -> Result<(), String> {
        if action == "sign_out" {
            state.pending = None;
            state.saved.session = None;
            state.shown = [false; 3];
            client.logout().await;
            return files::save(&self.path, &state.saved);
        }
        if action == "check_sign_in" {
            return Ok(());
        }
        if action == "cancel_sign_in" {
            state.pending = None;
            state.shown = [false; 3];
            return Ok(());
        }
        if state.saved.session.is_some() || state.pending.is_some() {
            return Err("Cancel or sign out before changing credentials".into());
        }
        for (key, value) in [
            ("app_id", &mut state.saved.credentials.app_id),
            ("app_secret", &mut state.saved.credentials.app_secret),
            ("private_key", &mut state.saved.credentials.private_key),
        ] {
            if let Some(text) = values.get(key).and_then(Value::as_str) {
                if text.len() > 512 {
                    return Err("Credential is too long".into());
                }
                *value = text.trim().to_owned();
            }
        }
        if let Some(index) = action
            .strip_prefix("visibility_")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|i| *i < 3)
        {
            state.shown[index] = !state.shown[index];
            return Ok(());
        }
        state.shown = [false; 3];
        match action {
            "fetch_credentials" => {
                state.saved.credentials = credentials::fetch(&self.http).await?;
                files::save(&self.path, &state.saved)
            }
            "sign_in" => {
                state.saved.credentials.validate()?;
                files::save(&self.path, &state.saved)?;
                client
                    .configure(
                        state.saved.credentials.app_id.clone(),
                        state.saved.credentials.app_secret.clone(),
                    )
                    .await
                    .map_err(|_| "Invalid app credentials")?;
                state.pending =
                    Some(oauth::begin(self.http.clone(), state.saved.credentials.clone()).await?);
                Ok(())
            }
            _ => Err("Unknown account action".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client() -> QobuzClient {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        QobuzClient::new().unwrap()
    }
    fn values() -> Value {
        json!({"app_id":"123456789","app_secret":"0123456789abcdef0123456789abcdef","private_key":"fixturekey"})
    }
    #[tokio::test]
    async fn stale_eye_actions_cannot_reveal_secrets_and_reopening_masks_them() {
        let dir = tempfile::tempdir().unwrap();
        let account = Account::new(dir.path());
        let client = client();
        let page = account.open(&client, "en").await;
        let shown = account
            .submit(
                &client,
                "en",
                json!({"revision":page["revision"],"action":"visibility_1","values":values()}),
            )
            .await;
        assert_eq!(shown["form"]["fields"][1]["control"]["type"], "text");
        let stale = account
            .submit(
                &client,
                "en",
                json!({"revision":page["revision"],"action":"visibility_0","values":values()}),
            )
            .await;
        assert!(stale["form"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["control"]["type"] == "secret"));
        let reopened = account.open(&client, "en").await;
        assert_eq!(reopened["form"]["fields"][1]["control"]["type"], "secret");
        assert_eq!(reopened["values"]["app_secret"], values()["app_secret"]);
    }
    #[tokio::test]
    async fn browser_session_is_saved_restored_without_login_and_erased_on_logout() {
        let _ = client();
        let dir = tempfile::tempdir().unwrap();
        let account = Account::new(dir.path());
        let client = client();
        {
            let mut state = account.state.lock().await;
            state.saved.credentials = serde_json::from_value(values()).unwrap();
            state.pending = Some(oauth::Pending::completed(UserSession {
                user_auth_token: "fixture-user-token".into(),
                user_id: 42,
                display_name: "Fixture".into(),
                ..Default::default()
            }));
        }
        tokio::task::yield_now().await;
        assert_eq!(account.status(&client).await["signed_in"], true);
        let restored = Account::new(dir.path());
        let second = QobuzClient::new().unwrap();
        restored.restore(&second).await.unwrap();
        assert_eq!(second.session().await.unwrap().user_id, 42);
        let page = restored.open(&second, "en").await;
        assert!(!page.to_string().contains("fixture-user-token"));
        restored.logout(&second).await.unwrap();
        assert!(!std::fs::read_to_string(dir.path().join("account.json"))
            .unwrap()
            .contains("fixture-user-token"));
        let third = Account::new(dir.path());
        third.restore(&second).await.unwrap();
        assert!(!second.is_logged_in().await);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[tokio::test]
    async fn invalid_saved_account_does_not_lock_the_settings_page() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("account.json"), b"{broken").unwrap();
        let account = Account::new(dir.path());
        let client = QobuzClient::new().unwrap();
        account.restore(&client).await.unwrap();
        let page = account.open(&client, "en").await;
        assert_eq!(page["form"]["fields"].as_array().unwrap().len(), 3);
        assert!(!page["notices"].as_array().unwrap().is_empty());
        assert!(!client.is_logged_in().await);
    }
}
