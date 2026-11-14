//! End-to-end tests of the read-only MCP slice on a fixture workspace built
//! through the real artifact pool (markdown + generated PDFs).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;

use lopdf::{
    Bookmark, Document, Object, ObjectId, Stream, content::Content, content::Operation, dictionary,
};
use mathscape_agent::{CommandOutput, CommandRunner};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, CanonicalMarkdownKey, CommandRecordStatus, ConversionExitStatus,
    ConversionParameters, ConversionRun, EntityHeader, EntityKind, LifecycleStatus, MediaType,
    PageIndexBase, PageRange, RevisionId, RightsRecord, RightsStatus, SchemaVersion, Timestamp,
};
use mathscape_mcp::tools::{
    MAX_GREP_HITS, MAX_OUTLINE_ENTRIES, MAX_OUTLINE_ENTRIES_BYTES, MAX_OUTLINE_TITLE_CHARS,
    MAX_READ_BYTES, MAX_SCAN_BYTES, TOOL_NAMES,
};
use mathscape_mcp::{McpServer, McpWorkspace};
use mathscape_query::search::{SearchKind, SearchQuery, SearchRegistry, search};
use mathscape_store_sqlite::SqliteArtifactStore;
use serde_json::{Value, json};
use tempfile::TempDir;

const UNKNOWN_ID: &str =
    "mathscape:artifact/sha256/0000000000000000000000000000000000000000000000000000000000000000";

/// CommandRunner where nothing is installed (Marker missing).
struct NothingInstalled;

impl CommandRunner for NothingInstalled {
    fn run(&self, _program: &Path, _args: &[&str]) -> io::Result<CommandOutput> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
    fn env_var(&self, _key: &str) -> Option<OsString> {
        None
    }
    fn is_file(&self, _path: &Path) -> bool {
        false
    }
}

struct Fixture {
    _dir: TempDir,
    server: McpServer,
    root: std::path::PathBuf,
    markdown: String,
    big_markdown: String,
    plain_text: String,
    pdf: String,
    scanned_pdf: String,
    outlined_pdf: String,
    markdown_body: String,
}

/// `pages[i]` = text lines of page i+1 (empty = no text layer).
fn pdf_bytes(pages: &[&[&str]]) -> Vec<u8> {
    let mut document = pdf_document(pages);
    finish_pdf(&mut document)
}

fn pdf_document(pages: &[&[&str]]) -> Document {
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
    for lines in pages {
        let mut operations = Vec::new();
        if !lines.is_empty() {
            operations.push(Operation::new("BT", vec![]));
            operations.push(Operation::new("Tf", vec!["F1".into(), 18.into()]));
            operations.push(Operation::new("Td", vec![72.into(), 700.into()]));
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    operations.push(Operation::new("Td", vec![0.into(), (-24).into()]));
                }
                operations.push(Operation::new("Tj", vec![Object::string_literal(*line)]));
            }
            operations.push(Operation::new("ET", vec![]));
        }
        let content = Content { operations };
        let content_id =
            document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        kids.push(page_id.into());
    }
    let count = kids.len() as i64;
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    document
}

/// Writes bookmarks into `/Outlines` before serializing. `save` alone does not.
fn finish_pdf(document: &mut Document) -> Vec<u8> {
    if let Some(outline_id) = document.build_outline() {
        document
            .catalog_mut()
            .expect("catalog")
            .set("Outlines", Object::Reference(outline_id));
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

/// Three text pages and a nested embedded outline.
/// 第一章 → page index 0, 引理 → 1, Chapter 2 → 2.
fn save_pdf(document: &mut Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn one_page_outline_document() -> (Document, ObjectId) {
    let document = pdf_document(&[&["Page"]]);
    let page_id = document.get_pages()[&1];
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

/// `/Dest` has one element. `get_toc` indexes `[0]` and `[1]`.
fn short_dest_pdf() -> Vec<u8> {
    let (mut document, page_id) = one_page_outline_document();
    let item = document.add_object(dictionary! {
        "Title" => Object::string_literal("Short"),
        "Dest" => vec![page_id.into()],
    });
    attach_outline(&mut document, item);
    save_pdf(&mut document)
}

/// `/First` points at the item itself.
fn first_cycle_pdf() -> Vec<u8> {
    let (mut document, page_id) = one_page_outline_document();
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
    save_pdf(&mut document)
}

/// `/Next` links A → B → A.
fn next_cycle_pdf() -> Vec<u8> {
    let (mut document, page_id) = one_page_outline_document();
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
    save_pdf(&mut document)
}

fn outlined_pdf_bytes() -> Vec<u8> {
    let mut document = pdf_document(&[&["Chapter page"], &["Section page"], &["Last page"]]);
    let pages = document.get_pages();
    let chapter = document.add_bookmark(
        Bookmark::new("第一章".into(), [0.0, 0.0, 0.0], 0, pages[&1]),
        None,
    );
    document.add_bookmark(
        Bookmark::new("引理".into(), [0.0, 0.0, 0.0], 0, pages[&2]),
        Some(chapter),
    );
    document.add_bookmark(
        Bookmark::new("Chapter 2".into(), [0.0, 0.0, 0.0], 0, pages[&3]),
        None,
    );
    finish_pdf(&mut document)
}

/// One page, top-level bookmarks with the given titles, all on page 1.
fn bookmarks_pdf_bytes(titles: &[String]) -> Vec<u8> {
    let mut document = pdf_document(&[&["Only page"]]);
    let page = document.get_pages()[&1];
    for title in titles {
        document.add_bookmark(Bookmark::new(title.clone(), [0.0, 0.0, 0.0], 0, page), None);
    }
    finish_pdf(&mut document)
}

fn import(
    pool: &mut ArtifactPool,
    bytes: Vec<u8>,
    media: &str,
    name: &str,
    pages: Option<u32>,
) -> String {
    let metadata = ArtifactMetadata::new(
        None,
        MediaType::parse(media).unwrap(),
        Some(name.to_owned()),
        pages,
        RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
    );
    pool.import_bytes(&bytes, metadata, ActorRef::parse("test").unwrap())
        .unwrap()
        .stable_id()
        .to_string()
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let markdown_body = "# 引理 Lemma\n\nEvery bounded monotone sequence converges.\n".to_owned();
    let markdown = import(
        &mut pool,
        markdown_body.clone().into_bytes(),
        "text/markdown",
        "lemma-notes.md",
        None,
    );
    let big: String = "αβγ lemma line\n".repeat(4000);
    let big_markdown = import(
        &mut pool,
        big.into_bytes(),
        "text/markdown",
        "big-lemma.md",
        None,
    );
    let plain_text = import(
        &mut pool,
        b"plain".to_vec(),
        "text/plain",
        "plain.txt",
        None,
    );
    let pdf = import(
        &mut pool,
        pdf_bytes(&[
            &["First text run", "Second text run"],
            &[],
            &["Third page text"],
        ]),
        "application/pdf",
        "lemma-paper.pdf",
        Some(3),
    );
    let scanned_pdf = import(
        &mut pool,
        pdf_bytes(&[&[]]),
        "application/pdf",
        "scan.pdf",
        Some(1),
    );
    let outlined_pdf = import(
        &mut pool,
        outlined_pdf_bytes(),
        "application/pdf",
        "outlined-paper.pdf",
        Some(3),
    );
    drop(pool);
    let workspace = McpWorkspace::open(&root)
        .unwrap()
        .with_round(Some("placeholder".into()))
        .with_capability_runner(Box::new(NothingInstalled));
    Fixture {
        _dir: dir,
        server: McpServer::new(workspace),
        root,
        markdown,
        big_markdown,
        plain_text,
        pdf,
        scanned_pdf,
        outlined_pdf,
        markdown_body,
    }
}

fn request(server: &McpServer, id: i64, method: &str, params: Value) -> Value {
    server
        .handle_message(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .expect("requests get a response")
}

/// Returns `(structuredContent, isError)` and checks the text mirror.
fn call(server: &McpServer, name: &str, arguments: Value) -> (Value, bool) {
    let response = request(
        server,
        7,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    let result = &response["result"];
    assert!(
        response.get("error").is_none(),
        "unexpected JSON-RPC error: {response}"
    );
    let structured = result["structuredContent"].clone();
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, structured, "text content mirrors structuredContent");
    (structured, result["isError"].as_bool().unwrap())
}

fn read(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "material.read", arguments);
    assert!(!is_error, "unexpected call error: {result}");
    result
}

fn read_error(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "material.read", arguments);
    assert!(is_error, "expected call error, got {result}");
    result["error"].clone()
}

fn grep(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "material.grep", arguments);
    assert!(!is_error, "unexpected call error: {result}");
    result
}

fn outline(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "pdf.outline", arguments);
    assert!(!is_error, "unexpected call error: {result}");
    result
}

fn grep_error(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "material.grep", arguments);
    assert!(is_error, "expected call error, got {result}");
    result["error"].clone()
}

fn outline_error(server: &McpServer, arguments: Value) -> Value {
    let (result, is_error) = call(server, "pdf.outline", arguments);
    assert!(is_error, "expected call error, got {result}");
    result["error"].clone()
}

fn artifact_count(root: &Path) -> usize {
    SqliteArtifactStore::open(&root.join("mathscape.sqlite3"))
        .unwrap()
        .read_catalog_snapshot()
        .unwrap()
        .artifacts()
        .len()
}

fn map_object_count(root: &Path) -> usize {
    let connection = rusqlite::Connection::open(root.join("mathscape.sqlite3")).unwrap();
    connection
        .query_row("SELECT COUNT(*) FROM map_objects", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap() as usize
}

fn seed_source_span(root: &Path, markdown: &str) -> String {
    let object_id = "mathscape:source-span/phase2-fixture";
    let revision = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let connection = rusqlite::Connection::open(root.join("mathscape.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO map_objects(object_id, object_shape, entity_kind, current_revision)
             VALUES (?1, 'entity', 'SourceSpan', ?2)",
            rusqlite::params![object_id, revision],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO map_object_revisions
             (object_id, revision_id, schema_version, lifecycle, created_at, created_by, generation_run, payload_json)
             VALUES (?1, ?2, 'mathscape-source-1', 'active', '2026-11-09T00:00:00Z', 'test', NULL, ?3)",
            rusqlite::params![
                object_id,
                revision,
                json!({
                    "payload_kind": "span",
                    "payload": { "markdown": { "artifact": markdown } }
                })
                .to_string()
            ],
        )
        .unwrap();
    object_id.to_owned()
}

#[test]
fn initialize_ping_and_notifications() {
    let f = fixture();
    let response = request(
        &f.server,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        }),
    );
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["serverInfo"]["name"], "mathscape");
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        response["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert!(
        f.server
            .handle_message(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none()
    );
    assert_eq!(
        request(&f.server, 2, "ping", Value::Null)["result"],
        json!({})
    );
    assert_eq!(
        request(&f.server, 3, "resources/list", Value::Null)["error"]["code"],
        -32601
    );
    assert_eq!(
        f.server.handle_line("{nope").unwrap()["error"]["code"],
        -32700
    );
    let unknown_tool = request(&f.server, 4, "tools/call", json!({ "name": "graph.path" }));
    assert_eq!(unknown_tool["error"]["code"], -32602);
    assert_eq!(f.server.workspace().round().as_deref(), Some("placeholder"));
}

#[test]
fn round_file_overrides_the_fixed_round_on_every_call() {
    let dir = tempfile::tempdir().unwrap();
    let f = fixture();
    let round_file = dir.path().join("current_round");
    let workspace = McpWorkspace::open(&f.root)
        .unwrap()
        .with_round(Some("startup".into()))
        .with_round_file(Some(round_file.clone()));
    // Missing or blank file: fall back to the startup value.
    assert_eq!(workspace.round().as_deref(), Some("startup"));
    std::fs::write(&round_file, "  \n").unwrap();
    assert_eq!(workspace.round().as_deref(), Some("startup"));
    std::fs::write(&round_file, "codex-1-1\n").unwrap();
    assert_eq!(workspace.round().as_deref(), Some("codex-1-1"));
    std::fs::write(&round_file, "codex-1-2").unwrap();
    assert_eq!(workspace.round().as_deref(), Some("codex-1-2"));
}

