use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_desktop::{
    AppendInkOperationRequest, CreateInkDocumentRequest, CreateInkSnapshotRequest, DesktopService,
};
use mathscape_domain::{
    ActorRef, ContentHash, InkLabDocument, InkLabOperation, InkLabPage, InkSelectionMode,
    InkSnapshotManifest, MediaType, RegisterInkEvidenceRequest, StableId,
};
use mathscape_llm::MemoryCredentialResolver;
use mathscape_store_sqlite::{MapObjectOutcome, SqliteArtifactStore, StoredInkSnapshot};
use rusqlite::Connection;
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
                session_id: "p2-session".into(),
                writer_id: "desktop-local".into(),
                device: serde_json::json!({}),
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
                session_id: "p2-session".into(),
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
    fn request(snapshot: &StoredInkSnapshot, request: &str) -> RegisterInkEvidenceRequest {
        RegisterInkEvidenceRequest {
            request_id: request.into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            expected_artifact_id: StableId::parse(&snapshot.artifact_id).unwrap(),
            expected_content_hash: ContentHash::parse(&snapshot.content_hash).unwrap(),
            page_id: "page-a".into(),
            stroke_ids: vec!["stroke-a".into()],
            selection_mode: InkSelectionMode::Lasso,
        }
    }
    fn connection(&self) -> Connection {
        Connection::open(self.root.join("mathscape.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn registered_ink_evidence_reopens_the_original_snapshot_after_erasing() {
    let mut f = Fixture::new("ink-evidence-roundtrip");
    f.append(f.trace[0].clone());
    f.append(f.trace[1].clone());
    let snapshot = f.snapshot("snapshot-original");
    let mut request = Fixture::request(&snapshot, "evidence-1");
    request.stroke_ids.push("stroke-a".into());
    let first = f.service.register_ink_evidence(request.clone()).unwrap();
    assert_eq!(first.outcome, MapObjectOutcome::Created);
    assert_eq!(first.stroke_ids, vec!["stroke-a"]);
    request.stroke_ids.pop();
    assert_eq!(
        f.service.register_ink_evidence(request.clone()).unwrap(),
        first
    );
    request.request_id = "evidence-2".into();
    let reused = f.service.register_ink_evidence(request.clone()).unwrap();
    assert_eq!(reused.outcome, MapObjectOutcome::Reused);
    assert_eq!(reused.span_id, first.span_id);
    assert_eq!(
        f.service
            .list_ink_evidence(snapshot.snapshot_id.clone())
            .unwrap()
            .len(),
        1
    );
    f.append(f.trace[2].clone()); // Erase stroke-a in the current document.
    let later = f.snapshot("snapshot-later");
    assert!(
        f.service
            .register_ink_evidence(Fixture::request(&later, "erased-evidence"))
            .is_err()
    );
    let reopened = DesktopService::open_with_credential_store(
        &f.root,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    let detail = reopened.source_node_detail(&first.span_id).unwrap();
    let location = detail.ink.unwrap();
    assert_eq!(location.snapshot_id, snapshot.snapshot_id);
    assert_eq!(location.page_id, "page-a");
    assert_eq!(location.stroke_ids, vec!["stroke-a"]);
    let original = reopened
        .open_ink_snapshot_for_artifact(snapshot.artifact_id.clone())
        .unwrap()
        .unwrap();
    let manifest: InkSnapshotManifest = serde_json::from_str(&original.manifest_json).unwrap();
    manifest
        .validate_selection("page-a", &location.stroke_ids)
        .unwrap();
    assert_eq!(original.manifest_json, snapshot.manifest_json);
    let graph = serde_json::to_value(reopened.snapshot().unwrap().landscape_graph()).unwrap();
    assert!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["domain_id"] == first.span_id)
    );
    assert!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["relation_kind"] == "located_in" && node["label"] == "来源于手稿快照")
    );
    assert!(
        reopened
            .content_info(&snapshot.artifact_id)
            .unwrap()
            .ink_snapshot_id
            .is_some()
    );
    let source_map = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3"))
        .unwrap()
        .source_map_snapshot()
        .unwrap();
    assert_eq!(source_map.objects.len(), 1); // No semantic unit or invented discovery run.
}

