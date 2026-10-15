//! Local test fixture control is filesystem-only and never part of the LAN API.
use mathscape_desktop::{CreateInkDocumentRequest, DesktopService};
use mathscape_domain::InkLabDocument;
use mathscape_llm::MemoryCredentialResolver;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn publish(path: &std::path::Path, value: &Value) {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(value).unwrap()).unwrap();
    fs::rename(temporary, path).unwrap();
}
#[tokio::main]
async fn main() {
    let output = PathBuf::from(std::env::args().nth(1).expect("test output directory"));
    fs::create_dir_all(&output).unwrap();
    let workspace = output.join(format!(
        "workspace-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut service = DesktopService::open_with_credential_store(
        &workspace,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    service.start_ink_lan(Some(1453)).await.unwrap();
    let mut ready = json!({"base":"https://127.0.0.1:1453","workspace":workspace});
    let mut document_id = String::new();
    let mut last = 0;
    publish(&output.join("ready.json"), &ready);
    loop {
        if let Ok(bytes) = fs::read(output.join("control.json")) {
            if let Ok(command) = serde_json::from_slice::<Value>(&bytes) {
                let id = command["id"].as_u64().unwrap_or(0);
                if id > last {
                    match command["action"].as_str().unwrap_or("") {
                        "fresh" => {
                            let mut document: InkLabDocument = serde_json::from_str(include_str!(
                                "../../../testdata/ink/p0/trace.json"
                            ))
                            .unwrap();
                            document.operations.clear();
                            document.document_id = format!("browser-document-{id}");
                            document.title = format!("P3 手稿 {id}");
                            document_id = document.document_id.clone();
                            service
                                .create_ink_document(CreateInkDocumentRequest {
                                    document,
                                    session_id: format!("browser-native-{id}"),
                                    writer_id: "desktop-local".into(),
                                    device: json!({"source":"test"}),
                                })
                                .unwrap();
                            let pair = service
                                .create_ink_pairing(document_id.clone(), 0)
                                .await
                                .unwrap();
                            ready["url"] = pair
                                .urls
                                .iter()
                                .find(|url| url.contains("127.0.0.1"))
                                .unwrap()
                                .clone()
                                .into();
                            ready["document_id"] = document_id.clone().into();
                        }
                        "pause" => {
                            service.shutdown_ink_lan(false).await.unwrap();
                        }
                        "resume" => {
                            service.start_ink_lan(Some(1453)).await.unwrap();
                        }
                        "restart" => {
                            service.shutdown_ink_lan(false).await.unwrap();
                            service = DesktopService::open_with_credential_store(
                                &workspace,
                                Arc::new(MemoryCredentialResolver::default()),
                            )
                            .unwrap();
                            service.start_ink_lan(Some(1453)).await.unwrap();
                        }
                        "reclaim" => {
                            let view = service
                                .ink_lan_mirror(&document_id, 0)
                                .unwrap()
                                .delta
                                .writer;
                            service
                                .reclaim_ink_writer(
                                    &document_id,
                                    view.writer_epoch,
                                    view.head_seq,
                                    &format!("browser-takeover-{id}"),
                                )
                                .unwrap();
                        }
                        "setup" => {
                            let setup = service.start_ink_setup().await.unwrap();
                            ready["setup_url"] = setup
                                .urls
                                .iter()
                                .find(|url| url.contains("127.0.0.1"))
                                .unwrap()
                                .clone()
                                .into();
                        }
                        "setup_bound" => {
                            let setup = service
                                .start_ink_setup_for(Some(document_id.clone()))
                                .await
                                .unwrap();
                            ready["setup_url"] = setup
                                .urls
                                .iter()
                                .find(|url| url.contains("127.0.0.1"))
                                .unwrap()
                                .clone()
                                .into();
                        }
                        "prepare_handoff" | "confirm_handoff" => {
                            let confirm = command["action"] == "confirm_handoff";
                            service
                                .prepare_ink_setup_handoff(None, confirm)
                                .await
                                .unwrap();
                        }
                        "transfer" => {
                            let pair = service
                                .create_ink_pairing_with(document_id.clone(), 0, true)
                                .await
                                .unwrap();
                            ready["transfer_url"] = pair
                                .urls
                                .iter()
                                .find(|url| url.contains("127.0.0.1"))
                                .unwrap()
                                .clone()
                                .into();
                        }
                        "status" => {}
                        "stop" => {
                            service.shutdown_ink_lan(false).await.unwrap();
                            break;
                        }
                        _ => {}
                    }
                    last = id;
                    ready["action_id"] = id.into();
                    ready["setup"] = serde_json::to_value(service.ink_setup_status()).unwrap();
                    if !document_id.is_empty() {
                        ready["document"] = serde_json::to_value(
                            service
                                .open_ink_document(document_id.clone())
                                .unwrap()
                                .unwrap(),
                        )
                        .unwrap();
                        ready["snapshots"] = serde_json::to_value(
                            service.list_ink_snapshots(document_id.clone()).unwrap(),
                        )
                        .unwrap();
                    }
                    publish(&output.join("ready.json"), &ready);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
