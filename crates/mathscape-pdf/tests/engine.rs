use lopdf::{
    Bookmark, Document, Object, ObjectId, Stream, content::Content, content::Operation, dictionary,
};
use mathscape_pdf::{
    ClassificationMode, NormalizedRect, PDF_RENDERER, PdfContentKind, PdfEngine, PdfError,
    PdfOpenOptions, PdfPageIndex, PdfSelection, ScanCoverage,
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

fn embed_bookmarks(document: &mut Document) {
    if let Some(outline_id) = document.build_outline() {
        document
            .catalog_mut()
            .expect("catalog")
            .set("Outlines", Object::Reference(outline_id));
    }
}

/// Three pages. Bookmarks: 第一章 → page 0, child 引理 → page 1, Chapter 2 → page 2.
fn write_outlined_pdf(path: &std::path::Path) {
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
    let mut kids = Vec::new();
    let mut page_ids = Vec::new();
    for line in ["Chapter page", "Section page", "Last page"] {
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 18.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(line)]),
            Operation::new("ET", vec![]),
        ];
        let content = Content { operations };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode outlined PDF content"),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        kids.push(page_id.into());
        page_ids.push(page_id);
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_ids.len() as i64,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let chapter = document.add_bookmark(
        Bookmark::new("第一章".into(), [0.0, 0.0, 0.0], 0, page_ids[0]),
        None,
    );
    document.add_bookmark(
        Bookmark::new("引理".into(), [0.0, 0.0, 0.0], 0, page_ids[1]),
        Some(chapter),
    );
    document.add_bookmark(
        Bookmark::new("Chapter 2".into(), [0.0, 0.0, 0.0], 0, page_ids[2]),
        None,
    );
    embed_bookmarks(&mut document);
    document.save(path).expect("save outlined PDF");
}

#[test]
fn reads_embedded_outline_as_zero_based_pages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outlined.pdf");
    write_outlined_pdf(&path);

    let outline = PdfEngine.outline(&path, &PdfOpenOptions::new()).unwrap();
    assert_eq!(outline.page_count(), 3);
    assert!(outline.warnings().is_empty(), "{:?}", outline.warnings());
    let entries: Vec<_> = outline
        .entries()
        .iter()
        .map(|entry| (entry.title(), entry.level(), entry.page().get()))
        .collect();
    assert_eq!(
        entries,
        vec![("第一章", 1, 0), ("引理", 2, 1), ("Chapter 2", 1, 2),]
    );
}

#[test]
fn pdf_without_bookmarks_has_an_empty_outline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.pdf");
    write_pdf(&path, true);

    let outline = PdfEngine.outline(&path, &PdfOpenOptions::new()).unwrap();
    assert_eq!(outline.page_count(), 1);
    assert!(outline.entries().is_empty());
    assert!(outline.warnings().is_empty());
    assert!(!outline.is_unreadable());
}

/// `lopdf` 0.42 folds equal titles in an `IndexMap`: the first insertion stays
/// in place, and the later bookmark overwrites page and level.
#[test]
fn duplicate_titles_keep_first_position_and_last_page_and_level() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("duplicate-titles.pdf");
    write_duplicate_title_pdf(&path);

    let outline = PdfEngine.outline(&path, &PdfOpenOptions::new()).unwrap();
    assert!(!outline.is_unreadable());
    assert!(outline.warnings().is_empty(), "{:?}", outline.warnings());
    let entries: Vec<_> = outline
        .entries()
        .iter()
        .map(|entry| (entry.title(), entry.level(), entry.page().get()))
        .collect();
    assert_eq!(entries, vec![("Same", 2, 2), ("Parent", 1, 1)]);
}

#[test]
fn short_dest_array_is_unreadable_instead_of_panicking() {
    let (mut document, page_id) = one_page_document();
    let item = document.add_object(dictionary! {
        "Title" => Object::string_literal("Short"),
        "Dest" => vec![page_id.into()],
    });
    attach_outline(&mut document, item);
    let (_dir, path) = save_document(&mut document);
    assert_unreadable(&path, "panicked");
}

