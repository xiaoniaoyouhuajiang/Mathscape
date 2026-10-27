use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_pdf::{
    ClassificationMode, PdfContentKind, PdfEngine, PdfError, PdfOpenOptions, PdfPageIndex,
    PdfSelection, ScanCoverage,
};

fn write_pdf(path: &std::path::Path, with_text: bool) {
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
    let operations = if with_text {
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 18.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("First text run")]),
            Operation::new("Td", vec![0.into(), (-24).into()]),
            Operation::new("Tj", vec![Object::string_literal("Second text run")]),
            Operation::new("Td", vec![0.into(), (-24).into()]),
            Operation::new("Tj", vec![Object::string_literal("Third text run")]),
            Operation::new("ET", vec![]),
        ]
    } else {
        Vec::new()
    };
    let content = Content { operations };
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        content.encode().expect("encode test PDF content"),
    ));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Rotate" => 90,
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
        "Title" => Object::string_literal("Mathscape PDF test"),
    });
    document.trailer.set("Root", catalog_id);
    document.trailer.set("Info", info_id);
    document.compress();
    document.save(path).expect("save test PDF");
}

#[test]
fn probes_geometry_classifies_and_extracts_selected_text() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.pdf");
    write_pdf(&path, true);

    let engine = PdfEngine;
    let options = PdfOpenOptions::new();
    let probe = engine.probe(&path, &options).unwrap();
    assert_eq!(probe.page_count(), 1);
    assert_eq!(probe.title(), Some("Mathscape PDF test"));
    assert_eq!(probe.pages()[0].page(), PdfPageIndex::new(0));
    assert_eq!(probe.pages()[0].rotation().degrees(), 90);
    assert_eq!(probe.pages()[0].displayed_width(), 792.0);
    assert_eq!(probe.pages()[0].displayed_height(), 612.0);

    let classification = engine
        .classify(&path, &options, ClassificationMode::Exhaustive)
        .unwrap();
    assert_eq!(classification.kind(), PdfContentKind::TextBased);
    assert_eq!(classification.coverage(), ScanCoverage::Exhaustive);
    assert!(classification.ocr_recommendations().is_empty());

    let selection = PdfSelection::all(probe.page_count()).unwrap();
    let extraction = engine
        .extract_positioned_text(&path, &selection, &options)
        .unwrap();
    assert!(!extraction.items().is_empty());
    assert!(
        extraction
            .items()
            .iter()
            .all(|item| item.page() == PdfPageIndex::new(0))
    );
    assert!(
        extraction
            .items()
            .iter()
            .any(|item| item.text().contains("First text run"))
    );
    assert!(
        extraction.items()[0]
            .normalized_bounds(probe.pages()[0])
            .unwrap()
            .is_some()
    );
}

#[test]
fn exhaustive_classification_produces_zero_based_ocr_route() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("blank.pdf");
    write_pdf(&path, false);

    let classification = PdfEngine
        .classify(
            &path,
            &PdfOpenOptions::new(),
            ClassificationMode::Exhaustive,
        )
        .unwrap();
    assert_eq!(classification.kind(), PdfContentKind::Scanned);
    assert_eq!(classification.ocr_recommendations()[0].page().get(), 0);
    assert_eq!(
        classification.ocr_selection().unwrap().unwrap().pages(),
        &[PdfPageIndex::new(0)]
    );
}

#[test]
fn page_texts_join_runs_into_lines_and_flag_pages_without_text() {
    let directory = tempfile::tempdir().unwrap();
    let text_path = directory.path().join("text.pdf");
    let blank_path = directory.path().join("blank.pdf");
    write_pdf(&text_path, true);
    write_pdf(&blank_path, false);
    let options = PdfOpenOptions::new();
    let selection = PdfSelection::all(1).unwrap();

    let texts = PdfEngine
        .extract_positioned_text(&text_path, &selection, &options)
        .unwrap()
        .page_texts();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].0, PdfPageIndex::new(0));
    let lines: Vec<_> = texts[0].1.lines().map(str::trim).collect();
    assert_eq!(
        lines,
        vec!["First text run", "Second text run", "Third text run"]
    );

    let blank = PdfEngine
        .extract_positioned_text(&blank_path, &selection, &options)
        .unwrap()
        .page_texts();
    assert_eq!(blank, vec![(PdfPageIndex::new(0), String::new())]);
}

#[test]
fn reports_non_pdf_input_without_leaking_backend_types() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-a-pdf.txt");
    std::fs::write(&path, b"plain text").unwrap();

    assert!(matches!(
        PdfEngine.probe(&path, &PdfOpenOptions::new()),
        Err(PdfError::NotPdf(_))
    ));
}

#[test]
#[ignore = "set MATHSCAPE_PDF_FIXTURE to exercise a local corpus PDF"]
fn probes_an_external_fixture() {
    let path = std::env::var_os("MATHSCAPE_PDF_FIXTURE")
        .expect("MATHSCAPE_PDF_FIXTURE must name a PDF fixture");
    let engine = PdfEngine;
    let options = PdfOpenOptions::new();
    let probe = engine.probe(&path, &options).unwrap();
    let classification = engine
        .classify(&path, &options, ClassificationMode::Fast)
        .unwrap();
    assert_eq!(probe.page_count(), classification.page_count());
    assert_eq!(probe.pages().len(), probe.page_count() as usize);

    let first_page = PdfSelection::inclusive_range(
        probe.page_count(),
        PdfPageIndex::new(0),
        PdfPageIndex::new(0),
    )
    .unwrap();
    let extraction = engine
        .extract_positioned_text(&path, &first_page, &options)
        .unwrap();
    eprintln!(
        "pages={}, kind={:?}, OCR pages={}, first-page items={}",
        probe.page_count(),
        classification.kind(),
        classification.ocr_recommendations().len(),
        extraction.items().len()
    );
}