#[test]
fn tools_list_order_and_text_are_fixed() {
    let f = fixture();
    let first = request(&f.server, 1, "tools/list", json!({}));
    let tools = first["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, TOOL_NAMES);
    assert_eq!(
        names,
        [
            "workspace.search",
            "material.read",
            "material.grep",
            "pdf.outline",
            "graph.get",
            "graph.neighborhood",
            "ink.page_image",
            "pdf.page_image",
            "mathlib.search",
            "lean.check",
            "artifact.stage",
            "propose_changes",
            "get_change_set",
            "material.locate",
            "list_change_sets"
        ]
    );
    for tool in tools {
        assert!(!tool["description"].as_str().unwrap().is_empty());
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], json!(false));
        if tool["name"] == "material.read" {
            assert_eq!(
                tool["annotations"],
                json!({
                    "readOnlyHint": false,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                }),
                "material.read may run a bounded ensure and must say so"
            );
        } else if matches!(
            tool["name"].as_str(),
            Some("artifact.stage") | Some("propose_changes")
        ) {
            assert_eq!(
                tool["annotations"],
                json!({
                    "readOnlyHint": false,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                }),
                "write-like tools are bounded staging/proposal exceptions"
            );
        } else if matches!(
            tool["name"].as_str(),
            Some("get_change_set") | Some("list_change_sets")
        ) {
            assert_eq!(
                tool["annotations"],
                json!({
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                })
            );
        } else {
            assert_eq!(
                tool["annotations"],
                json!({
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "openWorldHint": false,
                }),
                "{} must stay read-only for Codex approval",
                tool["name"]
            );
        }
    }
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["query"]));
    assert_eq!(
        tools[1]["inputSchema"]["required"],
        json!(["artifact_id", "locator"])
    );
    assert_eq!(tools[2]["name"], "material.grep");
    assert_eq!(
        tools[2]["inputSchema"]["required"],
        json!(["artifact_ids", "pattern"])
    );
    let grep_description = tools[2]["description"].as_str().unwrap();
    assert!(grep_description.contains("case-sensitive"));
    assert!(grep_description.contains("material.read"));
    assert!(grep_description.contains("non-empty"));
    assert_eq!(tools[3]["name"], "pdf.outline");
    assert_eq!(tools[3]["inputSchema"]["required"], json!(["artifact_id"]));
    let outline_description = tools[3]["description"].as_str().unwrap();
    assert!(
        outline_description.contains("0-based"),
        "{outline_description}"
    );
    assert!(
        outline_description.contains("1-based"),
        "{outline_description}"
    );
    assert_eq!(tools[4]["inputSchema"]["required"], json!(["node_id"]));
    assert!(tools[5]["inputSchema"].get("required").is_none());
    for tool in &tools[4..6] {
        let description = tool["description"].as_str().unwrap();
        assert!(description.contains("content_ref"), "{description}");
        assert!(description.contains("material.read"), "{description}");
        assert!(!description.contains("not callable"), "{description}");
    }
    // Phase1d-b: the 7th tool is appended; the first six are unchanged.
    assert_eq!(tools[6]["name"], "ink.page_image");
    assert_eq!(tools[6]["inputSchema"]["required"], json!(["node_id"]));
    assert_eq!(
        tools[6]["inputSchema"]["properties"]["format"]["enum"],
        json!(["png"])
    );
    assert_eq!(
        tools[6]["inputSchema"]["properties"]["crop"]["enum"],
        json!(["page", "strokes", "selection"])
    );
    // The 8th tool is appended after ink.page_image; earlier names keep their order.
    assert_eq!(tools[7]["name"], "pdf.page_image");
    assert_eq!(
        tools[7]["inputSchema"]["required"],
        json!(["artifact_id", "page"])
    );
    assert_eq!(
        tools[7]["inputSchema"]["properties"]["format"]["enum"],
        json!(["png"])
    );
    let page_image_description = tools[7]["description"].as_str().unwrap();
    assert!(
        page_image_description.contains("1-based"),
        "{page_image_description}"
    );
    assert_eq!(tools[8]["name"], "mathlib.search");
    assert_eq!(
        tools[8]["inputSchema"]["required"],
        json!(["profile_id", "query"])
    );
    assert!(tools[8]["description"].as_str().unwrap().contains("Loogle"));
    assert_eq!(tools[9]["name"], "lean.check");
    assert_eq!(
        tools[9]["inputSchema"]["required"],
        json!(["profile_id", "source_text"])
    );
    assert!(
        tools[9]["description"]
            .as_str()
            .unwrap()
            .contains("receipt")
    );
    assert_eq!(tools[10]["name"], "artifact.stage");
    assert_eq!(
        tools[10]["annotations"],
        json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        })
    );
    assert_eq!(
        tools[10]["inputSchema"]["required"],
        json!(["request_id", "media_type", "name", "content"])
    );
    assert_eq!(tools[11]["name"], "propose_changes");
    assert_eq!(
        tools[11]["inputSchema"]["required"],
        json!(["request_id", "rationale"])
    );
    assert_eq!(tools[11]["annotations"]["readOnlyHint"], false);
    assert_eq!(tools[12]["name"], "get_change_set");
    assert_eq!(
        tools[12]["inputSchema"]["required"],
        json!(["change_set_id"])
    );
    assert_eq!(tools[12]["annotations"]["readOnlyHint"], true);
    let second = request(&f.server, 2, "tools/list", json!({}));
    assert_eq!(
        first["result"], second["result"],
        "tools/list is byte-stable"
    );
}

#[test]
fn lean_tools_are_read_only_and_report_missing_managed_profiles() {
    let f = fixture();
    for (name, arguments) in [
        (
            "mathlib.search",
            json!({ "profile_id": "missing-profile", "query": "add_comm" }),
        ),
        (
            "lean.check",
            json!({
                "profile_id": "missing-profile",
                "source_text": "import Mathlib\nexample : True := by trivial"
            }),
        ),
    ] {
        let (result, is_error) = call(&f.server, name, arguments);
        assert!(!is_error, "{name} capability absence is a business result");
        assert_eq!(result["status"], "capability_unavailable");
        assert_eq!(result["profile_id"], "missing-profile");
        assert!(result["diagnostics"].is_array());
    }
    let (result, is_error) = call(
        &f.server,
        "mathlib.search",
        json!({ "profile_id": "missing-profile", "query": "add_comm", "extra": true }),
    );
    assert!(is_error);
    assert_eq!(result["error"]["code"], "invalid_arg");
}

#[test]
fn artifact_stage_requires_round_and_does_not_become_a_workspace_artifact() {
    let f = fixture();
    let (result, is_error) = call(
        &f.server,
        "artifact.stage",
        json!({
            "request_id": "stage-1",
            "media_type": "text/markdown",
            "name": "proof.md",
            "content": "# Proof\n\nThis is a staged draft."
        }),
    );
    assert!(!is_error);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["stage"]["status"], "staged");
    let staged_id = result["stage"]["staged_artifact_id"].as_str().unwrap();
    assert!(staged_id.starts_with("mathscape:staged-artifact/"));
    assert_eq!(artifact_count(&f.root), 6);

    let (retry, is_error) = call(
        &f.server,
        "artifact.stage",
        json!({
            "request_id": "stage-1",
            "media_type": "text/markdown",
            "name": "proof.md",
            "content": "# Proof\n\nThis is a staged draft."
        }),
    );
    assert!(!is_error);
    assert_eq!(retry["stage"]["staged_artifact_id"], staged_id);

    let (conflict, is_error) = call(
        &f.server,
        "artifact.stage",
        json!({
            "request_id": "stage-1",
            "media_type": "text/markdown",
            "name": "proof.md",
            "content": "changed"
        }),
    );
    assert!(is_error);
    assert_eq!(conflict["error"]["code"], "invalid_arg");
    assert_eq!(
        conflict["error"]["details"]["reason"],
        "idempotency_conflict"
    );
}

#[test]
fn artifact_stage_requires_round_and_blocks_unverified_validation_refs() {
    let f = fixture();
    let no_round = McpServer::new(McpWorkspace::open(&f.root).unwrap());
    let (result, is_error) = call(
        &no_round,
        "artifact.stage",
        json!({
            "request_id": "without-round",
            "media_type": "text/plain",
            "name": "draft.txt",
            "content": "draft"
        }),
    );
    assert!(is_error);
    assert_eq!(result["error"]["code"], "forbidden");
    assert_eq!(result["error"]["details"]["reason"], "round_not_bound");

    let (result, is_error) = call(
        &f.server,
        "artifact.stage",
        json!({
            "request_id": "unsupported-validator",
            "media_type": "text/markdown",
            "name": "proof.md",
            "content": "# Draft",
            "validation_refs": [{
                "validator": "unknown.validator",
                "receipt_id": "receipt-1"
            }]
        }),
    );
    assert!(!is_error);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["stage"]["status"], "blocked");
    assert_eq!(result["stage"]["validation"][0]["state"], "not_run");
    assert_eq!(
        result["stage"]["validation"][0]["diagnostics"][0]["code"],
        "validator_not_available"
    );

    let (result, is_error) = call(
        &f.server,
        "artifact.stage",
        json!({
            "request_id": "lean-validator-on-markdown",
            "media_type": "text/markdown",
            "name": "proof.md",
            "content": "# Draft",
            "validation_refs": [{
                "validator": "lean.check",
                "receipt_id": "receipt-1",
                "profile_id": "profile-1"
            }]
        }),
    );
    assert!(!is_error);
    assert_eq!(result["stage"]["status"], "blocked");
    assert_eq!(
        result["stage"]["validation"][0]["diagnostics"][0]["code"],
        "validator_media_type_mismatch"
    );
}

#[test]
fn phase2_source_proposals_stage_and_read_back_without_changing_the_map() {
    let f = fixture();
    let span = seed_source_span(&f.root, &f.markdown);
    let initial_objects = map_object_count(&f.root);
    let proposal = json!({
        "request_id": "proposal-one",
        "rationale": "登记一个来源定理单元",
        "proposed_objects": [{
            "local_ref": "u1",
            "kind": "source_unit",
            "op": "create",
            "payload": {
                "kind": "statement",
                "display_title": "测试定理",
                "parent": null,
                "source_spans": [span],
                "completeness": "complete"
            }
        }]
    });
    let (staged, is_error) = call(&f.server, "propose_changes", proposal.clone());
    assert!(!is_error, "unexpected proposal error: {staged}");
    assert_eq!(staged["status"], "staged", "{staged}");
    assert!(
        staged["change_set_id"]
            .as_str()
            .unwrap()
            .starts_with("mathscape:change-set/")
    );
    assert_eq!(staged["version"], 1);
    let staged_id = staged["staged_ids"]["u1"].as_str().unwrap();
    assert!(staged_id.starts_with("mathscape:source-unit/"));
    assert_eq!(map_object_count(&f.root), initial_objects);
    assert_eq!(artifact_count(&f.root), 6);

    let (read_back, is_error) = call(
        &f.server,
        "get_change_set",
        json!({ "change_set_id": staged["change_set_id"] }),
    );
    assert!(!is_error);
    assert_eq!(read_back["status"], "ok");
    assert_eq!(read_back["state"], "staged");
    assert_eq!(read_back["staged_ids"]["u1"], staged_id);
    assert_eq!(read_back["proposal"]["request_id"], "proposal-one");

    let (retry, is_error) = call(&f.server, "propose_changes", proposal);
    assert!(!is_error);
    assert_eq!(retry, staged);

    let (blocked, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "proposal-blocked",
            "rationale": "尝试写入尚未支持的节点种类",
            "proposed_objects": [{
                "local_ref": "entity",
                "kind": "representation_binding",
                "op": "create",
                "payload": {}
            }]
        }),
    );
    assert!(!is_error);
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["diagnostics"][0]["code"], "kind_not_supported");
    let (blocked_read, is_error) = call(
        &f.server,
        "get_change_set",
        json!({ "change_set_id": blocked["change_set_id"] }),
    );
    assert!(!is_error);
    assert_eq!(blocked_read["state"], "blocked");
    assert_eq!(blocked_read["diagnostics"][0]["code"], "kind_not_supported");
    assert_eq!(map_object_count(&f.root), initial_objects);
}

