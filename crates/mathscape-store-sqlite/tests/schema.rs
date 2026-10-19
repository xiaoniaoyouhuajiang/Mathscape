use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_domain::{
    ActorRef, ContentHash, DiscoveryAdmission, DiscoveryUnitKind, StableId, Timestamp,
    UnitCompleteness,
};
use mathscape_llm::{
    CapabilitySupport, CredentialRef, LLM_PROFILE_VERSION, LlmCapabilities, LlmProfile,
    WireProtocol,
};
use mathscape_store_sqlite::{
    AcceptedRegistrationPlanRecord, DiscoveryAttemptCheckpoint, DiscoveryCheckpointStatus,
    DiscoveryDiagnosticCheckpoint, DiscoveryFactCheckpointRecord, DiscoveryRunCheckpoint,
    DiscoveryUnitCheckpointRecord, DiscoveryValidationCheckpoint, PendingDiscoveryTask,
    SqliteArtifactStore, SqliteStoreError, StoredAcceptanceAuthority,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_directory(case: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".test-runs")
        .join(format!(
            "{case}-{timestamp}-{}-{sequence}",
            std::process::id()
        ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let parent = path.parent().unwrap().to_owned();
    fs::remove_dir_all(path).unwrap();
    let _ = fs::remove_dir(parent);
}

#[test]
fn ordinary_open_never_creates_or_upgrades_a_catalog() {
    let directory = test_directory("connect-only");
    let database = directory.join("catalog.sqlite3");
    assert!(SqliteArtifactStore::open(&database).is_err());
    assert!(!database.exists());
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0001_artifact_pool.sql"))
        .unwrap();
    assert!(matches!(
        SqliteArtifactStore::open(&database),
        Err(SqliteStoreError::UnsupportedSchemaVersion { actual: 1, .. })
    ));
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    drop(connection);
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    assert_eq!(
        SqliteArtifactStore::open(&database)
            .unwrap()
            .artifact_count()
            .unwrap(),
        0
    );
    cleanup(&directory);
}

#[test]
fn ordinary_open_does_not_repair_a_historical_v6_draft() {
    let directory = test_directory("prepare-only-reconciliation");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch("DROP INDEX discovery_acceptance_by_run;")
        .unwrap();
    drop(SqliteArtifactStore::open(&database).unwrap());
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'discovery_acceptance_by_run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'discovery_acceptance_by_run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(connection);
    cleanup(&directory);
}

#[test]
fn healthy_catalog_opens_concurrently_while_another_connection_holds_the_writer_lock() {
    let directory = test_directory("concurrent-read-open");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let writer = Connection::open(&database).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE;").unwrap();
    // Model independent PDF range requests. They must not need the writer slot.
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let database = &database;
                scope.spawn(move || {
                    let store = SqliteArtifactStore::open(database).unwrap();
                    assert_eq!(store.artifact_count().unwrap(), 0);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    writer.execute_batch("ROLLBACK;").unwrap();
    drop(writer);
    cleanup(&directory);
}

#[test]
fn creates_and_reopens_the_versioned_artifact_schema() {
    let directory = test_directory("migration");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());

    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 19);
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 13",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "ink_p1"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 14",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "ink_snapshots_p2"
    );
    let migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(migration, "artifact_pool_v1");
    let llm_migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(llm_migration, "llm_profiles_v1");
    let settings_migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 3",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(settings_migration, "workspace_settings_v1");
    let discovery_migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 4",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(discovery_migration, "discovery_runs_v1");
    let source_map_migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 5",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(source_map_migration, "source_map_v1");
    let discovery_v2_migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 6",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(discovery_v2_migration, "discovery_runs_v2");
    let result_fingerprint_column = connection
        .prepare("PRAGMA table_info(discovery_runs)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .any(|name| name == "result_fingerprint");
    assert!(result_fingerprint_column);
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 8",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "markdown_patches"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 9",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "discovery_patch_review"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 10",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "discovery_run_recovery"
    );
    for table in [
        "markdown_patch_sets",
        "markdown_patch_operations",
        "markdown_patch_decisions",
        "markdown_patch_applications",
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing Markdown patch table {table}"
        );
    }
    drop(connection);
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    cleanup(&directory);
}

