//! End-to-end tests of the Phase1c graph tools (`graph.get`,
//! `graph.neighborhood`) on a fixture workspace: a PDF converted to Markdown,
//! source spans/units/relations written in the stored source-map shape, one
//! dangling span and one ink note.

use std::collections::BTreeMap;
use std::path::Path;

use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, CommandRecordStatus, ConversionExitStatus, ConversionParameters, ConversionRun,
    EntityHeader, EntityKind, LifecycleStatus, MediaType, PageIndexBase, PageRange, RevisionId,
    RightsRecord, RightsStatus, SchemaVersion, StableId, Timestamp,
};
use mathscape_mcp::tools::{DEFAULT_MAX_NODES, MAX_EDGES, MAX_NODES, MAX_SEEDS};
use mathscape_mcp::{McpServer, McpWorkspace};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

const MARKDOWN_BODY: &str = "# 引理 Lemma\n\nEvery bounded monotone sequence converges.\n";
const MISSING_ID: &str = "mathscape:source-unit/does-not-exist";
const NOTE_ID: &str = "0123456789abcdef0123456789abcdef";
const CONVERSION_ID: &str = "mathscape:conversion/graph-test";

fn actor() -> ActorRef {
    ActorRef::parse("graph-test").unwrap()
}

fn rights() -> RightsRecord {
    RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap()
}

fn unit(name: &str) -> String {
    format!("mathscape:source-unit/{name}")
}

fn span(name: &str) -> String {
    format!("mathscape:source-span/{name}")
}

fn relation(name: &str) -> String {
    format!("mathscape:source-relation/{name}")
}

struct Fixture {
    _dir: TempDir,
    server: McpServer,
    root: std::path::PathBuf,
    pdf: String,
    markdown: String,
    snapshot_stand_in: String,
}

struct MapWriter {
    connection: Connection,
    revision: u64,
}

fn trust() -> Value {
    json!({
        "provenance": "exact", "parse": "reviewed", "segmentation": "checked",
        "typing": "unchecked", "proof": "none", "alignment": "unmatched",
        "curation": "accepted", "confidence": 0.95, "evidence": [],
        "reviewers": ["graph-test"]
    })
}

impl MapWriter {
    fn insert(&mut self, id: &str, shape: &str, kind: &str, payload: Value) -> String {
        self.revision += 1;
        let revision = format!("sha256:{:064x}", self.revision);
        self.connection
            .execute(
                "INSERT INTO map_objects (object_id, object_shape, entity_kind, current_revision)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, shape, kind, revision],
            )
            .unwrap();
        self.connection
            .execute(
                "INSERT INTO map_object_revisions
                 (object_id, revision_id, schema_version, lifecycle, created_at,
                  created_by, generation_run, payload_json)
                 VALUES (?1, ?2, 'mathscape-source-1', 'active',
                         '2026-10-28T10:00:00+08:00', 'graph-test', NULL, ?3)",
                params![id, revision, payload.to_string()],
            )
            .unwrap();
        revision
    }

    fn span(&mut self, id: &str, locator: Value) {
        let mut payload = json!({
            "granularity": "block", "layout": "other", "page": null,
            "bounding_boxes": [], "marker_block_ids": [], "markdown": null,
            "exact_text_artifact": null, "normalized_text_hash": null
        });
        for (key, value) in locator.as_object().unwrap() {
            payload[key] = value.clone();
        }
        self.insert(
            id,
            "entity",
            "SourceSpan",
            json!({ "payload_kind": "span", "payload": payload }),
        );
    }

    fn unit(&mut self, id: &str, kind: &str, title: &str, parent: Option<&str>, spans: &[&str]) {
        self.insert(
            id,
            "entity",
            "SourceSemanticUnit",
            json!({
                "payload_kind": "semantic_unit",
                "payload": {
                    "edition_binding": {"status": "unresolved"},
                    "kind": kind, "display_title": title, "source_title": null,
                    "source_labels": [], "parent_unit": parent, "source_spans": spans,
                    "dialect_binding": {"status": "uncontextualized"},
                    "resulting_dialect_scope": null, "surface_content_artifact": null,
                    "local_references": [], "completeness": "complete",
                    "facts_pending": false, "trust": trust()
                }
            }),
        );
    }

    fn relation(&mut self, id: &str, kind: &str, source: &str, target: &str, evidence: &[&str]) {
        let revision = self.insert(
            id,
            "relation",
            "SourceUnitRelation",
            json!({
                "payload_kind": "unit_relation",
                "payload": { "relation": kind, "trust": trust() }
            }),
        );
        let endpoint = |role: &str, ordinal_key: i64, ordinal: Option<i64>, entity: &str| {
            self.connection
                .execute(
                    "INSERT INTO map_relation_endpoints
                     (relation_id, revision_id, role, ordinal_key, ordinal, entity_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, revision, role, ordinal_key, ordinal, entity],
                )
                .unwrap();
        };
        endpoint("source", -1, None, source);
        endpoint("target", -1, None, target);
        for (ordinal, item) in evidence.iter().enumerate() {
            let ordinal = i64::try_from(ordinal).unwrap();
            endpoint("evidence", ordinal, Some(ordinal), item);
        }
    }
}

fn import(pool: &mut ArtifactPool, bytes: &[u8], metadata: ArtifactMetadata) -> String {
    pool.import_bytes(bytes, metadata, actor())
        .unwrap()
        .stable_id()
        .to_string()
}