#[test]
fn phase2_source_span_proposals_validate_ranges_and_use_content_identity() {
    let f = fixture();
    let end = "# 引理 Lemma".len() as u64;
    let proposal = json!({
        "request_id": "span-proposal",
        "rationale": "以精确 Markdown 范围登记定理",
        "proposed_objects": [
            {"local_ref": "s1", "kind": "source_span", "op": "create", "payload": {
                "artifact_id": f.markdown.clone(),
                "md_byte_range": {"start": 0, "end": end},
                "granularity": "block",
                "quote": "# 引理 Lemma"
            }},
            {"local_ref": "u1", "kind": "source_unit", "op": "create", "payload": {
                "kind": "statement", "display_title": "引理", "parent": null,
                "source_spans": ["$new:s1"], "completeness": "complete"
            }}
        ]
    });
    let (result, is_error) = call(&f.server, "propose_changes", proposal);
    assert!(!is_error, "{result}");
    assert_eq!(result["status"], "staged", "{result}");
    assert!(
        result["staged_ids"]["s1"]
            .as_str()
            .unwrap()
            .starts_with("mathscape:source-span/")
    );
    assert!(
        result["staged_ids"]["u1"]
            .as_str()
            .unwrap()
            .starts_with("mathscape:source-unit/")
    );

    let (blocked, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "span-bad-quote",
            "rationale": "错误范围",
            "proposed_objects": [{"local_ref": "s1", "kind": "source_span", "op": "create", "payload": {
                "artifact_id": f.markdown, "md_byte_range": {"start": 0, "end": end},
                "granularity": "inline", "quote": "不是这个范围"
            }}]
        }),
    );
    assert!(!is_error);
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["diagnostics"][0]["code"], "quote_mismatch");
}

#[test]
fn investigation_graph_reads_staged_nodes_without_changing_accepted_map() {
    let f = fixture();
    let server = McpServer::new(
        McpWorkspace::open(&f.root)
            .unwrap()
            .with_round(Some("round-1".into()))
            .with_investigation(Some("sandbox:session:thread".into())),
    );
    let (result, is_error) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "graph-staged",
            "rationale": "显示未审批来源单元",
            "proposed_objects": [{"local_ref": "u1", "kind": "source_unit", "op": "create", "payload": {
                "kind": "statement", "display_title": "暂存定理", "parent": null,
                "source_spans": [seed_source_span(&f.root, &f.markdown)], "completeness": "complete"
            }}]
        }),
    );
    assert!(!is_error);
    assert_eq!(result["status"], "staged");
    let staged_id = result["staged_ids"]["u1"].as_str().unwrap();
    let (node, is_error) = call(
        &server,
        "graph.get",
        json!({"node_id": staged_id, "scope": "investigation"}),
    );
    assert!(!is_error);
    assert_eq!(node["status"], "ok");
    assert_eq!(node["node"]["staged"], true);
    assert_eq!(node["node"]["change_set_id"], result["change_set_id"]);
}

#[test]
fn proposals_can_depend_on_another_staged_change_set_in_one_investigation() {
    let f = fixture();
    let span = seed_source_span(&f.root, &f.markdown);
    let server = McpServer::new(
        McpWorkspace::open(&f.root)
            .unwrap()
            .with_round(Some("round-a".into()))
            .with_investigation(Some("investigation-deps".into())),
    );
    let (first, is_error) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "dep-a",
            "rationale": "先登记父单元",
            "proposed_objects": [{"local_ref": "a", "kind": "source_unit", "op": "create", "payload": {
                "kind": "section", "display_title": "父", "parent": null,
                "source_spans": [span.clone()], "completeness": "complete"
            }}]
        }),
    );
    assert!(!is_error);
    assert_eq!(first["status"], "staged", "{first}");
    let parent = first["staged_ids"]["a"].as_str().unwrap().to_owned();
    let (second, is_error) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "dep-b",
            "rationale": "再登记子单元",
            "proposed_objects": [{"local_ref": "b", "kind": "source_unit", "op": "create", "payload": {
                "kind": "statement", "display_title": "子", "parent": parent,
                "source_spans": [span], "completeness": "complete"
            }}]
        }),
    );
    assert!(!is_error);
    assert_eq!(second["status"], "staged", "{second}");
    assert_eq!(
        second["dependencies"][0]["change_set_id"],
        first["change_set_id"]
    );
}

#[test]
fn search_matches_a_direct_search_registry_call() {
    let f = fixture();
    let corpus = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3"))
        .unwrap()
        .read_search_corpus()
        .unwrap();
    for (arguments, query) in [
        (
            json!({ "query": "lemma" }),
            SearchQuery {
                text: "lemma".into(),
                include_trashed: false,
                kinds: vec![],
                limit_per_group: None,
            },
        ),
        (
            json!({ "query": "lemma", "kinds": ["material"], "limit_per_group": 2, "include_trashed": true }),
            SearchQuery {
                text: "lemma".into(),
                include_trashed: true,
                kinds: vec![SearchKind::Material],
                limit_per_group: Some(2),
            },
        ),
    ] {
        let (mut result, is_error) = call(&f.server, "workspace.search", arguments);
        assert!(!is_error);
        assert_eq!(result["status"], "ok");
        assert_eq!(result["scope"], "accepted");
        assert!(result.get("diagnostics").is_none());
        assert!(
            result.get("revision").is_none(),
            "revision is omitted, never faked"
        );
        let object = result.as_object_mut().unwrap();
        object.remove("status");
        object.remove("scope");
        let direct =
            serde_json::to_value(search(&SearchRegistry::with_defaults(), &corpus, &query))
                .unwrap();
        assert_eq!(result, direct);
        assert!(direct["total"].as_u64().unwrap() >= 1);
    }
}

#[test]
fn search_scope_investigation_and_bad_args() {
    let f = fixture();
    let (result, is_error) = call(
        &f.server,
        "workspace.search",
        json!({ "query": "lemma", "scope": "investigation" }),
    );
    assert!(!is_error);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["scope"], "accepted");
    assert_eq!(result["diagnostics"][0]["code"], "scope_not_supported");

    for bad in [
        json!({}),
        json!({ "query": 3 }),
        json!({ "query": "x", "kinds": ["nope"] }),
        json!({ "query": "x", "extra": 1 }),
        json!("x"),
    ] {
        let (result, is_error) = call(&f.server, "workspace.search", bad);
        assert!(is_error);
        assert_eq!(result["error"]["code"], "invalid_arg");
    }
}

#[test]
fn read_markdown_range_echoes_actual_locator() {
    let f = fixture();
    let body = f.markdown_body.as_bytes();
    let result = read(
        &f.server,
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 0, "end": 8 } } }),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(result["content_type"], "markdown");
    assert_eq!(result["truncated"], false);
    assert_eq!(result["text"], "# 引理");
    assert_eq!(
        result["locator"],
        json!({ "md_byte_range": { "start": 0, "end": 8 } })
    );
    assert_eq!(result["total_bytes"], body.len());
    assert!(result["revision"].as_str().unwrap().starts_with("sha256:"));

    // Start inside "引" (bytes 2..5) and end inside "理" (5..8): widened to boundaries.
    let result = read(
        &f.server,
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 3, "end": 6 } } }),
    );
    assert_eq!(result["text"], "引理");
    assert_eq!(
        result["locator"],
        json!({ "md_byte_range": { "start": 2, "end": 8 } })
    );

    // expand is in bytes; end beyond EOF is clamped and reported.
    let result = read(
        &f.server,
        json!({
            "artifact_id": f.markdown,
            "locator": { "md_byte_range": { "start": 8, "end": 9999 } },
            "expand": { "before": 6, "after": 10 },
        }),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(
        result["locator"],
        json!({ "md_byte_range": { "start": 2, "end": body.len() } })
    );
    assert_eq!(result["text"], &f.markdown_body[2..]);
    assert_eq!(result["diagnostics"][0]["code"], "range_clamped");
}

#[test]
fn read_markdown_oversize_is_truncated_at_32kib() {
    let f = fixture();
    let result = read(
        &f.server,
        json!({ "artifact_id": f.big_markdown, "locator": { "md_byte_range": { "start": 1, "end": 50000 } } }),
    );
    assert_eq!(result["status"], "truncated");
    assert_eq!(result["truncated"], true);
    let text = result["text"].as_str().unwrap();
    assert!(text.len() <= MAX_READ_BYTES && text.len() > MAX_READ_BYTES - 4);
    let start = result["locator"]["md_byte_range"]["start"]
        .as_u64()
        .unwrap();
    let end = result["locator"]["md_byte_range"]["end"].as_u64().unwrap();
    assert_eq!(start, 0, "start snapped back to the 'α' boundary");
    assert_eq!((end - start) as usize, text.len());
    assert!(text.starts_with("αβγ lemma"));
}

#[test]
fn read_not_found_and_invalid_args() {
    let f = fixture();
    let result = read(
        &f.server,
        json!({ "artifact_id": UNKNOWN_ID, "locator": { "md_byte_range": { "start": 0, "end": 1 } } }),
    );
    assert_eq!(result["status"], "not_found");
    assert_eq!(result["artifact_id"], UNKNOWN_ID);
    assert!(result.get("text").is_none());

    let invalid = [
        json!({ "artifact_id": "not-an-id", "locator": { "md_byte_range": { "start": 0, "end": 1 } } }),
        json!({ "artifact_id": f.markdown }),
        json!({ "artifact_id": f.markdown, "locator": {} }),
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 0, "end": 1 }, "pdf_pages": { "from": 1, "to": 1 } } }),
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 5, "end": 1 } } }),
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 9999, "end": 10000 } } }),
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": -1, "end": 1 } } }),
        json!({ "artifact_id": f.markdown, "locator": { "pdf_pages": { "from": 1, "to": 1 } } }),
        json!({ "artifact_id": f.pdf, "locator": { "md_byte_range": { "start": 0, "end": 1 } } }),
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 0, "to": 1 } } }),
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 4, "to": 5 } } }),
        json!({ "artifact_id": f.plain_text, "locator": { "md_byte_range": { "start": 0, "end": 1 } } }),
        json!({ "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 0, "end": 1 } }, "bogus": true }),
    ];
    for arguments in invalid {
        let error = read_error(&f.server, arguments.clone());
        assert_eq!(error["code"], "invalid_arg", "for {arguments}: {error}");
        assert!(!error["message"].as_str().unwrap().is_empty());
    }
}

#[test]
fn read_pdf_text_layer_pages_never_serve_the_text_layer() {
    // Careful reads only serve canonical Marker markdown. Marker is missing
    // here and no snapshot exists, so even pages with a text layer are
    // capability_unavailable instead of returning raw extract.
    let f = fixture();
    let result = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 1, "to": 9 } } }),
    );
    assert_eq!(result["status"], "capability_unavailable", "{result}");
    assert_eq!(result["content_type"], "markdown");
    assert!(result.get("text").is_none(), "{result}");
    assert_eq!(result["total_pages"], 3);
    assert_eq!(
        result["locator"],
        json!({ "pdf_pages": { "from": 1, "to": 3 } })
    );
    assert!(result.get("page_spans").is_none(), "{result}");
    let codes: Vec<_> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "range_clamped",
            "pdf_text_layer_not_served",
            "canonical_ensure_capability_unavailable",
        ]
    );
    let not_served = &result["diagnostics"][1];
    assert_eq!(not_served["pages_with_text"], json!([1, 3]));
    assert_eq!(not_served["probed_pages"], json!({ "from": 1, "to": 3 }));
    assert!(
        !result.to_string().contains("First text run"),
        "text layer leaked into a careful read: {result}"
    );
    assert_eq!(
        result["details"]["ensure"]["status"],
        "capability_unavailable"
    );

    let single = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 3, "to": 3 } }, "expand": { "before": 1 } }),
    );
    assert_eq!(single["status"], "capability_unavailable");
    assert!(single.get("text").is_none(), "{single}");
    assert_eq!(
        single["locator"],
        json!({ "pdf_pages": { "from": 3, "to": 3 } })
    );
    assert_eq!(single["diagnostics"][0]["code"], "expand_not_applicable");
    assert_eq!(
        single["diagnostics"][1]["code"],
        "pdf_text_layer_not_served"
    );
}

