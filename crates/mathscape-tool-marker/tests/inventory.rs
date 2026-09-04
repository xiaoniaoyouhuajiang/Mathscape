#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mathscape_pdf::{PdfPageIndex, PdfSelection};
use mathscape_tool_marker::{
    MARKER_PRODUCT_INVENTORY_SCHEMA_VERSION, MarkerCancellation, MarkerExecutionOptions,
    MarkerInstallation, MarkerProductIssue, MarkerProductRole, MarkerProductScanError,
    MarkerProductScanLimits, MarkerProductScanner, MarkerProfile, MarkerRequest, MarkerRun,
    MarkerRunStatus, MarkerRunner, MarkerStagingArea,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct WorkspaceTestDirectory {
    path: PathBuf,
}

impl WorkspaceTestDirectory {
    fn create(case_name: &str) -> Self {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".test-runs");
        fs::create_dir_all(&parent).unwrap();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = parent.join(format!(
            "inventory-{case_name}-{timestamp}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(self) {
        let parent = self.path.parent().unwrap().to_owned();
        fs::remove_dir_all(&self.path).unwrap();
        let _ = fs::remove_dir(parent);
    }
}

fn execute_fixture(case_name: &str, output_commands: &str) -> (WorkspaceTestDirectory, MarkerRun) {
    let directory = WorkspaceTestDirectory::create(case_name);
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let executable = bin.join("marker_single");
    let script = format!(
        r#"#!/bin/sh
output_root=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        *) shift ;;
    esac
done
{output_commands}
"#
    );
    fs::write(&executable, script).unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let source_pdf = directory.path().join("source book.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n% inventory fixture\n").unwrap();
    let selection = PdfSelection::from_pages(1, vec![PdfPageIndex::new(0)]).unwrap();
    let request = MarkerRequest::new(source_pdf, selection, MarkerProfile::MathOcrV1).unwrap();
    let installation = MarkerInstallation::new(executable).unwrap();
    let staging = MarkerStagingArea::new(directory.path().join("staging")).unwrap();
    let options = MarkerExecutionOptions::new(Duration::from_secs(2))
        .unwrap()
        .poll_interval(Duration::from_millis(10))
        .unwrap();
    let run = MarkerRunner::new(options)
        .execute(
            &installation,
            &request,
            &staging,
            &MarkerCancellation::new(),
        )
        .unwrap();
    (directory, run)
}

fn complete_output(exit_code: u8) -> String {
    format!(
        r#"mkdir -p "$output_root/source book"
printf '# converted\n' > "$output_root/source book/source book.md"
printf '{{"page_stats":[]}}\n' > "$output_root/source book/source book_meta.json"
printf 'jpeg bytes' > "$output_root/source book/_page_0_Figure_1.jpeg"
printf 'support' > "$output_root/source book/layout.bin"
exit {exit_code}"#
    )
}

fn product<'a>(
    inventory: &'a mathscape_tool_marker::MarkerProductInventory,
    relative_path: &str,
) -> &'a mathscape_tool_marker::MarkerProduct {
    inventory
        .products()
        .iter()
        .find(|product| product.relative_path() == Path::new(relative_path))
        .unwrap()
}