fn record_conversion(pool: &mut ArtifactPool, pdf: &str, markdown: &str) {
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(0, 0, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(false),
            BTreeMap::new(),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-10-28T10:00:00+08:00").unwrap()),
        Some(Timestamp::parse("2026-10-28T10:01:00+08:00").unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            StableId::parse(CONVERSION_ID).unwrap(),
            RevisionId::parse(format!("sha256:{}", "7".repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("graph-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse("2026-10-28T10:01:00+08:00").unwrap(),
            actor(),
            None,
        ),
        vec![StableId::parse(pdf).unwrap()],
        vec![StableId::parse(markdown).unwrap()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
}

/// Graph shape (structural edges only):
///
/// ```text
/// pdf --conversion--> md
/// sec  -contains_span-> s_title  -located_in-> md
/// sec  -has_part-> lemma -contains_span-> s_body (md), s_page (pdf p.1)
/// sec  -has_part-> proof -contains_span-> s_body
/// sec  -has_part-> bare (no spans), inked -contains_span-> s_ink
/// sec  -has_part-> child-NN (extra_children, no spans)
/// proof -proves-> lemma (evidence s_body); lemma -cites-> sec
/// s_dangling -> markdown artifact that is not in the catalog
/// ```
fn fixture(extra_children: usize) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let pdf = import(
        &mut pool,
        b"%PDF-1.7\ngraph fixture\n",
        ArtifactMetadata::new(
            None,
            MediaType::parse("application/pdf").unwrap(),
            Some("lemma-paper.pdf".to_owned()),
            Some(1),
            rights(),
        ),
    );
    let markdown = import(
        &mut pool,
        MARKDOWN_BODY.as_bytes(),
        ArtifactMetadata::derived(
            MediaType::parse("text/markdown").unwrap(),
            Some("lemma-paper.md".to_owned()),
        )
        .unwrap(),
    );
    let snapshot_stand_in = import(
        &mut pool,
        br#"{"ink":"stand-in"}"#,
        ArtifactMetadata::new(
            None,
            MediaType::parse("application/json").unwrap(),
            Some("ink-stand-in.json".to_owned()),
            None,
            rights(),
        ),
    );
    record_conversion(&mut pool, &pdf, &markdown);
    drop(pool);

    let mut writer = MapWriter {
        connection: Connection::open(root.join("mathscape.sqlite3")).unwrap(),
        revision: 0,
    };
    let md_span = |start: usize, end: usize| json!({"markdown": {"artifact": markdown, "byte_range": {"start": start, "end": end}, "displayed_lines": null}});
    writer.span(&span("s_title"), md_span(0, 14));
    writer.span(&span("s_body"), md_span(16, 58));
    writer.span(
        &span("s_page"),
        json!({"page": {"artifact": pdf, "pdf_page_index": 0, "marker_page_id": null, "printed_page_label": null}}),
    );
    writer.span(
        &span("s_ink"),
        json!({"ink": {"snapshot_artifact": snapshot_stand_in, "page_id": "p", "stroke_ids": ["a"], "selection_mode": "lasso"}}),
    );
    writer.span(
        &span("s_dangling"),
        json!({"markdown": {"artifact": "mathscape:artifact/sha256/0000000000000000000000000000000000000000000000000000000000000000", "byte_range": {"start": 0, "end": 1}, "displayed_lines": null}}),
    );
    writer.unit(&unit("sec"), "section", "第一节", None, &[&span("s_title")]);
    writer.unit(
        &unit("lemma"),
        "lemma",
        "单调有界引理",
        Some(&unit("sec")),
        &[&span("s_body"), &span("s_page")],
    );
    writer.unit(
        &unit("proof"),
        "proof",
        "引理的证明",
        Some(&unit("sec")),
        &[&span("s_body")],
    );
    writer.unit(
        &unit("bare"),
        "remark",
        "无原文的注记",
        Some(&unit("sec")),
        &[],
    );
    writer.unit(
        &unit("inked"),
        "remark",
        "手写注记",
        Some(&unit("sec")),
        &[&span("s_ink")],
    );
    for index in 0..extra_children {
        writer.unit(
            &unit(&format!("child-{index:03}")),
            "remark",
            &format!("子单元 {index}"),
            Some(&unit("sec")),
            &[],
        );
    }
    writer.relation(
        &relation("proves"),
        "proves",
        &unit("proof"),
        &unit("lemma"),
        &[&span("s_body")],
    );
    writer.relation(
        &relation("cites"),
        "cites",
        &unit("lemma"),
        &unit("sec"),
        &[],
    );
    writer
        .connection
        .execute(
            "INSERT INTO ink_documents
             (document_id, version, title, created_at, exported_at, pages_json,
              capture_segments_json, head_seq, updated_at)
             VALUES (?1, '1', '极限笔记', '2026-10-28T10:00:00+08:00',
                     '2026-10-28T10:00:00+08:00', '[]', '[]', 0, '2026-10-28T10:00:00+08:00')",
            params![NOTE_ID],
        )
        .unwrap();
    drop(writer);

    let workspace = McpWorkspace::open(&root).unwrap();
    Fixture {
        _dir: dir,
        server: McpServer::new(workspace),
        root,
        pdf,
        markdown,
        snapshot_stand_in,
    }
}

fn call(server: &McpServer, name: &str, arguments: Value) -> (Value, bool) {
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .expect("requests get a response");
    assert!(
        response.get("error").is_none(),
        "JSON-RPC error: {response}"
    );
    let result = &response["result"];
    let structured = result["structuredContent"].clone();
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, structured, "text content mirrors structuredContent");
    (structured, result["isError"].as_bool().unwrap())
}

fn ok(server: &McpServer, name: &str, arguments: Value) -> Value {
    let (result, is_error) = call(server, name, arguments);
    assert!(!is_error, "unexpected call error: {result}");
    result
}

fn get(f: &Fixture, id: &str) -> Value {
    ok(&f.server, "graph.get", json!({ "node_id": id }))
}

fn hood(f: &Fixture, arguments: Value) -> Value {
    ok(&f.server, "graph.neighborhood", arguments)
}

fn codes(result: &Value) -> Vec<String> {
    result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

fn diag<'a>(result: &'a Value, code: &str) -> &'a Value {
    result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} diagnostic in {result}"))
}

fn node_ids(result: &Value) -> Vec<String> {
    result["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect()
}

fn edge_relations(result: &Value) -> Vec<String> {
    result["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["relation"].as_str().unwrap().to_owned())
        .collect()
}

fn assert_no_dangling_edges(result: &Value) {
    let ids = node_ids(result);
    for edge in result["edges"].as_array().unwrap() {
        for endpoint in edge["endpoints"].as_array().unwrap() {
            let node = endpoint["node_id"].as_str().unwrap();
            assert!(ids.iter().any(|id| id == node), "dangling {node} in {edge}");
        }
    }
}

fn table_counts(root: &Path) -> Vec<(String, i64)> {
    let connection = Connection::open(root.join("mathscape.sqlite3")).unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables
        .into_iter()
        .map(|table| {
            let count = connection
                .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (table, count)
        })
        .collect()
}

#[test]
fn neighborhood_argument_errors_are_invalid_arg() {
    let f = fixture(0);
    let seed = unit("lemma");
    let nine: Vec<String> = (0..=MAX_SEEDS).map(|i| unit(&format!("s{i}"))).collect();
    for arguments in [
        json!({}),
        json!({ "node_id": seed, "seed_ids": [seed] }),
        json!({ "seed_ids": [] }),
        json!({ "seed_ids": [seed, seed] }),
        json!({ "seed_ids": nine }),
        json!({ "node_id": seed, "depth": 4 }),
        json!({ "node_id": seed, "max_nodes": MAX_NODES + 1 }),
        json!({ "node_id": seed, "max_nodes": 0 }),
        json!({ "node_id": seed, "max_edges": MAX_EDGES + 1 }),
        json!({ "node_id": seed, "edge_types": ["unknown"] }),
        json!({ "node_id": seed, "node_kinds": ["relation"] }),
        json!({ "node_id": seed, "direction": "sideways" }),
        json!({ "node_id": seed, "include_virtual": true }),
        json!({ "node_id": seed, "bogus": 1 }),
        json!({ "node_id": "" }),
    ] {
        let (result, is_error) = call(&f.server, "graph.neighborhood", arguments.clone());
        assert!(is_error, "expected error for {arguments}: {result}");
        assert_eq!(
            result["error"]["code"], "invalid_arg",
            "{arguments}: {result}"
        );
    }
    for arguments in [
        json!({}),
        json!({ "node_id": "" }),
        json!({ "node_id": seed, "x": 1 }),
    ] {
        let (result, is_error) = call(&f.server, "graph.get", arguments.clone());
        assert!(is_error, "expected error for {arguments}: {result}");
        assert_eq!(result["error"]["code"], "invalid_arg");
    }
}

#[test]
fn missing_ids_are_not_found_all_or_nothing() {
    let f = fixture(0);
    let result = get(&f, MISSING_ID);
    assert_eq!(result["status"], "not_found");
    assert_eq!(result["truncated"], false);
    assert!(result["node"].is_null());
    assert!(codes(&result).contains(&"node_not_found".to_owned()));

    let result = hood(&f, json!({ "seed_ids": [unit("lemma"), MISSING_ID] }));
    assert_eq!(result["status"], "not_found", "{result}");
    assert_eq!(result["nodes"], json!([]));
    assert_eq!(result["edges"], json!([]));
    assert_eq!(
        diag(&result, "node_not_found")["details"]["missing_ids"],
        json!([MISSING_ID])
    );
}

#[test]
fn unit_get_points_at_markdown_and_material_read_accepts_it() {
    let f = fixture(0);
    let result = get(&f, &unit("lemma"));
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["truncated"], false);
    let node = &result["node"];
    assert_eq!(node["kind"], "source_unit");
    assert_eq!(node["parent_artifact_id"], f.markdown);
    let content_ref = &node["content_ref"];
    assert_eq!(content_ref["type"], "md_byte_range");
    assert_eq!(content_ref["artifact_id"], f.markdown);
    assert_eq!(content_ref["via"], span("s_body"));
    assert!(codes(&result).contains(&"multiple_spans".to_owned()));
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .find("Every bounded")
            .is_none(),
        "no body text"
    );
    let summary = &node["edge_summary"];
    assert!(summary.get("contains_span").is_some(), "{summary}");
    assert!(summary.get("proves").is_some(), "{summary}");

    let read = ok(
        &f.server,
        "material.read",
        json!({ "artifact_id": content_ref["artifact_id"], "locator": content_ref["locator"] }),
    );
    assert_eq!(
        read["text"], "Every bounded monotone sequence converges.",
        "{read}"
    );
}

#[test]
fn spans_artifacts_and_relations_resolve() {
    let f = fixture(0);
    let page = get(&f, &span("s_page"));
    assert_eq!(page["node"]["kind"], "source_span");
    assert_eq!(page["node"]["parent_artifact_id"], f.pdf);
    assert_eq!(page["node"]["content_ref"]["type"], "pdf_pages");
    assert_eq!(
        page["node"]["content_ref"]["locator"],
        json!({ "pdf_pages": { "from": 1, "to": 1 } })
    );

    let markdown = get(&f, &f.markdown);
    assert_eq!(markdown["node"]["kind"], "artifact");
    assert!(
        markdown["node"]["edge_summary"].get("conversion").is_some(),
        "{markdown}"
    );
    assert!(
        markdown["node"]["edge_summary"].get("located_in").is_some(),
        "{markdown}"
    );

    for id in [relation("proves"), CONVERSION_ID.to_owned()] {
        let result = get(&f, &id);
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["node"]["kind"], "relation", "{result}");
        assert!(result["node"]["content_ref"].is_null());
    }

    let bare = get(&f, &unit("bare"));
    assert!(bare["node"]["content_ref"].is_null());
    assert_eq!(
        diag(&bare, "content_unavailable")["details"]["reason"],
        "no_span_locator"
    );
    let inked = get(&f, &unit("inked"));
    let content_ref = &inked["node"]["content_ref"];
    assert_eq!(content_ref["type"], "ink_page", "{inked}");
    assert_eq!(content_ref["tool"], "ink.page_image");
    assert_eq!(content_ref["node_id"], span("s_ink"));
    assert_eq!(content_ref["page_id"], "p");
    assert_eq!(content_ref["stroke_ids"], json!(["a"]));
    assert_eq!(content_ref["via"], span("s_ink"));
    assert!(!codes(&inked).contains(&"content_unavailable".to_owned()));
    assert_eq!(inked["node"]["parent_artifact_id"], f.snapshot_stand_in);
    // Without an ink_snapshots row a plain application/json artifact stays an artifact.
    assert_eq!(get(&f, &f.snapshot_stand_in)["node"]["kind"], "artifact");
}