#[test]
fn snapshot_reservation_survives_later_edits_and_request_alias_retries() {
    let mut f = Fixture::new("ink-snapshot-reservation");
    f.append(f.trace[0].clone());
    let mut store = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    let reserved = store
        .reserve_ink_snapshot(
            "unfinished-snapshot",
            &f.document.document_id,
            1,
            "2026-10-13T00:00:00Z",
        )
        .unwrap();
    let page = InkLabPage {
        id: "future-page".into(),
        width: 1200.0,
        height: 1600.0,
    };
    f.document.pages.push(page.clone());
    f.append(InkLabOperation::PageAdd {
        seq: 2,
        command_id: "page-added-later".into(),
        page,
    });
    // Resume after the simulated gap between prefix reservation and Pool import.
    let snapshot = f
        .service
        .create_ink_snapshot(CreateInkSnapshotRequest {
            request_id: "unfinished-snapshot".into(),
            document_id: f.document.document_id.clone(),
            expected_head: 1,
        })
        .unwrap();
    assert_eq!(snapshot.manifest_json, reserved.manifest_json);
    let manifest: InkSnapshotManifest = serde_json::from_str(&snapshot.manifest_json).unwrap();
    assert_eq!(manifest.document.pages.len(), 2);
    assert_eq!(manifest.document.operations.len(), 1);
    assert!(
        f.service
            .create_ink_snapshot(CreateInkSnapshotRequest {
                request_id: "unfinished-snapshot".into(),
                document_id: f.document.document_id.clone(),
                expected_head: 2
            })
            .is_err()
    );
    let current = f.snapshot("snapshot-head-2");
    let alias = f.snapshot("snapshot-head-2-alias");
    assert_eq!(alias.snapshot_id, current.snapshot_id);
    assert_eq!(alias.request_id, "snapshot-head-2-alias");
    f.append(InkLabOperation::Undo {
        seq: 3,
        command_id: "undo-3".into(),
        target_command_id: "cmd-a".into(),
    });
    let retry = f
        .service
        .create_ink_snapshot(CreateInkSnapshotRequest {
            request_id: "snapshot-head-2-alias".into(),
            document_id: f.document.document_id.clone(),
            expected_head: 2,
        })
        .unwrap();
    assert_eq!(retry.snapshot_id, current.snapshot_id);
}

#[test]
fn invalid_selection_and_receipt_failure_leave_no_partial_map_objects() {
    let mut f = Fixture::new("ink-evidence-atomic");
    f.append(f.trace[0].clone());
    f.append(f.trace[1].clone());
    let snapshot = f.snapshot("snapshot-valid");
    for (page, ids) in [
        ("missing-page", vec!["stroke-a"]),
        ("page-a", vec!["stroke-b"]),
        ("page-a", vec!["missing-stroke"]),
    ] {
        let mut request = Fixture::request(&snapshot, "invalid");
        request.page_id = page.into();
        request.stroke_ids = ids.into_iter().map(str::to_owned).collect();
        assert!(f.service.register_ink_evidence(request).is_err());
    }
    let request = Fixture::request(&snapshot, "atomic-request");
    let c = f.connection();
    c.execute_batch("CREATE TRIGGER ink_fail_receipt BEFORE INSERT ON ink_evidence_requests BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    assert!(f.service.register_ink_evidence(request.clone()).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM map_objects", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM map_commits", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER ink_fail_receipt;").unwrap();
    let receipt = f.service.register_ink_evidence(request.clone()).unwrap();
    assert_eq!(receipt.outcome, MapObjectOutcome::Created);
    let mut changed = request.clone();
    changed.page_id = "page-b".into();
    changed.stroke_ids = vec!["stroke-b".into()];
    assert!(
        f.service
            .register_ink_evidence(changed)
            .unwrap_err()
            .contains("different content")
    );
    c.execute(
        "UPDATE map_object_revisions SET lifecycle='deprecated' WHERE object_id=?1",
        [&receipt.span_id],
    )
    .unwrap();
    assert_eq!(
        f.service.register_ink_evidence(request.clone()).unwrap(),
        receipt
    ); // Replay does not reactivate it.
    let mut fresh = request;
    fresh.request_id = "reactivation-attempt".into();
    assert!(
        f.service
            .register_ink_evidence(fresh)
            .unwrap_err()
            .contains("registration_review_required")
    );
}

#[test]
fn concurrent_ink_registration_creates_one_span_and_refuses_corrupted_evidence() {
    let mut f = Fixture::new("ink-evidence-concurrent");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("snapshot-concurrent");
    let receipts = std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|index| {
                let service = f.service.clone();
                let request = Fixture::request(&snapshot, &format!("concurrent-{index}"));
                scope.spawn(move || service.register_ink_evidence(request).unwrap())
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        receipts
            .iter()
            .filter(|receipt| receipt.outcome == MapObjectOutcome::Created)
            .count(),
        1
    );
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.span_id == receipts[0].span_id)
    );
    let pool = ArtifactPool::open(&f.root).unwrap();
    let path = pool
        .managed_path(&StableId::parse(&snapshot.artifact_id).unwrap())
        .unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"invalid manifest").unwrap();
    assert!(
        f.service
            .register_ink_evidence(Fixture::request(&snapshot, "corrupted"))
            .unwrap_err()
            .contains("完整性校验")
    );
}

