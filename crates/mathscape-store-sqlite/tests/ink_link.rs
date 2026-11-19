//! N2 NoteLink storage: migration, idempotency, uniqueness, primary links and
//! titles, trash filtering and isolation from the source graph.
use mathscape_domain::{InkLabDocument, InkLabOperation};
use mathscape_store_sqlite::{
    InkCleanBlankRequest, InkContextCreateRequest, InkLifecycleRequest, InkLinkRequest,
    InkLinkRole, InkLinkTarget, InkLinkTargetStatus, InkMarkdownTarget, InkRenameRequest,
    InkSetPrimaryRequest, InkTitleMode, InkUnlinkRequest, SqliteArtifactStore, SqliteStoreError,
    ink_auto_title,
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn fixture(id: &str) -> InkLabDocument {
    let mut document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations.clear();
    document.document_id = id.into();
    document
}

fn operation(index: usize) -> InkLabOperation {
    let document: InkLabDocument =
        serde_json::from_str(include_str!("../../../testdata/ink/p0/trace.json")).unwrap();
    document.operations[index].clone()
}

fn test_directory(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("mathscape-ink-link-{name}-{nanos}"));
    fs::create_dir_all(&directory).unwrap();
    directory
}

const THEOREM: &str = "mathscape:source-unit/theorem-3-2";
const LEMMA: &str = "mathscape:source-unit/lemma-1";
const RETIRED: &str = "mathscape:source-unit/retired";
const ARTIFACT: &str = "mathscape:artifact/notes-md";
const PATCHED: &str = "mathscape:artifact/notes-md-patched";
const HASH: &str = "sha256:aaaa";

/// Seeds source objects, Markdown artifacts and a patch application with
/// raw SQL; the store only reads these tables for links.
fn seed(path: &Path) {
    let connection = Connection::open(path).unwrap();
    let payload = |label: &str| {
        json!({"payload": {"kind": "statement", "display_title": "介值定理",
               "source_labels": [{"text": label}]}})
        .to_string()
    };
    for (id, lifecycle, label) in [
        (THEOREM, "active", "定理 3.2"),
        (LEMMA, "active", "引理 1"),
        (RETIRED, "deprecated", "旧定理"),
    ] {
        connection
            .execute(
                "INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision)
                 VALUES(?1,'entity','SourceSemanticUnit','rev-1')",
                [id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,
                   created_at,created_by,generation_run,payload_json)
                 VALUES(?1,'rev-1','1',?2,'2026-01-01T00:00:00Z','test',NULL,?3)",
                rusqlite::params![id, lifecycle, payload(label)],
            )
            .unwrap();
    }
    for (id, hash) in [(ARTIFACT, HASH), (PATCHED, "sha256:bbbb")] {
        connection
            .execute(
                "INSERT INTO artifacts(artifact_id,content_hash,byte_size,current_revision)
                 VALUES(?1,?2,10,'r1')",
                [id, hash],
            )
            .unwrap();
    }
}

fn seed_patch(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    connection
        .execute(
            "INSERT INTO markdown_patch_applications(application_id,patch_set_id,base_artifact_id,
               patched_artifact_id,conversion_id,old_input_fingerprint,new_input_fingerprint,
               safety_json,status,error,created_at)
             VALUES('app-1','patch-1',?1,?2,NULL,'old','new','{}','applied',NULL,'now')",
            [ARTIFACT, PATCHED],
        )
        .unwrap();
}