/// SQLite only reports a malformed foreign key ("foreign key mismatch") when a
/// row is written, so check every declared parent key against the schema.
#[test]
fn every_foreign_key_targets_a_primary_or_unique_key() {
    let directory = test_directory("foreign-keys");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();

    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let key_sets = |table: &str| -> Vec<Vec<String>> {
        let mut primary: Vec<(i64, String)> = connection
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, i64>(5)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .filter(|(position, _)| *position > 0)
            .collect();
        primary.sort();
        let mut sets = vec![primary.into_iter().map(|(_, name)| name).collect()];
        let unique: Vec<String> = connection
            .prepare(&format!("PRAGMA index_list(\"{table}\")"))
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .filter(|(_, unique)| *unique == 1)
            .map(|(name, _)| name)
            .collect();
        for index in unique {
            sets.push(
                connection
                    .prepare(&format!("PRAGMA index_info(\"{index}\")"))
                    .unwrap()
                    .query_map([], |row| row.get::<_, String>(2))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect(),
            );
        }
        sets
    };

    for table in &tables {
        let mut keys: Vec<(i64, String, Option<String>)> = connection
            .prepare(&format!("PRAGMA foreign_key_list(\"{table}\")"))
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(2)?, row.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        keys.sort_by_key(|(id, _, _)| *id);
        let mut grouped: Vec<(String, Vec<Option<String>>)> = Vec::new();
        let mut last = None;
        for (id, parent, column) in keys {
            if last != Some(id) {
                grouped.push((parent, Vec::new()));
                last = Some(id);
            }
            grouped.last_mut().unwrap().1.push(column);
        }
        for (parent, columns) in grouped {
            let parent_keys = key_sets(&parent);
            let matches = if columns.iter().all(Option::is_none) {
                !parent_keys[0].is_empty()
            } else {
                let mut wanted: Vec<String> = columns.into_iter().flatten().collect();
                wanted.sort();
                parent_keys.into_iter().any(|mut key| {
                    key.sort();
                    key == wanted
                })
            };
            assert!(
                matches,
                "{table} has a foreign key that is not a key of {parent}"
            );
        }
    }
    drop(connection);
    cleanup(&directory);
}

#[test]
fn refuses_to_downgrade_an_unknown_future_schema() {
    let directory = test_directory("future-schema");
    let database = directory.join("catalog.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);

    assert!(matches!(
        SqliteArtifactStore::prepare(&database),
        Err(SqliteStoreError::UnsupportedSchemaVersion {
            actual: 99,
            supported: 19
        })
    ));
    cleanup(&directory);
}

#[test]
fn upgrades_an_existing_artifact_pool_database_through_workspace_settings() {
    let directory = test_directory("upgrade-v1");
    let database = directory.join("catalog.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0001_artifact_pool.sql"))
        .unwrap();
    drop(connection);

    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 19);
    let migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(migration, "llm_profiles_v1");
    let migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 3",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(migration, "workspace_settings_v1");
    cleanup(&directory);
}

#[test]
fn upgrades_an_llm_profile_database_to_workspace_settings() {
    let directory = test_directory("upgrade-v2");
    let database = directory.join("catalog.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0001_artifact_pool.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0002_llm_profiles.sql"))
        .unwrap();
    drop(connection);

    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 19);
    let migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 3",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(migration, "workspace_settings_v1");
    cleanup(&directory);
}

#[test]
fn upgrades_workspace_settings_database_to_discovery_runs() {
    let directory = test_directory("upgrade-v3");
    let database = directory.join("catalog.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0001_artifact_pool.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0002_llm_profiles.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0003_workspace_settings.sql"
        ))
        .unwrap();
    drop(connection);

    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        19
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 4",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "discovery_runs_v1"
    );
    cleanup(&directory);
}