#[test]
fn read_pdf_without_text_layer_reports_marker_capability() {
    let f = fixture();
    let result = read(
        &f.server,
        json!({ "artifact_id": f.scanned_pdf, "locator": { "pdf_pages": { "from": 1, "to": 1 } } }),
    );
    assert_eq!(result["status"], "capability_unavailable");
    assert!(result.get("text").is_none());
    assert_eq!(result["details"]["capability"], "marker");
    assert_eq!(result["details"]["state"], "missing");
    assert_eq!(
        result["details"]["install_hint"],
        "scripts/install/marker.sh"
    );
    assert_eq!(result["diagnostics"][0]["code"], "pdf_no_text_layer");
    // Page 2 of the mixed PDF alone has no text either.
    let result = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 2, "to": 2 } } }),
    );
    assert_eq!(result["status"], "capability_unavailable");
}

#[test]
fn serve_speaks_newline_delimited_json_rpc() {
    let f = fixture();
    let input = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "material.read", "arguments": { "artifact_id": f.markdown, "locator": { "md_byte_range": { "start": 0, "end": 1 } } } } }),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let mut output = Vec::new();
    f.server.serve(input.as_bytes(), &mut output).unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "notifications get no response");
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"]["structuredContent"]["text"], "#");
}

#[test]
fn opening_a_non_workspace_fails() {
    let dir = tempfile::tempdir().unwrap();
    assert!(McpWorkspace::open(dir.path()).is_err());
}

#[test]
fn rejects_invalid_jsonrpc_envelopes_and_ignores_client_responses() {
    let f = fixture();
    let server = &f.server;

    let missing = server
        .handle_message(json!({ "id": 1, "method": "initialize", "params": {} }))
        .unwrap();
    assert_eq!(missing["error"]["code"], -32600);
    assert!(missing["id"].is_null());
    assert!(
        missing["error"]["message"]
            .as_str()
            .unwrap()
            .contains("jsonrpc")
    );

    let wrong = server
        .handle_message(json!({ "jsonrpc": "1.0", "id": "req", "method": "ping" }))
        .unwrap();
    assert_eq!(wrong["error"]["code"], -32600);
    assert!(wrong["id"].is_null());

    for id in [json!(null), json!(true), json!({ "n": 1 }), json!([1])] {
        let response = server
            .handle_message(json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }))
            .unwrap();
        assert_eq!(response["error"]["code"], -32600, "id {id}");
        assert!(response["id"].is_null(), "id {id}");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("id"),
            "id {id}: {response}"
        );
    }

    assert!(
        server
            .handle_message(json!({ "jsonrpc": "2.0", "id": 9, "result": { "ok": true } }))
            .is_none(),
        "client responses get no reply"
    );
    assert!(
        server
            .handle_message(json!({ "id": "resp", "error": { "code": -32601, "message": "nope" } }))
            .is_none()
    );

    let input = [
        json!({ "jsonrpc": "2.0", "id": { "bad": true }, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 4, "result": {} }),
        json!({
            "jsonrpc": "2.0",
            "id": "init",
            "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" },
        }),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let mut output = Vec::new();
    server.serve(input.as_bytes(), &mut output).unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "the client response produced no line: {lines:?}"
    );
    assert_eq!(lines[0]["error"]["code"], -32600);
    assert!(lines[0]["id"].is_null());
    assert_eq!(lines[1]["id"], "init");
    assert_eq!(lines[1]["result"]["protocolVersion"], "2025-06-18");
    assert!(lines[1].get("error").is_none());
}

#[test]
fn unsupported_protocol_version_falls_back_to_latest() {
    let f = fixture();
    for version in ["2025-06-18", "2025-03-26", "2024-11-05"] {
        let response = request(
            &f.server,
            1,
            "initialize",
            json!({ "protocolVersion": version }),
        );
        assert_eq!(response["result"]["protocolVersion"], version);
    }
    let newer = request(
        &f.server,
        2,
        "initialize",
        json!({ "protocolVersion": "2025-11-25" }),
    );
    assert_eq!(
        newer["result"]["protocolVersion"], "2025-06-18",
        "2025-11-25 is not implemented"
    );
}

#[test]
fn outline_entries_are_zero_based_and_drive_material_read() {
    let f = fixture();
    let before = artifact_count(&f.root);
    let result = outline(&f.server, json!({ "artifact_id": f.outlined_pdf }));
    assert_eq!(result["status"], "ok");
    assert_eq!(result["source"], "embedded_toc");
    assert_eq!(result["artifact_id"], f.outlined_pdf);
    assert_eq!(result["total_pages"], 3);
    assert!(!result["revision"].as_str().unwrap().is_empty());
    assert!(result.get("document_index_id").is_none());
    assert!(result.get("diagnostics").is_none());
    assert_eq!(result["truncated"], false);
    assert_eq!(result["total_entries"], 3);
    assert!(result["entries"][0].get("printed_label").is_none());
    assert_eq!(
        result["entries"],
        json!([
            { "title": "第一章", "level": 1, "pdf_page_index": 0 },
            { "title": "引理", "level": 2, "pdf_page_index": 1 },
            { "title": "Chapter 2", "level": 1, "pdf_page_index": 2 },
        ])
    );
    assert_eq!(
        artifact_count(&f.root),
        before,
        "pdf.outline must not persist a catalog row"
    );

    let page = result["entries"][1]["pdf_page_index"].as_u64().unwrap() as u32 + 1;
    let body = read(
        &f.server,
        json!({
            "artifact_id": f.outlined_pdf,
            "locator": { "pdf_pages": { "from": page, "to": page } },
        }),
    );
    // The outline page drives the read locator; without Marker the careful
    // read is capability_unavailable rather than the raw text layer.
    assert_eq!(body["status"], "capability_unavailable", "{body}");
    assert_eq!(
        body["locator"],
        json!({ "pdf_pages": { "from": page, "to": page } })
    );
    assert!(body.get("text").is_none(), "{body}");
    assert_eq!(body["diagnostics"][0]["code"], "pdf_text_layer_not_served");
}

#[test]
fn outline_without_bookmarks_is_ok_and_empty() {
    let f = fixture();
    let result = outline(
        &f.server,
        json!({ "artifact_id": f.pdf, "persist_index": false }),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(result["entries"], json!([]));
    assert_eq!(result["source"], "embedded_toc");
    assert_eq!(result["total_pages"], 3);
    assert!(result.get("document_index_id").is_none());
    assert!(result.get("revision").is_some());
    let codes: Vec<_> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["outline_empty"]);
}

#[test]
fn outline_hostile_bookmark_graphs_are_ok_and_unreadable() {
    let f = fixture();
    let mut pool = ArtifactPool::open(&f.root).unwrap();
    let samples = [
        (short_dest_pdf(), "short.pdf", "panicked"),
        (first_cycle_pdf(), "first-cycle.pdf", "cycle"),
        (next_cycle_pdf(), "next-cycle.pdf", "cycle"),
    ];
    let mut imported = Vec::new();
    for (bytes, name, fragment) in samples {
        let id = import(&mut pool, bytes, "application/pdf", name, Some(1));
        imported.push((id, fragment));
    }
    drop(pool);

    for (id, fragment) in imported {
        let result = outline(&f.server, json!({ "artifact_id": id }));
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["entries"], json!([]), "{result}");
        assert_eq!(result["source"], "embedded_toc");
        assert_eq!(result["total_pages"], 1, "{result}");
        assert!(result.get("revision").is_some());
        let diagnostics = result["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{result}");
        assert_eq!(diagnostics[0]["code"], "outline_unreadable", "{result}");
        let message = diagnostics[0]["message"].as_str().unwrap();
        assert!(message.contains(fragment), "{message}");
    }
}

fn import_outline_pdf(f: &Fixture, titles: &[String], name: &str) -> String {
    let mut pool = ArtifactPool::open(&f.root).unwrap();
    let id = import(
        &mut pool,
        bookmarks_pdf_bytes(titles),
        "application/pdf",
        name,
        Some(1),
    );
    drop(pool);
    id
}

fn assert_truncated(result: &Value) {
    assert_eq!(result["status"], "truncated", "{result}");
    assert_eq!(result["truncated"], true, "{result}");
    assert_eq!(result["source"], "embedded_toc");
    let codes: Vec<&str> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"outline_truncated"), "{codes:?}");
    let bytes = serde_json::to_vec(&result["entries"]).unwrap().len();
    assert!(
        bytes <= MAX_OUTLINE_ENTRIES_BYTES,
        "entries are {bytes} bytes"
    );
}

#[test]
fn outline_huge_title_is_cut_on_a_char_boundary() {
    let f = fixture();
    let huge = format!("题{}", "x".repeat(4 * 1024 * 1024));
    let id = import_outline_pdf(&f, &[huge, "Short".to_owned()], "huge-title.pdf");
    let result = outline(&f.server, json!({ "artifact_id": id }));
    assert_truncated(&result);
    assert_eq!(result["total_entries"], 2);
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{entries:?}");
    let title = entries[0]["title"].as_str().unwrap();
    assert_eq!(title.chars().count(), MAX_OUTLINE_TITLE_CHARS);
    assert!(title.starts_with('题'));
    assert_eq!(entries[0]["title_truncated"], true);
    assert_eq!(
        entries[1],
        json!({ "title": "Short", "level": 1, "pdf_page_index": 0 })
    );
    let response = serde_json::to_vec(&result).unwrap().len();
    assert!(response < 8 * 1024, "response is {response} bytes");
}

#[test]
fn outline_entry_count_is_capped() {
    let f = fixture();
    let total = MAX_OUTLINE_ENTRIES + 500;
    let titles: Vec<String> = (0..total).map(|index| format!("E{index:05}")).collect();
    let id = import_outline_pdf(&f, &titles, "many-entries.pdf");
    let result = outline(&f.server, json!({ "artifact_id": id }));
    assert_truncated(&result);
    assert_eq!(result["total_entries"], total);
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), MAX_OUTLINE_ENTRIES);
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry["title"], format!("E{index:05}"));
        assert!(entry.get("title_truncated").is_none());
    }
}

#[test]
fn outline_total_bytes_are_capped() {
    // 900 × ~500-char titles ≈ 450 KB uncapped, under the entry-count cap;
    // the byte budget cuts first.
    let f = fixture();
    let titles: Vec<String> = (0..900)
        .map(|index| format!("{index:04} {}", "lemma ".repeat(82)))
        .collect();
    assert!(titles[0].chars().count() <= MAX_OUTLINE_TITLE_CHARS);
    let id = import_outline_pdf(&f, &titles, "wide-entries.pdf");
    let result = outline(&f.server, json!({ "artifact_id": id }));
    assert_truncated(&result);
    assert_eq!(result["total_entries"], 900);
    let entries = result["entries"].as_array().unwrap();
    assert!(
        entries.len() > 50 && entries.len() < 900,
        "{} entries",
        entries.len()
    );
    for (index, entry) in entries.iter().enumerate() {
        assert_eq!(entry["title"].as_str().unwrap(), titles[index]);
    }
    let response = serde_json::to_vec(&result).unwrap().len();
    assert!(response < 80 * 1024, "response is {response} bytes");
}

#[test]
fn outline_file_that_will_not_open_is_internal() {
    let f = fixture();
    let mut pool = ArtifactPool::open(&f.root).unwrap();
    let id = import(
        &mut pool,
        b"this is not a pdf".to_vec(),
        "application/pdf",
        "broken.pdf",
        Some(1),
    );
    drop(pool);
    let error = outline_error(&f.server, json!({ "artifact_id": id }));
    assert_eq!(error["code"], "internal", "{error}");
}

