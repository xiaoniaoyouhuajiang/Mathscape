//! End-to-end tests of `ink.page_image` (Phase1d-b) on a temp workspace: a
//! real ink note written through the store, a snapshot of it registered as an
//! artifact, ink evidence spans in the source map, plus a non-ink artifact.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{ActorRef, InkLabDocument, InkLabOperation, InkSnapshotManifest, MediaType};
use mathscape_mcp::tools::{
    DEFAULT_MAX_EDGE_PX, MAX_MAX_BYTES, MAX_MAX_EDGE_PX, MIN_MAX_BYTES, MIN_MAX_EDGE_PX,
};
use mathscape_mcp::{McpServer, McpWorkspace};
use mathscape_store_sqlite::{InkLifecycleRequest, InkTitleMode, SqliteArtifactStore};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const NOTE_ID: &str = "0123456789abcdef0123456789abcdef";
const BLANK_ID: &str = "fedcba9876543210fedcba9876543210";
const DENSE_ID: &str = "00112233445566778899aabbccddeeff";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn segment() -> Value {
    json!({
        "id": "segment-1", "started_at": "2026-10-28T00:00:00.000Z", "user_agent": "test",
        "secure_context": true, "coalesced_events": true, "pointer_raw_update": false,
        "device_pixel_ratio": 2, "time_origin_ms": 0, "pressure_capability": "unknown",
        "tilt_capability": "unknown"
    })
}

fn empty_document(id: &str, title: &str, pages: Value) -> InkLabDocument {
    serde_json::from_value(json!({
        "version": "mathscape-ink-lab-1", "document_id": id, "title": title,
        "created_at": "2026-10-28T00:00:00.000Z", "exported_at": "2026-10-28T00:00:00.000Z",
        "pages": pages, "capture_segments": [segment()], "operations": []
    }))
    .unwrap()
}

fn stroke(
    seq: usize,
    id: &str,
    page: &str,
    color: &str,
    width: f64,
    points: &[(f64, f64)],
) -> Value {
    let points: Vec<Value> = points
        .iter()
        .enumerate()
        .map(|(index, (x, y))| {
            json!({"x": x, "y": y, "t_us": 8000 * (index + 1), "pressure": null,
                   "tilt_x_deg": null, "tilt_y_deg": null, "buttons": 1})
        })
        .collect();
    json!({
        "kind": "stroke", "seq": seq, "command_id": format!("cmd-{seq}"),
        "stroke": {
            "id": id, "page_id": page, "capture_segment_id": "segment-1",
            "pointer_type": "pen", "status": "completed",
            "brush": {"renderer": "perfect-freehand@1.2.3", "color": color, "width": width,
                      "smoothing": 0.35, "streamline": 0.15, "thinning": 0.4,
                      "simulate_pressure": false},
            "points": points
        }
    })
}