#[test]
fn first_self_cycle_is_unreadable_instead_of_overflowing() {
    let (mut document, page_id) = one_page_document();
    let item_id = document.new_object_id();
    document.set_object(
        item_id,
        dictionary! {
            "Title" => Object::string_literal("Loop"),
            "Dest" => fit_dest(page_id),
            "First" => item_id,
        },
    );
    attach_outline(&mut document, item_id);
    let (_dir, path) = save_document(&mut document);
    assert_unreadable(&path, "cycle");
}

#[test]
fn next_sibling_cycle_is_unreadable_instead_of_looping() {
    let (mut document, page_id) = one_page_document();
    let left = document.new_object_id();
    let right = document.new_object_id();
    document.set_object(
        left,
        dictionary! {
            "Title" => Object::string_literal("A"),
            "Dest" => fit_dest(page_id),
            "Next" => right,
        },
    );
    document.set_object(
        right,
        dictionary! {
            "Title" => Object::string_literal("B"),
            "Dest" => fit_dest(page_id),
            "Next" => left,
        },
    );
    attach_outline(&mut document, left);
    let (_dir, path) = save_document(&mut document);
    assert_unreadable(&path, "cycle");
}

#[test]
fn parent_self_reference_is_unreadable() {
    let (mut document, page_id) = one_page_document();
    let item_id = document.new_object_id();
    document.set_object(
        item_id,
        dictionary! {
            "Title" => Object::string_literal("Self"),
            "Dest" => fit_dest(page_id),
            "Parent" => item_id,
        },
    );
    attach_outline(&mut document, item_id);
    let (_dir, path) = save_document(&mut document);
    assert_unreadable(&path, "/Parent");
}

#[test]
fn named_destination_kids_cycle_is_unreadable() {
    let (mut document, page_id) = one_page_document();
    let node_id = document.new_object_id();
    document.set_object(
        node_id,
        dictionary! {
            "Kids" => vec![Object::Reference(node_id)],
        },
    );
    document
        .catalog_mut()
        .expect("catalog")
        .set("Dests", Object::Reference(node_id));
    let item = document.add_object(dictionary! {
        "Title" => Object::string_literal("Chapter"),
        "Dest" => fit_dest(page_id),
    });
    attach_outline(&mut document, item);
    let (_dir, path) = save_document(&mut document);
    assert_unreadable(&path, "named destination");
}

/// `/Dests` `/Kids` is not an array. `get_toc` returns a parse error (it does
/// not panic); that must stay an empty unreadable outline.
#[test]
fn outline_parse_error_is_unreadable_without_a_panic() {
    let (mut document, page_id) = one_page_document();
    let dests_id = document.add_object(dictionary! {
        "Kids" => 1,
    });
    document
        .catalog_mut()
        .expect("catalog")
        .set("Dests", Object::Reference(dests_id));
    let item = document.add_object(dictionary! {
        "Title" => Object::string_literal("Chapter"),
        "Dest" => fit_dest(page_id),
    });
    attach_outline(&mut document, item);
    let (_dir, path) = save_document(&mut document);
    let outline = PdfEngine.outline(&path, &PdfOpenOptions::new()).unwrap();
    assert!(outline.is_unreadable(), "{outline:?}");
    assert!(outline.entries().is_empty());
    let warning = outline.warnings().join("\n");
    assert!(
        warning.contains("unreadable") && !warning.contains("panicked"),
        "{warning}"
    );
}