#[test]
fn note_content_ref_is_ink_page() {
    let f = fixture(0);
    let result = get(&f, NOTE_ID);
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["node"]["kind"], "note");
    let content_ref = &result["node"]["content_ref"];
    assert_eq!(content_ref["type"], "ink_page", "{result}");
    assert_eq!(content_ref["tool"], "ink.page_image");
    assert_eq!(content_ref["node_id"], NOTE_ID);
    assert_eq!(content_ref["page_count"], 0);
    assert_eq!(content_ref["media_type"], "image/png");
    assert!(!codes(&result).contains(&"content_unavailable".to_owned()));
    // No snapshots or links in the base fixture: the note is isolated.
    let result = hood(&f, json!({ "node_id": NOTE_ID }));
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(node_ids(&result), [NOTE_ID]);
}

#[test]
fn unsupported_kind_is_a_soft_ok() {
    let f = fixture(0);
    let mut writer = MapWriter {
        connection: Connection::open(f.root.join("mathscape.sqlite3")).unwrap(),
        revision: 1000,
    };
    writer.insert(
        "mathscape:local-symbol/unsupported",
        "entity",
        "LocalSymbol",
        json!({ "payload_kind": "local_symbol", "payload": {} }),
    );
    drop(writer);
    let result = get(&f, "mathscape:local-symbol/unsupported");
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["node"]["kind"], "unsupported");
    assert!(codes(&result).contains(&"unsupported_node_kind".to_owned()));
    let result = hood(
        &f,
        json!({ "node_id": "mathscape:local-symbol/unsupported", "depth": 2 }),
    );
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["edges"], json!([]));
}

