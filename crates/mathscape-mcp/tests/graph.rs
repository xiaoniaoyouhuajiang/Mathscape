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
    assert!(inked["node"]["content_ref"].is_null());
    assert_eq!(
        diag(&inked, "content_unavailable")["details"]["reason"],
        "ink_out_of_scope"
    );
    assert_eq!(inked["node"]["parent_artifact_id"], f.snapshot_stand_in);
}

#[test]
fn note_has_no_content_ref() {
    let f = fixture(0);
    let result = get(&f, NOTE_ID);
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["node"]["kind"], "note");
    assert!(result["node"]["content_ref"].is_null());
    assert_eq!(
        diag(&result, "content_unavailable")["details"]["reason"],
        "ink_out_of_scope"
    );
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
fn default_edge_types_exclude_semantic_relations() {
    let f = fixture(0);
    let default = hood(&f, json!({ "node_id": unit("lemma") }));
    assert_eq!(default["status"], "ok", "{default}");
    assert_eq!(
        default["applied"]["edge_types"],
        json!(["conversion", "located_in", "contains_span", "has_part"])
    );
    assert_eq!(default["applied"]["direction"], "both");
    assert_eq!(default["applied"]["depth"], 1);
    let relations = edge_relations(&default);
    assert!(
        !relations.iter().any(|r| r == "proves" || r == "cites"),
        "{relations:?}"
    );
    assert!(codes(&default).contains(&"edge_types_filtered".to_owned()));
    assert_no_dangling_edges(&default);
    let ids = node_ids(&default);
    assert_eq!(ids[0], unit("lemma"));
    assert!(
        ids.contains(&unit("sec")) && ids.contains(&span("s_body")),
        "{ids:?}"
    );
    assert!(
        !ids.contains(&unit("proof")),
        "proof only reached via has_part at depth 2"
    );

    let semantic = hood(
        &f,
        json!({ "node_id": unit("lemma"), "edge_types": ["proves", "cites"] }),
    );
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
fn direction_follows_from_and_to_sides() {
    let f = fixture(0);
    let out = hood(
        &f,
        json!({ "node_id": unit("lemma"), "direction": "out", "depth": 2 }),
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
        json!({ "node_id": f.markdown, "direction": "in", "depth": 2, "node_kinds": ["source_unit"] }),
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
        json!({ "node_id": unit("sec"), "direction": "out", "depth": 1 }),
    );
    let ids = node_ids(&parent);
    assert!(
        ids.contains(&unit("lemma")) && ids.contains(&unit("bare")),
        "{ids:?}"
    );

    let conversion = hood(&f, json!({ "node_id": f.pdf, "direction": "out" }));
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
