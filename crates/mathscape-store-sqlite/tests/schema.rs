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
fn creates_and_reopens_the_versioned_artifact_schema() {
    let directory = test_directory("migration");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::open(&database).unwrap());

    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 5);
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
    drop(connection);
    drop(SqliteArtifactStore::open(&database).unwrap());
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
        SqliteArtifactStore::open(&database),
        Err(SqliteStoreError::UnsupportedSchemaVersion {
            actual: 99,
            supported: 5
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

    drop(SqliteArtifactStore::open(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 5);
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

    drop(SqliteArtifactStore::open(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap();
    assert_eq!(version, 5);
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

    drop(SqliteArtifactStore::open(&database).unwrap());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        5
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FixtureSettings {
    version: String,
    timeout_seconds: u64,
}

#[test]
fn persists_and_replaces_a_versioned_workspace_settings_document() {
    let directory = test_directory("workspace-settings");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::open(&database).unwrap();
    assert_eq!(store.workspace_settings::<FixtureSettings>().unwrap(), None);

    let mut settings = FixtureSettings {
        version: "fixture-settings-1".into(),
        timeout_seconds: 600,
    };
    store.save_workspace_settings(&settings).unwrap();
    settings.timeout_seconds = 900;
    store.save_workspace_settings(&settings).unwrap();
    drop(store);

    let store = SqliteArtifactStore::open(&database).unwrap();
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
    let mut store = SqliteArtifactStore::open(&database).unwrap();
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

    let store = SqliteArtifactStore::open(&database).unwrap();
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
    drop(SqliteArtifactStore::open(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/discovery-fixture").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('a'));

    let run_id = StableId::parse("mathscape:discovery-run/fixture").unwrap();
    let mut checkpoint = discovery_checkpoint(run_id.clone(), artifact_id);
    let mut store = SqliteArtifactStore::open(&database).unwrap();
    store.save_discovery_checkpoint(&checkpoint).unwrap();
    assert_eq!(store.discovery_run_count().unwrap(), 1);
    let stored = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(stored.checkpoint, checkpoint);

    checkpoint.status = DiscoveryCheckpointStatus::Completed;
    checkpoint.pending_tasks.clear();
    checkpoint.snapshot = json!({"status":"completed"});
    store.save_discovery_checkpoint(&checkpoint).unwrap();
    let replaced = store.discovery_checkpoint(&run_id).unwrap().unwrap();
    assert_eq!(replaced.checkpoint, checkpoint);
    assert_eq!(store.discovery_run_count().unwrap(), 1);
    cleanup(&directory);
}

#[test]
fn persists_an_immutable_acceptance_decision_and_registration_plan() {
    let directory = test_directory("acceptance-plan");
    let database = directory.join("catalog.sqlite3");
    drop(SqliteArtifactStore::open(&database).unwrap());
    let artifact_id = StableId::parse("mathscape:artifact/acceptance-fixture").unwrap();
    insert_fixture_artifact(&database, &artifact_id, &hash('b'));
    let run_id = StableId::parse("mathscape:discovery-run/acceptance-fixture").unwrap();
    let mut store = SqliteArtifactStore::open(&database).unwrap();
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