#[test]
fn dangling_span_is_skipped_with_a_diagnostic() {
    let f = fixture(0);
    let result = get(&f, &unit("lemma"));
    let skipped = diag(&result, "dangling_endpoint_skipped");
    assert!(skipped.to_string().contains("s_dangling"), "{skipped}");
    // The span itself stays readable; only its located_in edge is dropped.
    let dangling = get(&f, &span("s_dangling"));
    assert_eq!(dangling["status"], "ok", "{dangling}");
    assert_eq!(dangling["node"]["kind"], "source_span");
    assert_eq!(dangling["node"]["edge_summary"], json!({}));
    assert!(dangling["node"]["content_ref"].is_null());
}

#[test]
fn heuristic_is_default_and_raw_preset_keeps_legacy_structural_five() {
    let f = fixture(0);
    // Default call uses `heuristic`: 0-cost provenance reaches markdown & PDF,
    // and intra-source reasoning (`proves`, `cites`) is traversed at depth 1.
    let heuristic = hood(&f, json!({ "node_id": unit("lemma") }));
    assert_eq!(heuristic["status"], "ok", "{heuristic}");
    assert_eq!(heuristic["applied"]["preset"], "heuristic");
    assert_eq!(heuristic["applied"]["depth"], 1);
    assert_no_dangling_edges(&heuristic);
    let h_ids = node_ids(&heuristic);
    assert!(
        h_ids.contains(&unit("proof"))
            && h_ids.contains(&unit("sec"))
            && h_ids.contains(&f.markdown)
            && h_ids.contains(&f.pdf),
        "{h_ids:?}"
    );
    let h_rels = edge_relations(&heuristic);
    assert!(
        h_rels.iter().any(|r| r == "proves") && h_rels.iter().any(|r| r == "cites"),
        "{h_rels:?}"
    );

    // Explicit `preset: "raw"` preserves the legacy 1-relation-1-hop structural five.
    let raw = hood(&f, json!({ "node_id": unit("lemma"), "preset": "raw" }));
    assert_eq!(raw["status"], "ok", "{raw}");
    assert_eq!(raw["applied"]["preset"], "raw");
    assert_eq!(
        raw["applied"]["edge_types"],
        json!([
            "conversion",
            "located_in",
            "contains_span",
            "has_part",
            "has_snapshot"
        ])
    );
    assert_eq!(raw["applied"]["direction"], "both");
    assert_eq!(raw["applied"]["depth"], 1);
    let relations = edge_relations(&raw);
    assert!(
        !relations.iter().any(|r| r == "proves" || r == "cites"),
        "{relations:?}"
    );
    assert!(codes(&raw).contains(&"edge_types_filtered".to_owned()));
    assert_no_dangling_edges(&raw);
    let ids = node_ids(&raw);
    assert_eq!(ids[0], unit("lemma"));
    assert!(
        ids.contains(&unit("sec")) && ids.contains(&span("s_body")),
        "{ids:?}"
    );
    assert!(
        !ids.contains(&unit("proof")),
        "proof only reached via has_part at depth 2 in raw preset"
    );

    let semantic = hood(
        &f,
        json!({ "node_id": unit("lemma"), "edge_types": ["proves", "cites"] }),
    );
    assert_eq!(semantic["applied"]["preset"], "raw");
    let mut relations = edge_relations(&semantic);
    relations.sort();
    assert_eq!(relations, ["cites", "proves"], "{semantic}");
    let ids = node_ids(&semantic);
    assert!(
        ids.contains(&unit("proof")) && ids.contains(&unit("sec")),
        "{ids:?}"
    );
    assert!(
        ids.contains(&span("s_body")),
        "evidence endpoint is returned: {ids:?}"
    );
    assert_no_dangling_edges(&semantic);

    let none = hood(&f, json!({ "node_id": unit("lemma"), "edge_types": [] }));
    assert_eq!(node_ids(&none), [unit("lemma")]);
    assert_eq!(none["edges"], json!([]));
    let zero = hood(&f, json!({ "node_id": unit("lemma"), "depth": 0 }));
    assert_eq!(node_ids(&zero), [unit("lemma")]);
}

