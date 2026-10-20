//! N3d search corpus: light rows only, one read transaction.
use mathscape_store_sqlite::{SearchSourceUnitRow, SqliteArtifactStore};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

fn insert_unit(connection: &Connection, id: &str, lifecycle: &str, payload: serde_json::Value) {
    connection
        .execute(
            "INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision)
             VALUES(?1,'entity','SourceSemanticUnit','rev-1')",
            [id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,created_at,created_by,generation_run,payload_json)
             VALUES(?1,'rev-1','1',?2,'2026-10-20T00:00:00Z','test',NULL,?3)",
            params![id, lifecycle, payload.to_string()],
        )
        .unwrap();
}

#[test]
fn corpus_reads_only_source_unit_names_not_bodies() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("mathscape-search-corpus-{nanos}"));
    fs::create_dir_all(&directory).unwrap();
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    // Stored payloads use the SourcePayload envelope.
    insert_unit(
        &connection,
        "mathscape:unit/a",
        "active",
        json!({"payload_kind":"semantic_unit","payload":{
            "kind":"definition",
            "display_title":"  一致连续 ",
            "source_title":{"text":"定义 2.1 一致连续性","source_span":"s"},
            "source_labels":[{"text":"定义 2.1","source_span":"s"},{"text":"  ","source_span":"s"}],
            "body":"这是正文，不应进入搜索语料"
        }}),
    );
    insert_unit(
        &connection,
        "mathscape:unit/b",
        "superseded",
        json!({"payload_kind":"semantic_unit","payload":{"kind":"proof"}}),
    );
    // Non-unit map objects are not part of this section.
    connection
        .execute(
            "INSERT INTO map_objects(object_id,object_shape,entity_kind,current_revision)
             VALUES('mathscape:span/x','entity','SourceSpan','rev-1')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO map_object_revisions(object_id,revision_id,schema_version,lifecycle,created_at,created_by,generation_run,payload_json)
             VALUES('mathscape:span/x','rev-1','1','active','t','test',NULL,'{\"display_title\":\"span\"}')",
            [],
        )
        .unwrap();
    drop(connection);

    let store = SqliteArtifactStore::open(&database).unwrap();
    let corpus = store.read_search_corpus().unwrap();
    assert!(corpus.notes.is_empty() && corpus.snapshots.is_empty() && corpus.artifacts.is_empty());
    assert_eq!(
        corpus.source_units,
        vec![
            SearchSourceUnitRow {
                object_id: "mathscape:unit/a".into(),
                lifecycle: "active".into(),
                unit_kind: Some("definition".into()),
                display_title: Some("一致连续".into()),
                source_title: Some("定义 2.1 一致连续性".into()),
                source_labels: vec!["定义 2.1".into()],
            },
            SearchSourceUnitRow {
                object_id: "mathscape:unit/b".into(),
                lifecycle: "superseded".into(),
                unit_kind: Some("proof".into()),
                display_title: None,
                source_title: None,
                source_labels: vec![],
            },
        ]
    );
    let serialized = serde_json::to_string(&corpus).unwrap();
    assert!(
        !serialized.contains("正文"),
        "bodies never enter the corpus"
    );
    drop(store);
    fs::remove_dir_all(directory).ok();
}