#[test]
fn repairs_a_v5_discovery_schema_missing_result_fingerprint() {
    let directory = test_directory("repair-v5-discovery-schema");
    let database = directory.join("catalog.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0001_artifact_pool.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0002_llm_profiles.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0003_workspace_settings.sql"
        ))
        .unwrap();
    let legacy_discovery_migration = include_str!("../../../migrations/0004_discovery_runs.sql")
        .replace("    result_fingerprint     TEXT,\n", "");
    connection
        .execute_batch(&legacy_discovery_migration)
        .unwrap();
    connection
        .execute_batch(
            "DROP INDEX discovery_acceptance_by_run;
             DROP TABLE discovery_acceptance_decisions;",
        )
        .unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0005_source_map.sql"))
        .unwrap();
    drop(connection);

    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        19
    );
    let has_result_fingerprint = connection
        .prepare("PRAGMA table_info(discovery_runs)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .any(|name| name == "result_fingerprint");
    assert!(has_result_fingerprint);
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name = 'discovery_acceptance_decisions'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "discovery_acceptance_decisions"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'index' AND name = 'discovery_acceptance_by_run'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "discovery_acceptance_by_run"
    );
    cleanup(&directory);
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FixtureSettings {
    version: String,
    timeout_seconds: u64,
}

#[test]
fn persists_and_replaces_a_versioned_workspace_settings_document() {
    let directory = test_directory("workspace-settings");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    assert_eq!(store.workspace_settings::<FixtureSettings>().unwrap(), None);

    let mut settings = FixtureSettings {
        version: "fixture-settings-1".into(),
        timeout_seconds: 600,
    };
    store.save_workspace_settings(&settings).unwrap();
    settings.timeout_seconds = 900;
    store.save_workspace_settings(&settings).unwrap();
    drop(store);

    let store = SqliteArtifactStore::prepare(&database).unwrap();
    assert_eq!(
        store.workspace_settings::<FixtureSettings>().unwrap(),
        Some(settings)
    );
    let connection = Connection::open(&database).unwrap();
    let count = connection
        .query_row("SELECT COUNT(*) FROM workspace_settings", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    cleanup(&directory);
}

#[test]
fn persists_versioned_llm_profiles_without_secret_material() {
    let directory = test_directory("llm-profile");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let profile = LlmProfile {
        display_name: None,
        version: LLM_PROFILE_VERSION.to_string(),
        id: "research-default".to_string(),
        provider: "custom-relay".to_string(),
        wire_protocol: WireProtocol::OpenAiChatCompletions,
        model: "fixture-model".to_string(),
        endpoint: Some(Url::parse("https://relay.example/v1/").unwrap()),
        credential: Some(CredentialRef::new("workspace:research-default").unwrap()),
        capabilities: LlmCapabilities {
            structured_output: CapabilitySupport::Supported,
            ..LlmCapabilities::default()
        },
    };
    let stored = store.upsert_llm_profile(&profile, true).unwrap();
    assert!(stored.is_default());
    drop(store);

    let store = SqliteArtifactStore::prepare(&database).unwrap();
    let reopened = store.default_llm_profile().unwrap().unwrap();
    assert_eq!(reopened.profile(), &profile);
    let bytes = fs::read(&database).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("fixture-secret"));
    cleanup(&directory);
}

fn hash(digit: char) -> ContentHash {
    ContentHash::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
}

fn insert_fixture_artifact(database: &Path, artifact_id: &StableId, content_hash: &ContentHash) {
    let connection = Connection::open(database).unwrap();
    connection
        .execute(
            "INSERT INTO artifacts (artifact_id, content_hash, byte_size, current_revision)
             VALUES (?1, ?2, 128, ?3)",
            [
                artifact_id.as_str(),
                content_hash.as_str(),
                hash('f').as_str(),
            ],
        )
        .unwrap();
}