#[test]
fn heuristic_hub_sink_stops_theory_and_foreign_book_explosion() {
    let f = fixture(0);
    let mut writer = MapWriter {
        connection: Connection::open(f.root.join("mathscape.sqlite3")).unwrap(),
        revision: 3000,
    };
    let theory = "mathscape:semantic-entity/th_seq";
    let cassini = "mathscape:semantic-entity/e_cassini";
    let qpow = "mathscape:semantic-entity/e_qpow";
    let unrelated = "mathscape:semantic-entity/e_unrelated";
    let debt = "mathscape:debt/d_forward";
    let book_b_sec = unit("book_b_sec");
    let book_b_thm = unit("book_b_thm");
    let book_b_other = unit("book_b_other");

    writer.unit(&book_b_sec, "section", "B书 §2", None, &[]);
    writer.unit(
        &book_b_thm,
        "statement",
        "B书 定理 2.1",
        Some(&book_b_sec),
        &[],
    );
    writer.unit(
        &book_b_other,
        "problem",
        "B书 无关习题 2.9",
        Some(&book_b_sec),
        &[],
    );
    writer.insert(
        theory,
        "entity",
        "SemanticEntity",
        json!({ "payload_kind": "semantic_entity", "payload": {
            "semantic_kind": "theory",
            "preferred_labels": [{ "lang": "zh", "label": "数列与递推" }]
        }}),
    );
    writer.insert(
        qpow,
        "entity",
        "SemanticEntity",
        json!({ "payload_kind": "semantic_entity", "payload": {
            "semantic_kind": "declaration",
            "home_theory": theory,
            "plain_label": "矩阵幂引理"
        }}),
    );
    writer.insert(
        cassini,
        "entity",
        "SemanticEntity",
        json!({ "payload_kind": "semantic_entity", "payload": {
            "semantic_kind": "declaration",
            "home_theory": theory,
            "plain_label": "Cassini 恒等式",
            "declaration": { "uses_entities": [qpow] }
        }}),
    );
    writer.insert(
        unrelated,
        "entity",
        "SemanticEntity",
        json!({ "payload_kind": "semantic_entity", "payload": {
            "semantic_kind": "declaration",
            "home_theory": theory,
            "plain_label": "同书架无关定理"
        }}),
    );
    writer.insert(
        "mathscape:alignment/a_lemma",
        "entity",
        "Alignment",
        json!({ "payload_kind": "alignment", "payload": {
            "source": unit("lemma"), "target": cassini, "relation": "specializes", "status": "retrieved_candidate"
        }}),
    );
    writer.insert(
        "mathscape:alignment/a_book_b",
        "entity",
        "Alignment",
        json!({ "payload_kind": "alignment", "payload": {
            "source": book_b_thm, "target": cassini, "relation": "expresses", "status": "retrieved_candidate"
        }}),
    );
    writer.insert(
        debt,
        "entity",
        "Debt",
        json!({ "payload_kind": "debt", "payload": {
            "attached_to": "mathscape:alignment/a_lemma",
            "status": "claimed_paid",
            "obligation": "代入验证",
            "paid_by": [{ "ref": qpow, "note": "取行列式" }]
        }}),
    );
    drop(writer);

    // Golden 2-hop from `lemma`: crosses `cassini` to `book_b_thm` and `qpow`,
    // attaches `theory`, `debt` and `book_b_sec`, but NEVER bounces into
    // `unrelated` on the same theory shelf or `book_b_other` in Book B!
    let res = hood(&f, json!({ "node_id": unit("lemma"), "depth": 2 }));
    assert_eq!(res["status"], "ok", "{res}");
    assert_no_dangling_edges(&res);
    let ids = node_ids(&res);
    for expected in [
        cassini,
        qpow,
        theory,
        debt,
        book_b_thm.as_str(),
        book_b_sec.as_str(),
    ] {
        assert!(
            ids.contains(&expected.to_owned()),
            "missing {expected} in {ids:?}"
        );
    }
    assert!(
        !ids.contains(&unrelated.to_owned()),
        "theory hub leaked {unrelated}: {ids:?}"
    );
    assert!(
        !ids.contains(&book_b_other),
        "foreign section leaked {book_b_other}: {ids:?}"
    );

    // Seeding directly on `theory` DOES list all shelf members at depth 1.
    let shelf = hood(&f, json!({ "node_id": theory, "depth": 1 }));
    let shelf_ids = node_ids(&shelf);
    for member in [cassini, qpow, unrelated] {
        assert!(
            shelf_ids.contains(&member.to_owned()),
            "missing shelf member {member}: {shelf_ids:?}"
        );
    }
}

