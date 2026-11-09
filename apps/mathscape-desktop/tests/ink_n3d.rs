//! N3d: named snapshots (v20) and the workspace search command.

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::{
    AppendInkOperationRequest, CreateInkDocumentRequest, CreateInkSnapshotRequest, DesktopService,
};
use mathscape_domain::{
    ActorRef, ContentHash, InkLabDocument, InkLabOperation, InkSelectionMode, MediaType,
    RegisterInkEvidenceRequest, StableId,
};
use mathscape_llm::MemoryCredentialResolver;
use mathscape_query::search::{SearchKind, SearchQuery, SearchResponse, SearchTarget};
use mathscape_store_sqlite::{
    InkLifecycleRequest, InkRenameRequest, InkSnapshotLabelRequest, SqliteArtifactStore,
    StoredInkSnapshot,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    service: DesktopService,
    document: InkLabDocument,
    trace: Vec<InkLabOperation>,
}

impl Fixture {
    fn new(case: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".test-runs")
            .join(format!(
                "{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let service = DesktopService::open_with_credential_store(
            &root,
            Arc::new(MemoryCredentialResolver::default()),
        )
        .unwrap();
        let mut document: InkLabDocument =
            serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
        let trace = std::mem::take(&mut document.operations);
        service
            .create_ink_document(CreateInkDocumentRequest {
                document: document.clone(),
                session_id: "n3d-session".into(),
                writer_id: "desktop-local".into(),
                device: serde_json::json!({}),
                request_id: None,
                title_mode: None,
            })
            .unwrap();
        Self {
            root,
            service,
            document,
            trace,
        }
    }
    fn append(&mut self, operation: InkLabOperation) {
        let seq = self.document.operations.len() as u64;
        self.service
            .append_ink_operation(AppendInkOperationRequest {
                request_id: format!("op-{}", seq + 1),
                document_id: self.document.document_id.clone(),
                session_id: "n3d-session".into(),
                writer_epoch: 1,
                expected_head: seq,
                metadata: self.document.metadata(),
                operation: operation.clone(),
            })
            .unwrap();
        self.document.operations.push(operation);
    }
    fn snapshot(&self, request: &str) -> StoredInkSnapshot {
        self.service
            .create_ink_snapshot(CreateInkSnapshotRequest {
                request_id: request.into(),
                document_id: self.document.document_id.clone(),
                expected_head: self.document.operations.len() as u64,
            })
            .unwrap()
    }
    fn label(
        &self,
        request: &str,
        snapshot: &str,
        label: &str,
        expected: u64,
    ) -> InkSnapshotLabelRequest {
        InkSnapshotLabelRequest {
            request_id: request.into(),
            snapshot_id: snapshot.into(),
            label: label.into(),
            expected_label_revision: expected,
        }
    }
    fn store(&self) -> SqliteArtifactStore {
        SqliteArtifactStore::open(&self.root.join("mathscape.sqlite3")).unwrap()
    }
    fn connection(&self) -> Connection {
        Connection::open(self.root.join("mathscape.sqlite3")).unwrap()
    }
    fn search(&self, text: &str, include_trashed: bool) -> SearchResponse {
        self.service
            .search_workspace(&SearchQuery {
                text: text.into(),
                include_trashed,
                ..SearchQuery::default()
            })
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn artifact_sha256(root: &std::path::Path, snapshot: &StoredInkSnapshot) -> String {
    // Re-hash the stored Artifact bytes through the integrity-checked open.
    let service = DesktopService::open_with_credential_store(
        root,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    let opened = service
        .open_ink_snapshot(snapshot.snapshot_id.clone())
        .unwrap()
        .expect("snapshot opens and passes the integrity check");
    format!(
        "sha256:{:x}",
        Sha256::digest(opened.manifest_json.as_bytes())
    )
}

#[test]
fn naming_a_snapshot_never_changes_its_bytes_hash_or_identity() {
    let mut f = Fixture::new("n3d-label-hash");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("snap-1");
    assert_eq!(snapshot.label, None);
    assert_eq!(snapshot.label_revision, 0);
    let before_hash = artifact_sha256(&f.root, &snapshot);
    assert_eq!(before_hash, snapshot.content_hash);
    let artifacts_before: i64 = f
        .connection()
        .query_row("SELECT count(*) FROM artifacts", [], |row| row.get(0))
        .unwrap();

    let receipt = f
        .service
        .rename_ink_snapshot(f.label("label-1", &snapshot.snapshot_id, "  整理稿  ", 0))
        .unwrap();
    assert!(!receipt.replayed);
    assert_eq!(receipt.snapshot.label.as_deref(), Some("整理稿"));
    assert_eq!(receipt.snapshot.label_revision, 1);
    assert_eq!(receipt.snapshot.content_hash, snapshot.content_hash);
    assert_eq!(receipt.snapshot.artifact_id, snapshot.artifact_id);

    let opened = f
        .service
        .open_ink_snapshot(snapshot.snapshot_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(opened.manifest_json, snapshot.manifest_json);
    assert_eq!(opened.content_hash, snapshot.content_hash);
    assert_eq!(opened.label.as_deref(), Some("整理稿"));
    assert!(
        !opened.manifest_json.contains("整理稿"),
        "label stays outside the manifest"
    );
    assert_eq!(artifact_sha256(&f.root, &snapshot), before_hash);
    let artifacts_after: i64 = f
        .connection()
        .query_row("SELECT count(*) FROM artifacts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        artifacts_after, artifacts_before,
        "rename creates no Artifact"
    );

    // Saving the same head again yields the same, already named snapshot.
    let again = f.snapshot("snap-1-again");
    assert_eq!(again.snapshot_id, snapshot.snapshot_id);
    assert_eq!(again.label.as_deref(), Some("整理稿"));
    let list = f
        .service
        .list_ink_snapshots(f.document.document_id.clone())
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label.as_deref(), Some("整理稿"));

    // Empty clears the name (and still bumps the revision).
    let cleared = f
        .service
        .rename_ink_snapshot(f.label("label-2", &snapshot.snapshot_id, "   ", 1))
        .unwrap();
    assert_eq!(cleared.snapshot.label, None);
    assert_eq!(cleared.snapshot.label_revision, 2);
    assert_eq!(artifact_sha256(&f.root, &snapshot), before_hash);
}

#[test]
fn snapshot_names_follow_length_rules_and_need_not_be_unique() {
    let mut f = Fixture::new("n3d-label-rules");
    f.append(f.trace[0].clone());
    let first = f.snapshot("snap-a");
    f.append(f.trace[1].clone());
    let second = f.snapshot("snap-b");
    // 40 CJK chars = 120 bytes: allowed, counted in chars.
    let forty = "第".repeat(40);
    let named = f
        .service
        .rename_ink_snapshot(f.label("r-40", &first.snapshot_id, &forty, 0))
        .unwrap();
    assert_eq!(named.snapshot.label.as_deref(), Some(forty.as_str()));
    let too_long = f
        .service
        .rename_ink_snapshot(f.label("r-41", &second.snapshot_id, &"第".repeat(41), 0))
        .unwrap_err();
    assert_eq!(too_long.code, "snapshot_label_invalid");
    assert!(too_long.message.contains("40"));
    let control = f
        .service
        .rename_ink_snapshot(f.label("r-ctl", &second.snapshot_id, "a\u{7}b", 0))
        .unwrap_err();
    assert_eq!(control.code, "snapshot_label_invalid");
    // Duplicate names are fine.
    f.service
        .rename_ink_snapshot(f.label("r-dup", &second.snapshot_id, &forty, 0))
        .unwrap();
    let list = f
        .service
        .list_ink_snapshots(f.document.document_id.clone())
        .unwrap();
    assert!(
        list.iter()
            .all(|item| item.label.as_deref() == Some(forty.as_str()))
    );
    assert_ne!(list[0].through_seq, list[1].through_seq);
    // The SQLite CHECK guards the rule even below the API.
    assert!(
        f.connection()
            .execute(
                "UPDATE ink_snapshots SET label=?1 WHERE snapshot_id=?2",
                rusqlite::params!["第".repeat(41), first.snapshot_id],
            )
            .is_err()
    );
    let missing = f
        .service
        .rename_ink_snapshot(f.label("r-missing", "mathscape:ink-snapshot/none", "x", 0))
        .unwrap_err();
    assert_eq!(missing.code, "snapshot_not_found");
}

#[test]
fn stale_snapshot_rename_reports_a_conflict_and_requests_replay() {
    let mut f = Fixture::new("n3d-label-conflict");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("snap");
    let request = f.label("tab-a", &snapshot.snapshot_id, "第一次尝试", 0);
    f.service.rename_ink_snapshot(request.clone()).unwrap();
    // A second window still holds revision 0.
    let conflict = f
        .service
        .rename_ink_snapshot(f.label("tab-b", &snapshot.snapshot_id, "整理稿", 0))
        .unwrap_err();
    assert_eq!(conflict.code, "snapshot_label_conflict");
    assert_eq!(conflict.details["current_label"], "第一次尝试");
    assert_eq!(conflict.details["current_label_revision"], 1);
    assert!(conflict.message.contains("保留哪一个"));
    // Retrying the winning request replays instead of bumping again.
    let replay = f.service.rename_ink_snapshot(request.clone()).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.snapshot.label_revision, 1);
    // The same request id with another payload is rejected.
    let mut reused = request;
    reused.label = "别的名字".into();
    assert_eq!(
        f.service.rename_ink_snapshot(reused).unwrap_err().code,
        "idempotency_conflict"
    );
    // Choosing "mine" retries with the current revision.
    let resolved = f
        .service
        .rename_ink_snapshot(f.label("tab-b-2", &snapshot.snapshot_id, "整理稿", 1))
        .unwrap();
    assert_eq!(resolved.snapshot.label.as_deref(), Some("整理稿"));
    assert_eq!(resolved.snapshot.label_revision, 2);
}

#[test]
fn trashed_notes_keep_snapshot_names_read_only() {
    let mut f = Fixture::new("n3d-label-trash");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("snap");
    f.service
        .rename_ink_snapshot(f.label("name", &snapshot.snapshot_id, "整理稿", 0))
        .unwrap();
    let note = f
        .store()
        .ink_note(&f.document.document_id)
        .unwrap()
        .unwrap();
    f.store()
        .trash_ink_note(&InkLifecycleRequest {
            request_id: "trash".into(),
            document_id: f.document.document_id.clone(),
            expected_lifecycle_revision: note.lifecycle_revision,
        })
        .unwrap();
    let error = f
        .service
        .rename_ink_snapshot(f.label("rename-trashed", &snapshot.snapshot_id, "新名字", 1))
        .unwrap_err();
    assert_eq!(error.code, "document_trashed");
    assert_eq!(
        f.service
            .open_ink_snapshot(snapshot.snapshot_id.clone())
            .unwrap()
            .unwrap()
            .label
            .as_deref(),
        Some("整理稿")
    );
}

#[test]
fn upgrading_v19_keeps_snapshots_unnamed_and_renamable() {
    let mut f = Fixture::new("n3d-v19-upgrade");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("old-snap");
    let c = f.connection();
    c.execute_batch(
        "DROP TABLE ink_snapshot_label_requests;
         ALTER TABLE ink_snapshots DROP COLUMN label_revision;
         ALTER TABLE ink_snapshots DROP COLUMN label;
         DELETE FROM schema_migrations WHERE version=20;
         PRAGMA user_version=19;",
    )
    .unwrap();
    drop(c);
    assert!(SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).is_err());
    let upgraded = DesktopService::open_with_credential_store(
        &f.root,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    let version: i64 = f
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 24);
    let reopened = upgraded
        .open_ink_snapshot(snapshot.snapshot_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(reopened.label, None);
    assert_eq!(reopened.label_revision, 0);
    assert_eq!(reopened.content_hash, snapshot.content_hash);
    let named = upgraded
        .rename_ink_snapshot(f.label("after-upgrade", &snapshot.snapshot_id, "整理稿", 0))
        .unwrap();
    assert_eq!(named.snapshot.label.as_deref(), Some("整理稿"));
    // The old snapshot request still replays to the same snapshot.
    assert_eq!(
        upgraded
            .create_ink_snapshot(CreateInkSnapshotRequest {
                request_id: "old-snap".into(),
                document_id: f.document.document_id.clone(),
                expected_head: 1,
            })
            .unwrap()
            .snapshot_id,
        snapshot.snapshot_id
    );
    drop(upgraded);
    // Guard path: migration 20 recorded but user_version left behind.
    f.connection()
        .execute_batch("PRAGMA user_version=19;")
        .unwrap();
    let store = SqliteArtifactStore::prepare(&f.root.join("mathscape.sqlite3")).unwrap();
    assert_eq!(
        store
            .open_ink_snapshot(&snapshot.snapshot_id)
            .unwrap()
            .unwrap()
            .label
            .as_deref(),
        Some("整理稿")
    );
}

#[test]
fn snapshot_names_show_on_the_map_node_and_evidence_summary() {
    let mut f = Fixture::new("n3d-label-map");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("snap");
    let evidence = f
        .service
        .register_ink_evidence(RegisterInkEvidenceRequest {
            request_id: "evidence".into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            expected_artifact_id: StableId::parse(&snapshot.artifact_id).unwrap(),
            expected_content_hash: ContentHash::parse(&snapshot.content_hash).unwrap(),
            page_id: "page-a".into(),
            stroke_ids: vec!["stroke-a".into()],
            selection_mode: InkSelectionMode::Lasso,
        })
        .unwrap();
    let node = |graph: &serde_json::Value, id: &str| {
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["domain_id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("node {id} missing"))
    };
    let graph = serde_json::to_value(f.service.snapshot().unwrap().landscape_graph()).unwrap();
    let unnamed = node(&graph, &snapshot.artifact_id);
    assert!(!unnamed["label"].as_str().unwrap().contains('「'));
    assert!(
        node(&graph, &evidence.span_id)["summary"]
            .as_str()
            .unwrap()
            .starts_with("手稿快照选区")
    );
    f.service
        .rename_ink_snapshot(f.label("name", &snapshot.snapshot_id, "整理稿", 0))
        .unwrap();
    let graph = serde_json::to_value(f.service.snapshot().unwrap().landscape_graph()).unwrap();
    let named = node(&graph, &snapshot.artifact_id);
    assert!(
        named["label"]
            .as_str()
            .unwrap()
            .starts_with("「整理稿」 · "),
        "{named}"
    );
    let summary = node(&graph, &evidence.span_id)["summary"].clone();
    assert_eq!(summary, "手稿快照「整理稿」选区 · 1 笔", "{summary}");
}

fn titles(response: &SearchResponse, kind: SearchKind) -> Vec<String> {
    response
        .groups
        .iter()
        .find(|group| group.kind == kind)
        .map(|group| group.hits.iter().map(|hit| hit.title.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn search_workspace_finds_notes_named_snapshots_and_respects_trash() {
    let mut f = Fixture::new("n3d-search");
    f.append(f.trace[0].clone());
    let note = f
        .store()
        .ink_note(&f.document.document_id)
        .unwrap()
        .unwrap();
    f.service
        .rename_ink_note(InkRenameRequest {
            request_id: "rename-note".into(),
            document_id: f.document.document_id.clone(),
            title: "Cauchy 中值定理练习".into(),
            expected_title_revision: note.title_revision,
        })
        .unwrap();
    let unnamed = f.snapshot("unnamed");
    f.append(f.trace[1].clone());
    let named = f.snapshot("named");
    f.service
        .rename_ink_snapshot(f.label("name", &named.snapshot_id, "整理稿", 0))
        .unwrap();

    let response = f.search("ｃａｕｃｈｙ 定理", false);
    assert_eq!(
        titles(&response, SearchKind::Note),
        vec!["Cauchy 中值定理练习"]
    );
    let hit = &response.groups[0].hits[0];
    assert_eq!(
        hit.target,
        SearchTarget::InkNote {
            document_id: f.document.document_id.clone()
        }
    );
    let response = f.search("整理", false);
    assert_eq!(titles(&response, SearchKind::Snapshot), vec!["整理稿"]);
    let snapshot_hit = &response.groups[0].hits[0];
    assert_eq!(
        snapshot_hit.target,
        SearchTarget::InkSnapshot {
            snapshot_id: named.snapshot_id.clone(),
            artifact_id: named.artifact_id.clone(),
            document_id: f.document.document_id.clone(),
        }
    );
    assert_eq!(snapshot_hit.highlights, vec![[0, 2]]);
    assert!(
        snapshot_hit
            .subtitle
            .as_deref()
            .unwrap()
            .contains("第 2 步")
    );
    // Unnamed snapshots and snapshot Artifacts (file name contains the
    // note title) are not separate results.
    let response = f.search("手稿快照", false);
    assert!(titles(&response, SearchKind::Snapshot).is_empty());
    assert!(titles(&response, SearchKind::Material).is_empty());
    assert!(
        response
            .groups
            .iter()
            .all(|group| group.hits.iter().all(|hit| hit.id != unnamed.snapshot_id))
    );

    let lifecycle = f
        .store()
        .ink_note(&f.document.document_id)
        .unwrap()
        .unwrap();
    f.store()
        .trash_ink_note(&InkLifecycleRequest {
            request_id: "trash".into(),
            document_id: f.document.document_id.clone(),
            expected_lifecycle_revision: lifecycle.lifecycle_revision,
        })
        .unwrap();
    assert_eq!(f.search("cauchy", false).total, 0);
    assert_eq!(f.search("整理稿", false).total, 0);
    let included = f.search("cauchy", true);
    assert!(included.groups[0].hits[0].trashed);
    assert_eq!(
        titles(&f.search("整理稿", true), SearchKind::Snapshot),
        vec!["整理稿"]
    );
    assert_eq!(f.search("   ", true).total, 0);

    // Materials by file name; parsed images are not searchable.
    let mut pool = ArtifactPool::open(&f.root).unwrap();
    let actor = ActorRef::parse("n3d-test").unwrap();
    let markdown = pool
        .import_bytes(
            "# 实变函数".as_bytes(),
            ArtifactMetadata::derived(
                MediaType::parse("text/markdown").unwrap(),
                Some("实变函数讲义.md".into()),
            )
            .unwrap(),
            actor.clone(),
        )
        .unwrap();
    pool.import_bytes(
        b"not really a png",
        ArtifactMetadata::derived(
            MediaType::parse("image/png").unwrap(),
            Some("实变函数插图.png".into()),
        )
        .unwrap(),
        actor,
    )
    .unwrap();
    drop(pool);
    let response = f.search("实变函数", false);
    assert_eq!(
        titles(&response, SearchKind::Material),
        vec!["实变函数讲义.md"]
    );
    let material = &response.groups[0].hits[0];
    assert_eq!(
        material.target,
        SearchTarget::Artifact {
            artifact_id: markdown.stable_id().as_str().to_owned()
        }
    );
    assert_eq!(material.subtitle.as_deref(), Some("解析正文"));
}
