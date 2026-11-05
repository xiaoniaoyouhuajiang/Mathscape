//! Contract tests for `pdf.page_image`: read-only PDF page rendering.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus};
use mathscape_mcp::{McpServer, McpWorkspace};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const UNKNOWN_ID: &str =
    "mathscape:artifact/sha256/0000000000000000000000000000000000000000000000000000000000000000";

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    server: McpServer,
    pdf: String,
    markdown: String,
}

/// Two Letter pages; page 2 is rotated 90°.
fn pdf_bytes() -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut kids = Vec::new();
    for (text, rotate) in [("Page one heading", 0), ("Rotated page two", 90)] {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 36.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id =
            document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "Rotate" => rotate,
        });
        kids.push(page_id.into());
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => kids, "Count" => 2,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn import(pool: &mut ArtifactPool, bytes: &[u8], media: &str, name: &str) -> String {
    let metadata = ArtifactMetadata::new(
        None,
        MediaType::parse(media).unwrap(),
        Some(name.to_owned()),
        None,
        RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
    );
    pool.import_bytes(bytes, metadata, ActorRef::parse("test").unwrap())
        .unwrap()
        .stable_id()
        .to_string()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let pdf = import(&mut pool, &pdf_bytes(), "application/pdf", "pages.pdf");
    let markdown = import(&mut pool, b"# note\n", "text/markdown", "note.md");
    drop(pool);
    let server = McpServer::new(McpWorkspace::open(&root).unwrap());
    Fixture {
        _dir: dir,
        root,
        server,
        pdf,
        markdown,
    }
}

fn raw(server: &McpServer, arguments: Value) -> Value {
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "pdf.page_image", "arguments": arguments }
        }))
        .unwrap();
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}

/// Metadata text block (parsed), plus PNG bytes when an image block is present.
fn image(server: &McpServer, arguments: Value) -> (Value, Option<Vec<u8>>) {
    use base64::Engine as _;
    let result = raw(server, arguments);
    assert_eq!(result["isError"], false, "{result}");
    let content = result["content"].as_array().unwrap();
    let meta: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    let png = content.get(1).map(|block| {
        assert_eq!(block["type"], "image");
        assert_eq!(block["mimeType"], "image/png");
        base64::engine::general_purpose::STANDARD
            .decode(block["data"].as_str().unwrap())
            .unwrap()
    });
    if png.is_some() {
        assert!(result.get("structuredContent").is_none(), "{result}");
        assert_eq!(content.len(), 2);
    } else {
        assert_eq!(content.len(), 1);
        assert_eq!(result["structuredContent"], meta, "{result}");
    }
    (meta, png)
}

fn error(server: &McpServer, arguments: Value) -> Value {
    let result = raw(server, arguments);
    assert_eq!(result["isError"], true, "{result}");
    result["structuredContent"]["error"].clone()
}

fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    (
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    )
}

/// Every file under the workspace with its SHA-256 (catalog included).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let digest = Sha256::digest(std::fs::read(&path).unwrap()).to_vec();
                out.insert(path, digest);
            }
        }
    }
    out
}

#[test]
fn renders_a_page_with_metadata_and_never_writes() {
    let f = fixture();
    let before = snapshot(&f.root);

    let (meta, png) = image(&f.server, json!({ "artifact_id": f.pdf, "page": 1 }));
    let png = png.expect("image block");
    assert_eq!(meta["status"], "ok", "{meta}");
    assert_eq!(meta["truncated"], false);
    assert_eq!(meta["artifact_id"], f.pdf);
    assert!(meta["revision"].is_string());
    assert_eq!(meta["page"], 1);
    assert_eq!(meta["page_count"], 2);
    assert_eq!(
        meta["page_size"],
        json!({ "width": 612.0, "height": 792.0, "unit": "pt", "rotation": 0 })
    );
    assert_eq!(
        meta["crop"],
        json!({ "left": 0.0, "top": 0.0, "right": 1.0, "bottom": 1.0 })
    );
    assert_eq!(meta["renderer"], "hayro-0.8");
    let (width, height) = png_size(&png);
    assert_eq!(height, 1024, "default max_edge_px bounds the long edge");
    assert_eq!(
        (
            meta["image"]["width"].as_u64(),
            meta["image"]["height"].as_u64()
        ),
        (Some(u64::from(width)), Some(u64::from(height)))
    );
    assert_eq!(meta["image"]["format"], "png");
    assert_eq!(meta["image"]["bytes"], png.len());
    let digest: String = Sha256::digest(&png)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(meta["image"]["sha256"], digest);

    // A normalized crop of the rotated page: displayed size is 792×612.
    let (meta, png) = image(
        &f.server,
        json!({
            "artifact_id": f.pdf, "page": 2, "max_edge_px": 512,
            "crop": { "left": 0.0, "top": 0.0, "right": 1.0, "bottom": 0.5 },
        }),
    );
    let (width, height) = png_size(&png.expect("image block"));
    assert_eq!(meta["page_size"]["rotation"], 90);
    assert_eq!(meta["page_size"]["width"], 792.0);
    assert_eq!((width, height), (512, 198));

    // Metadata only: no image block, structuredContent mirrors the text.
    let (meta, png) = image(
        &f.server,
        json!({ "artifact_id": f.pdf, "page": 1, "include_image": false, "max_edge_px": 256 }),
    );
    assert!(png.is_none());
    assert_eq!(meta["status"], "ok");
    assert_eq!(meta["image"]["height"], 256);
    assert!(meta["image"]["bytes"].is_null());

    assert_eq!(snapshot(&f.root), before, "pdf.page_image writes nothing");
}

#[test]
fn out_of_range_page_is_not_found_with_page_count() {
    let f = fixture();
    let (meta, png) = image(&f.server, json!({ "artifact_id": f.pdf, "page": 3 }));
    assert!(png.is_none());
    assert_eq!(meta["status"], "not_found");
    assert_eq!(meta["page_count"], 2);
    assert_eq!(meta["diagnostics"][0]["code"], "page_not_found");
    assert_eq!(meta["diagnostics"][0]["details"]["page_count"], 2);

    let (meta, _) = image(&f.server, json!({ "artifact_id": UNKNOWN_ID, "page": 1 }));
    assert_eq!(meta["status"], "not_found");
    assert_eq!(meta["diagnostics"][0]["code"], "artifact_not_found");
}

#[test]
fn rejects_bad_arguments_and_non_pdf_materials() {
    let f = fixture();
    for arguments in [
        json!({ "artifact_id": f.pdf, "page": 0 }),
        json!({ "artifact_id": f.pdf }),
        json!({ "artifact_id": f.pdf, "page": 1, "crop": { "left": 0.5, "top": 0.0, "right": 0.5, "bottom": 1.0 } }),
        json!({ "artifact_id": f.pdf, "page": 1, "crop": { "left": 0.0, "top": 0.0, "right": 1.5, "bottom": 1.0 } }),
        json!({ "artifact_id": f.pdf, "page": 1, "format": "jpeg" }),
        json!({ "artifact_id": f.pdf, "page": 1, "max_edge_px": 64 }),
        json!({ "artifact_id": f.pdf, "page": 1, "page_index": 0 }),
        json!({ "artifact_id": "not an id", "page": 1 }),
    ] {
        let error = error(&f.server, arguments.clone());
        assert_eq!(error["code"], "invalid_arg", "{arguments} -> {error}");
    }
    let error = error(&f.server, json!({ "artifact_id": f.markdown, "page": 1 }));
    assert_eq!(error["code"], "invalid_arg");
    assert_eq!(
        error["details"]["supported_media_types"],
        json!(["application/pdf"])
    );
}
