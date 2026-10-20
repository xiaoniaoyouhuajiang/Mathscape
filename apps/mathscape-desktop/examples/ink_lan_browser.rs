//! Local test fixture control is filesystem-only and never part of the LAN API.
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::{
    CreateInkDocumentRequest, DesktopService, InkLinkTargetRequest, InkMarkdownTargetRequest,
    LinkInkNoteRequest,
};
use mathscape_domain::{ActorRef, InkLabDocument, MediaType, RightsRecord, RightsStatus};
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
                                    request_id: None,
                                    title_mode: None,
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
                        "trash" | "restore" => {
                            let note = service.ink_note(document_id.clone()).unwrap().unwrap();
                            let request = mathscape_store_sqlite::InkLifecycleRequest {
                                request_id: format!("browser-lifecycle-{id}"),
                                document_id: document_id.clone(),
                                expected_lifecycle_revision: note.lifecycle_revision,
                            };
                            let receipt = if command["action"] == "trash" {
                                service.trash_ink_note(request).await.unwrap()
                            } else {
                                service.restore_ink_note(request).unwrap()
                            };
                            ready["lifecycle_receipt"] = serde_json::to_value(receipt).unwrap();
                        }
                        "rename" => {
                            let note = service.ink_note(document_id.clone()).unwrap().unwrap();
                            let receipt = service
                                .rename_ink_note(mathscape_store_sqlite::InkRenameRequest {
                                    request_id: format!("browser-rename-{id}"),
                                    document_id: document_id.clone(),
                                    title: command["title"].as_str().unwrap_or("").into(),
                                    expected_title_revision: note.title_revision,
                                })
                                .unwrap();
                            ready["rename_receipt"] = serde_json::to_value(receipt).unwrap();
                        }
                        "push" => {
                            // Create a second note and push it to every online tablet slot.
                            let mut document: InkLabDocument = serde_json::from_str(include_str!(
                                "../../../testdata/ink/p0/trace.json"
                            ))
                            .unwrap();
                            document.operations.clear();
                            document.document_id = format!("browser-push-{id}");
                            document.title = format!("推送手稿 {id}");
                            let target_id = document.document_id.clone();
                            service
                                .create_ink_document(CreateInkDocumentRequest {
                                    document,
                                    session_id: format!("browser-push-session-{id}"),
                                    writer_id: "desktop-local".into(),
                                    device: json!({"source": "test"}),
                                    request_id: Some(format!("browser-push-req-{id}")),
                                    title_mode: None,
                                })
                                .unwrap();
                            let allow = command["allow_transfer"].as_bool().unwrap_or(false);
                            let result =
                                service.push_ink_to_tablet(&target_id, allow).await.unwrap();
                            ready["push"] = serde_json::to_value(&result).unwrap();
                            ready["push_document_id"] = target_id.clone().into();
                            ready["push_title"] = format!("推送手稿 {id}").into();
                            // Keep document_id pointing at the original A for status/reclaim.
                        }
                        "push_current" => {
                            let allow = command["allow_transfer"].as_bool().unwrap_or(false);
                            let result = service
                                .push_ink_to_tablet(&document_id, allow)
                                .await
                                .unwrap();
                            ready["push"] = serde_json::to_value(&result).unwrap();
                            ready["push_document_id"] = document_id.clone().into();
                        }
                        "link_primary" => {
                            // Create a target note and set it as the current note's primary link.
                            let mut target: InkLabDocument = serde_json::from_str(include_str!(
                                "../../../testdata/ink/p0/trace.json"
                            ))
                            .unwrap();
                            target.operations.clear();
                            target.document_id = format!("browser-card-target-{id}");
                            target.title = command["target_title"]
                                .as_str()
                                .unwrap_or("卡片目标题")
                                .into();
                            let target_id = target.document_id.clone();
                            service
                                .create_ink_document(CreateInkDocumentRequest {
                                    document: target,
                                    session_id: format!("browser-card-session-{id}"),
                                    writer_id: "desktop-local".into(),
                                    device: json!({"source": "test"}),
                                    request_id: Some(format!("browser-card-create-{id}")),
                                    title_mode: None,
                                })
                                .unwrap();
                            let receipt = service
                                .link_ink_note(LinkInkNoteRequest {
                                    request_id: format!("browser-card-link-{id}"),
                                    document_id: document_id.clone(),
                                    target: InkLinkTargetRequest::InkNote {
                                        document_id: target_id.clone(),
                                    },
                                    role: mathscape_store_sqlite::InkLinkRole::Attempt,
                                    primary: true,
                                    replaces_link_id: None,
                                })
                                .unwrap();
                            ready["link"] = serde_json::to_value(&receipt).unwrap();
                            ready["card_target_id"] = target_id.into();
                            ready["problem_card"] = serde_json::to_value(
                                service.ink_problem_card(&document_id).unwrap(),
                            )
                            .unwrap();
                        }
                        "link_primary_content" => {
                            let source = "# 不属于主关联的相邻材料\n\n## 求极限\n\n设 **数列** 满足 $a_n=\\frac{1}{n}$。\n\n\\[\n\\lim_{n\\to\\infty}a_n=0\n\\]\n\n![图](https://example.com/unused.png)\n";
                            let start = source.find("## 求极限").unwrap();
                            let path = output.join(format!("card-{id}.md"));
                            fs::write(&path, source).unwrap();
                            let mut pool = ArtifactPool::open(&workspace).unwrap();
                            let artifact = pool
                                .import_file(
                                    &path,
                                    ArtifactMetadata::new(
                                        None,
                                        MediaType::parse("text/markdown").unwrap(),
                                        Some("card.md".into()),
                                        None,
                                        RightsRecord::new(RightsStatus::NotAssessed, vec![], None)
                                            .unwrap(),
                                    ),
                                    ActorRef::parse("browser-fixture").unwrap(),
                                )
                                .unwrap()
                                .stable_id()
                                .to_string();
                            let target = if command["target_kind"] == "markdown" {
                                let markdown = service.markdown_document(&artifact, None).unwrap();
                                let first = markdown
                                    .blocks
                                    .iter()
                                    .find(|block| block.range.start as usize >= start)
                                    .unwrap();
                                InkLinkTargetRequest::Markdown(InkMarkdownTargetRequest {
                                    artifact_id: artifact,
                                    content_hash: markdown.snapshot.content_hash.to_string(),
                                    first_block: first.id.clone(),
                                    last_block: markdown.blocks.last().unwrap().id.clone(),
                                })
                            } else {
                                let unit = format!("mathscape:source-unit/card-{id}");
                                let span = format!("mathscape:source-span/card-{id}");
                                let end =
                                    source.len() + if command["missing"] == true { 100 } else { 0 };
                                let connection =
                                    rusqlite::Connection::open(workspace.join("mathscape.sqlite3"))
                                        .unwrap();
                                for (object, kind, payload) in [
                                    (
                                        &span,
                                        "SourceSpan",
                                        json!({"payload":{"markdown":{"artifact":artifact,"byte_range":{"start":start,"end":end}}}}),
                                    ),
                                    (
                                        &unit,
                                        "SourceSemanticUnit",
                                        json!({"payload":{"kind":"statement","display_title":"卡片节点标题","source_spans":[span.clone()]}}),
                                    ),
                                ] {
                                    connection.execute("INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision) VALUES(?1,'entity',?2,'card-rev')", rusqlite::params![object,kind]).unwrap();
                                    connection.execute("INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,created_at,created_by,generation_run,payload_json) VALUES(?1,'card-rev','1','active','2026-10-20T00:00:00Z','test',NULL,?2)",rusqlite::params![object,payload.to_string()]).unwrap();
                                }
                                ready["card_span_id"] = span.into();
                                ready["card_source_end"] = source.len().into();
                                InkLinkTargetRequest::SourceObject { object_id: unit }
                            };
                            service
                                .link_ink_note(LinkInkNoteRequest {
                                    request_id: format!("browser-content-link-{id}"),
                                    document_id: document_id.clone(),
                                    target,
                                    role: mathscape_store_sqlite::InkLinkRole::Attempt,
                                    primary: true,
                                    replaces_link_id: None,
                                })
                                .unwrap();
                            ready["problem_card"] = serde_json::to_value(
                                service.ink_problem_card(&document_id).unwrap(),
                            )
                            .unwrap();
                        }
                        "repair_card_source" => {
                            let connection =
                                rusqlite::Connection::open(workspace.join("mathscape.sqlite3"))
                                    .unwrap();
                            connection.execute(
                                "UPDATE map_object_revisions SET payload_json=json_set(payload_json,'$.payload.markdown.byte_range.end',?1) WHERE object_id=?2",
                                rusqlite::params![ready["card_source_end"].as_i64().unwrap(),ready["card_span_id"].as_str().unwrap()],
                            ).unwrap();
                        }
                        "clear_primary" => {
                            let receipt = service
                                .set_ink_note_primary_link(
                                    mathscape_store_sqlite::InkSetPrimaryRequest {
                                        request_id: format!("browser-card-clear-{id}"),
                                        document_id: document_id.clone(),
                                        link_id: None,
                                    },
                                )
                                .unwrap();
                            ready["link"] = serde_json::to_value(&receipt).unwrap();
                            ready["problem_card"] = serde_json::to_value(
                                service.ink_problem_card(&document_id).unwrap(),
                            )
                            .unwrap();
                        }
                        "problem_card" => {
                            ready["problem_card"] = serde_json::to_value(
                                service.ink_problem_card(&document_id).unwrap(),
                            )
                            .unwrap();
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
                        ready["note"] =
                            serde_json::to_value(service.ink_note(document_id.clone()).unwrap())
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
