use qobuz_downloader::runtime::serve;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn exchange(requests: Vec<Value>) -> Vec<Value> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let (mut input, reader) = tokio::io::duplex(8192);
    let (writer, output) = tokio::io::duplex(8192);
    let service = tokio::spawn(serve(reader, writer));
    input.write_all(b"not-json\n").await.unwrap();
    for request in requests {
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    input.shutdown().await.unwrap();
    let mut responses = Vec::new();
    let mut lines = BufReader::new(output).lines();
    while let Some(line) = lines.next_line().await.unwrap() {
        responses.push(serde_json::from_str(&line).unwrap());
    }
    service.await.unwrap();
    responses
}

fn reply(responses: &[Value], id: u64) -> &Value {
    responses.iter().find(|reply| reply["id"] == id).unwrap()
}

#[tokio::test]
async fn initialization_rejects_missing_relative_and_malformed_directories() {
    let profile = tempfile::tempdir().unwrap();
    let data = profile.path().join("private data");
    let cache = profile.path().join("cache");
    let replies = exchange(vec![
        json!({"id":1,"method":"auth.logout"}),
        json!({"id":2,"method":"initialize","params":{}}),
        json!({"id":3,"method":"initialize","params":{"data_dir":"relative","cache_dir":cache}}),
        json!({"id":4,"method":"initialize","params":{"data_dir":data,"cache_dir":""}}),
        json!({"id":5,"method":"initialize","params":{"data_dir":data,"cache_dir":null}}),
        json!({"id":6,"method":"initialize","params":{"data_dir":data,"cache_dir":cache,"export_dir":"relative"}}),
        json!({"id":7,"method":"initialize","params":{"data_dir":"/invalid\u{0}path","cache_dir":cache}}),
    ]).await;
    assert_eq!(replies.len(), 7);
    assert_eq!(reply(&replies, 1)["error"]["code"], -32002);
    for id in 2..=7 {
        assert_eq!(reply(&replies, id)["error"]["code"], -32602);
    }
    assert!(!data.exists());
    assert!(!cache.exists());
}

#[tokio::test]
async fn missing_export_disables_downloads_and_private_state_survives_restart() {
    let profile = tempfile::tempdir().unwrap();
    let data = profile.path().join("private data");
    let cache = profile.path().join("cache");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&cache).unwrap();
    let params = json!({"data_dir":data,"cache_dir":cache});
    let replies = exchange(vec![
        json!({"id":1,"method":"initialize","params":params}),
        json!({"id":2,"method":"initialize","params":params}),
        json!({"id":3,"method":"initialize","params":{"data_dir":data,"cache_dir":cache,"export_dir":profile.path()}}),
        json!({"id":4,"method":"downloads.open"}),
        json!({"id":5,"method":"auth.logout"}),
    ]).await;
    assert_eq!(reply(&replies, 1)["result"]["protocol"], 1);
    assert_eq!(reply(&replies, 2)["result"]["protocol"], 1);
    assert_eq!(reply(&replies, 3)["error"]["code"], -32602);
    assert_eq!(reply(&replies, 4)["result"]["destination_ready"], false);
    let persisted = std::fs::read(data.join("account.json")).unwrap();
    assert!(!cache.join("account.json").exists());
    let restarted = exchange(vec![
        json!({"id":1,"method":"initialize","params":params}),
        json!({"id":2,"method":"auth.status"}),
    ])
    .await;
    assert_eq!(reply(&restarted, 2)["result"]["signed_in"], false);
    assert_eq!(std::fs::read(data.join("account.json")).unwrap(), persisted);
}

#[tokio::test]
async fn approved_export_is_used_without_assuming_a_mountpoint() {
    let profile = tempfile::tempdir().unwrap();
    let export = tempfile::tempdir().unwrap();
    let replies = exchange(vec![
        json!({"id":1,"method":"initialize","params":{"data_dir":profile.path(),"cache_dir":profile.path(),"export_dir":export.path()}}),
        json!({"id":2,"method":"downloads.open"}),
    ]).await;
    assert_eq!(reply(&replies, 2)["result"]["destination_ready"], true);
    assert_eq!(reply(&replies, 2)["result"]["signed_in"], false);
    assert_eq!(std::fs::read_dir(export.path()).unwrap().count(), 0);
}