fn discovery_checkpoint(run_id: StableId, artifact_id: StableId) -> DiscoveryRunCheckpoint {
    DiscoveryRunCheckpoint {
        run_id,
        artifact_id,
        conversion_id: Some(StableId::parse("mathscape:conversion/fixture").unwrap()),
        pipeline_version: "mathscape-discovery-pipeline-2".into(),
        input_fingerprint: hash('1'),
        selection_fingerprint: hash('2'),
        unit_set_fingerprint: Some(hash('3')),
        result_fingerprint: Some(hash('4')),
        status: DiscoveryCheckpointStatus::Partial,
        admission: DiscoveryAdmission::ReviewBeforePublish,
        profile_id: "fixture-profile".into(),
        snapshot: json!({"status":"partial"}),
        attempts: vec![DiscoveryAttemptCheckpoint {
            stage: "unit-discovery".into(),
            batch_fingerprint: None,
            repair: false,
            status: DiscoveryCheckpointStatus::Completed,
            request: json!({"task":"unit-discovery"}),
            raw_text: Some("{\"units\":[]}".into()),
            run: json!({"profile_id":"fixture-profile"}),
            error: None,
        }],
        reports: vec![DiscoveryValidationCheckpoint {
            report: json!({"diagnostics":[{"code":"ambiguous_quote"}]}),
            diagnostics: vec![DiscoveryDiagnosticCheckpoint {
                code: "ambiguous_quote".into(),
                path: "tasks.t1.references[0]".into(),
                message: "2 matches".into(),
                checked: true,
            }],
        }],
        units: vec![DiscoveryUnitCheckpointRecord {
            unit_ref: "u1".into(),
            parent_ref: None,
            unit_kind: DiscoveryUnitKind::Statement,
            completeness: UnitCompleteness::Complete,
            payload: json!({"ref":"u1","selectors":[{"start":0,"end":20}]}),
        }],
        facts: vec![DiscoveryFactCheckpointRecord {
            task_ref: "t1".into(),
            owner_unit_ref: "u1".into(),
            processed: true,
            requires_unit_revision: false,
            payload: json!({"task_ref":"t1","owner":"u1"}),
        }],
        pending_tasks: vec![PendingDiscoveryTask {
            task_ref: "t2".into(),
            owner_unit_ref: Some("u1".into()),
        }],
    }
}

#[test]
fn atomically_persists_and_replaces_discovery_checkpoints() {
    let directory = test_directory("discovery-checkpoint");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/discovery-fixture").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('a'));

    let run_id = StableId::parse("mathscape:discovery-run/fixture").unwrap();
    let mut checkpoint = discovery_checkpoint(run_id.clone(), artifact_id);
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    store.save_discovery_checkpoint(&checkpoint).unwrap();
    assert_eq!(store.discovery_run_count().unwrap(), 1);
    let stored = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(stored.checkpoint, checkpoint);
    checkpoint.status = DiscoveryCheckpointStatus::Completed;
    checkpoint.result_fingerprint = Some(hash('4'));
    checkpoint.pending_tasks.clear();
    checkpoint.snapshot = json!({"status":"completed"});
    store.save_discovery_checkpoint(&checkpoint).unwrap();
    let replaced = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(replaced.checkpoint, checkpoint);
    assert_eq!(store.discovery_run_count().unwrap(), 1);
    cleanup(&directory);
}