#[test]
fn outline_rejects_missing_id_non_pdf_and_bad_args() {
    let f = fixture();
    let missing = outline(&f.server, json!({ "artifact_id": UNKNOWN_ID }));
    assert_eq!(missing["status"], "not_found");
    assert_eq!(missing["entries"], json!([]));
    assert_eq!(missing["details"]["reason"], "artifact_not_found");
    assert!(missing.get("source").is_none());
    assert!(missing.get("revision").is_none());

    for id in [f.markdown.as_str(), f.plain_text.as_str()] {
        let error = outline_error(&f.server, json!({ "artifact_id": id }));
        assert_eq!(error["code"], "invalid_arg", "{error}");
        assert!(error["details"]["media_type"].as_str().is_some());
    }

    let persist = outline_error(
        &f.server,
        json!({ "artifact_id": f.pdf, "persist_index": true }),
    );
    assert_eq!(persist["code"], "invalid_arg");
    assert!(
        persist["message"]
            .as_str()
            .unwrap()
            .contains("persist_index"),
        "{persist}"
    );

    for bad in [
        json!({ "artifact_id": "not-an-id" }),
        json!({}),
        json!({ "artifact_id": f.pdf, "bogus": true }),
        json!({ "artifact_id": f.outlined_pdf, "persist_index": false, "document_index_id": "x" }),
    ] {
        let error = outline_error(&f.server, bad);
        assert_eq!(error["code"], "invalid_arg", "{error}");
    }
}

fn managed_object_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.join("objects")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                dirs.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    assert!(!files.is_empty(), "fixture should contain managed objects");
    files
}

#[test]
fn missing_managed_object_is_business_not_found() {
    let f = fixture();
    for path in managed_object_files(&f.root) {
        std::fs::remove_file(path).unwrap();
    }
    for (id, locator) in [
        (
            f.markdown.as_str(),
            json!({ "md_byte_range": { "start": 0, "end": 1 } }),
        ),
        (
            f.pdf.as_str(),
            json!({ "pdf_pages": { "from": 1, "to": 1 } }),
        ),
    ] {
        let result = read(&f.server, json!({ "artifact_id": id, "locator": locator }));
        assert_eq!(result["status"], "not_found", "{result}");
        assert_eq!(result["details"]["reason"], "content_unavailable");
        assert!(result.get("text").is_none());
    }
    let outlined = outline(&f.server, json!({ "artifact_id": f.outlined_pdf }));
    assert_eq!(outlined["status"], "not_found", "{outlined}");
    assert_eq!(outlined["details"]["reason"], "content_unavailable");
    assert_eq!(outlined["entries"], json!([]));
}

#[test]
fn symlinked_managed_object_is_forbidden() {
    let f = fixture();
    let target = f.root.join("secret-managed-target");
    std::fs::write(&target, b"not the managed bytes").unwrap();
    for path in managed_object_files(&f.root) {
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
    }
    for (id, locator) in [
        (
            f.markdown.as_str(),
            json!({ "md_byte_range": { "start": 0, "end": 1 } }),
        ),
        (
            f.pdf.as_str(),
            json!({ "pdf_pages": { "from": 1, "to": 1 } }),
        ),
    ] {
        let error = read_error(&f.server, json!({ "artifact_id": id, "locator": locator }));
        assert_eq!(error["code"], "forbidden", "{error}");
        let message = error["message"].as_str().unwrap();
        assert!(!message.contains("secret-managed-target"), "{message}");
        assert!(
            !message.contains(&f.root.display().to_string()),
            "{message}"
        );
    }
    let error = outline_error(&f.server, json!({ "artifact_id": f.outlined_pdf }));
    assert_eq!(error["code"], "forbidden", "{error}");
    assert!(
        !error["message"]
            .as_str()
            .unwrap()
            .contains("secret-managed-target")
    );
}

#[test]
fn grep_markdown_hits_round_trip_through_material_read() {
    let f = fixture();
    let result = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma" }),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(result["truncated"], false);
    assert!(result.get("diagnostics").is_none());
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit["artifact_id"], f.markdown);
    assert_eq!(hit["score"], 1);
    assert!(hit["revision"].as_str().unwrap().starts_with("sha256:"));
    let start = f.markdown_body.find("Lemma").unwrap();
    assert_eq!(
        hit["locator"],
        json!({ "md_byte_range": { "start": start, "end": start + "Lemma".len() } })
    );
    assert_eq!(hit["snippet"], "# 引理 Lemma");
    let quoted = read(
        &f.server,
        json!({ "artifact_id": f.markdown, "locator": hit["locator"] }),
    );
    assert_eq!(quoted["status"], "ok");
    assert_eq!(quoted["text"], "Lemma");

    let lemma = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown], "pattern": "引理" }),
    );
    assert_eq!(lemma["hits"].as_array().unwrap().len(), 1);
    let quoted = read(
        &f.server,
        json!({
            "artifact_id": f.markdown,
            "locator": lemma["hits"][0]["locator"],
        }),
    );
    assert_eq!(quoted["text"], "引理");

    // Case-sensitive literal: "lemma" and a regex-looking pattern miss "Lemma".
    for pattern in ["lemma", "L.mma"] {
        let missed = grep(
            &f.server,
            json!({ "artifact_ids": [f.markdown], "pattern": pattern }),
        );
        assert_eq!(missed["status"], "ok", "{pattern}");
        assert_eq!(missed["hits"].as_array().unwrap().len(), 0, "{pattern}");
        assert_eq!(missed["truncated"], false);
    }

    // Only the named materials are scanned, in the given order.
    let ordered = grep(
        &f.server,
        json!({
            "artifact_ids": [f.big_markdown, f.markdown],
            "pattern": "Lemma",
            "max_hits": 5,
        }),
    );
    assert_eq!(ordered["status"], "ok");
    assert_eq!(ordered["hits"].as_array().unwrap().len(), 1);
    assert_eq!(ordered["hits"][0]["artifact_id"], f.markdown);

    let first_file = grep(
        &f.server,
        json!({
            "artifact_ids": [f.markdown, f.big_markdown],
            "pattern": "e",
            "max_hits": 1,
        }),
    );
    assert_eq!(first_file["status"], "truncated");
    assert_eq!(first_file["hits"][0]["artifact_id"], f.markdown);
}

#[test]
fn grep_rejects_empty_ids_and_bad_args() {
    let f = fixture();
    let invalid = [
        json!({ "artifact_ids": [], "pattern": "Lemma" }),
        json!({ "pattern": "Lemma" }),
        json!({ "artifact_ids": [f.markdown] }),
        json!({ "artifact_ids": [f.markdown], "pattern": "" }),
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma", "terms": ["Lemma"] }),
        json!({ "artifact_ids": [f.markdown, f.markdown], "pattern": "Lemma" }),
        json!({ "artifact_ids": ["not-an-id"], "pattern": "Lemma" }),
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma", "max_hits": 0 }),
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma", "max_hits": MAX_GREP_HITS + 1 }),
        json!({ "artifact_ids": [f.markdown], "pattern": "a".repeat(201) }),
        json!({ "artifact_ids": [f.plain_text], "pattern": "plain" }),
        json!({ "artifact_ids": [f.markdown, f.plain_text], "pattern": "Lemma" }),
        json!("Lemma"),
    ];
    for arguments in invalid {
        let error = grep_error(&f.server, arguments.clone());
        assert_eq!(error["code"], "invalid_arg", "for {arguments}: {error}");
        assert!(!error["message"].as_str().unwrap().is_empty());
    }
    let empty = grep_error(&f.server, json!({ "artifact_ids": [], "pattern": "Lemma" }));
    assert!(
        empty["message"].as_str().unwrap().contains("non-empty"),
        "{empty}"
    );
    let plain = grep_error(
        &f.server,
        json!({ "artifact_ids": [f.markdown, f.plain_text], "pattern": "plain" }),
    );
    assert!(
        plain["message"].as_str().unwrap().contains("text/markdown"),
        "{plain}"
    );
    assert_eq!(
        plain["details"]["supported_media_types"],
        json!(["text/markdown", "application/pdf"])
    );
    assert_eq!(plain["details"]["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        plain["details"]["artifacts"][0]["artifact_id"],
        f.plain_text
    );
}

#[test]
fn grep_pdf_text_layer_hits_locate_pages_only() {
    let f = fixture();
    let result = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown, f.pdf, f.scanned_pdf], "pattern": "text" }),
    );
    assert_eq!(result["status"], "ok", "{result}");
    let hits = result["hits"].as_array().unwrap();
    let pages: Vec<_> = hits
        .iter()
        .map(|hit| hit["locator"]["pdf_pages"].clone())
        .collect();
    assert_eq!(
        pages,
        [
            json!({ "from": 1, "to": 1 }),
            json!({ "from": 1, "to": 1 }),
            json!({ "from": 3, "to": 3 }),
        ]
    );
    for hit in hits {
        assert_eq!(hit["artifact_id"], f.pdf);
        assert_eq!(hit["surface"], "pdf_text_layer");
        assert!(hit.get("canonical_id").is_none(), "{hit}");
    }
    assert!(
        hits[2]["snippet"]
            .as_str()
            .unwrap()
            .contains("Third page text")
    );
    let codes: Vec<_> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["pdf_text_layer_hits"]);

    // Markdown hits keep md_byte_range and no surface marker.
    let md = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown, f.pdf], "pattern": "Lemma" }),
    );
    assert_eq!(md["hits"].as_array().unwrap().len(), 1);
    assert!(md["hits"][0]["locator"]["md_byte_range"].is_object());
    assert!(md["hits"][0].get("surface").is_none());

    // max_hits applies across PDF pages.
    let limited = grep(
        &f.server,
        json!({ "artifact_ids": [f.pdf], "pattern": "text", "max_hits": 2 }),
    );
    assert_eq!(limited["status"], "truncated");
    assert_eq!(limited["truncated"], true);
    assert_eq!(limited["hits"].as_array().unwrap().len(), 2);

    // The locator feeds material.read, which never returns the text layer.
    let page = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": hits[2]["locator"] }),
    );
    assert_eq!(page["status"], "capability_unavailable");
    assert!(page.get("text").is_none(), "{page}");
}

#[test]
fn grep_missing_artifact_is_not_found() {
    let f = fixture();
    let result = grep(
        &f.server,
        json!({ "artifact_ids": [UNKNOWN_ID], "pattern": "Lemma" }),
    );
    assert_eq!(result["status"], "not_found");
    assert_eq!(result["hits"].as_array().unwrap().len(), 0);
    assert_eq!(result["details"]["reason"], "artifact_not_found");
    assert_eq!(result["details"]["artifact_ids"], json!([UNKNOWN_ID]));
    assert_eq!(result["diagnostics"][0]["code"], "artifact_not_found");
    assert!(result.get("truncated").is_none());

    // A missing id fails the whole call; the existing material is not partially scanned.
    let mixed = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown, UNKNOWN_ID], "pattern": "Lemma" }),
    );
    assert_eq!(mixed["status"], "not_found");
    assert_eq!(mixed["hits"].as_array().unwrap().len(), 0);
    assert_eq!(mixed["details"]["artifact_ids"], json!([UNKNOWN_ID]));
}

#[test]
fn grep_hit_limit_is_truncated() {
    let f = fixture();
    let line = "αβγ lemma line\n";
    let result = grep(
        &f.server,
        json!({ "artifact_ids": [f.big_markdown], "pattern": "lemma", "max_hits": 2 }),
    );
    assert_eq!(result["status"], "truncated");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["diagnostics"][0]["code"], "hit_limit");
    let hits = result["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    let first = line.find("lemma").unwrap() as u64;
    let second = line.len() as u64 + first;
    assert_eq!(
        hits[0]["locator"],
        json!({ "md_byte_range": { "start": first, "end": first + 5 } })
    );
    assert_eq!(
        hits[1]["locator"],
        json!({ "md_byte_range": { "start": second, "end": second + 5 } })
    );
    assert_eq!(hits[0]["snippet"], "αβγ lemma line");
    assert_eq!(hits[0]["artifact_id"], f.big_markdown);
    let quoted = read(
        &f.server,
        json!({ "artifact_id": f.big_markdown, "locator": hits[0]["locator"] }),
    );
    assert_eq!(quoted["text"], "lemma");

    let all_default = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown], "pattern": "converges" }),
    );
    assert_eq!(all_default["status"], "ok");
    assert_eq!(all_default["hits"].as_array().unwrap().len(), 1);
    assert_eq!(all_default["truncated"], false);
}

