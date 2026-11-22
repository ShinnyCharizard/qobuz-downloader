mod process;

use crate::{access::Account, purchases::Purchases};
pub use process::{run, serve};
use qobuz_api::QobuzClient;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, OnceCell};

pub struct Runtime {
    client: Arc<QobuzClient>,
    account: Account,
    ready: OnceCell<()>,
    downloads: Purchases,
    language: Mutex<String>,
}

impl Runtime {
    pub fn at(data: PathBuf, export: Option<PathBuf>) -> Self {
        let client = Arc::new(QobuzClient::new().expect("service client"));
        Self {
            account: Account::new(&data),
            downloads: Purchases::new(client.clone(), export),
            client,
            ready: OnceCell::new(),
            language: Mutex::new("en".into()),
        }
    }

    async fn ready(&self) -> Result<(), String> {
        self.ready
            .get_or_try_init(|| self.account.restore(&self.client))
            .await
            .map(|_| ())
    }

    pub async fn handle(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "initialize" => {
                if let Some(locale) = params["locale"].as_str() {
                    *self.language.lock().await = locale.into();
                }
                Ok(
                    json!({"protocol":1,"capabilities":{"settings":true,"auth":"browser","downloads":true}}),
                )
            }
            "inspect" => Ok(json!({"settings_pages":["account"]})),
            "auth.status" => {
                self.ready().await?;
                Ok(self.account.status(&self.client).await)
            }
            "auth.logout" => {
                self.ready().await?;
                self.downloads.clear().await;
                self.account.logout(&self.client).await?;
                Ok(json!({"signed_in":false}))
            }
            "settings.open" | "auth.login" => {
                self.ready().await?;
                Ok(self
                    .account
                    .open(&self.client, &self.language.lock().await)
                    .await)
            }
            "settings.submit" => {
                self.ready().await?;
                if params["action"] == "sign_out" {
                    self.downloads.clear().await;
                }
                Ok(self
                    .account
                    .submit(&self.client, &self.language.lock().await, params)
                    .await)
            }
            "downloads.open" => {
                self.ready().await?;
                let signed_in = self.account.status(&self.client).await["signed_in"] == true;
                Ok(self.downloads.open(signed_in).await)
            }
            "downloads.action" => {
                self.ready().await?;
                if self.account.status(&self.client).await["signed_in"] != true {
                    return Err("Sign in before opening purchases".into());
                }
                self.downloads.action(params).await
            }
            _ => Err(format!("no method {method}")),
        }
    }
}