#[test]
fn discarding_a_run_is_idempotent_and_blocks_acceptance() {
    let directory = test_directory("discovery-run-disposition");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/disposition-fixture").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('a'));
    let run_id = StableId::parse("mathscape:discovery-run/disposition-fixture").unwrap();
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    store
        .save_discovery_checkpoint(&discovery_checkpoint(run_id.clone(), artifact_id))
        .unwrap();
    let actor = ActorRef::parse("fixture-reviewer").unwrap();
    let request_id = StableId::parse("mathscape:discovery-disposition/fixture").unwrap();
    let receipt = store
        .discard_discovery_run(
            &run_id,
            &request_id,
            &hash('4'),
            &actor,
            "fixture discard",
            &Timestamp::parse("2026-09-28T00:00:00Z").unwrap(),
        )
        .unwrap();
    assert_eq!(receipt.disposition, "discarded");
    assert!(store.discovery_run_is_discarded(&run_id).unwrap());
    let repeated = store
        .discard_discovery_run(
            &run_id,
            &StableId::parse("mathscape:discovery-disposition/other").unwrap(),
            &hash('4'),
            &actor,
            "different text",
            &Timestamp::parse("2026-09-28T00:00:01Z").unwrap(),
        )
        .unwrap();
    assert_eq!(repeated.request_id, request_id);
    let record = AcceptedRegistrationPlanRecord {
        decision_id: StableId::parse("mathscape:acceptance/disposition-fixture").unwrap(),
        run_id,
        actor,
        decided_at: Timestamp::parse("2026-09-28T00:00:00Z").unwrap(),
        authority: StoredAcceptanceAuthority::HumanReview,
        expected_result_fingerprint: hash('4'),
        selection: json!({"kind":"all_validated"}),
        plan_fingerprint: hash('5'),
        plan: json!({"version":"fixture"}),
    };
    assert!(store.save_registration_plan(&record).is_err());
    cleanup(&directory);
}