fn line(from: (f64, f64), to: (f64, f64), steps: usize) -> Vec<(f64, f64)> {
    (0..=steps)
        .map(|step| {
            let t = step as f64 / steps as f64;
            (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        })
        .collect()
}

/// Writes a note through the store: create, then append every operation.
fn write_note(
    store: &mut SqliteArtifactStore,
    mut document: InkLabDocument,
    operations: Vec<Value>,
) -> InkLabDocument {
    let session = format!("session-{}", document.document_id);
    store
        .create_ink_note(
            &document,
            &session,
            "writer",
            &json!({}),
            InkTitleMode::Manual,
            None,
        )
        .unwrap();
    for (index, operation) in operations.into_iter().enumerate() {
        let operation: InkLabOperation = serde_json::from_value(operation).unwrap();
        store
            .append_ink_operation(
                &format!("{}-op-{}", document.document_id, index + 1),
                &document.document_id,
                &session,
                1,
                index as u64,
                &document.metadata(),
                &operation,
            )
            .unwrap();
        document.operations.push(operation);
    }
    document
}

/// Deterministic pseudo-random scribbles (LCG) in many colours.
fn dense_operations(count: usize) -> Vec<Value> {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        ((state >> 33) as f64) / f64::from(u32::MAX >> 1)
    };
    (0..count)
        .map(|index| {
            let points: Vec<(f64, f64)> = (0..12)
                .map(|_| (next() * 1200.0, next() * 1600.0))
                .collect();
            let color = format!(
                "#{:02x}{:02x}{:02x}",
                (next() * 255.0) as u8,
                (next() * 255.0) as u8,
                (next() * 255.0) as u8
            );
            stroke(
                index + 1,
                &format!("d{index}"),
                "pa",
                &color,
                2.0 + next() * 10.0,
                &points,
            )
        })
        .collect()
}

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    server: McpServer,
    markdown: String,
    snapshot_artifact: String,
    snapshot_id: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let markdown = pool
        .import_bytes(
            b"# not ink\n",
            ArtifactMetadata::derived(
                MediaType::parse("text/markdown").unwrap(),
                Some("a.md".to_owned()),
            )
            .unwrap(),
            ActorRef::parse("ink-test").unwrap(),
        )
        .unwrap()
        .stable_id()
        .to_string();

    let mut store = SqliteArtifactStore::open(&root.join("mathscape.sqlite3")).unwrap();
    let pages = json!([
        {"id": "pa", "width": 1200, "height": 1600},
        {"id": "pb", "width": 800, "height": 600}
    ]);
    let operations = vec![
        stroke(
            1,
            "s1",
            "pa",
            "#1f2933",
            4.0,
            &line((100.0, 100.0), (500.0, 140.0), 20),
        ),
        stroke(
            2,
            "s2",
            "pa",
            "#2e86de",
            6.0,
            &line((120.0, 300.0), (400.0, 700.0), 30),
        ),
        stroke(
            3,
            "s3",
            "pa",
            "#c0392b",
            4.0,
            &line((800.0, 900.0), (1000.0, 1200.0), 20),
        ),
        stroke(
            4,
            "gone",
            "pa",
            "#00ff00",
            4.0,
            &line((600.0, 600.0), (700.0, 650.0), 10),
        ),
        json!({"kind": "erase", "seq": 5, "command_id": "cmd-5", "stroke_ids": ["gone"]}),
        stroke(
            6,
            "w1",
            "pb",
            "#1f2933",
            8.0,
            &line((100.0, 100.0), (700.0, 500.0), 30),
        ),
    ];
    let document = write_note(
        &mut store,
        empty_document(NOTE_ID, "极限笔记", pages.clone()),
        operations,
    );
    write_note(
        &mut store,
        empty_document(BLANK_ID, "空白笔记", pages),
        vec![],
    );
    drop(store);

    // Snapshot through seq 6, stored as its own artifact.
    let manifest = InkSnapshotManifest::from_document(&document, 6).unwrap();
    let manifest_json = serde_json::to_string(&manifest).unwrap();
    let digest = hex(&Sha256::digest(manifest_json.as_bytes()));
    let snapshot_artifact = pool
        .import_bytes(
            manifest_json.as_bytes(),
            ArtifactMetadata::derived(
                MediaType::parse("application/json").unwrap(),
                Some("snapshot.json".to_owned()),
            )
            .unwrap(),
            ActorRef::parse("ink-test").unwrap(),
        )
        .unwrap()
        .stable_id()
        .to_string();
    assert_eq!(
        snapshot_artifact,
        format!("mathscape:artifact/sha256/{digest}")
    );
    drop(pool);
    let snapshot_id = format!("mathscape:ink-snapshot/{digest}");
    let connection = Connection::open(root.join("mathscape.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO ink_snapshots
             (snapshot_id, request_id, document_id, through_seq, artifact_id, content_hash,
              byte_size, manifest_json, created_at, label)
             VALUES (?1, 'req-1', ?2, 6, ?3, ?4, ?5, ?6, '2026-10-28T10:00:00+08:00', '第一版')",
            params![
                snapshot_id,
                NOTE_ID,
                snapshot_artifact,
                format!("sha256:{digest}"),
                manifest_json.len() as i64,
                manifest_json
            ],
        )
        .unwrap();
    let mut revision = 0;
    let mut span = |id: &str, page: &str, strokes: &[&str]| {
        revision += 1;
        let revision_id = format!("sha256:{revision:064x}");
        let payload = json!({"payload_kind": "span", "payload": {
            "granularity": "block", "layout": "other", "page": null, "bounding_boxes": [],
            "marker_block_ids": [], "markdown": null, "exact_text_artifact": null,
            "normalized_text_hash": null,
            "ink": {"snapshot_artifact": snapshot_artifact, "page_id": page,
                    "stroke_ids": strokes, "selection_mode": "lasso"}
        }});
        connection
            .execute(
                "INSERT INTO map_objects (object_id, object_shape, entity_kind, current_revision)
                 VALUES (?1, 'entity', 'SourceSpan', ?2)",
                params![id, revision_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO map_object_revisions
                 (object_id, revision_id, schema_version, lifecycle, created_at,
                  created_by, generation_run, payload_json)
                 VALUES (?1, ?2, 'mathscape-source-1', 'active',
                         '2026-10-28T10:00:00+08:00', 'ink-test', NULL, ?3)",
                params![id, revision_id, payload.to_string()],
            )
            .unwrap();
    };
    span(&span_id("pair"), "pa", &["s1", "s2"]);
    span(&span_id("partial"), "pa", &["s1", "gone", "missing"]);
    span(&span_id("hidden"), "pa", &["gone"]);
    span(&span_id("other-page"), "pb", &["w1"]);
    drop(connection);

    let workspace = McpWorkspace::open(&root).unwrap();
    Fixture {
        _dir: dir,
        root,
        server: McpServer::new(workspace),
        markdown,
        snapshot_artifact,
        snapshot_id,
    }
}

fn span_id(name: &str) -> String {
    format!("mathscape:ink-evidence/{:0>64}", hex(name.as_bytes()))
}

/// Raw `tools/call` result.
fn raw(server: &McpServer, arguments: Value) -> Value {
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "ink.page_image", "arguments": arguments }
        }))
        .unwrap();
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
    assert!(result.get("structuredContent").is_some());
    result["structuredContent"]["error"].clone()
}