#[test]
fn outline_depth_cap_allows_64_and_rejects_65() {
    let (mut allowed, _page) = nested_outline(64);
    let (_dir, path) = save_document(&mut allowed);
    let outline = PdfEngine.outline(&path, &PdfOpenOptions::new()).unwrap();
    assert!(!outline.is_unreadable(), "{:?}", outline.warnings());
    assert_eq!(outline.entries().len(), 64);
    assert_eq!(outline.entries()[0].title(), "L1");
    assert_eq!(outline.entries()[0].level(), 1);
    assert_eq!(outline.entries()[0].page().get(), 0);
    assert_eq!(outline.entries()[63].title(), "L64");
    assert_eq!(outline.entries()[63].level(), 64);
    assert_eq!(outline.entries()[63].page().get(), 0);

    let (mut rejected, _page) = nested_outline(65);
    let (_dir, path) = save_document(&mut rejected);
    assert_unreadable(&path, "nesting");
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

fn one_page_document() -> (Document, ObjectId) {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    (document, page_id)
}

fn fit_dest(page_id: ObjectId) -> Object {
    Object::Array(vec![page_id.into(), Object::Name(b"Fit".to_vec())])
}

fn attach_outline(document: &mut Document, first: ObjectId) {
    let outlines_id = document.add_object(dictionary! {
        "Type" => "Outlines",
        "First" => first,
        "Last" => first,
        "Count" => 1,
    });
    document
        .catalog_mut()
        .expect("catalog")
        .set("Outlines", Object::Reference(outlines_id));
}

fn save_document(document: &mut Document) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outline.pdf");
    document.save(&path).expect("save outline PDF");
    (directory, path)
}

fn assert_unreadable(path: &std::path::Path, fragment: &str) {
    let outline = PdfEngine
        .outline(path, &PdfOpenOptions::new())
        .expect("hostile outline must not fail the open");
    assert!(outline.is_unreadable(), "{outline:?}");
    assert!(outline.entries().is_empty(), "{outline:?}");
    assert_eq!(outline.page_count(), 1);
    let warning = outline.warnings().join("\n");
    assert!(warning.contains(fragment), "{warning}");
}

/// Nest `depth` bookmarks with `/Parent` back-edges, the shape a real outline uses.
fn nested_outline(depth: usize) -> (Document, ObjectId) {
    let (mut document, page_id) = one_page_document();
    let mut child: Option<ObjectId> = None;
    for level in (1..=depth).rev() {
        let id = document.new_object_id();
        if let Some(child_id) = child {
            document
                .get_object_mut(child_id)
                .expect("child")
                .as_dict_mut()
                .expect("child dict")
                .set("Parent", id);
        }
        let mut item = dictionary! {
            "Title" => Object::string_literal(format!("L{level}")),
            "Dest" => fit_dest(page_id),
        };
        if let Some(child_id) = child {
            item.set("First", child_id);
            item.set("Last", child_id);
        }
        document.set_object(id, item);
        child = Some(id);
    }
    attach_outline(&mut document, child.expect("depth >= 1"));
    (document, page_id)
}

/// Three pages. Top-level "Same" on page 0, then "Parent" on page 1, whose
/// child is another "Same" on page 2. Insertion order is the fold position.
fn write_duplicate_title_pdf(path: &std::path::Path) {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    let mut page_ids = Vec::new();
    for _ in 0..3 {
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        kids.push(page_id.into());
        page_ids.push(page_id);
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => 3,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    document.add_bookmark(
        Bookmark::new("Same".into(), [0.0, 0.0, 0.0], 0, page_ids[0]),
        None,
    );
    let parent = document.add_bookmark(
        Bookmark::new("Parent".into(), [0.0, 0.0, 0.0], 0, page_ids[1]),
        None,
    );
    document.add_bookmark(
        Bookmark::new("Same".into(), [0.0, 0.0, 0.0], 0, page_ids[2]),
        Some(parent),
    );
    embed_bookmarks(&mut document);
    document.save(path).expect("save duplicate-title PDF");
}

/// Decodes a PNG and counts pixels that are not white.
fn decode_ink(png_bytes: &[u8]) -> (u32, u32, usize) {
    let decoder = png::Decoder::new(png_bytes);
    let mut reader = decoder.read_info().expect("png header");
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).expect("png frame");
    let channels = info.color_type.samples();
    let dark = buffer[..info.buffer_size()]
        .chunks(channels)
        .filter(|pixel| pixel.iter().take(3).any(|&value| value < 200))
        .count();
    (info.width, info.height, dark)
}