#[test]
fn direction_follows_from_and_to_sides() {
    let f = fixture(0);
    let out = hood(
        &f,
        json!({ "node_id": unit("lemma"), "preset": "raw", "direction": "out", "depth": 2 }),
    );
    let ids = node_ids(&out);
    assert!(
        ids.contains(&span("s_body")) && ids.contains(&f.markdown),
        "{ids:?}"
    );
    assert!(
        !ids.contains(&unit("sec")),
        "parent is on the from side: {ids:?}"
    );

    let into = hood(
        &f,
        json!({ "node_id": f.markdown, "preset": "raw", "direction": "in", "depth": 2, "node_kinds": ["source_unit"] }),
    );
    let ids = node_ids(&into);
    assert_eq!(ids[0], f.markdown, "seed is kept despite node_kinds");
    for expected in [unit("sec"), unit("lemma"), unit("proof")] {
        assert!(ids.contains(&expected), "{expected} missing from {ids:?}");
    }
    assert!(!ids.contains(&span("s_body")), "spans filtered from output");
    assert!(codes(&into).contains(&"nodes_filtered".to_owned()));
    assert_no_dangling_edges(&into);

    let parent = hood(
        &f,
        json!({ "node_id": unit("sec"), "preset": "raw", "direction": "out", "depth": 1 }),
    );
    let ids = node_ids(&parent);
    assert!(
        ids.contains(&unit("lemma")) && ids.contains(&unit("bare")),
        "{ids:?}"
    );

    let conversion = hood(
        &f,
        json!({ "node_id": f.pdf, "preset": "raw", "direction": "out" }),
    );
    assert!(node_ids(&conversion).contains(&f.markdown), "{conversion}");
    assert!(edge_relations(&conversion).contains(&"conversion".to_owned()));
}

#[test]
fn large_neighborhood_truncates_deterministically() {
    let f = fixture(60);
    let arguments = json!({ "node_id": unit("sec") });
    let first = hood(&f, arguments.clone());
    assert_eq!(first["status"], "truncated", "{}", first["counts"]);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["nodes"].as_array().unwrap().len(), DEFAULT_MAX_NODES);
    assert!(first["counts"]["nodes_seen"].as_u64().unwrap() > DEFAULT_MAX_NODES as u64);
    assert!(codes(&first).contains(&"max_nodes_reached".to_owned()));
    assert_no_dangling_edges(&first);
    let second = hood(&f, arguments);
    assert_eq!(first["nodes"], second["nodes"]);
    assert_eq!(first["edges"], second["edges"]);

    let capped = hood(
        &f,
        json!({ "node_id": unit("sec"), "max_nodes": 100, "max_edges": 5 }),
    );
    assert_eq!(capped["status"], "truncated");
    assert_eq!(capped["edges"].as_array().unwrap().len(), 5);
    assert!(codes(&capped).contains(&"max_edges_reached".to_owned()));
}

#[test]
fn graph_tools_do_not_write_the_workspace_database() {
    let f = fixture(3);
    let before = table_counts(&f.root);
    let bytes_before = std::fs::read(f.root.join("mathscape.sqlite3")).unwrap();
    get(&f, &unit("lemma"));
    get(&f, NOTE_ID);
    get(&f, MISSING_ID);
    hood(
        &f,
        json!({ "node_id": unit("sec"), "depth": 3, "edge_types": ["has_part", "contains_span", "located_in", "conversion", "proves", "cites"] }),
    );
    hood(&f, json!({ "seed_ids": [unit("lemma"), f.pdf] }));
    assert_eq!(table_counts(&f.root), before);
    assert_eq!(
        std::fs::read(f.root.join("mathscape.sqlite3")).unwrap(),
        bytes_before
    );
}

/// link_id, kind, key, target_id, target_json, role, primary, state, removed_at.
type LinkRow<'a> = (
    &'a str,
    &'a str,
    String,
    String,
    String,
    &'a str,
    i64,
    &'a str,
    Option<i64>,
);

const SNAPSHOT_ID: &str = "snap-0123456789abcdef";