fn source_graph_fingerprint(path: &Path) -> String {
    let connection = Connection::open(path).unwrap();
    let mut out = String::new();
    for table in [
        "map_objects",
        "map_object_revisions",
        "map_relation_endpoints",
    ] {
        let rows: Vec<String> = connection
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap()
            .query_map([], |row| {
                let count = row.as_ref().column_count();
                let mut values = Vec::new();
                for index in 0..count {
                    values.push(format!("{:?}", row.get_ref(index)?));
                }
                Ok(values.join("|"))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.push_str(&format!("{table}:{rows:?};"));
    }
    out
}

fn open(name: &str) -> (PathBuf, PathBuf, SqliteArtifactStore) {
    let directory = test_directory(name);
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    seed(&database);
    let store = SqliteArtifactStore::open(&database).unwrap();
    (directory, database, store)
}

fn create(store: &mut SqliteArtifactStore, id: &str) -> InkLabDocument {
    let document = fixture(id);
    store
        .create_ink_note(
            &document,
            &format!("{id}-session"),
            "desktop-local",
            &json!({"source": "desktop"}),
            InkTitleMode::Auto,
            None,
        )
        .unwrap();
    document
}

fn source(id: &str) -> InkLinkTarget {
    InkLinkTarget::SourceObject {
        object_id: id.into(),
    }
}

fn markdown(first: &str, last: &str) -> InkLinkTarget {
    let blocks: Vec<String> = if first == last {
        vec![first.into()]
    } else {
        vec![first.into(), last.into()]
    };
    InkLinkTarget::Markdown(InkMarkdownTarget {
        artifact_id: ARTIFACT.into(),
        content_hash: HASH.into(),
        first_block: first.into(),
        last_block: last.into(),
        block_ids: blocks,
        start: 0,
        end: 10,
        block_hash: "c".repeat(64),
        excerpt: "例 4 求极限".into(),
        label: "例 4".into(),
    })
}

fn link(
    request: &str,
    document: &str,
    target: InkLinkTarget,
    role: InkLinkRole,
    primary: bool,
) -> InkLinkRequest {
    InkLinkRequest {
        request_id: request.into(),
        document_id: document.into(),
        target,
        role,
        primary,
        replaces_link_id: None,
    }
}

fn context(
    request: &str,
    id: &str,
    target: InkLinkTarget,
    title: Option<&str>,
) -> InkContextCreateRequest {
    InkContextCreateRequest {
        request_id: request.into(),
        document: fixture(id),
        session_id: format!("{id}-session"),
        writer_id: "desktop-local".into(),
        device: json!({"source": "desktop"}),
        target,
        role: InkLinkRole::AltProof,
        title: title.map(str::to_owned),
    }
}

#[test]
fn create_from_context_is_atomic_and_idempotent() {
    let (directory, database, mut store) = open("context");
    let before = source_graph_fingerprint(&database);
    let request = context("ctx-1", "alt", source(THEOREM), None);
    let receipt = store.create_ink_note_from_context(&request).unwrap();
    assert!(!receipt.replayed);
    assert_eq!(receipt.note.title, "另证 · 定理 3.2");
    assert_eq!(receipt.note.title_mode, InkTitleMode::Auto);
    assert!(receipt.link.is_primary);
    assert_eq!(receipt.link.target_revision.as_deref(), Some("rev-1"));
    assert_eq!(receipt.session.document_id, "alt");

    let again = store.create_ink_note_from_context(&request).unwrap();
    assert!(again.replayed);
    assert_eq!(again.link.link_id, receipt.link.link_id);
    assert_eq!(store.list_ink_documents().unwrap().len(), 1);

    let mut changed = request.clone();
    changed.role = InkLinkRole::Check;
    assert!(matches!(
        store.create_ink_note_from_context(&changed),
        Err(SqliteStoreError::InkRequestConflict(_))
    ));

    // A rejected target leaves neither a note nor a link behind.
    let retired = context("ctx-2", "never", source(RETIRED), None);
    assert!(matches!(
        store.create_ink_note_from_context(&retired),
        Err(SqliteStoreError::InkLinkTargetRetired(_))
    ));
    assert!(store.ink_note("never").unwrap().is_none());
    let mut stale = context("ctx-3", "never", markdown("b1", "b1"), None);
    if let InkLinkTarget::Markdown(target) = &mut stale.target {
        target.content_hash = "sha256:other".into();
    }
    assert!(matches!(
        store.create_ink_note_from_context(&stale),
        Err(SqliteStoreError::InkLinkTargetInvalid(_))
    ));
    assert!(store.ink_note("never").unwrap().is_none());

    // Manual title wins over the automatic one.
    let manual = store
        .create_ink_note_from_context(&context(
            "ctx-4",
            "manual",
            markdown("b1", "b2"),
            Some(" 我的证明 "),
        ))
        .unwrap();
    assert_eq!(manual.note.title, "我的证明");
    assert_eq!(manual.note.title_mode, InkTitleMode::Manual);
    assert_eq!(manual.link.target_key, format!("md:{ARTIFACT}:b1:b2"));

    // Acceptance 6: links never touch source objects or their revisions.
    assert_eq!(source_graph_fingerprint(&database), before);
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn links_are_unique_idempotent_and_role_changes_are_audited() {
    let (directory, database, mut store) = open("unique");
    let before = source_graph_fingerprint(&database);
    create(&mut store, "note");
    let first = store
        .link_ink_note(&link(
            "l1",
            "note",
            source(THEOREM),
            InkLinkRole::Attempt,
            false,
        ))
        .unwrap();
    let link_id = first.link.clone().unwrap().link_id;
    assert!(!first.existing && !first.title_changed);
    assert!(
        store
            .link_ink_note(&link(
                "l1",
                "note",
                source(THEOREM),
                InkLinkRole::Attempt,
                false
            ))
            .unwrap()
            .replayed
    );
    assert!(matches!(
        store.link_ink_note(&link(
            "l1",
            "note",
            source(LEMMA),
            InkLinkRole::Attempt,
            false
        )),
        Err(SqliteStoreError::InkRequestConflict(_))
    ));
    // Duplicate with a new request returns the existing link.
    let duplicate = store
        .link_ink_note(&link(
            "l2",
            "note",
            source(THEOREM),
            InkLinkRole::Attempt,
            false,
        ))
        .unwrap();
    assert!(duplicate.existing);
    assert_eq!(duplicate.link.unwrap().link_id, link_id);
    // Another role on the same target is a separate link.
    store
        .link_ink_note(&link(
            "l3",
            "note",
            source(THEOREM),
            InkLinkRole::Question,
            false,
        ))
        .unwrap();
    assert_eq!(store.ink_note_links("note").unwrap().len(), 2);

    // Role change = remove old + add new, atomically.
    let mut change = link("l4", "note", source(THEOREM), InkLinkRole::Check, false);
    change.replaces_link_id = Some(link_id.clone());
    let changed = store.link_ink_note(&change).unwrap();
    assert_eq!(changed.link.unwrap().role, InkLinkRole::Check);
    let roles: Vec<_> = store
        .ink_note_links("note")
        .unwrap()
        .into_iter()
        .map(|link| link.role)
        .collect();
    assert_eq!(roles.len(), 2);
    assert!(!roles.contains(&InkLinkRole::Attempt));
    let removed: (String, Option<i64>) = Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT state,removed_at FROM ink_note_links WHERE link_id=?1",
            [&link_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(removed.0, "removed");
    assert!(removed.1.is_some());

    // Self links and unknown targets are rejected.
    assert!(matches!(
        store.link_ink_note(&link(
            "l5",
            "note",
            InkLinkTarget::InkNote {
                document_id: "note".into()
            },
            InkLinkRole::Reference,
            false
        )),
        Err(SqliteStoreError::InkLinkTargetInvalid(_))
    ));
    assert!(matches!(
        store.link_ink_note(&link(
            "l6",
            "note",
            source("mathscape:source-unit/none"),
            InkLinkRole::Reference,
            false
        )),
        Err(SqliteStoreError::InkLinkTargetInvalid(_))
    ));

    // Unlink is soft and replayable; unknown links are reported.
    let any = store.ink_note_links("note").unwrap()[0].link_id.clone();
    let unlink = InkUnlinkRequest {
        request_id: "u1".into(),
        document_id: "note".into(),
        link_id: any.clone(),
    };
    assert_eq!(
        store.unlink_ink_note(&unlink).unwrap().link.unwrap().state,
        "removed"
    );
    assert!(store.unlink_ink_note(&unlink).unwrap().replayed);
    assert!(matches!(
        store.unlink_ink_note(&InkUnlinkRequest {
            request_id: "u2".into(),
            document_id: "note".into(),
            link_id: "link-missing".into(),
        }),
        Err(SqliteStoreError::InkLinkNotFound(_))
    ));
    // The database-level unique indexes hold too.
    let connection = Connection::open(&database).unwrap();
    let survivor = store.ink_note_links("note").unwrap()[0].clone();
    assert!(connection
        .execute(
            "INSERT INTO ink_note_links SELECT 'dup',document_id,target_kind,target_key,target_id,
               target_json,target_revision,target_label,role,0,'active',1,NULL
             FROM ink_note_links WHERE link_id=?1",
            [&survivor.link_id],
        )
        .is_err());
    assert_eq!(source_graph_fingerprint(&database), before);
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn primary_links_drive_auto_titles_only() {
    let (directory, _database, mut store) = open("primary");
    let document = create(&mut store, "note");
    let time_title = ink_auto_title(&document.created_at).unwrap();
    assert_eq!(store.ink_note("note").unwrap().unwrap().title, time_title);

    let a = store
        .link_ink_note(&link(
            "p1",
            "note",
            source(THEOREM),
            InkLinkRole::AltProof,
            true,
        ))
        .unwrap();
    assert!(a.title_changed);
    assert_eq!(a.note.title, "另证 · 定理 3.2");
    // A non-primary link never retitles.
    let b = store
        .link_ink_note(&link(
            "p2",
            "note",
            source(LEMMA),
            InkLinkRole::Check,
            false,
        ))
        .unwrap();
    assert!(!b.title_changed);
    assert_eq!(b.note.title, "另证 · 定理 3.2");
    let b_id = b.link.unwrap().link_id;
    // Switching the primary retitles and leaves exactly one primary.
    let switched = store
        .set_ink_note_primary_link(&InkSetPrimaryRequest {
            request_id: "p3".into(),
            document_id: "note".into(),
            link_id: Some(b_id.clone()),
        })
        .unwrap();
    assert_eq!(switched.note.title, "验算 · 引理 1");
    let primaries = store
        .ink_note_links("note")
        .unwrap()
        .into_iter()
        .filter(|link| link.is_primary)
        .count();
    assert_eq!(primaries, 1);
    assert_eq!(store.ink_note_links("note").unwrap()[0].link_id, b_id);

    // Linking with primary=true replaces the current primary.
    let markdown_primary = store
        .link_ink_note(&link(
            "p4",
            "note",
            markdown("b9", "b9"),
            InkLinkRole::Study,
            true,
        ))
        .unwrap();
    assert_eq!(markdown_primary.note.title, "学习笔记 · 例 4");

    // Unlinking the primary goes back to the time title.
    let primary_id = markdown_primary.link.unwrap().link_id;
    let unlinked = store
        .unlink_ink_note(&InkUnlinkRequest {
            request_id: "p5".into(),
            document_id: "note".into(),
            link_id: primary_id,
        })
        .unwrap();
    assert!(unlinked.title_changed);
    assert_eq!(unlinked.note.title, time_title);

    // Manual titles are never rewritten; clearing the title restores the
    // primary-based automatic one.
    store
        .set_ink_note_primary_link(&InkSetPrimaryRequest {
            request_id: "p6".into(),
            document_id: "note".into(),
            link_id: Some(b_id.clone()),
        })
        .unwrap();
    let revision = store.ink_note("note").unwrap().unwrap().title_revision;
    store
        .rename_ink_note(&InkRenameRequest {
            request_id: "r1".into(),
            document_id: "note".into(),
            title: "我的验算".into(),
            expected_title_revision: revision,
        })
        .unwrap();
    let cleared = store
        .set_ink_note_primary_link(&InkSetPrimaryRequest {
            request_id: "p7".into(),
            document_id: "note".into(),
            link_id: None,
        })
        .unwrap();
    assert!(!cleared.title_changed);
    assert_eq!(cleared.note.title, "我的验算");
    store
        .set_ink_note_primary_link(&InkSetPrimaryRequest {
            request_id: "p8".into(),
            document_id: "note".into(),
            link_id: Some(b_id),
        })
        .unwrap();
    let revision = store.ink_note("note").unwrap().unwrap().title_revision;
    let restored = store
        .rename_ink_note(&InkRenameRequest {
            request_id: "r2".into(),
            document_id: "note".into(),
            title: "  ".into(),
            expected_title_revision: revision,
        })
        .unwrap();
    assert_eq!(restored.note.title, "验算 · 引理 1");
    assert_eq!(restored.note.title_mode, InkTitleMode::Auto);
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn trash_hides_links_and_rejects_link_changes() {
    let (directory, database, mut store) = open("trash");
    let ctx = store
        .create_ink_note_from_context(&context("t1", "alt", source(THEOREM), None))
        .unwrap();
    store
        .create_ink_note_from_context(&context("t2", "md", markdown("b1", "b1"), None))
        .unwrap();
    assert_eq!(
        store
            .ink_links_to_target("source_object", THEOREM)
            .unwrap()
            .len(),
        1
    );
    let overlay = store.ink_link_overlay().unwrap();
    assert_eq!(overlay.counts.len(), 2);
    assert_eq!(overlay.links.len(), 2);

    let note = ctx.note;
    store
        .trash_ink_note(&InkLifecycleRequest {
            request_id: "trash".into(),
            document_id: "alt".into(),
            expected_lifecycle_revision: note.lifecycle_revision,
        })
        .unwrap();
    assert!(
        store
            .ink_links_to_target("source_object", THEOREM)
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.ink_link_overlay().unwrap().counts.len(), 1);
    assert!(matches!(
        store.link_ink_note(&link("t3", "alt", source(LEMMA), InkLinkRole::Check, false)),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    assert!(matches!(
        store.unlink_ink_note(&InkUnlinkRequest {
            request_id: "t4".into(),
            document_id: "alt".into(),
            link_id: ctx.link.link_id.clone(),
        }),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    assert!(matches!(
        store.set_ink_note_primary_link(&InkSetPrimaryRequest {
            request_id: "t5".into(),
            document_id: "alt".into(),
            link_id: None,
        }),
        Err(SqliteStoreError::InkDocumentTrashed(_))
    ));
    // Links into a trashed note are rejected and existing ones are labelled.
    create(&mut store, "other");
    assert!(matches!(
        store.link_ink_note(&link(
            "t6",
            "other",
            InkLinkTarget::InkNote {
                document_id: "alt".into()
            },
            InkLinkRole::Reference,
            false
        )),
        Err(SqliteStoreError::InkLinkTargetRetired(_))
    ));

    let trashed = store.ink_note("alt").unwrap().unwrap();
    store
        .restore_ink_note(&InkLifecycleRequest {
            request_id: "restore".into(),
            document_id: "alt".into(),
            expected_lifecycle_revision: trashed.lifecycle_revision,
        })
        .unwrap();
    assert_eq!(
        store
            .ink_links_to_target("source_object", THEOREM)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.ink_link_overlay().unwrap().counts.len(), 2);
    store
        .link_ink_note(&link(
            "t7",
            "other",
            InkLinkTarget::InkNote {
                document_id: "alt".into(),
            },
            InkLinkRole::Reference,
            false,
        ))
        .unwrap();
    let trashed = store.ink_note("alt").unwrap().unwrap();
    store
        .trash_ink_note(&InkLifecycleRequest {
            request_id: "trash-2".into(),
            document_id: "alt".into(),
            expected_lifecycle_revision: trashed.lifecycle_revision,
        })
        .unwrap();
    assert_eq!(
        store.ink_note_links("other").unwrap()[0].target_status,
        InkLinkTargetStatus::Trashed
    );
    drop(store);
    drop(database);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn retired_targets_are_shown_and_can_be_unlinked() {
    let (directory, database, mut store) = open("retired");
    create(&mut store, "note");
    store
        .link_ink_note(&link(
            "r1",
            "note",
            source(THEOREM),
            InkLinkRole::Attempt,
            true,
        ))
        .unwrap();
    // The object is retired later by a source edit.
    Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE map_object_revisions SET lifecycle='deprecated' WHERE object_id=?1",
            [THEOREM],
        )
        .unwrap();
    let links = store.ink_note_links("note").unwrap();
    assert_eq!(links[0].target_status, InkLinkTargetStatus::Retired);
    assert_eq!(links[0].target_label, "定理 3.2");
    store
        .unlink_ink_note(&InkUnlinkRequest {
            request_id: "r2".into(),
            document_id: "note".into(),
            link_id: links[0].link_id.clone(),
        })
        .unwrap();
    assert!(store.ink_note_links("note").unwrap().is_empty());
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn blank_notes_have_no_links_and_clean_blank_skips_linked_ones() {
    let (directory, _database, mut store) = open("blank");
    create(&mut store, "plain");
    store
        .create_ink_note_from_context(&context("b1", "from-context", source(THEOREM), None))
        .unwrap();
    create(&mut store, "target");
    create(&mut store, "linker");
    store
        .link_ink_note(&link(
            "b2",
            "linker",
            InkLinkTarget::InkNote {
                document_id: "target".into(),
            },
            InkLinkRole::Reference,
            false,
        ))
        .unwrap();
    let impact = store.ink_trash_impact("from-context").unwrap().unwrap();
    assert_eq!(impact.active_link_count, 1);
    assert!(!impact.blank);
    assert!(!store.ink_trash_impact("target").unwrap().unwrap().blank);
    assert!(store.ink_trash_impact("plain").unwrap().unwrap().blank);
    let receipt = store
        .clean_blank_ink_notes(&InkCleanBlankRequest {
            request_id: "clean".into(),
            document_ids: vec![
                "plain".into(),
                "from-context".into(),
                "target".into(),
                "linker".into(),
            ],
        })
        .unwrap();
    let trashed: Vec<_> = receipt
        .trashed
        .iter()
        .map(|note| note.document_id.as_str())
        .collect();
    assert_eq!(trashed, ["plain"]);
    assert!(
        receipt
            .skipped
            .iter()
            .all(|skip| skip.reason == "has_links")
    );
    assert_eq!(receipt.skipped.len(), 3);
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn patch_hint_counts_notes_on_the_original_only() {
    let (directory, database, mut store) = open("patch");
    seed_patch(&database);
    store
        .create_ink_note_from_context(&context("h1", "one", markdown("b1", "b1"), None))
        .unwrap();
    store
        .create_ink_note_from_context(&context("h2", "two", markdown("b2", "b2"), None))
        .unwrap();
    assert_eq!(store.ink_markdown_patch_hint(PATCHED).unwrap(), 2);
    assert_eq!(store.ink_markdown_patch_hint(ARTIFACT).unwrap(), 0);
    assert!(
        store
            .ink_links_to_target("markdown", PATCHED)
            .unwrap()
            .is_empty()
    );
    drop(store);
    fs::remove_dir_all(directory).ok();
}

#[test]
fn upgrading_v17_rebuilds_requests_and_keeps_them_replayable() {
    let directory = test_directory("upgrade");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let document = fixture("old");
    store
        .create_ink_note(
            &document,
            "old-session",
            "desktop-local",
            &json!({}),
            InkTitleMode::Auto,
            Some("create-old"),
        )
        .unwrap();
    store
        .append_ink_operation(
            "op-old",
            "old",
            "old-session",
            1,
            0,
            &document.metadata(),
            &operation(0),
        )
        .unwrap();
    drop(store);
    // Recreate the v17 shape: the N1 request table and no links.
    Connection::open(&database)
        .unwrap()
        .execute_batch(
            "DROP TABLE ink_note_links;
             CREATE TABLE ink_note_requests_v17 (
               request_id TEXT PRIMARY KEY NOT NULL,
               kind TEXT NOT NULL CHECK (kind IN ('create','rename','trash','restore','clean_blank')),
               document_id TEXT, request_fingerprint TEXT NOT NULL,
               response_json TEXT NOT NULL CHECK (json_valid(response_json)),
               created_at INTEGER NOT NULL) STRICT;
             INSERT INTO ink_note_requests_v17 SELECT * FROM ink_note_requests;
             DROP TABLE ink_note_requests;
             ALTER TABLE ink_note_requests_v17 RENAME TO ink_note_requests;
             CREATE INDEX ink_note_requests_by_document ON ink_note_requests(document_id, created_at);
             DELETE FROM schema_migrations WHERE version IN (18,19);
             ALTER TABLE ink_lan_settings DROP COLUMN preferred_host;
             ALTER TABLE ink_pairings DROP COLUMN claim_reason;
             PRAGMA user_version=17;",
        )
        .unwrap();
    assert!(matches!(
        SqliteArtifactStore::open(&database),
        Err(SqliteStoreError::UnsupportedSchemaVersion { actual: 17, .. })
    ));
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let replay = store
        .create_ink_note(
            &document,
            "old-session",
            "desktop-local",
            &json!({}),
            InkTitleMode::Auto,
            Some("create-old"),
        )
        .unwrap();
    assert!(replay.replayed);
    assert!(store.ink_note_links("old").unwrap().is_empty());
    drop(store);
    seed(&database);
    let mut store = SqliteArtifactStore::open(&database).unwrap();
    store
        .link_ink_note(&link(
            "new-link",
            "old",
            source(THEOREM),
            InkLinkRole::Attempt,
            false,
        ))
        .unwrap();
    drop(store);
    // Guard path: migration 18 recorded but user_version left behind.
    Connection::open(&database)
        .unwrap()
        .execute_batch("PRAGMA user_version=17;")
        .unwrap();
    let store = SqliteArtifactStore::prepare(&database).unwrap();
    assert_eq!(store.ink_note_links("old").unwrap().len(), 1);
    drop(store);
    let version: i64 = Connection::open(&database)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 26);
    fs::remove_dir_all(directory).ok();
}

/// PR #4 regression (D12): a role change onto a (target, role) link that
/// already exists merges into it atomically, and "设为主关联" on a duplicate
/// promotes it; neither may report success while changing nothing.
#[test]
fn role_change_onto_an_existing_role_merges_and_transfers_the_primary() {
    let (directory, database, mut store) = open("merge");
    let before = source_graph_fingerprint(&database);
    create(&mut store, "note");
    create(&mut store, "other");
    let attempt = store
        .link_ink_note(&link(
            "m1",
            "note",
            source(THEOREM),
            InkLinkRole::Attempt,
            true,
        ))
        .unwrap()
        .link
        .unwrap();
    let question = store
        .link_ink_note(&link(
            "m2",
            "note",
            source(THEOREM),
            InkLinkRole::Question,
            false,
        ))
        .unwrap()
        .link
        .unwrap();
    assert_eq!(
        store.ink_note("note").unwrap().unwrap().title,
        "尝试 · 定理 3.2"
    );

    // 尝试 (primary) → 疑问, while a 疑问 link to the same target exists.
    let mut change = link("m3", "note", source(THEOREM), InkLinkRole::Question, true);
    change.replaces_link_id = Some(attempt.link_id.clone());
    let merged = store.link_ink_note(&change).unwrap();
    assert!(merged.existing && merged.merged && merged.primary_changed && merged.title_changed);
    let kept = merged.link.clone().unwrap();
    assert_eq!(kept.link_id, question.link_id);
    assert!(kept.is_primary);
    assert_eq!(merged.note.title, "疑问 · 定理 3.2");
    let links = store.ink_note_links("note").unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(
        (links[0].role, links[0].is_primary),
        (InkLinkRole::Question, true)
    );
    let state: String = Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT state FROM ink_note_links WHERE link_id=?1",
            [&attempt.link_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "removed");
    // The replay returns the same merge receipt and changes nothing more.
    let replay = store.link_ink_note(&change).unwrap();
    assert!(replay.replayed && replay.merged);
    assert_eq!(store.ink_note_links("note").unwrap().len(), 1);

    // A non-primary role change onto an existing link merges without
    // touching the primary.
    let check = store
        .link_ink_note(&link(
            "m4",
            "note",
            source(LEMMA),
            InkLinkRole::Check,
            false,
        ))
        .unwrap()
        .link
        .unwrap();
    let reference = store
        .link_ink_note(&link(
            "m5",
            "note",
            source(LEMMA),
            InkLinkRole::Reference,
            false,
        ))
        .unwrap()
        .link
        .unwrap();
    let mut change = link("m6", "note", source(LEMMA), InkLinkRole::Reference, false);
    change.replaces_link_id = Some(check.link_id.clone());
    let merged = store.link_ink_note(&change).unwrap();
    assert!(merged.merged && !merged.primary_changed && !merged.title_changed);
    assert_eq!(merged.link.unwrap().link_id, reference.link_id);
    assert_eq!(store.ink_note_links("note").unwrap().len(), 2);

    // "设为主关联" on an existing duplicate promotes it.
    let promoted = store
        .link_ink_note(&link(
            "m7",
            "note",
            source(LEMMA),
            InkLinkRole::Reference,
            true,
        ))
        .unwrap();
    assert!(promoted.existing && !promoted.merged && promoted.primary_changed);
    assert!(promoted.link.unwrap().is_primary);
    assert_eq!(promoted.note.title, "参考 · 引理 1");
    let primaries: Vec<_> = store
        .ink_note_links("note")
        .unwrap()
        .into_iter()
        .filter(|link| link.is_primary)
        .collect();
    assert_eq!(primaries.len(), 1);
    // A plain duplicate still reports "existing" and changes nothing.
    let plain = store
        .link_ink_note(&link(
            "m8",
            "note",
            source(LEMMA),
            InkLinkRole::Reference,
            false,
        ))
        .unwrap();
    assert!(plain.existing && !plain.merged && !plain.primary_changed && !plain.title_changed);
    assert!(plain.link.unwrap().is_primary);

    // A role change onto a role whose link was removed earlier creates a new
    // active link (removed links never block).
    let mut back = link("m9", "note", source(THEOREM), InkLinkRole::Attempt, false);
    back.replaces_link_id = Some(question.link_id.clone());
    let recreated = store.link_ink_note(&back).unwrap();
    assert!(!recreated.existing && !recreated.merged);
    let recreated = recreated.link.unwrap();
    assert_ne!(recreated.link_id, attempt.link_id);
    assert_eq!(recreated.role, InkLinkRole::Attempt);

    // Another note may hold the same target and role.
    let other = store
        .link_ink_note(&link(
            "m10",
            "other",
            source(LEMMA),
            InkLinkRole::Reference,
            true,
        ))
        .unwrap();
    assert!(!other.existing);
    assert_eq!(source_graph_fingerprint(&database), before);
    drop(store);
    let _ = std::fs::remove_dir_all(directory);
}