#[test]
fn restoring_review_preserves_audit_cycles_and_rejects_stale_decisions() {
    let directory = test_directory("restore-review");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let artifact = StableId::parse("mathscape:artifact/restore").unwrap();
    insert_fixture_artifact(&database, &artifact, &hash('a'));
    let run = StableId::parse("mathscape:run/restore").unwrap();
    Connection::open(&database).unwrap().execute("INSERT INTO artifact_revisions(artifact_id,revision_id,media_type,rights_status,permitted_uses,schema_version,lifecycle,created_at,created_by) VALUES(?1,?2,'text/markdown','not_assessed','[]','mathscape-source-1','active','unix:1','reviewer')",rusqlite::params![artifact.as_str(),hash('f').as_str()]).unwrap();
    let checkpoint = discovery_checkpoint(run.clone(), artifact);
    store.save_discovery_checkpoint(&checkpoint).unwrap();
    let actor = ActorRef::parse("reviewer").unwrap();
    let time = Timestamp::parse("unix:1").unwrap();
    let discard = mathscape_domain::DiscoveryDispositionRequest {
        run_id: run.clone(),
        request_id: StableId::parse("mathscape:decision/discard-1").unwrap(),
        expected_result_fingerprint: hash('4'),
        expected_disposition_request_id: None,
        reason: "放弃".into(),
    };
    let first = store
        .discard_discovery_run_checked(&discard, &actor, &time)
        .unwrap();
    let restore = mathscape_domain::DiscoveryDispositionRequest {
        request_id: StableId::parse("mathscape:decision/restore-1").unwrap(),
        expected_disposition_request_id: Some(first.request_id.clone()),
        reason: "恢复".into(),
        ..discard.clone()
    };
    let mut wrong = checkpoint.clone();
    wrong.snapshot = json!({"not":"the validated checkpoint"});
    assert!(
        store
            .restore_discovery_review(&restore, &actor, &time, &wrong)
            .unwrap_err()
            .to_string()
            .contains("checkpoint_conflict")
    );
    assert!(store.discovery_run_is_discarded(&run).unwrap());
    assert_eq!(store.discovery_disposition_history(&run).unwrap().len(), 1);
    let restored = store
        .restore_discovery_review(&restore, &actor, &time, &checkpoint)
        .unwrap();
    assert_eq!(restored.disposition, "restored");
    assert!(!store.discovery_run_is_discarded(&run).unwrap());
    let pending = AcceptedRegistrationPlanRecord {
        decision_id: StableId::parse("mathscape:acceptance/restored-pending").unwrap(),
        run_id: run.clone(),
        actor: actor.clone(),
        decided_at: time.clone(),
        authority: StoredAcceptanceAuthority::HumanReview,
        expected_result_fingerprint: hash('4'),
        selection: json!({"kind":"all_validated"}),
        plan_fingerprint: hash('5'),
        plan: json!({"version":"fixture"}),
    };
    store.save_registration_plan(&pending).unwrap();
    assert_eq!(
        store
            .pending_registration_decisions(run.as_str())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .discovery_checkpoint(&run)
            .unwrap()
            .unwrap()
            .checkpoint,
        checkpoint
    );
    assert_eq!(store.map_objects().unwrap().len(), 0);
    assert_eq!(
        store
            .restore_discovery_review(&restore, &actor, &time, &checkpoint)
            .unwrap(),
        restored
    );
    let second = mathscape_domain::DiscoveryDispositionRequest {
        request_id: StableId::parse("mathscape:decision/discard-2").unwrap(),
        expected_disposition_request_id: Some(restored.request_id.clone()),
        ..discard.clone()
    };
    let second_receipt = store
        .discard_discovery_run_checked(&second, &actor, &time)
        .unwrap();
    assert!(store.discovery_run_is_discarded(&run).unwrap());
    assert_eq!(
        store
            .restore_discovery_review(&restore, &actor, &time, &checkpoint)
            .unwrap(),
        restored
    ); // replay is historical, never changes current pointer
    let stale = mathscape_domain::DiscoveryDispositionRequest {
        request_id: StableId::parse("mathscape:decision/stale").unwrap(),
        ..restore.clone()
    };
    assert!(
        store
            .restore_discovery_review(&stale, &actor, &time, &checkpoint)
            .unwrap_err()
            .to_string()
            .contains("disposition_conflict")
    );
    let mut conflict = restore.clone();
    conflict.reason = "different".into();
    assert!(
        store
            .restore_discovery_review(&conflict, &actor, &time, &checkpoint)
            .unwrap_err()
            .to_string()
            .contains("idempotency_conflict")
    );
    let fresh = mathscape_domain::DiscoveryDispositionRequest {
        request_id: StableId::parse("mathscape:decision/restore-2").unwrap(),
        expected_disposition_request_id: Some(second_receipt.request_id),
        ..restore
    };
    store
        .restore_discovery_review(&fresh, &actor, &time, &checkpoint)
        .unwrap();
    assert_eq!(
        store
            .discovery_disposition_history(&run)
            .unwrap()
            .iter()
            .map(|r| r.disposition.as_str())
            .collect::<Vec<_>>(),
        ["discarded", "restored", "discarded", "restored"]
    );
    assert_eq!(
        store.list_discovery_run_summaries(None).unwrap()[0].review_status,
        "accepted"
    );
    drop(store);
    let reopened = SqliteArtifactStore::open(&database).unwrap();
    assert!(!reopened.discovery_run_is_discarded(&run).unwrap());
    assert_eq!(
        reopened.discovery_disposition_history(&run).unwrap().len(),
        4
    );
    cleanup(&directory);
}