/// Adds one unnamed snapshot of NOTE_ID (the JSON stand-in, label NULL) and
/// note links: lemma as study + check (two roles, one primary), a Markdown
/// range, a removed link and a link to a missing object.
fn add_ink(f: &Fixture) {
    let connection = Connection::open(f.root.join("mathscape.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE ink_documents SET pages_json = ?2 WHERE document_id = ?1",
            params![NOTE_ID, r#"[{"id":"p1"},{"id":"p2"},{"id":"p3"}]"#],
        )
        .unwrap();
    let manifest = json!({"document": {"pages": [{"id": "p"}, {"id": "q"}]}});
    connection
        .execute(
            "INSERT INTO ink_snapshots
             (snapshot_id, request_id, document_id, through_seq, artifact_id, content_hash,
              byte_size, manifest_json, created_at, label)
             VALUES (?1, 'req-1', ?2, 7, ?3, 'hash', 18, ?4, '2026-10-28T10:00:00+08:00', NULL)",
            params![
                SNAPSHOT_ID,
                NOTE_ID,
                f.snapshot_stand_in,
                manifest.to_string()
            ],
        )
        .unwrap();
    let markdown_target = json!({
        "kind": "markdown", "artifact_id": f.markdown, "content_hash": "h",
        "first_block": "b1", "last_block": "b2", "block_ids": ["b1", "b2"],
        "start": 16, "end": 58, "block_hash": "bh", "excerpt": "Every bounded", "label": "引理段落"
    });
    let lemma = unit("lemma");
    let ghost = unit("ghost");
    let object = |id: &str| json!({"kind": "source_object", "object_id": id}).to_string();
    let rows: Vec<LinkRow> = vec![
        (
            "link-study",
            "source_object",
            format!("src:{lemma}"),
            lemma.clone(),
            object(&lemma),
            "study",
            1,
            "active",
            None,
        ),
        (
            "link-check",
            "source_object",
            format!("src:{lemma}"),
            lemma.clone(),
            object(&lemma),
            "check",
            0,
            "active",
            None,
        ),
        (
            "link-md",
            "markdown",
            format!("md:{}:b1:b2", f.markdown),
            f.markdown.clone(),
            markdown_target.to_string(),
            "reference",
            0,
            "active",
            None,
        ),
        (
            "link-removed",
            "source_object",
            format!("src:{}", unit("proof")),
            unit("proof"),
            object(&unit("proof")),
            "attempt",
            0,
            "removed",
            Some(5),
        ),
        (
            "link-ghost",
            "source_object",
            format!("src:{ghost}"),
            ghost.clone(),
            object(&ghost),
            "question",
            0,
            "active",
            None,
        ),
    ];
    for (link_id, kind, key, target_id, target_json, role, primary, state, removed_at) in rows {
        connection
            .execute(
                "INSERT INTO ink_note_links
                 (link_id, document_id, target_kind, target_key, target_id, target_json,
                  target_revision, target_label, role, is_primary, state, created_at, removed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, 1, ?11)",
                params![
                    link_id,
                    NOTE_ID,
                    kind,
                    key,
                    target_id,
                    target_json,
                    format!("{link_id} label"),
                    role,
                    primary,
                    state,
                    removed_at
                ],
            )
            .unwrap();
    }
}

fn edges_of<'a>(result: &'a Value, relation: &str) -> Vec<&'a Value> {
    result["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["relation"] == relation)
        .collect()
}

#[test]
fn unnamed_snapshot_is_a_snapshot_with_ink_page() {
    let f = fixture(0);
    add_ink(&f);
    let result = get(&f, &f.snapshot_stand_in);
    assert_eq!(result["status"], "ok", "{result}");
    let node = &result["node"];
    assert_eq!(node["kind"], "snapshot", "{result}");
    assert_eq!(node["snapshot_id"], SNAPSHOT_ID);
    assert_eq!(node["document_id"], NOTE_ID);
    let content_ref = &node["content_ref"];
    assert_eq!(
        *content_ref,
        json!({
            "type": "ink_page", "tool": "ink.page_image", "node_id": SNAPSHOT_ID,
            "page_id": "p", "page_count": 2, "media_type": "image/png", "via": null
        })
    );
    assert!(!codes(&result).contains(&"content_unavailable".to_owned()));
    assert_eq!(
        node["edge_summary"]["has_snapshot"],
        json!({"in": 1, "out": 0})
    );

    let note = get(&f, NOTE_ID);
    assert_eq!(note["node"]["content_ref"]["page_id"], "p1");
    assert_eq!(note["node"]["content_ref"]["page_count"], 3);

    // material.read refuses the snapshot JSON but points at ink.page_image.
    let (read, is_error) = call(
        &f.server,
        "material.read",
        json!({ "artifact_id": f.snapshot_stand_in, "locator": {"md_byte_range": {"start": 0, "end": 1}} }),
    );
    assert!(is_error, "{read}");
    assert_eq!(read["error"]["code"], "invalid_arg", "{read}");
    assert_eq!(
        read["error"]["details"]["hint"], "use ink.page_image",
        "{read}"
    );

    // The ink span picks up page_count from the snapshot row.
    let ink_span = get(&f, &span("s_ink"));
    assert_eq!(
        ink_span["node"]["content_ref"]["page_count"], 2,
        "{ink_span}"
    );
}

#[test]
fn has_snapshot_and_note_link_behave_under_heuristic_and_raw() {
    let f = fixture(0);
    add_ink(&f);
    // Default (`heuristic`) surfaces both the snapshot and linked note targets.
    let heuristic = hood(&f, json!({ "node_id": NOTE_ID }));
    assert_eq!(heuristic["status"], "ok", "{heuristic}");
    assert_no_dangling_edges(&heuristic);
    assert_eq!(edges_of(&heuristic, "has_snapshot").len(), 1);
    assert_eq!(edges_of(&heuristic, "note_link").len(), 3);
    assert!(node_ids(&heuristic).contains(&unit("lemma")));

    // `raw` keeps `note_link` opt-in while including `has_snapshot`.
    let result = hood(&f, json!({ "node_id": NOTE_ID, "preset": "raw" }));
    assert_eq!(result["status"], "ok", "{result}");
    assert_no_dangling_edges(&result);
    let snapshots = edges_of(&result, "has_snapshot");
    assert_eq!(snapshots.len(), 1, "{result}");
    let edge = snapshots[0];
    assert_eq!(edge["from"], NOTE_ID);
    assert_eq!(edge["to"], f.snapshot_stand_in);
    assert_eq!(edge["synthetic"], true);
    assert_eq!(edge["label"], "snapshot @7");
    assert_eq!(edge["attrs"], json!({"through_seq": 7, "label": null}));
    assert_eq!(
        edge["endpoints"],
        json!([
            {"node_id": NOTE_ID, "role": "note", "side": "from", "ordinal": null},
            {"node_id": f.snapshot_stand_in, "role": "snapshot", "side": "to", "ordinal": null}
        ])
    );
    assert!(
        edges_of(&result, "note_link").is_empty(),
        "note_link is opt-in in raw preset"
    );
    assert!(!node_ids(&result).contains(&unit("lemma")));
    // The edge is navigable from the snapshot side as well, and get resolves it.
    let back = hood(&f, json!({ "node_id": f.snapshot_stand_in }));
    assert!(node_ids(&back).contains(&NOTE_ID.to_owned()), "{back}");
    let id = edge["id"].as_str().unwrap();
    let relation = get(&f, id);
    assert_eq!(relation["node"]["kind"], "relation", "{relation}");
    assert_eq!(relation["node"]["relation"], "has_snapshot");
}

#[test]
fn note_link_is_opt_in_and_counted_in_edge_summary() {
    let f = fixture(0);
    add_ink(&f);
    let note = get(&f, NOTE_ID);
    let summary = &note["node"]["edge_summary"];
    assert_eq!(summary["note_link"], json!({"in": 0, "out": 3}), "{note}");
    assert_eq!(summary["has_snapshot"], json!({"in": 0, "out": 1}));
    let lemma = get(&f, &unit("lemma"));
    assert_eq!(
        lemma["node"]["edge_summary"]["note_link"],
        json!({"in": 2, "out": 0}),
        "{lemma}"
    );
    // The removed link and the ghost target never become edges.
    let skipped = diag(&note, "dangling_endpoint_skipped");
    assert!(skipped.to_string().contains("link-ghost"), "{skipped}");
    assert!(!skipped.to_string().contains("link-removed"), "{skipped}");

    let result = hood(
        &f,
        json!({ "node_id": NOTE_ID, "edge_types": ["note_link"] }),
    );
    assert_eq!(result["status"], "ok", "{result}");
    assert_no_dangling_edges(&result);
    let links = edges_of(&result, "note_link");
    assert_eq!(links.len(), 3, "{result}");
    let lemma_roles: Vec<&Value> = links
        .iter()
        .filter(|e| e["to"] == unit("lemma"))
        .map(|e| &e["attrs"]["role"])
        .collect();
    assert_eq!(lemma_roles.len(), 2, "one edge per role: {result}");
    assert!(lemma_roles.contains(&&json!("study")) && lemma_roles.contains(&&json!("check")));
    let study = links
        .iter()
        .find(|e| e["attrs"]["role"] == "study")
        .unwrap();
    assert_eq!(study["attrs"]["is_primary"], true);
    assert_eq!(study["attrs"]["target_kind"], "source_object");
    assert_eq!(study["attrs"]["target_status"], "active");
    assert!(study["attrs"].get("locator").is_none());
    assert_eq!(study["endpoints"][1]["role"], "target");
    assert!(edges_of(&result, "has_snapshot").is_empty());
    let ids = node_ids(&result);
    assert!(
        ids.contains(&unit("lemma")) && ids.contains(&f.markdown),
        "{ids:?}"
    );
    assert!(!ids.contains(&unit("proof")), "removed link: {ids:?}");
}

#[test]
fn markdown_note_link_carries_md_byte_range() {
    let f = fixture(0);
    add_ink(&f);
    let result = hood(
        &f,
        json!({ "node_id": NOTE_ID, "edge_types": ["note_link"] }),
    );
    let edge = edges_of(&result, "note_link")
        .into_iter()
        .find(|e| e["attrs"]["target_kind"] == "markdown")
        .unwrap_or_else(|| panic!("no markdown link in {result}"));
    assert_eq!(edge["to"], f.markdown);
    assert_eq!(edge["attrs"]["role"], "reference");
    assert_eq!(
        edge["attrs"]["locator"],
        json!({"md_byte_range": {"start": 16, "end": 58}})
    );
    // The locator reads straight through material.read.
    let read = ok(
        &f.server,
        "material.read",
        json!({ "artifact_id": f.markdown, "locator": edge["attrs"]["locator"] }),
    );
    assert_eq!(
        read["text"], "Every bounded monotone sequence converges.",
        "{read}"
    );
}

#[test]
fn ink_span_stroke_ids_are_capped() {
    let f = fixture(0);
    let mut writer = MapWriter {
        connection: Connection::open(f.root.join("mathscape.sqlite3")).unwrap(),
        revision: 2000,
    };
    let strokes: Vec<String> = (0..60).map(|i| format!("st{i:02}")).collect();
    writer.span(
        &span("s_many"),
        json!({"ink": {"snapshot_artifact": f.snapshot_stand_in, "page_id": "p", "stroke_ids": strokes, "selection_mode": "lasso"}}),
    );
    drop(writer);
    let result = get(&f, &span("s_many"));
    let content_ref = &result["node"]["content_ref"];
    assert_eq!(content_ref["type"], "ink_page", "{result}");
    assert_eq!(content_ref["stroke_ids"].as_array().unwrap().len(), 50);
    assert_eq!(content_ref["stroke_ids"][49], "st49");
    assert_eq!(content_ref["stroke_ids_truncated"], true);
}
