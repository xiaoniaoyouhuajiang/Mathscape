use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_desktop::{DesktopService, DesktopServiceError, PdfImportOutcome};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create(case: &str) -> Self {
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
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn cleanup(self) {
        let parent = self.0.parent().unwrap().to_owned();
        fs::remove_dir_all(self.0).unwrap();
        let _ = fs::remove_dir(parent);
    }
}

#[test]
fn imports_a_pdf_into_a_reopenable_workspace_and_refreshes_the_graph() {
    let directory = TestDirectory::create("pdf-import");
    let source = directory.path().join("analysis.pdf");
    write_pdf(&source);
    let workspace = directory.path().join("workspace");
    let service = DesktopService::open(&workspace).unwrap();

    let initial = service.snapshot().unwrap();
    assert_eq!(initial.artifact_count(), 0);
    assert!(initial.graph().nodes().is_empty());

    let imported = service.import_pdf(&source).unwrap();
    assert_eq!(imported.outcome(), PdfImportOutcome::Inserted);
    assert_eq!(imported.page_count(), 1);
    assert_eq!(imported.snapshot().artifact_count(), 1);
    assert_eq!(imported.snapshot().conversion_count(), 0);
    assert_eq!(imported.snapshot().graph().nodes().len(), 1);
    assert!(
        imported
            .artifact_id()
            .starts_with("mathscape:artifact/sha256/")
    );

    let duplicate = service.import_pdf(&source).unwrap();
    assert_eq!(duplicate.outcome(), PdfImportOutcome::Deduplicated);
    assert_eq!(duplicate.snapshot().artifact_count(), 1);

    fs::remove_file(&source).unwrap();
    let reopened = DesktopService::open(&workspace).unwrap();
    assert_eq!(reopened.snapshot().unwrap().artifact_count(), 1);
    drop(reopened);
    directory.cleanup();
}

#[test]
fn rejects_malformed_pdf_bytes_before_artifact_registration() {
    let directory = TestDirectory::create("malformed-pdf");
    let source = directory.path().join("broken.pdf");
    fs::write(&source, b"this has a PDF extension but no PDF structure").unwrap();
    let service = DesktopService::open(directory.path().join("workspace")).unwrap();

    assert!(matches!(
        service.import_pdf(&source),
        Err(DesktopServiceError::Pdf(_))
    ));
    assert_eq!(service.snapshot().unwrap().artifact_count(), 0);
    directory.cleanup();
}

#[test]
fn rejects_non_pdf_imports_before_artifact_registration() {
    let directory = TestDirectory::create("non-pdf");
    let source = directory.path().join("notes.md");
    fs::write(&source, b"not a PDF").unwrap();
    let service = DesktopService::open(directory.path().join("workspace")).unwrap();

    assert!(matches!(
        service.import_pdf(&source),
        Err(DesktopServiceError::UnsupportedImportType(path)) if path == source
    ));
    assert_eq!(service.snapshot().unwrap().artifact_count(), 0);
    directory.cleanup();
}

fn write_pdf(path: &Path) {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 18.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
        Operation::new(
            "Tj",
            vec![Object::string_literal("Mathscape desktop import test")],
        ),
        Operation::new("ET", vec![]),
    ];
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().unwrap(),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = document.add_object(dictionary! {
        "Title" => Object::string_literal("Desktop import fixture"),
    });
    document.trailer.set("Root", catalog_id);
    document.trailer.set("Info", info_id);
    document.compress();
    document.save(path).unwrap();
}