#[test]
fn restoring_review_is_atomic_and_v11_discard_migrates_to_history() {
    let directory = test_directory("restore-migration");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let artifact = StableId::parse("mathscape:artifact/restore-migration").unwrap();
    insert_fixture_artifact(&database, &artifact, &hash('a'));
    let run = StableId::parse("mathscape:run/restore-migration").unwrap();
    let cp = discovery_checkpoint(run.clone(), artifact);
    store.save_discovery_checkpoint(&cp).unwrap();
    let actor = ActorRef::parse("reviewer").unwrap();
    let time = Timestamp::parse("unix:1").unwrap();
    let discard = StableId::parse("mathscape:decision/discard-old").unwrap();
    store
        .discard_discovery_run(&run, &discard, &hash('4'), &actor, "old discard", &time)
        .unwrap();
    drop(store);
    let sql = Connection::open(&database).unwrap();
    sql.execute_batch("PRAGMA foreign_keys=OFF; ALTER TABLE discovery_run_dispositions RENAME TO temp_dispositions; CREATE TABLE discovery_run_dispositions(run_id TEXT PRIMARY KEY,disposition TEXT NOT NULL CHECK(disposition='discarded'),reason TEXT NOT NULL,actor TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,decided_at TEXT NOT NULL); INSERT INTO discovery_run_dispositions SELECT * FROM temp_dispositions; DROP TABLE temp_dispositions; DROP TABLE discovery_run_disposition_events; CREATE INDEX discovery_run_dispositions_by_decision ON discovery_run_dispositions(request_id); DELETE FROM schema_migrations WHERE version=12; PRAGMA user_version=11;").unwrap();
    drop(sql);
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    assert_eq!(store.discovery_disposition_history(&run).unwrap().len(), 1);
    assert!(store.discovery_run_is_discarded(&run).unwrap());
    let request = mathscape_domain::DiscoveryDispositionRequest {
        run_id: run.clone(),
        request_id: StableId::parse("mathscape:decision/restore-migrated").unwrap(),
        expected_result_fingerprint: hash('4'),
        expected_disposition_request_id: Some(discard),
        reason: "restore".into(),
    };
    let sql = Connection::open(&database).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_restore BEFORE UPDATE ON discovery_run_dispositions BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        store
            .restore_discovery_review(&request, &actor, &time, &cp)
            .is_err()
    );
    assert_eq!(store.discovery_disposition_history(&run).unwrap().len(), 1);
    assert!(store.discovery_run_is_discarded(&run).unwrap());
    sql.execute_batch("DROP TRIGGER fail_restore;").unwrap();
    store
        .restore_discovery_review(&request, &actor, &time, &cp)
        .unwrap();
    assert_eq!(store.discovery_disposition_history(&run).unwrap().len(), 2);
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    drop(sql);
    drop(store);
    cleanup(&directory);
}

#[test]
fn only_a_run_awaiting_patch_review_moves_onto_a_patched_snapshot() {
    let directory = test_directory("patch-review-rebind");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let base = StableId::parse("mathscape:artifact/patch-base").unwrap();
    let patched = StableId::parse("mathscape:artifact/patch-revised").unwrap();
    insert_fixture_artifact(&database, &base, &hash('a'));
    insert_fixture_artifact(&database, &patched, &hash('b'));
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();

    let run_id = StableId::parse("mathscape:discovery-run/patch-review").unwrap();
    let mut awaiting = discovery_checkpoint(run_id.clone(), base.clone());
    awaiting.status = DiscoveryCheckpointStatus::AwaitingPatchReview;
    awaiting.result_fingerprint = None;
    awaiting.facts.clear();
    awaiting.pending_tasks.clear();
    store.save_discovery_checkpoint(&awaiting).unwrap();
    let stored = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(
        stored.checkpoint.status,
        DiscoveryCheckpointStatus::AwaitingPatchReview
    );

    let mut continued = discovery_checkpoint(run_id.clone(), patched.clone());
    continued.conversion_id =
        Some(StableId::parse("mathscape:conversion/markdown-patch/x").unwrap());
    continued.input_fingerprint = hash('6');
    continued.selection_fingerprint = hash('7');
    continued.status = DiscoveryCheckpointStatus::Completed;
    store.save_discovery_checkpoint(&continued).unwrap();
    let stored = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(stored.checkpoint, continued);

    // Once B has run on the patched snapshot the identity is frozen again.
    let mut moved_back = continued.clone();
    moved_back.artifact_id = base.clone();
    moved_back.input_fingerprint = hash('1');
    assert!(matches!(
        store.save_discovery_checkpoint(&moved_back),
        Err(SqliteStoreError::DiscoveryInvariant(_))
    ));

    let mut invalid = awaiting.clone();
    invalid.run_id = StableId::parse("mathscape:discovery-run/patch-review-invalid").unwrap();
    invalid.result_fingerprint = Some(hash('4'));
    assert!(matches!(
        store.save_discovery_checkpoint(&invalid),
        Err(SqliteStoreError::DiscoveryInvariant(_))
    ));
    cleanup(&directory);
}

