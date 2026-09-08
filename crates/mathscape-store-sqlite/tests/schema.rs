use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_llm::{
    CapabilitySupport, CredentialRef, LLM_PROFILE_VERSION, LlmCapabilities, LlmProfile,
    WireProtocol,
};
use mathscape_store_sqlite::{SqliteArtifactStore, SqliteStoreError};
use rusqlite::Connection;
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
    assert_eq!(version, 2);
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
            supported: 2
        })
    ));
    cleanup(&directory);
}

#[test]
fn upgrades_an_existing_artifact_pool_database_to_llm_profiles() {
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
    assert_eq!(version, 2);
    let migration = connection
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(migration, "llm_profiles_v1");
    cleanup(&directory);
}

#[test]
fn persists_versioned_llm_profiles_without_secret_material() {
    let directory = test_directory("llm-profile");
    let database = directory.join("catalog.sqlite3");
    let mut store = SqliteArtifactStore::open(&database).unwrap();
    let profile = LlmProfile {
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