fn codes(meta: &Value) -> Vec<String> {
    meta["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

fn diag<'a>(meta: &'a Value, code: &str) -> &'a Value {
    meta["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {meta}"))
}

fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    (
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    )
}

fn files(root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path);
            }
        }
    }
    out
}

#[test]
fn snapshot_page_is_a_text_plus_png_envelope_without_structured_content() {
    let f = fixture();
    let result = raw(&f.server, json!({ "node_id": f.snapshot_id }));
    let text = result["content"][0]["text"].as_str().unwrap();
    // Metadata keeps the contract field order.
    let order = [
        "\"status\"",
        "\"truncated\"",
        "\"as_of\"",
        "\"source\"",
        "\"revision\"",
        "\"page\"",
        "\"page_count\"",
        "\"crop\"",
        "\"strokes\"",
        "\"image\"",
        "\"renderer\"",
        "\"diagnostics\"",
    ];
    let positions: Vec<usize> = order.iter().map(|key| text.find(key).unwrap()).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    let source_order = [
        "\"kind\"",
        "\"node_id\"",
        "\"snapshot_id\"",
        "\"artifact_id\"",
        "\"document_id\"",
        "\"note_title\"",
        "\"label\"",
        "\"through_seq\"",
        "\"manifest_version\"",
    ];
    let positions: Vec<usize> = source_order
        .iter()
        .map(|key| text.find(key).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");

    let (meta, png) = image(&f.server, json!({ "node_id": f.snapshot_id }));
    let png = png.unwrap();
    assert_eq!(meta["status"], "ok");
    assert_eq!(meta["truncated"], false);
    assert_eq!(meta["source"]["kind"], "snapshot");
    assert_eq!(meta["source"]["snapshot_id"], f.snapshot_id);
    assert_eq!(meta["source"]["artifact_id"], f.snapshot_artifact);
    assert_eq!(meta["source"]["document_id"], NOTE_ID);
    assert_eq!(meta["source"]["note_title"], "极限笔记");
    assert_eq!(meta["source"]["label"], "第一版");
    assert_eq!(meta["source"]["through_seq"], 6);
    assert_eq!(
        meta["source"]["manifest_version"],
        mathscape_domain::INK_SNAPSHOT_VERSION
    );
    assert!(meta["source"].get("head_seq").is_none());
    assert_eq!(meta["revision"], format!("sha256:{}", &f.snapshot_id[23..]));
    assert_eq!(
        meta["page"],
        json!({"id": "pa", "index": 0, "width": 1200.0, "height": 1600.0})
    );
    assert_eq!(meta["page_count"], 2);
    assert_eq!(meta["crop"]["mode"], "page");
    assert_eq!(meta["strokes"]["visible_on_page"], 3);
    assert_eq!(meta["strokes"]["rendered"], 3);
    assert_eq!(meta["strokes"]["selected"], Value::Null);
    assert_eq!(meta["image"]["format"], "png");
    assert_eq!(meta["image"]["width"], 768);
    assert_eq!(meta["image"]["height"], DEFAULT_MAX_EDGE_PX);
    assert_eq!(meta["image"]["scale"], 0.64);
    assert_eq!(png_size(&png), (768, 1024));
    assert_eq!(meta["image"]["bytes"], png.len());
    assert_eq!(meta["image"]["sha256"], hex(&Sha256::digest(&png)));
    assert!(
        meta["renderer"]
            .as_str()
            .unwrap()
            .contains("perfect-freehand@1.2.3")
    );
    assert!(codes(&meta).is_empty(), "{meta}");
}

#[test]
fn rendering_is_deterministic_and_read_only() {
    let f = fixture();
    let before = files(&f.root);
    let (first_meta, first) = image(&f.server, json!({ "node_id": f.snapshot_id }));
    let (second_meta, second) = image(&f.server, json!({ "node_id": f.snapshot_id }));
    assert_eq!(first, second);
    assert_eq!(first_meta["image"], second_meta["image"]);
    image(&f.server, json!({ "node_id": NOTE_ID, "crop": "strokes" }));
    image(&f.server, json!({ "node_id": span_id("pair") }));
    assert_eq!(files(&f.root), before, "no cache or temp files");
}

#[test]
fn snapshot_artifact_id_is_canonicalized() {
    let f = fixture();
    let (meta, png) = image(&f.server, json!({ "node_id": f.snapshot_artifact }));
    assert!(png.is_some());
    assert_eq!(meta["source"]["node_id"], f.snapshot_id);
    let canonical = diag(&meta, "id_canonicalized");
    assert_eq!(canonical["details"]["canonical_id"], f.snapshot_id);
    assert_eq!(canonical["details"]["input_id"], f.snapshot_artifact);
}

#[test]
fn note_renders_live_head_without_revision() {
    let f = fixture();
    let (meta, png) = image(&f.server, json!({ "node_id": NOTE_ID, "page_index": 1 }));
    assert!(png.is_some());
    assert_eq!(meta["source"]["kind"], "note");
    assert_eq!(meta["source"]["head_seq"], 6);
    assert_eq!(meta["revision"], Value::Null);
    assert_eq!(meta["page"]["id"], "pb");
    assert_eq!(meta["image"]["width"], 1024);
    assert_eq!(meta["image"]["height"], 768);
    assert_eq!(diag(&meta, "live_note_head")["details"]["head_seq"], 6);

    let (by_id, _) = image(&f.server, json!({ "node_id": NOTE_ID, "page_id": "pb" }));
    assert_eq!(by_id["image"], meta["image"]);
}

#[test]
fn include_image_false_keeps_structured_metadata() {
    let f = fixture();
    let (meta, png) = image(
        &f.server,
        json!({ "node_id": f.snapshot_id, "include_image": false }),
    );
    assert!(png.is_none());
    assert_eq!(meta["status"], "ok");
    assert_eq!(meta["image"]["width"], 768);
    assert_eq!(meta["image"]["height"], 1024);
    assert_eq!(meta["image"]["bytes"], Value::Null);
    assert_eq!(meta["image"]["sha256"], Value::Null);
}

#[test]
fn span_defaults_to_selection_crop_and_reports_hidden_strokes() {
    let f = fixture();
    let (meta, png) = image(&f.server, json!({ "node_id": span_id("pair") }));
    assert!(png.is_some());
    assert_eq!(meta["source"]["kind"], "span");
    assert_eq!(meta["source"]["span_id"], span_id("pair"));
    assert_eq!(meta["source"]["snapshot_id"], f.snapshot_id);
    assert_eq!(meta["revision"], format!("sha256:{:064x}", 1));
    assert_eq!(meta["crop"]["mode"], "selection");
    assert_eq!(meta["strokes"]["selected"], 2);
    let bounds = &meta["crop"]["bounds"];
    assert!(
        bounds["x"].as_f64().unwrap() < 100.0 && bounds["width"].as_f64().unwrap() < 600.0,
        "{meta}"
    );
    assert!(codes(&meta).is_empty(), "{meta}");

    let (page, _) = image(
        &f.server,
        json!({ "node_id": span_id("pair"), "crop": "page" }),
    );
    assert_eq!(page["crop"]["mode"], "page");
    assert_eq!(page["strokes"]["rendered"], 3);

    let (partial, png) = image(&f.server, json!({ "node_id": span_id("partial") }));
    assert!(png.is_some());
    assert_eq!(partial["strokes"]["selected"], 1);
    let missing = &diag(&partial, "selection_partially_hidden")["details"]["missing_stroke_ids"];
    assert_eq!(*missing, json!(["gone", "missing"]));

    let (hidden, _) = image(&f.server, json!({ "node_id": span_id("hidden") }));
    assert_eq!(hidden["status"], "ok");
    assert!(
        codes(&hidden).contains(&"selection_hidden".to_owned()),
        "{hidden}"
    );

    let (other, _) = image(&f.server, json!({ "node_id": span_id("other-page") }));
    assert_eq!(other["page"]["id"], "pb");
    assert_eq!(other["strokes"]["selected"], 1);
}

#[test]
fn bounds_override_crop_and_must_intersect_the_page() {
    let f = fixture();
    let (meta, png) = image(
        &f.server,
        json!({ "node_id": f.snapshot_id, "crop": "strokes",
                "bounds": {"x": 1000, "y": 1400, "width": 400, "height": 400} }),
    );
    assert_eq!(meta["crop"]["mode"], "bounds");
    assert_eq!(
        meta["crop"]["bounds"],
        json!({"x": 1000.0, "y": 1400.0, "width": 200.0, "height": 200.0})
    );
    assert_eq!(png_size(&png.unwrap()), (1024, 1024));
    let outside = error(
        &f.server,
        json!({ "node_id": f.snapshot_id, "bounds": {"x": 5000, "y": 0, "width": 10, "height": 10} }),
    );
    assert_eq!(outside["code"], "invalid_arg", "{outside}");
}

#[test]
fn argument_errors_are_invalid_arg() {
    let f = fixture();
    let snapshot = f.snapshot_id.as_str();
    let cases = [
        json!({}),
        json!({ "node_id": snapshot, "unknown": 1 }),
        json!({ "node_id": snapshot, "max_edge_px": MIN_MAX_EDGE_PX - 1 }),
        json!({ "node_id": snapshot, "max_edge_px": MAX_MAX_EDGE_PX + 1 }),
        json!({ "node_id": snapshot, "max_bytes": MIN_MAX_BYTES - 1 }),
        json!({ "node_id": snapshot, "max_bytes": MAX_MAX_BYTES + 1 }),
        json!({ "node_id": snapshot, "page_id": "pa", "page_index": 0 }),
        json!({ "node_id": snapshot, "page_index": 100 }),
        json!({ "node_id": snapshot, "crop": "lasso" }),
        json!({ "node_id": snapshot, "crop": "selection" }),
        json!({ "node_id": NOTE_ID, "crop": "selection" }),
        json!({ "node_id": snapshot, "bounds": {"x": 0, "y": 0, "width": 0, "height": 10} }),
        json!({ "node_id": "not an id" }),
        json!({ "node_id": "" }),
    ];
    for arguments in cases {
        let error = error(&f.server, arguments.clone());
        assert_eq!(error["code"], "invalid_arg", "{arguments} -> {error}");
    }
    let webp = error(&f.server, json!({ "node_id": snapshot, "format": "webp" }));
    assert_eq!(webp["code"], "invalid_arg");
    assert_eq!(
        webp["details"]["supported_formats"],
        json!(["png"]),
        "{webp}"
    );

    let markdown = error(&f.server, json!({ "node_id": f.markdown }));
    assert_eq!(markdown["code"], "invalid_arg");
    assert_eq!(markdown["details"]["node_kind"], "artifact", "{markdown}");
    assert_eq!(
        markdown["details"]["hint"],
        "use material.read with content_ref"
    );
}

#[test]
fn missing_nodes_pages_and_trashed_notes_are_not_found() {
    let f = fixture();
    let missing_snapshot = format!("mathscape:ink-snapshot/{}", "0".repeat(64));
    for node_id in [
        missing_snapshot.as_str(),
        "ffffffffffffffffffffffffffffffff",
        "mathscape:artifact/sha256/1111111111111111111111111111111111111111111111111111111111111111",
        &span_id("nope"),
    ] {
        let (meta, png) = image(&f.server, json!({ "node_id": node_id }));
        assert!(png.is_none());
        assert_eq!(meta["status"], "not_found", "{node_id}: {meta}");
    }

    let (page, png) = image(
        &f.server,
        json!({ "node_id": f.snapshot_id, "page_id": "zz" }),
    );
    assert!(png.is_none());
    assert_eq!(page["status"], "not_found");
    assert_eq!(page["page_count"], 2);
    assert_eq!(page["page_ids"], json!(["pa", "pb"]));
    let (index, _) = image(
        &f.server,
        json!({ "node_id": f.snapshot_id, "page_index": 2 }),
    );
    assert_eq!(index["status"], "not_found");

    let mut store = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    let revision = store
        .open_ink_document(NOTE_ID)
        .unwrap()
        .unwrap()
        .note
        .lifecycle_revision;
    store
        .trash_ink_note(&InkLifecycleRequest {
            request_id: "trash-1".to_owned(),
            document_id: NOTE_ID.to_owned(),
            expected_lifecycle_revision: revision,
        })
        .unwrap();
    drop(store);
    let (trashed, png) = image(&f.server, json!({ "node_id": NOTE_ID }));
    assert!(png.is_none());
    assert_eq!(trashed["status"], "not_found");
    assert!(codes(&trashed).contains(&"trashed".to_owned()), "{trashed}");
    let (included, png) = image(
        &f.server,
        json!({ "node_id": NOTE_ID, "include_trashed": true }),
    );
    assert!(png.is_some());
    assert_eq!(included["status"], "ok");
}

#[test]
fn blank_page_is_ok_with_a_diagnostic() {
    let f = fixture();
    let (meta, png) = image(&f.server, json!({ "node_id": BLANK_ID, "crop": "strokes" }));
    assert!(png.is_some());
    assert_eq!(meta["status"], "ok");
    assert_eq!(meta["strokes"]["visible_on_page"], 0);
    assert_eq!(meta["strokes"]["ink_bounds"], Value::Null);
    assert!(codes(&meta).contains(&"page_blank".to_owned()), "{meta}");
}

#[test]
fn byte_budget_downscales_then_truncates() {
    let f = fixture();
    // Only this test needs the dense note; building it is the slow part.
    let mut store = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    write_note(
        &mut store,
        empty_document(
            DENSE_ID,
            "密集笔记",
            json!([{"id": "pa", "width": 1200, "height": 1600}]),
        ),
        dense_operations(150),
    );
    drop(store);
    let (full, png) = image(
        &f.server,
        json!({ "node_id": DENSE_ID, "max_edge_px": 2048, "max_bytes": MAX_MAX_BYTES }),
    );
    let full_bytes = full["image"]["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("{full}"));
    assert!(png.is_some(), "{full}");

    // A budget just under the full-size PNG forces at least one downscale.
    let budget = (full_bytes - 1).max(MIN_MAX_BYTES as u64);
    let (meta, png) = image(
        &f.server,
        json!({ "node_id": DENSE_ID, "max_edge_px": 2048, "max_bytes": budget }),
    );
    if let Some(png) = png {
        assert_eq!(meta["status"], "ok");
        let downscaled = diag(&meta, "image_downscaled");
        assert_eq!(downscaled["details"]["requested_edge_px"], 2048);
        assert_eq!(downscaled["details"]["reason"], "max_bytes");
        let returned = downscaled["details"]["returned_edge_px"].as_u64().unwrap();
        assert!(returned < 2048);
        assert_eq!(meta["image"]["height"], returned);
        assert!(png.len() as u64 <= budget);
    } else {
        assert_eq!(meta["status"], "truncated", "{meta}");
    }

    let (cut, png) = image(
        &f.server,
        json!({ "node_id": DENSE_ID, "max_edge_px": 2048, "max_bytes": MIN_MAX_BYTES }),
    );
    assert!(png.is_none());
    assert_eq!(cut["status"], "truncated", "{cut}");
    assert_eq!(cut["truncated"], true);
    assert_eq!(cut["image"], Value::Null);
    assert!(codes(&cut).contains(&"image_too_large".to_owned()), "{cut}");
}

#[test]
fn tampered_manifest_is_internal_integrity_failed() {
    let f = fixture();
    let connection = Connection::open(f.root.join("mathscape.sqlite3")).unwrap();
    let changed = connection.execute(
        "UPDATE ink_snapshots SET manifest_json = manifest_json || ' ' WHERE snapshot_id = ?1",
        params![f.snapshot_id],
    );
    if changed.is_err() {
        // Immutable rows: nothing to tamper with through SQL.
        return;
    }
    drop(connection);
    let error = error(&f.server, json!({ "node_id": f.snapshot_id }));
    assert_eq!(error["code"], "internal", "{error}");
    assert_eq!(error["details"]["reason"], "integrity_failed", "{error}");
    let span = error_for_span(&f);
    assert_eq!(span["details"]["reason"], "integrity_failed", "{span}");
}

fn error_for_span(f: &Fixture) -> Value {
    error(&f.server, json!({ "node_id": span_id("pair") }))
}