#[test]
fn inventories_the_fixed_marker_layout_deterministically() {
    let (directory, run) = execute_fixture("complete", &complete_output(0));
    let scanner = MarkerProductScanner::default();
    let inventory = scanner.scan(&run).unwrap();
    let repeated = scanner.scan(&run).unwrap();

    assert_eq!(inventory, repeated);
    assert_eq!(
        inventory.schema_version(),
        MARKER_PRODUCT_INVENTORY_SCHEMA_VERSION
    );
    assert_eq!(inventory.run_status(), MarkerRunStatus::Succeeded);
    assert_eq!(inventory.profile(), MarkerProfile::MathOcrV1);
    assert_eq!(inventory.output_directory(), Path::new("source book"));
    assert_eq!(
        inventory.primary_markdown(),
        Some(Path::new("source book/source book.md"))
    );
    assert_eq!(
        inventory.metadata(),
        Some(Path::new("source book/source book_meta.json"))
    );
    assert!(inventory.issues().is_empty());
    assert!(inventory.is_promotable());
    assert_eq!(inventory.products().len(), 6);
    assert_eq!(inventory.derived_products().count(), 4);
    assert_eq!(
        product(&inventory, "source book/source book.md").role(),
        MarkerProductRole::PrimaryMarkdown
    );
    assert_eq!(
        product(&inventory, "source book/source book_meta.json").media_type(),
        "application/json"
    );
    assert_eq!(
        product(&inventory, "source book/_page_0_Figure_1.jpeg").role(),
        MarkerProductRole::Image
    );
    assert_eq!(
        product(&inventory, "source book/_page_0_Figure_1.jpeg").media_type(),
        "image/jpeg"
    );
    let markdown = product(&inventory, "source book/source book.md");
    assert_eq!(markdown.byte_size(), 12);
    assert!(markdown.digest().as_str().starts_with("sha256:"));
    assert_eq!(markdown.digest().as_str().len(), 71);
    assert!(inventory.source_path(markdown).is_file());
    assert_eq!(
        inventory.total_bytes(),
        inventory
            .products()
            .iter()
            .map(|product| product.byte_size())
            .sum()
    );

    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn reports_an_incomplete_success_without_promoting_it() {
    let output = r#"mkdir -p "$output_root/source book"
printf '# converted\n' > "$output_root/source book/source book.md"
printf '# unrelated\n' > "$output_root/source book/extra.md"
printf 'unexpected' > "$output_root/rogue.bin"
exit 0"#;
    let (directory, run) = execute_fixture("incomplete", output);
    let inventory = MarkerProductScanner::default().scan(&run).unwrap();

    assert_eq!(run.status(), MarkerRunStatus::Succeeded);
    assert!(!inventory.is_promotable());
    assert!(
        inventory
            .issues()
            .iter()
            .any(|issue| matches!(issue, MarkerProductIssue::MissingMetadata { .. }))
    );
    assert!(inventory.issues().iter().any(|issue| matches!(
        issue,
        MarkerProductIssue::AdditionalMarkdown { paths }
            if paths == &[PathBuf::from("source book/extra.md")]
    )));
    assert!(inventory.issues().iter().any(|issue| matches!(
        issue,
        MarkerProductIssue::UnexpectedOutputLocation { paths }
            if paths == &[PathBuf::from("rogue.bin")]
    )));

    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn inventories_complete_partial_files_but_never_promotes_a_failed_run() {
    let (directory, run) = execute_fixture("failed", &complete_output(7));
    let inventory = MarkerProductScanner::default().scan(&run).unwrap();

    assert_eq!(run.status(), MarkerRunStatus::Failed);
    assert!(inventory.issues().is_empty());
    assert!(inventory.primary_markdown().is_some());
    assert!(!inventory.is_promotable());

    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn rejects_symbolic_links_without_following_them() {
    let (directory, run) = execute_fixture("symlink", &complete_output(0));
    let link = run.staging().path().join("source book/external-link");
    symlink(directory.path().join("source book.pdf"), &link).unwrap();

    assert!(matches!(
        MarkerProductScanner::default().scan(&run),
        Err(MarkerProductScanError::SymbolicLink(path)) if path == link
    ));

    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn enforces_file_count_limits_before_returning_an_inventory() {
    let (directory, run) = execute_fixture("limit", &complete_output(0));
    let limits = MarkerProductScanLimits::new(1, 1024 * 1024, 4).unwrap();

    assert!(matches!(
        MarkerProductScanner::new(limits).scan(&run),
        Err(MarkerProductScanError::MaximumFileCountExceeded { maximum: 1 })
    ));

    run.cleanup().unwrap();
    directory.cleanup();
}
