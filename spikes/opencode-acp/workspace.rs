// Build with rustc against the workspace's already-built dependencies (see README).
fn main() {
    let path = std::env::args().nth(1).expect("workspace path");
    let root = mathscape_artifact::WorkspaceRoot::create(path).unwrap();
    mathscape_store_sqlite::SqliteArtifactStore::prepare(&root.database_path()).unwrap();
    println!("Prepared {}", root.path().display());
}
