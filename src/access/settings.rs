//! Account form using only the host's existing fields, actions and browser links.
use super::credentials::Credentials;
use serde_json::{json, Value};

pub fn document(
    language: &str,
    revision: u64,
    credentials: &Credentials,
    shown: [bool; 3],
    user: Option<&str>,
    pending: Option<&str>,
    notice: Option<&str>,
) -> Value {
    let es = language == "es";
    let say = |en: &str, spanish: &str| {
        if es {
            spanish.to_string()
        } else {
            en.to_string()
        }
    };
    let mut fields = Vec::new();
    let mut actions = Vec::new();
    let mut values = json!({});
    let mut links = Vec::new();
    let content;
    if let Some(user) = user {
        content = format!("{} {user}", say("Signed in as", "Conectado como"));
        actions.push(json!({"key":"sign_out", "label":say("Sign out", "Salir"), "fields":[], "destructive":"account"}));
    } else if let Some(url) = pending {
        content = say("Open Qobuz in your browser, then check sign-in here. This attempt expires in five minutes.", "Abre Qobuz en el navegador y después comprueba el acceso aquí. Este intento caduca en cinco minutos.");
        links.push(json!({"label":say("Open Qobuz", "Abrir Qobuz"), "url":url}));
        actions.push(json!({"key":"check_sign_in", "label":say("Check sign-in", "Comprobar acceso"), "fields":[]}));
        actions
            .push(json!({"key":"cancel_sign_in", "label":say("Cancel", "Cancelar"), "fields":[]}));
    } else {
        content = say("Fetch app credentials or enter them, then sign in using your browser. Your account password stays in the browser.", "Obtén las credenciales de la aplicación o introdúcelas y entra desde el navegador. La contraseña de tu cuenta se queda en el navegador.");
        let keys = ["app_id", "app_secret", "private_key"];
        for (index, (key, label, value)) in [
            ("app_id", "App ID", &credentials.app_id),
            ("app_secret", "App Secret", &credentials.app_secret),
            ("private_key", "Private Key", &credentials.private_key),
        ]
        .iter()
        .enumerate()
        {
            fields.push(
                json!({"key":key,"label":label,"description":"","required":true,
                "control":{"type":if shown[index] {"text"} else {"secret"},"max_length":512}}),
            );
            values[*key] = json!(value);
            actions.push(json!({"key":format!("visibility_{index}"),"label":format!("{} {label}", if shown[index] {say("Hide", "Ocultar")} else {say("Show", "Mostrar")}),"fields":keys}));
        }
        actions.push(json!({"key":"fetch_credentials","label":say("Fetch credentials", "Obtener credenciales"),"fields":[]}));
        actions.push(json!({"key":"sign_in","label":say("Sign in", "Entrar"),"fields":keys}));
    }
    json!({"revision":revision,"form":{"fields":fields,"actions":actions,"unsupported":[]},"values":values,
        "content":[{"text":content}],"links":links,"notices":notice.into_iter().collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_masked_and_each_eye_only_reveals_its_field() {
        let credentials = Credentials::default();
        let masked = document("es", 1, &credentials, [false; 3], None, None, None);
        assert!(masked["form"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["control"]["type"] == "secret"));
        let shown = document(
            "es",
            2,
            &credentials,
            [false, true, false],
            None,
            None,
            None,
        );
        assert_eq!(shown["form"]["fields"][1]["control"]["type"], "text");
        assert_eq!(shown["form"]["fields"][0]["control"]["type"], "secret");
        let pending = document(
            "es",
            3,
            &credentials,
            [true; 3],
            None,
            Some("https://www.qobuz.com/signin/oauth"),
            None,
        );
        assert_eq!(pending["values"], json!({}));
        assert!(pending["form"]["fields"].as_array().unwrap().is_empty());
    }
}