#[test]
fn renders_rotated_page_and_crop_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.pdf");
    write_pdf(&path, true);
    let before = std::fs::read(&path).unwrap();
    let engine = PdfEngine;
    assert_eq!(PDF_RENDERER, "hayro-0.8");

    // Rotate 90: displayed page is 792 x 612 pt, longest edge 512 px.
    let full = engine
        .render_page(&path, PdfPageIndex::new(0), None, 512, 1 << 20)
        .unwrap();
    assert_eq!(full.page_box.page_count, 1);
    assert_eq!(full.page_box.rotation_degrees, 90);
    assert_eq!(
        (full.page_box.width_pt, full.page_box.height_pt),
        (792.0, 612.0)
    );
    assert_eq!(
        engine.page_box(&path, PdfPageIndex::new(0)).unwrap(),
        full.page_box
    );
    assert_eq!(full.crop, NormalizedRect::FULL);
    let image = full.image.expect("fits the byte budget");
    assert_eq!((image.width, image.height), (512, 396));
    let (width, height, dark) = decode_ink(&image.png);
    assert_eq!((width, height), (512, 396));
    assert!(dark > 0, "the text layer must be painted");

    let top_half = NormalizedRect::try_new(0.0, 0.0, 1.0, 0.5).unwrap();
    let cropped = engine
        .render_page(&path, PdfPageIndex::new(0), Some(top_half), 512, 1 << 20)
        .unwrap();
    let image = cropped.image.unwrap();
    assert_eq!((image.width, image.height), (512, 198));
    assert_eq!(cropped.crop, top_half);

    let blank_path = directory.path().join("blank.pdf");
    write_pdf(&blank_path, false);
    let blank = engine
        .render_page(&blank_path, PdfPageIndex::new(0), None, 256, 1 << 20)
        .unwrap();
    assert_eq!(decode_ink(&blank.image.unwrap().png).2, 0);

    assert!(matches!(
        engine.render_page(
            directory.path().join("missing.pdf"),
            PdfPageIndex::new(0),
            None,
            512,
            1 << 20
        ),
        Err(PdfError::Io(_))
    ));
    let text_path = directory.path().join("plain.txt");
    std::fs::write(&text_path, b"not a pdf").unwrap();
    assert!(matches!(
        engine.render_page(&text_path, PdfPageIndex::new(0), None, 512, 1 << 20),
        Err(PdfError::NotPdf(_))
    ));
    assert!(matches!(
        engine.render_page(&path, PdfPageIndex::new(1), None, 512, 1 << 20),
        Err(PdfError::PageOutOfBounds {
            index: 1,
            page_count: 1
        })
    ));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "rendering is read-only"
    );
}

#[test]
fn render_downscales_to_the_byte_budget_or_returns_no_image() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.pdf");
    write_pdf(&path, true);
    let render = PdfEngine
        .render_page(&path, PdfPageIndex::new(0), None, 1024, 64)
        .unwrap();
    assert!(render.image.is_none());
    assert_eq!(render.requested_edge_px, 1024);
    assert!(render.returned_edge_px < 1024 && render.returned_edge_px >= 256);
    assert!(render.last_bytes > 64);
}

#[test]
fn normalized_rect_rejects_out_of_range_or_empty_crops() {
    assert!(NormalizedRect::try_new(0.0, 0.0, 1.0, 1.0).is_ok());
    assert!(NormalizedRect::try_new(-0.1, 0.0, 1.0, 1.0).is_err());
    assert!(NormalizedRect::try_new(0.5, 0.0, 0.5, 1.0).is_err());
    assert!(NormalizedRect::try_new(0.0, 0.6, 1.0, 0.2).is_err());
    assert!(NormalizedRect::try_new(0.0, 0.0, f32::NAN, 1.0).is_err());
}
