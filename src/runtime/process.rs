use super::Runtime;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

#[derive(PartialEq)]
struct Directories {
    data: PathBuf,
    cache: PathBuf,
    export: Option<PathBuf>,
}

fn directory(params: &Value, name: &str) -> Result<PathBuf, String> {
    let value = params[name]
        .as_str()
        .ok_or_else(|| format!("Invalid {name}"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.contains('\0') {
        return Err(format!("Invalid {name}"));
    }
    Ok(path)
}

fn initialize(
    service: &mut Option<(Directories, Arc<Runtime>)>,
    params: &Value,
) -> Result<Arc<Runtime>, String> {
    let paths = Directories {
        data: directory(params, "data_dir")?,
        cache: directory(params, "cache_dir")?,
        export: params
            .get("export_dir")
            .map(|_| directory(params, "export_dir"))
            .transpose()?,
    };
    if let Some((current, runtime)) = service {
        if *current != paths {
            return Err("Plugin directories cannot change during a run".into());
        }
        return Ok(runtime.clone());
    }
    let runtime = Arc::new(Runtime::at(paths.data.clone(), paths.export.clone()));
    *service = Some((paths, runtime.clone()));
    Ok(runtime)
}

fn respond(
    out: &mpsc::UnboundedSender<String>,
    id: Value,
    result: Result<Value, String>,
    code: i64,
) {
    let reply = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(message) => json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
    };
    let _ = out.send(reply.to_string());
}

pub async fn run() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    serve(tokio::io::stdin(), tokio::io::stdout()).await;
}

pub async fn serve<R, W>(input: R, mut output: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let mut service = None;
    let (out, mut receiver) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = receiver.recv().await {
            if output.write_all(line.as_bytes()).await.is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });
    let mut input = BufReader::new(input).lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = input.next_line().await {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or("").to_string();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if method == "initialize" {
            match initialize(&mut service, &params) {
                Ok(runtime) => respond(&out, id, runtime.handle(&method, params).await, -32000),
                Err(message) => respond(&out, id, Err(message), -32602),
            }
            continue;
        }
        let Some((_, runtime)) = &service else {
            respond(&out, id, Err("Initialize the plugin first".into()), -32002);
            continue;
        };
        let runtime = runtime.clone();
        let out = out.clone();
        while tasks.len() >= 32 {
            tasks.join_next().await;
        }
        tasks.spawn(async move {
            respond(&out, id, runtime.handle(&method, params).await, -32000);
        });
    }
    while tasks.join_next().await.is_some() {}
    drop(out);
    let _ = writer.await;
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[tokio::test]
    async fn descriptor_export_retains_its_identity_after_the_folder_moves() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let profile = tempfile::tempdir().unwrap();
        let export = profile.path().join("approved");
        std::fs::create_dir(&export).unwrap();
        let held = std::fs::File::open(&export).unwrap();
        let descriptor = format!("/proc/self/fd/{}", held.as_raw_fd());
        let mut service = None;
        let runtime = initialize(
            &mut service,
            &json!({
                "data_dir":profile.path(),"cache_dir":profile.path(),"export_dir":descriptor
            }),
        )
        .unwrap();
        std::fs::rename(&export, profile.path().join("moved")).unwrap();
        let view = runtime.handle("downloads.open", json!({})).await.unwrap();
        assert_eq!(view["destination_ready"], true);
        assert_eq!(
            service.as_ref().unwrap().0.export,
            Some(PathBuf::from(descriptor))
        );
    }
}