#[test]
fn grep_does_not_scan_past_the_byte_cap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let head = "token at head\n";
    let mut body = head.to_owned();
    let pad = (MAX_SCAN_BYTES as usize).saturating_sub(body.len()) + 8;
    body.push_str(&"x".repeat(pad));
    body.push_str("TAIL_ONLY");
    assert!(body.len() as u64 > MAX_SCAN_BYTES);
    assert!((body.find("TAIL_ONLY").unwrap() as u64) >= MAX_SCAN_BYTES);
    let id = import(
        &mut pool,
        body.into_bytes(),
        "text/markdown",
        "capped.md",
        None,
    );
    drop(pool);
    let server = McpServer::new(
        McpWorkspace::open(&root)
            .unwrap()
            .with_capability_runner(Box::new(NothingInstalled)),
    );

    let head_hit = grep(&server, json!({ "artifact_ids": [id], "pattern": "token" }));
    assert_eq!(head_hit["status"], "truncated");
    assert_eq!(head_hit["truncated"], true);
    assert_eq!(head_hit["hits"].as_array().unwrap().len(), 1);
    assert_eq!(head_hit["diagnostics"][0]["code"], "scan_capped");
    assert_eq!(head_hit["hits"][0]["snippet"], "token at head");
    let quoted = read(
        &server,
        json!({ "artifact_id": id, "locator": head_hit["hits"][0]["locator"] }),
    );
    assert_eq!(quoted["text"], "token");

    let tail = grep(
        &server,
        json!({ "artifact_ids": [id], "pattern": "TAIL_ONLY" }),
    );
    assert_eq!(tail["status"], "truncated");
    assert_eq!(tail["truncated"], true);
    assert!(tail["hits"].as_array().unwrap().is_empty());
    assert_eq!(tail["diagnostics"][0]["code"], "scan_capped");
}

#[test]
fn grep_missing_object_is_not_found_and_symlink_is_forbidden() {
    let f = fixture();
    for path in managed_object_files(&f.root) {
        std::fs::remove_file(path).unwrap();
    }
    let missing = grep(
        &f.server,
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma" }),
    );
    assert_eq!(missing["status"], "not_found");
    assert_eq!(missing["details"]["reason"], "content_unavailable");
    assert!(missing["hits"].as_array().unwrap().is_empty());

    let f = fixture();
    let target = f.root.join("secret-managed-target");
    std::fs::write(&target, b"not the managed bytes").unwrap();
    for path in managed_object_files(&f.root) {
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
    }
    let error = grep_error(
        &f.server,
        json!({ "artifact_ids": [f.markdown], "pattern": "Lemma" }),
    );
    assert_eq!(error["code"], "forbidden", "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(!message.contains("secret-managed-target"), "{message}");
    assert!(
        !message.contains(&f.root.display().to_string()),
        "{message}"
    );
}

#[test]
fn scanned_pdf_read_reports_canonical_coverage_without_running_marker() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let actor = ActorRef::parse("canonical-test").unwrap();
    let pdf = import(
        &mut pool,
        pdf_bytes(&[&[], &[]]),
        "application/pdf",
        "scan-book.pdf",
        Some(2),
    );
    let markdown_body = "扫描页正文\n";
    let markdown = import(
        &mut pool,
        markdown_body.as_bytes().to_vec(),
        "text/markdown",
        "scan-book.md",
        None,
    );
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(0, 0, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(false),
            BTreeMap::from([(
                "marker_profile".to_owned(),
                "mathscape.marker.math-ocr.v1".to_owned(),
            )]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-02T00:00:00+08:00").unwrap()),
        Some(Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            mathscape_domain::StableId::parse("mathscape:conversion/marker/scan").unwrap(),
            RevisionId::parse(format!("sha256:{}", "c".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("canonical-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap(),
            actor,
            None,
        ),
        vec![mathscape_domain::StableId::parse(&pdf).unwrap()],
        vec![mathscape_domain::StableId::parse(&markdown).unwrap()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
    drop(pool);

    let server = McpServer::new(
        McpWorkspace::open(&root)
            .unwrap()
            .with_capability_runner(Box::new(NothingInstalled)),
    );
    let canonical = CanonicalMarkdownKey::new(
        mathscape_domain::StableId::parse(&pdf).unwrap(),
        "mathscape.marker.math-ocr.v1",
        "marker-2.0.0",
    )
    .unwrap();

    let covered = read(
        &server,
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 1, "to": 1 } } }),
    );
    assert_eq!(covered["status"], "ok");
    assert_eq!(covered["content_type"], "markdown");
    assert_eq!(covered["text"], markdown_body);
    assert_eq!(covered["partial"], true);
    assert_eq!(covered["canonical_id"], canonical.canonical_id().as_str());
    assert_eq!(covered["served_artifact_id"], markdown);
    assert_eq!(
        covered["details"]["served_segments"],
        json!([{
            "artifact_id": markdown, "canonical_id": canonical.canonical_id().as_str(),
            "pages": { "from": 1, "to": 1 }, "snapshot_pages": { "from": 1, "to": 1 },
            "md_byte_range": { "start": 0, "end": markdown_body.len() },
            "trimmed_to_pages": true, "is_head": true
        }]),
        "{covered}"
    );
    assert_eq!(covered["coverage"]["complete"], false);
    assert_eq!(covered["coverage"]["page_index_base"], "one");
    assert_eq!(
        covered["coverage"]["covered"],
        json!([{ "from": 1, "to": 1 }])
    );
    assert_eq!(
        covered["coverage"]["missing"],
        json!([{ "from": 2, "to": 2 }])
    );
    assert_eq!(covered["coverage"]["total_pages"], 2);
    assert!(
        covered["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "served_canonical_markdown")
    );

    let missing = read(
        &server,
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 2, "to": 2 } } }),
    );
    assert_eq!(missing["status"], "capability_unavailable");
    assert!(missing.get("text").is_none());
    assert_eq!(missing["partial"], true);
    assert_eq!(missing["canonical_id"], canonical.canonical_id().as_str());
    assert_eq!(missing["details"]["state"], "missing");

    let via_canonical = read(
        &server,
        json!({
            "artifact_id": canonical.canonical_id().as_str(),
            "locator": { "md_byte_range": { "start": 0, "end": 64 } },
        }),
    );
    assert_eq!(via_canonical["text"], markdown_body);
    assert_eq!(via_canonical["artifact_id"], markdown);
    assert_eq!(via_canonical["partial"], true);
    assert!(
        via_canonical["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "canonical_id_resolved"),
        "{via_canonical}"
    );

    // A PDF id with a current canonical head greps its snapshots' markdown for
    // covered pages, the (empty) text layer only for the rest, and never runs
    // Marker.
    let via_pdf = grep(&server, json!({ "artifact_ids": [pdf], "pattern": "扫描" }));
    assert_eq!(via_pdf["status"], "ok", "{via_pdf}");
    let hit = &via_pdf["hits"][0];
    assert_eq!(hit["artifact_id"], markdown, "{via_pdf}");
    assert_eq!(hit["canonical_id"], canonical.canonical_id().as_str());
    assert_eq!(hit["partial"], true);
    assert!(hit.get("surface").is_none(), "{hit}");
    assert_eq!(hit["locator"]["md_byte_range"]["start"], 0, "{hit}");
    let resolved = via_pdf["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "pdf_resolved_to_canonical")
        .unwrap_or_else(|| panic!("missing pdf_resolved_to_canonical: {via_pdf}"));
    assert_eq!(resolved["artifact_id"], pdf);
    assert_eq!(resolved["canonical_id"], canonical.canonical_id().as_str());
    assert_eq!(resolved["searched_pages"], json!([{ "from": 1, "to": 1 }]));
    // The uncovered page 2 is searched in its (empty) text layer, honestly.
    assert_eq!(
        resolved["text_layer_pages"],
        json!([{ "from": 2, "to": 2 }])
    );
    assert_eq!(resolved["unsearched_pages"], json!([]));
    assert_eq!(via_pdf["hits"].as_array().unwrap().len(), 1, "{via_pdf}");
    let round_trip = read(
        &server,
        json!({ "artifact_id": hit["artifact_id"], "locator": hit["locator"] }),
    );
    assert!(
        round_trip["text"].as_str().unwrap().contains("扫描"),
        "{round_trip}"
    );

    let hits = grep(
        &server,
        json!({ "artifact_ids": [markdown], "pattern": "扫描" }),
    );
    assert_eq!(
        hits["hits"][0]["canonical_id"],
        canonical.canonical_id().as_str()
    );
    assert_eq!(hits["hits"][0]["partial"], true);
    assert_eq!(
        hits["hits"][0]["coverage"]["covered"],
        json!([{ "from": 1, "to": 1 }])
    );

    let (found, is_error) = call(
        &server,
        "workspace.search",
        json!({ "query": "scan-book", "kinds": ["material"] }),
    );
    assert!(!is_error);
    let annotated = found["groups"][0]["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| {
            hit["canonical_id"] == canonical.canonical_id().as_str() && hit["partial"] == true
        });
    assert!(annotated, "{found}");

    let outline = outline(&server, json!({ "artifact_id": pdf }));
    assert_eq!(outline["status"], "ok");
    assert_eq!(outline["canonical_id"], canonical.canonical_id().as_str());
    assert_eq!(outline["partial"], true);
}

#[test]
fn scanned_pdf_read_refuses_snapshot_that_only_overlaps_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let actor = ActorRef::parse("canonical-test").unwrap();
    let pdf = import(
        &mut pool,
        pdf_bytes(&[&[], &[], &[]]),
        "application/pdf",
        "scan-overlap.pdf",
        Some(3),
    );
    let markdown_body = "仅第二、三页\n";
    let markdown = import(
        &mut pool,
        markdown_body.as_bytes().to_vec(),
        "text/markdown",
        "scan-overlap.md",
        None,
    );
    // Snapshot covers 1-based pages 2–3 only (0-based 1..=2).
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(1, 2, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(false),
            BTreeMap::from([(
                "marker_profile".to_owned(),
                "mathscape.marker.math-ocr.v1".to_owned(),
            )]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-02T00:00:00+08:00").unwrap()),
        Some(Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            mathscape_domain::StableId::parse("mathscape:conversion/marker/overlap").unwrap(),
            RevisionId::parse(format!("sha256:{}", "d".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("canonical-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap(),
            actor,
            None,
        ),
        vec![mathscape_domain::StableId::parse(&pdf).unwrap()],
        vec![mathscape_domain::StableId::parse(&markdown).unwrap()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
    drop(pool);

    let server = McpServer::new(
        McpWorkspace::open(&root)
            .unwrap()
            .with_capability_runner(Box::new(NothingInstalled)),
    );

    // Request 1–3 only overlaps the 2–3 snapshot: the body must not be
    // returned under a 1–3 locator.
    let overlapping = read(
        &server,
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 1, "to": 3 } } }),
    );
    assert_eq!(
        overlapping["status"], "capability_unavailable",
        "{overlapping}"
    );
    assert!(overlapping.get("text").is_none(), "{overlapping}");
    assert!(
        overlapping.get("served_artifact_id").is_none(),
        "{overlapping}"
    );
    assert_eq!(overlapping["partial"], true);

    // A fully covered request is still served.
    let covered = read(
        &server,
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 2, "to": 3 } } }),
    );
    assert_eq!(covered["status"], "ok", "{covered}");
    assert_eq!(covered["text"], markdown_body);
    assert_eq!(covered["served_artifact_id"], markdown);
    assert_eq!(covered["locator"]["pdf_pages"]["from"], 2);
    assert_eq!(covered["locator"]["pdf_pages"]["to"], 3);
}

