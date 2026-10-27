//! End-to-end tests of the read-only MCP slice on a fixture workspace built
//! through the real artifact pool (markdown + generated PDFs).

use std::ffi::OsString;
use std::io;
use std::path::Path;

use lopdf::{Bookmark, Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_agent::{CommandOutput, CommandRunner};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{ActorRef, MediaType, RightsRecord, RightsStatus};
use mathscape_mcp::tools::{MAX_GREP_HITS, MAX_READ_BYTES, MAX_SCAN_BYTES, TOOL_NAMES};
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
    let unknown_tool = request(&f.server, 4, "tools/call", json!({ "name": "graph.get" }));
    assert_eq!(unknown_tool["error"]["code"], -32602);
    assert_eq!(f.server.workspace().round(), Some("placeholder"));
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
        ["workspace.search", "material.read", "material.grep", "pdf.outline"]
    );
    for tool in tools {
        assert!(!tool["description"].as_str().unwrap().is_empty());
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], json!(false));
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
    let second = request(&f.server, 2, "tools/list", json!({}));
    assert_eq!(
        first["result"], second["result"],
        "tools/list is byte-stable"
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
fn read_pdf_text_layer_pages() {
    let f = fixture();
    let result = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 1, "to": 9 } } }),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(result["content_type"], "pdf_text");
    assert_eq!(result["truncated"], false);
    assert_eq!(result["total_pages"], 3);
    assert_eq!(
        result["locator"],
        json!({ "pdf_pages": { "from": 1, "to": 3 } })
    );
    let text = result["text"].as_str().unwrap();
    let spans = result["page_spans"].as_array().unwrap();
    assert_eq!(spans.len(), 3);
    let page = |index: usize| {
        let start = spans[index]["start"].as_u64().unwrap() as usize;
        let end = spans[index]["end"].as_u64().unwrap() as usize;
        &text[start..end]
    };
    let first: Vec<_> = page(0).lines().map(str::trim).collect();
    assert_eq!(first, ["First text run", "Second text run"]);
    assert_eq!(page(1), "");
    assert_eq!(page(2).trim(), "Third page text");
    let codes: Vec<_> = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["range_clamped", "pdf_pages_without_text"]);
    let without_text = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "pdf_pages_without_text")
        .unwrap();
    assert_eq!(without_text["pages"], json!([2]));

    let single = read(
        &f.server,
        json!({ "artifact_id": f.pdf, "locator": { "pdf_pages": { "from": 3, "to": 3 } }, "expand": { "before": 1 } }),
    );
    assert_eq!(single["text"].as_str().unwrap().trim(), "Third page text");
    assert_eq!(
        single["locator"],
        json!({ "pdf_pages": { "from": 3, "to": 3 } })
    );
    assert_eq!(single["diagnostics"][0]["code"], "expand_not_applicable");
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
    assert_eq!(body["status"], "ok");
    assert!(
        body["text"].as_str().unwrap().contains("Section page"),
        "{}",
        body["text"]
    );
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
        json!({ "artifact_ids": [f.pdf], "pattern": "First" }),
        json!({ "artifact_ids": [f.markdown, f.pdf], "pattern": "Lemma" }),
        json!({ "artifact_ids": [f.plain_text], "pattern": "plain" }),
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
    let pdf = grep_error(
        &f.server,
        json!({ "artifact_ids": [f.pdf], "pattern": "First" }),
    );
    assert!(
        pdf["message"].as_str().unwrap().contains("text/markdown"),
        "{pdf}"
    );
    assert_eq!(pdf["details"]["artifacts"][0]["artifact_id"], f.pdf);
    assert_eq!(
        pdf["details"]["artifacts"][0]["media_type"],
        "application/pdf"
    );
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