#[test]
fn upgrading_v14_recovers_existing_snapshot_request_bindings() {
    let mut f = Fixture::new("ink-v14-upgrade");
    f.append(f.trace[0].clone());
    let snapshot = f.snapshot("old-request");
    let c = f.connection();
    c.execute_batch("DROP TABLE ink_evidence_requests; DROP TABLE ink_snapshot_requests; DELETE FROM schema_migrations WHERE version=15; PRAGMA user_version=14;").unwrap();
    c.execute(
        "UPDATE artifact_revisions SET media_type='application/json' WHERE artifact_id=?1",
        [&snapshot.artifact_id],
    )
    .unwrap();
    drop(c);
    let upgraded = DesktopService::open_with_credential_store(
        &f.root,
        Arc::new(MemoryCredentialResolver::default()),
    )
    .unwrap();
    assert_eq!(
        upgraded
            .content_info(&snapshot.artifact_id)
            .unwrap()
            .ink_snapshot_id
            .as_deref(),
        Some(snapshot.snapshot_id.as_str())
    );
    assert_eq!(
        upgraded
            .create_ink_snapshot(CreateInkSnapshotRequest {
                request_id: "old-request".into(),
                document_id: f.document.document_id.clone(),
                expected_head: 1
            })
            .unwrap()
            .snapshot_id,
        snapshot.snapshot_id
    );
    assert_eq!(
        upgraded
            .register_ink_evidence(Fixture::request(&snapshot, "post-upgrade"))
            .unwrap()
            .outcome,
        MapObjectOutcome::Created
    );
}

#[test]
fn partial_erase_fragments_are_stable_evidence_even_after_undo() {
    let mut f = Fixture::new("ink-fragment-evidence");
    f.append(f.trace[0].clone());
    let InkLabOperation::Stroke { stroke, .. } = &f.trace[0] else {
        panic!("stroke fixture");
    };
    let mut fragment = stroke.clone();
    fragment.id = "fragment-a".into();
    fragment.points = fragment.points[1..].to_vec();
    f.append(InkLabOperation::PartialErase {
        seq: 2,
        command_id: "partial-erase-2".into(),
        source_stroke_id: "stroke-a".into(),
        fragments: vec![fragment],
    });
    let snapshot = f.snapshot("fragment-snapshot");
    assert!(
        f.service
            .register_ink_evidence(Fixture::request(&snapshot, "original-stroke"))
            .is_err()
    );
    let mut request = Fixture::request(&snapshot, "fragment-evidence");
    request.stroke_ids = vec!["fragment-a".into()];
    let receipt = f.service.register_ink_evidence(request).unwrap();
    let legacy_json = include_str!("../../../testdata/ink/p2/legacy-fragments.json");
    let mut pool = ArtifactPool::open(&f.root).unwrap();
    let imported = pool
        .import_bytes(
            legacy_json.as_bytes(),
            ArtifactMetadata::derived(
                MediaType::parse("application/json").unwrap(),
                Some("legacy-fragments.json".into()),
            )
            .unwrap(),
            ActorRef::parse("fixture").unwrap(),
        )
        .unwrap();
    let legacy_id = "mathscape:ink-snapshot/legacy-fragments";
    let connection = f.connection();
    connection.execute("INSERT INTO ink_snapshots(snapshot_id,request_id,document_id,through_seq,artifact_id,content_hash,byte_size,manifest_json,created_at) VALUES(?1,'legacy-fragments',?2,2,?3,?4,?5,?6,'unix:1791676800')",rusqlite::params![legacy_id,f.document.document_id,imported.stable_id().as_str(),imported.content_hash().as_str(),imported.byte_size() as i64,legacy_json]).unwrap();
    connection.execute("INSERT INTO ink_snapshot_requests(request_id,document_id,through_seq,manifest_json,created_at,snapshot_id) VALUES('legacy-fragments',?1,2,NULL,'unix:1791676800',?2)",rusqlite::params![f.document.document_id,legacy_id]).unwrap();
    let old = f
        .service
        .open_ink_snapshot(legacy_id.into())
        .unwrap()
        .unwrap();
    let mut old_request = Fixture::request(&old, "legacy-fragment-evidence");
    old_request.stroke_ids = vec!["fragment-a".into()];
    let old_receipt = f.service.register_ink_evidence(old_request).unwrap();
    assert_eq!(old_receipt.stroke_ids, vec!["fragment-a"]);
    assert_eq!(
        f.service
            .open_ink_snapshot(legacy_id.into())
            .unwrap()
            .unwrap()
            .manifest_json,
        legacy_json
    );
    f.append(InkLabOperation::Undo {
        seq: 3,
        command_id: "undo-fragment-erase".into(),
        target_command_id: "partial-erase-2".into(),
    });
    let detail = f.service.source_node_detail(&receipt.span_id).unwrap();
    assert_eq!(detail.ink.unwrap().stroke_ids, vec!["fragment-a"]);
    let list = f
        .service
        .list_ink_snapshots(f.document.document_id.clone())
        .unwrap();
    assert!(
        serde_json::to_value(list).unwrap()[0]
            .get("manifest_json")
            .is_none()
    );
    assert_eq!(
        f.connection()
            .query_row(
                "SELECT count(*) FROM ink_snapshot_requests WHERE manifest_json IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