/// A `pages`-page PDF plus one Marker snapshot of the zero-based PDF pages
/// `from..=to` with `body`, recorded as a conversion so it is a canonical
/// segment. Returns `(dir, root, pdf, markdown)`.
fn canonical_book(
    pages: usize,
    from: u32,
    to: u32,
    body: &str,
) -> (TempDir, std::path::PathBuf, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let blank: Vec<&[&str]> = vec![&[]; pages];
    let pdf = import(
        &mut pool,
        pdf_bytes(&blank),
        "application/pdf",
        "book.pdf",
        Some(pages as u32),
    );
    let markdown = import(
        &mut pool,
        body.as_bytes().to_vec(),
        "text/markdown",
        "book.md",
        None,
    );
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(from, to, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(true),
            BTreeMap::from([(
                "marker_profile".to_owned(),
                "mathscape.marker.math-ocr.v1".to_owned(),
            )]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-02T00:00:00+08:00").unwrap()),
        Some(Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            mathscape_domain::StableId::parse("mathscape:conversion/marker/book").unwrap(),
            RevisionId::parse(format!("sha256:{}", "d".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("canonical-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-11-02T00:01:00+08:00").unwrap(),
            ActorRef::parse("canonical-test").unwrap(),
            None,
        ),
        vec![mathscape_domain::StableId::parse(&pdf).unwrap()],
        vec![mathscape_domain::StableId::parse(&markdown).unwrap()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
    drop(pool);
    // The Desktop syncs the canonical registry on open; do the same here.
    SqliteArtifactStore::open(&root.join("mathscape.sqlite3"))
        .unwrap()
        .sync_canonical_markdown_from_catalog(
            &Timestamp::parse("2026-11-02T00:02:00+08:00").unwrap(),
        )
        .unwrap();
    (dir, root, pdf, markdown)
}

fn book_server(root: &Path, round: Option<&str>) -> McpServer {
    McpServer::new(
        McpWorkspace::open(root)
            .unwrap()
            .with_capability_runner(Box::new(NothingInstalled))
            .with_round(round.map(str::to_owned))
            .with_investigation(Some("sandbox:book".to_owned())),
    )
}

fn diagnostic_codes(value: &Value) -> Vec<String> {
    value["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["code"].as_str().map(str::to_owned))
        .collect()
}

/// Round-2 gap: `material.locate` on a snapshot with Marker page markers
/// resolves the page, and a page-granularity span stores it; an inline span
/// over the same range gets the non-blocking `page_granularity_available`.
#[test]
fn locate_on_a_paginated_snapshot_resolves_the_page_and_page_spans_store_it() {
    let body = "{7}----------------\n\n习题 8 设 $f$ 在 $[a,b]$ 上连续。\n\n{8}----------------\n\n习题 9 求极限。\n";
    let (_dir, root, _pdf, markdown) = canonical_book(10, 7, 8, body);
    let server = book_server(&root, Some("round-book"));
    // The agent reads the pages first; this also syncs the canonical view.
    let (_, _) = call(
        &server,
        "material.read",
        json!({ "artifact_id": _pdf, "locator": { "pdf_pages": { "from": 8, "to": 9 } } }),
    );
    for (quote, page) in [
        ("习题 8 设 $f$ 在 $[a,b]$ 上连续。", 8),
        ("习题 9 求极限。", 9),
    ] {
        let (found, is_error) = call(
            &server,
            "material.locate",
            json!({ "artifact_id": markdown, "quote": quote }),
        );
        assert!(!is_error, "{found}");
        assert_eq!(found["status"], "ok", "{found}");
        assert_eq!(found["page"]["page_number"], page, "{found}");
        assert_eq!(found["page"]["pdf_page_index"], page - 1, "{found}");
        assert_eq!(found["page_granularity_available"], true, "{found}");
        let codes = diagnostic_codes(&found);
        assert!(
            !codes
                .iter()
                .any(|code| code == "page_markers_absent" || code == "page_unresolved"),
            "{found}"
        );
    }
    let (found, _) = call(
        &server,
        "material.locate",
        json!({ "artifact_id": markdown, "quote": "习题 9 求极限。" }),
    );
    let proposal = |request: &str, granularity: &str| {
        json!({
            "request_id": request, "rationale": "登记习题 9",
            "proposed_objects": [
                { "local_ref": "s", "kind": "source_span", "op": "create", "payload": {
                    "artifact_id": markdown, "md_byte_range": found["md_byte_range"],
                    "granularity": granularity, "quote": "习题 9 求极限。" }},
                { "local_ref": "u", "kind": "source_unit", "op": "create", "payload": {
                    "kind": "problem", "display_title": "习题 9", "parent": null,
                    "source_spans": ["$new:s"], "completeness": "complete" }}
            ]
        })
    };
    let (paged, is_error) = call(&server, "propose_changes", proposal("paged", "page"));
    assert!(!is_error, "{paged}");
    assert_eq!(paged["status"], "staged", "{paged}");
    let page_payload = paged["compiled_operations"].to_string();
    assert!(page_payload.contains("\"pdf_page_index\":8"), "{paged}");
    let (inline, is_error) = call(&server, "propose_changes", proposal("inline", "inline"));
    assert!(!is_error, "{inline}");
    assert_eq!(inline["status"], "staged", "{inline}");
    let hint = inline["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "page_granularity_available")
        .unwrap_or_else(|| panic!("{inline}"));
    assert_eq!(hint["level"], "info");
    assert!(
        hint["message"].as_str().unwrap().contains("第 9 页"),
        "{hint}"
    );
}

/// The sandbox b1455 case: Marker snapshots begin with blank lines before
/// the first page's own `{7}------` marker, and `material.read` serves the
/// first page as `0..{8}`. That segment must map to page 8 (it used to be
/// treated as crossing the leading marker), so locate offers page
/// granularity and a page span over it is staged with the page stored.
#[test]
fn first_page_served_segment_with_leading_marker_maps_to_its_page() {
    let dashes = "-".repeat(48);
    let body = format!(
        "\n\n{{7}}{dashes}\n\n习题 8 设 $f$ 在 $[a,b]$ 上连续。\n\n习题 9 求极限。\n\n{{8}}{dashes}\n\n习题 10 证明之。\n"
    );
    let (_dir, root, pdf, markdown) = canonical_book(10, 7, 8, &body);
    let server = book_server(&root, Some("round-book"));
    let (read, is_error) = call(
        &server,
        "material.read",
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 8, "to": 8 } } }),
    );
    assert!(!is_error, "{read}");
    let segment = read["details"]["served_segments"][0]["md_byte_range"].clone();
    let end = body.find("{8}").unwrap();
    assert_eq!(segment, json!({ "start": 0, "end": end }), "{read}");
    let quote = &body[..end];
    let (found, is_error) = call(
        &server,
        "material.locate",
        json!({ "artifact_id": markdown, "quote": quote, "within": segment }),
    );
    assert!(!is_error, "{found}");
    assert_eq!(found["status"], "ok", "{found}");
    assert_eq!(found["md_byte_range"], segment, "{found}");
    assert_eq!(found["page"]["page_number"], 8, "{found}");
    assert_eq!(found["page_granularity_available"], true, "{found}");
    assert!(
        !diagnostic_codes(&found)
            .iter()
            .any(|code| code == "page_unresolved"),
        "{found}"
    );
    let (paged, is_error) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "first-page", "rationale": "登记第 8 页",
            "proposed_objects": [
                { "local_ref": "s", "kind": "source_span", "op": "create", "payload": {
                    "artifact_id": markdown, "md_byte_range": segment,
                    "granularity": "page", "quote": quote }},
                { "local_ref": "u", "kind": "source_unit", "op": "create", "payload": {
                    "kind": "problem", "display_title": "第 8 页习题", "parent": null,
                    "source_spans": ["$new:s"], "completeness": "complete" }}
            ]
        }),
    );
    assert!(!is_error, "{paged}");
    assert_eq!(paged["status"], "staged", "{paged}");
    assert!(
        !diagnostic_codes(&paged)
            .iter()
            .any(|code| code == "page_mapping_unavailable"),
        "{paged}"
    );
    let payload = paged["compiled_operations"].to_string();
    assert!(payload.contains("\"pdf_page_index\":7"), "{paged}");
}

/// The sandbox 854f case: a snapshot of PDF pages 1–3 without page markers.
/// `material.locate` cannot give a page and now says why
/// (`page_markers_absent`, like `material.read`).
#[test]
fn locate_on_an_unmarked_multi_page_snapshot_reports_page_markers_absent() {
    let body = "# 第一章\n\n定理 1.1 有界单调数列收敛。\n\n定理 1.2 闭区间上的连续函数有界。\n";
    let (_dir, root, pdf, markdown) = canonical_book(5, 0, 2, body);
    let server = book_server(&root, None);
    let (_, _) = call(
        &server,
        "material.read",
        json!({ "artifact_id": pdf, "locator": { "pdf_pages": { "from": 1, "to": 3 } } }),
    );
    let (found, is_error) = call(
        &server,
        "material.locate",
        json!({ "artifact_id": markdown, "quote": "定理 1.1 有界单调数列收敛。" }),
    );
    assert!(!is_error, "{found}");
    assert_eq!(found["status"], "ok", "{found}");
    assert_eq!(found["page"], Value::Null, "{found}");
    assert_eq!(found["page_granularity_available"], false, "{found}");
    let absent = found["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "page_markers_absent")
        .unwrap_or_else(|| panic!("{found}"));
    assert_eq!(absent["snapshot_pages"], json!({ "from": 1, "to": 3 }));
    assert_eq!(absent["artifact_id"], markdown);
    // A plain imported Markdown (no PDF) is not "markers absent": it maps to no PDF at all.
    let f = fixture();
    let (plain, _) = call(
        &f.server,
        "material.locate",
        json!({ "artifact_id": f.markdown, "quote": "Every bounded monotone sequence converges." }),
    );
    assert_eq!(plain["status"], "ok", "{plain}");
    assert_eq!(diagnostic_codes(&plain), ["page_unresolved"], "{plain}");
}

fn semantic_base(span: &str) -> Vec<Value> {
    vec![
        json!({
            "local_ref": "u1",
            "kind": "source_unit",
            "op": "create",
            "payload": {
                "kind": "statement",
                "display_title": "有界单调数列收敛",
                "parent": null,
                "source_spans": [span],
                "completeness": "complete"
            }
        }),
        json!({
            "local_ref": "e.theory",
            "kind": "semantic_entity",
            "op": "create",
            "payload": {
                "semantic_kind": "theory",
                "preferred_labels": [{ "lang": "zh", "label": "实数列" }]
            }
        }),
        json!({
            "local_ref": "e.mct",
            "kind": "semantic_entity",
            "op": "create",
            "payload": {
                "semantic_kind": "concept",
                "home_theory": "$new:e.theory",
                "preferred_labels": [{ "lang": "zh", "label": "单调收敛定理" }],
                "plain_label": "单调又有界的数列一定收敛",
                "mathlib_baseline": {
                    "decl": "tendsto_atTop_ciSup",
                    "status": "unchecked"
                }
            }
        }),
        json!({
            "local_ref": "a1",
            "kind": "alignment",
            "op": "create",
            "payload": {
                "source": "$new:u1",
                "target": "$new:e.mct",
                "relation": "expresses",
                "source_context": {
                    "binding": "uncontextualized",
                    "reason": "测试材料没有约定卡"
                },
                "evidence_spans": [span],
                "status": "retrieved_candidate"
            }
        }),
        json!({
            "local_ref": "d1",
            "kind": "debt",
            "op": "create",
            "payload": {
                "debt_kind": "alignment_debt",
                "attached_to": "$new:a1",
                "status": "open",
                "obligation": "核对教材里的“有界”是上有界",
                "quantifier": "对所有实数列",
                "failure_mode": "statement"
            }
        }),
    ]
}

fn propose_semantic(f: &Fixture, request_id: &str, objects: Vec<Value>) -> Value {
    let (result, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": request_id,
            "rationale": "语义层测试",
            "target_object": "e.mct",
            "proposed_objects": objects
        }),
    );
    assert!(!is_error, "unexpected tool error: {result}");
    result
}