#[test]
fn upgrading_to_v10_preserves_discovery_runs_and_their_children() {
    let directory = test_directory("upgrade-v8-patch-review");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/upgrade-v8").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('c'));
    let run_id = StableId::parse("mathscape:discovery-run/upgrade-v8").unwrap();
    let checkpoint = discovery_checkpoint(run_id.clone(), artifact_id);
    {
        let mut store = SqliteArtifactStore::prepare(&database).unwrap();
        store.save_discovery_checkpoint(&checkpoint).unwrap();
    }
    // Recreate the v8 table shape (no awaiting_patch_review) with data in place.
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             BEGIN;
             CREATE TABLE discovery_runs_v8 AS SELECT * FROM discovery_runs;
             DROP TABLE discovery_runs;
             CREATE TABLE discovery_runs (
                run_id TEXT PRIMARY KEY NOT NULL,
                artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id) ON DELETE RESTRICT,
                conversion_id TEXT,
                pipeline_version TEXT NOT NULL,
                input_fingerprint TEXT NOT NULL,
                selection_fingerprint TEXT NOT NULL,
                unit_set_fingerprint TEXT,
                result_fingerprint TEXT,
                status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'partial',
                    'needs_attention', 'failed', 'cancelled')),
                admission TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                snapshot_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             ) STRICT;
             INSERT INTO discovery_runs SELECT * FROM discovery_runs_v8;
             DROP TABLE discovery_runs_v8;
             CREATE INDEX discovery_runs_by_source
                ON discovery_runs(artifact_id, input_fingerprint, updated_at);
             DROP TABLE discovery_run_dispositions;
             DROP TABLE discovery_run_disposition_events;
             DROP TABLE source_registration_requests;
             DROP TABLE discovery_registration_reversals;
             DROP INDEX map_commit_objects_by_object;
             DELETE FROM schema_migrations WHERE version IN (9, 10, 11, 12);
             PRAGMA user_version = 8;
             COMMIT;",
        )
        .unwrap();
    drop(connection);

    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    let stored = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(stored.checkpoint, checkpoint);
    let mut awaiting = checkpoint.clone();
    awaiting.status = DiscoveryCheckpointStatus::AwaitingPatchReview;
    awaiting.result_fingerprint = None;
    awaiting.facts.clear();
    awaiting.pending_tasks.clear();
    store.save_discovery_checkpoint(&awaiting).unwrap();
    drop(store);
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        19
    );
    let violations = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count();
    assert_eq!(violations, 0);
    cleanup(&directory);
}

#[test]
fn persists_an_immutable_acceptance_decision_and_registration_plan() {
    let directory = test_directory("acceptance-plan");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::prepare(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/acceptance-fixture").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('b'));
    let run_id = StableId::parse("mathscape:discovery-run/acceptance-fixture").unwrap();
    let mut store = SqliteArtifactStore::prepare(&database).unwrap();
    store
        .save_discovery_checkpoint(&discovery_checkpoint(run_id.clone(), artifact_id))
        .unwrap();
    let decision_id = StableId::parse("mathscape:acceptance/fixture").unwrap();
    let record = AcceptedRegistrationPlanRecord {
        decision_id: decision_id.clone(),
        run_id,
        actor: ActorRef::parse("fixture-reviewer").unwrap(),
        decided_at: Timestamp::parse("2026-09-12T12:00:00+08:00").unwrap(),
        authority: StoredAcceptanceAuthority::HumanReview,
        expected_result_fingerprint: hash('4'),
        selection: json!({"kind":"explicit_units","unit_refs":["u1"]}),
        plan_fingerprint: hash('5'),
        plan: json!({"version":"mathscape-source-registration-plan-1","units":["u1"]}),
    };
    store.save_registration_plan(&record).unwrap();
    store.save_registration_plan(&record).unwrap();
    assert_eq!(store.registration_plan(&decision_id).unwrap(), Some(record));
    cleanup(&directory);
}