fn codes(result: &Value) -> Vec<String> {
    result["diagnostics"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|d| d["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn semantic_layer_proposals_stage_with_review_items_and_never_touch_the_map() {
    let f = fixture();
    let span = seed_source_span(&f.root, &f.markdown);
    let initial_objects = map_object_count(&f.root);
    let staged = propose_semantic(&f, "semantic-ok", semantic_base(&span));
    assert_eq!(staged["status"], "staged", "{staged}");
    for (local_ref, prefix) in [
        ("e.theory", "mathscape:semantic-entity/"),
        ("e.mct", "mathscape:semantic-entity/"),
        ("a1", "mathscape:alignment/"),
        ("d1", "mathscape:debt/"),
    ] {
        let id = staged["staged_ids"][local_ref].as_str().unwrap_or_default();
        assert!(id.starts_with(prefix), "{local_ref}: {staged}");
    }
    let found = codes(&staged);
    assert!(
        found.contains(&"mathlib_baseline_unchecked".to_owned()),
        "{staged}"
    );
    assert!(found.contains(&"uncontextualized".to_owned()), "{staged}");
    assert!(
        found.contains(&"theory_aliases_missing".to_owned()),
        "{staged}"
    );
    assert_eq!(map_object_count(&f.root), initial_objects);

    let mut formula_theory = semantic_base(&span);
    formula_theory[1]["payload"]["preferred_labels"] =
        json!([{ "lang": "zh", "label": "递推 a_{n+1} = 3n + 1" }]);
    let formula_res = propose_semantic(&f, "semantic-formula-theory", formula_theory);
    assert_eq!(formula_res["status"], "staged", "{formula_res}");
    assert!(
        codes(&formula_res).contains(&"theory_name_too_specific".to_owned()),
        "{formula_res}"
    );
}

#[test]
fn semantic_layer_proposals_block_with_contract_codes() {
    let f = fixture();
    let span = seed_source_span(&f.root, &f.markdown);
    type Mutate = fn(&mut Vec<Value>);
    let cases: Vec<(&str, Mutate, &str)> = vec![
        (
            "same-as",
            |o| o[3]["payload"]["relation"] = json!("same_as"),
            "same_as_forbidden",
        ),
        (
            "bad-relation",
            |o| o[3]["payload"]["relation"] = json!("restricts"),
            "invalid_relation",
        ),
        (
            "accepted-status",
            |o| o[3]["payload"]["status"] = json!("accepted"),
            "forbidden_alignment_status",
        ),
        (
            "paid-debt",
            |o| o[4]["payload"]["status"] = json!("paid"),
            "forbidden_debt_status",
        ),
        (
            "confirmed-baseline",
            |o| o[2]["payload"]["mathlib_baseline"]["status"] = json!("confirmed"),
            "forbidden_baseline_status",
        ),
        (
            "trust-claim",
            |o| o[2]["payload"]["trust"] = json!({ "alignment": "accepted" }),
            "forbidden_trust_claim",
        ),
        (
            "bare-ref",
            |o| o[3]["payload"]["target"] = json!("e.mct"),
            "dangling_new_ref",
        ),
        (
            "target-is-source",
            |o| o[3]["payload"]["target"] = json!("$new:u1"),
            "invalid_reference_kind",
        ),
        (
            "missing-home",
            |o| {
                o[2]["payload"]
                    .as_object_mut()
                    .unwrap()
                    .remove("home_theory");
            },
            "missing_required_field",
        ),
        (
            "no-evidence",
            |o| o[3]["payload"]["evidence_spans"] = json!([]),
            "missing_required_field",
        ),
    ];
    for (name, mutate, expected) in cases {
        let mut objects = semantic_base(&span);
        mutate(&mut objects);
        let result = propose_semantic(&f, &format!("semantic-{name}"), objects);
        assert_eq!(result["status"], "blocked", "{name}: {result}");
        assert!(
            codes(&result).contains(&expected.to_owned()),
            "{name}: expected {expected}, got {result}"
        );
    }
}

// ------------------------------------------------ EMIR fixture (S11)

fn emir_fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/emir/fibonacci-two-books")
}

/// Rewrites every fixture-computed id the earlier change sets staged into
/// the id this workspace actually staged (the fixture ids are derived from
/// the fixture's own investigation id).
fn remap_ids(value: &mut Value, ids: &std::collections::BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(actual) = ids.get(text.as_str()) {
                *text = actual.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| remap_ids(item, ids)),
        Value::Object(map) => map.values_mut().for_each(|item| remap_ids(item, ids)),
        _ => {}
    }
}

#[test]
fn emir_fixture_change_sets_all_stage_in_order() {
    let dir = emir_fixture_dir();
    let objects: Value =
        serde_json::from_slice(&std::fs::read(dir.join("objects.json")).unwrap()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    for artifact in objects["artifacts"].as_object().unwrap().values() {
        let file = artifact["file"].as_str().unwrap();
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let id = import(
            &mut pool,
            bytes,
            "text/markdown",
            std::path::Path::new(file)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            None,
        );
        assert_eq!(
            id, artifact["artifact_id"],
            "content-addressed artifact id of {file}"
        );
    }
    drop(pool);
    let workspace = McpWorkspace::open(&root)
        .unwrap()
        .with_round(Some("placeholder".into()))
        .with_capability_runner(Box::new(NothingInstalled));
    let server = McpServer::new(workspace);
    let initial_objects = map_object_count(&root);

    let mut ids = std::collections::BTreeMap::new();
    let mut targets = Vec::new();
    for change_set in objects["change_sets"].as_array().unwrap() {
        let mut proposed = change_set["proposed_objects"].clone();
        remap_ids(&mut proposed, &ids);
        let mut relations = change_set["proposed_relations"].clone();
        remap_ids(&mut relations, &ids);
        let (result, is_error) = call(
            &server,
            "propose_changes",
            json!({
                "request_id": change_set["request_id"],
                "rationale": change_set["rationale"],
                "target_object": change_set["target_object"],
                "proposed_objects": proposed,
                "proposed_relations": relations,
            }),
        );
        assert!(!is_error, "{}: {result}", change_set["request_id"]);
        assert_eq!(
            result["status"], "staged",
            "{}: {result}",
            change_set["request_id"]
        );
        assert!(
            !codes(&result)
                .iter()
                .any(|code| code == "same_as_forbidden"),
            "{result}"
        );
        let expected = change_set["staged_ids"].as_object().unwrap();
        for (local_ref, fixture_id) in expected {
            let actual = result["staged_ids"][local_ref]
                .as_str()
                .unwrap_or_else(|| panic!("{local_ref} not staged: {result}"));
            let prefix = fixture_id.as_str().unwrap().rsplit_once('/').unwrap().0;
            assert!(actual.starts_with(prefix), "{local_ref}: {actual}");
            ids.insert(fixture_id.as_str().unwrap().to_owned(), actual.to_owned());
        }
        targets.push(change_set["target_object"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        targets,
        [
            "jia.u.sec",
            "yi.u.sec",
            "e.fib",
            "e.qpow",
            "e.cassini",
            "e.puzzle"
        ]
    );
    // Staging never touches the accepted map.
    assert_eq!(map_object_count(&root), initial_objects);

    // Search with scope=accepted sees no semantic objects yet; scope=investigation
    // finds the staged theory and shared objects by labels, aliases and mathlib name.
    let (accepted_search, err) = call(
        &server,
        "workspace.search",
        json!({ "query": "Cassini", "scope": "accepted" }),
    );
    assert!(!err);
    assert_eq!(accepted_search["scope"], "accepted");
    assert_eq!(accepted_search["total"], 0);

    let (theory_search, err) = call(
        &server,
        "workspace.search",
        json!({ "query": "线性递推", "kinds": ["theory"], "scope": "investigation" }),
    );
    assert!(!err);
    assert_eq!(theory_search["scope"], "investigation");
    let theory_id = ids
        .iter()
        .find_map(|(_, actual)| {
            let (node, _) = call(
                &server,
                "graph.get",
                json!({ "node_id": actual, "scope": "investigation" }),
            );
            (node["node"]["semantic_kind"] == "theory").then(|| actual.clone())
        })
        .expect("staged theory");
    assert_eq!(theory_search["groups"][0]["kind"], "theory");
    assert_eq!(theory_search["groups"][0]["hits"][0]["id"], theory_id);

    let (entity_search, err) = call(
        &server,
        "workspace.search",
        json!({
            "query": "fib_succ_mul_fib_pred",
            "kinds": ["semantic_entity"],
            "scope": "investigation"
        }),
    );
    assert!(!err);
    assert_eq!(entity_search["groups"][0]["kind"], "semantic_entity");
    let cassini_id = entity_search["groups"][0]["hits"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // graph.get returns semantic payload details and edge_summary (not unsupported).
    let (got, err) = call(
        &server,
        "graph.get",
        json!({ "node_id": cassini_id, "scope": "investigation" }),
    );
    assert!(!err);
    assert_eq!(got["node"]["kind"], "semantic_entity");
    assert_eq!(got["node"]["semantic_kind"], "declaration");
    assert_eq!(got["node"]["semantic"]["home_theory"], theory_id);
    assert!(
        got["node"]["edge_summary"]["aligns"]["in"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert_eq!(got["node"]["edge_summary"]["in_theory"]["out"], 1);

    // graph.neighborhood traverses aligns, in_theory and debt_on when requested.
    let (hood, err) = call(
        &server,
        "graph.neighborhood",
        json!({
            "node_id": cassini_id,
            "depth": 1,
            "direction": "both",
            "edge_types": ["aligns", "in_theory", "debt_on"],
            "scope": "investigation"
        }),
    );
    assert!(!err, "{hood}");
    assert_eq!(hood["status"], "ok");
    let kinds: Vec<&str> = hood["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"semantic_entity"), "{hood}");
    assert!(kinds.contains(&"source_unit"), "{hood}");
    assert!(kinds.contains(&"debt"), "{hood}");

    // home_theory can be edited later to move an entity to another theory shelf,
    // while theory-name warnings fire when formulas are stuffed into a theory label.
    let (reorg, err) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "reorg-theory-shelf",
            "rationale": "新建「平面几何剪拼」理论书架，把剪拼题迁过去",
            "target_object": "t.geom",
            "proposed_objects": [
                {
                    "local_ref": "t.geom",
                    "kind": "semantic_entity",
                    "op": "create",
                    "payload": {
                        "semantic_kind": "theory",
                        "preferred_labels": [{ "lang": "zh", "label": "平面几何剪拼" }],
                        "aliases": ["dissection puzzles", "几何剪拼", "面积悖论"],
                        "plain_label": "理论：平面几何剪拼"
                    }
                },
                {
                    "local_ref": "move.puzzle",
                    "kind": "semantic_entity",
                    "op": "edit",
                    "target_id": got["node"]["semantic"]["home_theory"],
                    "expected_revision": got["node"]["revision"],
                    "changes": {}
                }
            ]
        }),
    );
    assert!(!err);
    // Editing the wrong revision blocks cleanly; editing with the live staged
    // revision of e.puzzle succeeds.
    assert_eq!(reorg["status"], "blocked");
    let puzzle_id = objects["change_sets"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|cs| cs["staged_ids"]["e.puzzle"].as_str())
        .map(|fid| ids[fid].clone())
        .unwrap();
    let (puzzle_node, _) = call(
        &server,
        "graph.get",
        json!({ "node_id": puzzle_id, "scope": "investigation" }),
    );
    let puzzle_rev = puzzle_node["node"]["revision"].as_str().unwrap();
    let (moved, err) = call(
        &server,
        "propose_changes",
        json!({
            "request_id": "reorg-theory-shelf-ok",
            "rationale": "新建「平面几何剪拼」理论书架，把剪拼题迁过去",
            "target_object": "t.geom",
            "proposed_objects": [
                {
                    "local_ref": "t.geom",
                    "kind": "semantic_entity",
                    "op": "create",
                    "payload": {
                        "semantic_kind": "theory",
                        "preferred_labels": [{ "lang": "zh", "label": "平面几何剪拼" }],
                        "aliases": ["dissection puzzles", "几何剪拼", "面积悖论"],
                        "plain_label": "理论：平面几何剪拼"
                    }
                },
                {
                    "local_ref": "move.puzzle",
                    "kind": "semantic_entity",
                    "op": "edit",
                    "target_id": puzzle_id,
                    "expected_revision": puzzle_rev,
                    "changes": { "home_theory": "$new:t.geom" }
                }
            ]
        }),
    );
    assert!(!err);
    assert_eq!(moved["status"], "staged", "{moved}");
    let new_theory_id = moved["staged_ids"]["t.geom"].as_str().unwrap();
    let (puzzle_after, _) = call(
        &server,
        "graph.get",
        json!({ "node_id": puzzle_id, "scope": "investigation" }),
    );
    assert_eq!(
        puzzle_after["node"]["semantic"]["home_theory"],
        new_theory_id
    );
}
