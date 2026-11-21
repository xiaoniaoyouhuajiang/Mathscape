//! End-to-end tests of Module 3 (Markdown Errata Overlay across MCP tools):
//! `material.read` (`md_byte_range`, single-snapshot `pdf_pages`, stitched `pdf_pages`),
//! `material.grep` (effective text search + physical `md_byte_range` back-mapping),
//! `material.locate` (effective text + raw-OCR fallback), and
//! `propose_changes` (R2 quote validation for full errata blocks, sub-passages inside
//! errata blocks, overly-wide anti-slop rejection, legacy raw-OCR compatibility, and
//! unmodified shifted passages).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
use mathscape_agent::{CommandOutput, CommandRunner};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, CommandRecordStatus, ConversionExitStatus, ConversionParameters, ConversionRun,
    EntityHeader, EntityKind, LifecycleStatus, MediaType, PageIndexBase, PageRange, RevisionId,
    RightsRecord, RightsStatus, SchemaVersion, StableId, Timestamp,
};
use mathscape_mcp::{McpServer, McpWorkspace};
use mathscape_orchestrator::{
    anchor_markdown, normalize_errata_corrected_text, resolve_errata_target_range,
};
use mathscape_store_sqlite::{MarkdownErrataSourceKind, SqliteArtifactStore, UpsertMarkdownErrata};
use serde_json::{Value, json};
use tempfile::TempDir;

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

fn pdf_bytes(pages: &[&[&str]]) -> Vec<u8> {
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
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn import_artifact(
    pool: &mut ArtifactPool,
    bytes: Vec<u8>,
    media_type: &str,
    filename: &str,
    page_count: Option<u32>,
) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(filename);
    std::fs::write(&path, &bytes).unwrap();
    let metadata = ArtifactMetadata::new(
        None,
        MediaType::parse(media_type).unwrap(),
        Some(filename.to_owned()),
        page_count,
        RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
    );
    pool.import_file(&path, metadata, ActorRef::parse("errata-mcp-test").unwrap())
        .unwrap()
        .stable_id()
        .to_string()
}

fn record_conversion(
    pool: &mut ArtifactPool,
    conversion_id: &str,
    revision_char: char,
    pdf_id: &str,
    md_id: &str,
    zero_from: u32,
    zero_to: u32,
    finished_at: &str,
) {
    let conversion = ConversionRun::new(
        "marker_single".to_owned(),
        Some("2.0.0".to_owned()),
        None,
        CommandRecordStatus::Recorded,
        ConversionParameters::new(
            Some(PageRange::new(zero_from, zero_to, PageIndexBase::Zero).unwrap()),
            "markdown".to_owned(),
            Some(true),
            BTreeMap::from([(
                "marker_profile".to_owned(),
                "mathscape.marker.math-ocr.v1".to_owned(),
            )]),
        )
        .unwrap(),
        None,
        Some(Timestamp::parse("2026-11-19T10:00:00+08:00").unwrap()),
        Some(Timestamp::parse(finished_at).unwrap()),
        ConversionExitStatus::Succeeded,
        vec![],
        None,
    )
    .unwrap()
    .into_record(
        EntityHeader::new(
            StableId::parse(conversion_id).unwrap(),
            RevisionId::parse(format!("sha256:{}", revision_char.to_string().repeat(64))).unwrap(),
            EntityKind::ConversionRun,
            SchemaVersion::parse("errata-mcp-test-v1").unwrap(),
            LifecycleStatus::Active,
            Timestamp::parse(finished_at).unwrap(),
            ActorRef::parse("errata-mcp-test").unwrap(),
            None,
        ),
        vec![StableId::parse(pdf_id).unwrap()],
        vec![StableId::parse(md_id).unwrap()],
    )
    .unwrap();
    pool.record_conversion(&conversion).unwrap();
}

fn call(server: &McpServer, tool: &str, arguments: Value) -> (Value, bool) {
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    })
    .to_string();
    let reply = server.handle_line(&line).expect("tools/call replies");
    assert!(reply.get("error").is_none(), "protocol error: {reply}");
    let result = &reply["result"];
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let structured = result["structuredContent"].clone();
    (structured, is_error)
}

fn diagnostic_codes(value: &Value) -> Vec<String> {
    value["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["code"].as_str().map(str::to_owned))
        .collect()
}

struct ErrataFixture {
    _dir: TempDir,
    root: PathBuf,
    server: McpServer,
    pdf: String,
    snap1: String,
    snap1_body: String,
    snap2: String,
    errata1_id: String,
    errata1_range: (u64, u64),
    errata2_id: String,
}

impl ErrataFixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let mut pool = ArtifactPool::create(&root).unwrap();
        let pdf = import_artifact(
            &mut pool,
            pdf_bytes(&[&["Page 1"], &["Page 2"], &["Page 3"]]),
            "application/pdf",
            "combinatorics.pdf",
            Some(3),
        );
        // Snapshot 1 covers PDF pages 1–2 (0-based 0..=1).
        let snap1_body = concat!(
            "{0}----------------\n\n",
            "组合数恒等式：$C_n^r + C_r^{n+1} = C_{n+1}^{r+1}$ (3)，是杨辉三角的基础性质。\n\n",
            "紧随其后的未校对段落：二项式系数满足对称性 $C_n^r = C_n^{n-r}$。\n\n",
            "{1}----------------\n\n",
            "第二页正文：由母函数展开可得范德蒙德恒等式。\n",
        )
        .to_owned();
        // Snapshot 2 covers PDF page 3 (0-based 2..=2).
        let snap2_body = concat!(
            "{2}----------------\n\n",
            "第三页定理：错误定理名称给出反演公式。\n",
        )
        .to_owned();

        let snap1 = import_artifact(
            &mut pool,
            snap1_body.as_bytes().to_vec(),
            "text/markdown",
            "snap-p1-2.md",
            None,
        );
        let snap2 = import_artifact(
            &mut pool,
            snap2_body.as_bytes().to_vec(),
            "text/markdown",
            "snap-p3.md",
            None,
        );

        record_conversion(
            &mut pool,
            "mathscape:conversion/marker/p1-2",
            'a',
            &pdf,
            &snap1,
            0,
            1,
            "2026-11-19T10:01:00+08:00",
        );
        record_conversion(
            &mut pool,
            "mathscape:conversion/marker/p3",
            'b',
            &pdf,
            &snap2,
            2,
            2,
            "2026-11-19T10:02:00+08:00",
        );
        drop(pool);

        let mut store = SqliteArtifactStore::open(&root.join("mathscape.sqlite3")).unwrap();
        store
            .sync_canonical_markdown_from_catalog(
                &Timestamp::parse("2026-11-19T10:03:00+08:00").unwrap(),
            )
            .unwrap();

        // Save a manual errata on Snapshot 1 fixing `C_r^{n+1}` -> `C_n^{r+1}` (and slightly lengthening the block).
        let doc1 =
            anchor_markdown(StableId::parse(&snap1).unwrap(), None, snap1_body.clone()).unwrap();
        let target1_block = doc1
            .blocks
            .iter()
            .find(|b| {
                snap1_body[b.range.start as usize..b.range.end as usize].contains("C_r^{n+1}")
            })
            .unwrap();
        let resolved1 = resolve_errata_target_range(
            &doc1,
            Some(doc1.snapshot.content_hash.as_str()),
            &target1_block.id,
            &target1_block.id,
            &[],
        )
        .unwrap();
        let corrected1 = normalize_errata_corrected_text(
            &resolved1.original_text,
            "组合数恒等式（已人工核对原书）：$C_n^r + C_n^{r+1} = C_{n+1}^{r+1}$ (3)，是杨辉三角的核心递推性质。",
        );
        let outcome1 = store
            .upsert_markdown_errata(UpsertMarkdownErrata {
                errata_id: "errata-snap1-pascal".to_owned(),
                artifact_id: snap1.clone(),
                base_content_hash: doc1.snapshot.content_hash.as_str().to_owned(),
                first_block_id: resolved1.first_block_id,
                last_block_id: resolved1.last_block_id,
                covered_block_ids: resolved1.covered_block_ids,
                base_byte_start: resolved1.base_byte_start,
                base_byte_end: resolved1.base_byte_end,
                base_text_hash: resolved1.base_text_hash.as_str().to_owned(),
                original_text: resolved1.original_text,
                corrected_text: corrected1,
                source_kind: MarkdownErrataSourceKind::Manual,
                actor: "user-test".to_owned(),
                note: Some("核对原书第 1 页公式 (3)".to_owned()),
                verified_pdf_page: Some(1),
                discovery_run_id: None,
                patch_set_id: None,
                now: "2026-11-19T10:05:00+08:00".to_owned(),
            })
            .unwrap();

        // Save a discovery_patch errata on Snapshot 2 fixing `错误定理名称` -> `二项式反演定理`.
        let doc2 =
            anchor_markdown(StableId::parse(&snap2).unwrap(), None, snap2_body.clone()).unwrap();
        let target2_block = doc2
            .blocks
            .iter()
            .find(|b| {
                snap2_body[b.range.start as usize..b.range.end as usize].contains("错误定理名称")
            })
            .unwrap();
        let resolved2 = resolve_errata_target_range(
            &doc2,
            Some(doc2.snapshot.content_hash.as_str()),
            &target2_block.id,
            &target2_block.id,
            &[],
        )
        .unwrap();
        let corrected2 = normalize_errata_corrected_text(
            &resolved2.original_text,
            "第三页定理：二项式反演定理给出反演公式。",
        );
        let outcome2 = store
            .upsert_markdown_errata(UpsertMarkdownErrata {
                errata_id: "errata-snap2-inversion".to_owned(),
                artifact_id: snap2.clone(),
                base_content_hash: doc2.snapshot.content_hash.as_str().to_owned(),
                first_block_id: resolved2.first_block_id,
                last_block_id: resolved2.last_block_id,
                covered_block_ids: resolved2.covered_block_ids,
                base_byte_start: resolved2.base_byte_start,
                base_byte_end: resolved2.base_byte_end,
                base_text_hash: resolved2.base_text_hash.as_str().to_owned(),
                original_text: resolved2.original_text,
                corrected_text: corrected2,
                source_kind: MarkdownErrataSourceKind::DiscoveryPatch,
                actor: "discovery-test".to_owned(),
                note: None,
                verified_pdf_page: Some(3),
                discovery_run_id: Some("run-1".to_owned()),
                patch_set_id: Some("ps-1".to_owned()),
                now: "2026-11-19T10:06:00+08:00".to_owned(),
            })
            .unwrap();
        drop(store);

        let server = McpServer::new(
            McpWorkspace::open(&root)
                .unwrap()
                .with_capability_runner(Box::new(NothingInstalled))
                .with_round(Some("round-errata".to_owned()))
                .with_investigation(Some("sandbox:errata-test".to_owned())),
        );

        Self {
            _dir: dir,
            root,
            server,
            pdf,
            snap1,
            snap1_body,
            snap2,
            errata1_id: outcome1.errata.errata_id,
            errata1_range: (resolved1.base_byte_start, resolved1.base_byte_end),
            errata2_id: outcome2.errata.errata_id,
        }
    }
}

#[test]
fn material_read_applies_errata_for_md_byte_range_single_pdf_snapshot_and_stitched_pdf_pages() {
    let f = ErrataFixture::new();

    // 1. Direct md_byte_range read covering the errata block.
    let (md_read, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": f.snap1,
            "locator": { "md_byte_range": { "start": 0, "end": f.snap1_body.len() } }
        }),
    );
    assert!(!is_error, "{md_read}");
    assert_eq!(md_read["status"], "ok", "{md_read}");
    let text = md_read["text"].as_str().unwrap();
    assert!(text.contains("C_n^{r+1}"), "{text}");
    assert!(!text.contains("C_r^{n+1}"), "{text}");
    assert!(diagnostic_codes(&md_read).contains(&"markdown_errata_applied".to_owned()));
    let applied = md_read["details"]["applied_errata"].as_array().unwrap();
    assert_eq!(applied.len(), 1, "{md_read}");
    assert_eq!(applied[0]["errata_id"], f.errata1_id);
    assert_eq!(applied[0]["source_kind"], "manual");
    assert_eq!(
        applied[0]["physical_byte_range"]["start"],
        f.errata1_range.0
    );
    assert_eq!(applied[0]["physical_byte_range"]["end"], f.errata1_range.1);

    // 2. Direct md_byte_range read strictly inside page 2 (no errata overlap).
    let page2_start = f.snap1_body.find("第二页正文").unwrap();
    let (clean_read, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": f.snap1,
            "locator": { "md_byte_range": { "start": page2_start, "end": f.snap1_body.len() } }
        }),
    );
    assert!(!is_error, "{clean_read}");
    assert!(
        clean_read["text"]
            .as_str()
            .unwrap()
            .contains("范德蒙德恒等式")
    );
    assert!(!diagnostic_codes(&clean_read).contains(&"markdown_errata_applied".to_owned()));
    assert!(clean_read["details"]["applied_errata"].is_null());

    // 3. Single covering PDF snapshot read (pages 1–2).
    let (single_pdf, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": f.pdf,
            "locator": { "pdf_pages": { "from": 1, "to": 2 } }
        }),
    );
    assert!(!is_error, "{single_pdf}");
    assert_eq!(single_pdf["status"], "ok", "{single_pdf}");
    let single_text = single_pdf["text"].as_str().unwrap();
    assert!(single_text.contains("C_n^{r+1}"), "{single_text}");
    assert!(!single_text.contains("C_r^{n+1}"), "{single_text}");
    assert!(diagnostic_codes(&single_pdf).contains(&"markdown_errata_applied".to_owned()));
    assert_eq!(
        single_pdf["details"]["applied_errata"][0]["errata_id"],
        f.errata1_id
    );

    // 4. Stitched PDF read across Snapshot 1 (pages 1–2) + Snapshot 2 (page 3).
    let (stitched, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": f.pdf,
            "locator": { "pdf_pages": { "from": 1, "to": 3 } }
        }),
    );
    assert!(!is_error, "{stitched}");
    assert_eq!(stitched["status"], "ok", "{stitched}");
    let stitched_text = stitched["text"].as_str().unwrap();
    assert!(stitched_text.contains("C_n^{r+1}"), "{stitched_text}");
    assert!(stitched_text.contains("二项式反演定理"), "{stitched_text}");
    assert!(!stitched_text.contains("错误定理名称"), "{stitched_text}");
    assert!(diagnostic_codes(&stitched).contains(&"markdown_errata_applied".to_owned()));
    let stitched_applied = stitched["details"]["applied_errata"].as_array().unwrap();
    assert_eq!(stitched_applied.len(), 2, "{stitched}");
    assert_eq!(stitched_applied[0]["errata_id"], f.errata1_id);
    assert_eq!(stitched_applied[0]["source_kind"], "manual");
    assert_eq!(stitched_applied[1]["errata_id"], f.errata2_id);
    assert_eq!(stitched_applied[1]["source_kind"], "discovery_patch");

    // 5. Reverting Snapshot 1 errata immediately restores raw OCR text on material.read.
    let mut store = SqliteArtifactStore::open(&f.root.join("mathscape.sqlite3")).unwrap();
    store
        .revert_markdown_errata(&f.errata1_id, "2026-11-19T10:10:00+08:00")
        .unwrap();
    drop(store);

    let (after_revert, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": f.snap1,
            "locator": { "md_byte_range": { "start": 0, "end": f.snap1_body.len() } }
        }),
    );
    assert!(!is_error, "{after_revert}");
    assert!(after_revert["text"].as_str().unwrap().contains("C_r^{n+1}"));
    assert!(!diagnostic_codes(&after_revert).contains(&"markdown_errata_applied".to_owned()));
}

#[test]
fn material_grep_searches_effective_text_and_maps_hits_back_to_readable_physical_ranges() {
    let f = ErrataFixture::new();

    // 1. Grep newly corrected symbol on Snapshot 1 directly.
    let (hit_new, is_error) = call(
        &f.server,
        "material.grep",
        json!({ "artifact_ids": [f.snap1], "pattern": "C_n^{r+1}" }),
    );
    assert!(!is_error, "{hit_new}");
    assert_eq!(hit_new["status"], "ok", "{hit_new}");
    let hits = hit_new["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{hit_new}");
    assert_eq!(
        hits[0]["locator"]["md_byte_range"]["start"],
        f.errata1_range.0
    );
    assert_eq!(
        hits[0]["locator"]["md_byte_range"]["end"],
        f.errata1_range.1
    );
    assert!(hits[0]["snippet"].as_str().unwrap().contains("C_n^{r+1}"));

    // Passing the hit's locator unchanged to material.read quotes the corrected text.
    let (follow_read, is_error) = call(
        &f.server,
        "material.read",
        json!({
            "artifact_id": hits[0]["artifact_id"],
            "locator": hits[0]["locator"],
        }),
    );
    assert!(!is_error, "{follow_read}");
    assert!(follow_read["text"].as_str().unwrap().contains("C_n^{r+1}"));

    // 2. Old OCR typo is no longer matched in effective text.
    let (hit_old, is_error) = call(
        &f.server,
        "material.grep",
        json!({ "artifact_ids": [f.snap1], "pattern": "C_r^{n+1}" }),
    );
    assert!(!is_error, "{hit_old}");
    assert!(hit_old["hits"].as_array().unwrap().is_empty(), "{hit_old}");

    // 3. Unmodified text AFTER a lengthened errata block still maps 1:1 to exact physical bytes.
    let (after_hit, is_error) = call(
        &f.server,
        "material.grep",
        json!({ "artifact_ids": [f.snap1], "pattern": "范德蒙德恒等式" }),
    );
    assert!(!is_error, "{after_hit}");
    let after_hits = after_hit["hits"].as_array().unwrap();
    assert_eq!(after_hits.len(), 1, "{after_hit}");
    let phys_start = after_hits[0]["locator"]["md_byte_range"]["start"]
        .as_u64()
        .unwrap() as usize;
    let phys_end = after_hits[0]["locator"]["md_byte_range"]["end"]
        .as_u64()
        .unwrap() as usize;
    assert_eq!(&f.snap1_body[phys_start..phys_end], "范德蒙德恒等式");

    // 4. Grepping the PDF id scans canonical snapshots through their effective text.
    let (pdf_grep, is_error) = call(
        &f.server,
        "material.grep",
        json!({ "artifact_ids": [f.pdf], "pattern": "二项式反演定理" }),
    );
    assert!(!is_error, "{pdf_grep}");
    let pdf_hits = pdf_grep["hits"].as_array().unwrap();
    assert_eq!(pdf_hits.len(), 1, "{pdf_grep}");
    assert_eq!(pdf_hits[0]["artifact_id"], f.snap2);
    assert!(
        pdf_hits[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("二项式反演定理")
    );
}

#[test]
fn material_locate_and_propose_changes_roundtrip_with_errata_and_r2_soundness() {
    let f = ErrataFixture::new();

    // A. Locate & propose the FULL corrected block.
    let full_quote = "组合数恒等式（已人工核对原书）：$C_n^r + C_n^{r+1} = C_{n+1}^{r+1}$ (3)，是杨辉三角的核心递推性质。";
    let (loc_full, is_error) = call(
        &f.server,
        "material.locate",
        json!({ "artifact_id": f.snap1, "quote": full_quote }),
    );
    assert!(!is_error, "{loc_full}");
    assert_eq!(loc_full["status"], "ok", "{loc_full}");
    assert_eq!(loc_full["md_byte_range"]["start"], f.errata1_range.0);
    assert_eq!(loc_full["md_byte_range"]["end"], f.errata1_range.1);
    assert_eq!(loc_full["page"]["page_number"], 1);
    assert_eq!(loc_full["page_granularity_available"], true);

    let (staged_full, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "req-errata-full",
            "rationale": "引用已人工校对的帕斯卡恒等式全块",
            "proposed_objects": [
                {
                    "local_ref": "s1",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": f.snap1,
                        "md_byte_range": loc_full["md_byte_range"],
                        "granularity": "page",
                        "quote": full_quote,
                    }
                },
                {
                    "local_ref": "u1",
                    "kind": "source_unit",
                    "op": "create",
                    "payload": {
                        "kind": "statement",
                        "display_title": "组合数递推公式 (3)",
                        "parent": null,
                        "source_spans": ["$new:s1"],
                        "completeness": "complete",
                    }
                }
            ]
        }),
    );
    assert!(!is_error, "{staged_full}");
    assert_eq!(staged_full["status"], "staged", "{staged_full}");

    // B. Locate & propose a SUB-PASSAGE strictly inside the corrected block.
    let sub_quote = "$C_n^r + C_n^{r+1} = C_{n+1}^{r+1}$";
    let (loc_sub, is_error) = call(
        &f.server,
        "material.locate",
        json!({ "artifact_id": f.snap1, "quote": sub_quote }),
    );
    assert!(!is_error, "{loc_sub}");
    assert_eq!(loc_sub["status"], "ok", "{loc_sub}");
    assert_eq!(loc_sub["md_byte_range"]["start"], f.errata1_range.0);
    assert_eq!(loc_sub["md_byte_range"]["end"], f.errata1_range.1);

    let (staged_sub, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "req-errata-sub",
            "rationale": "引用校对块中的核心公式短语",
            "proposed_objects": [
                {
                    "local_ref": "s2",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": f.snap1,
                        "md_byte_range": loc_sub["md_byte_range"],
                        "granularity": "page",
                        "quote": sub_quote,
                    }
                },
                {
                    "local_ref": "u2",
                    "kind": "source_unit",
                    "op": "create",
                    "payload": {
                        "kind": "statement",
                        "display_title": "公式 (3) 子引用",
                        "parent": null,
                        "source_spans": ["$new:s2"],
                        "completeness": "complete",
                    }
                }
            ]
        }),
    );
    assert!(!is_error, "{staged_sub}");
    assert_eq!(staged_sub["status"], "staged", "{staged_sub}");

    // C. Anti-slop R2 soundness: overly-wide range [0, len) with sub_quote MUST fail quote_mismatch.
    let (sloppy, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "req-errata-sloppy",
            "rationale": "过宽范围必须被 R2 拦截",
            "proposed_objects": [
                {
                    "local_ref": "s_bad",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": f.snap1,
                        "md_byte_range": { "start": 0, "end": f.snap1_body.len() },
                        "granularity": "inline",
                        "quote": sub_quote,
                    }
                },
                {
                    "local_ref": "u_bad",
                    "kind": "source_unit",
                    "op": "create",
                    "payload": {
                        "kind": "statement",
                        "display_title": "不应通过",
                        "parent": null,
                        "source_spans": ["$new:s_bad"],
                        "completeness": "complete",
                    }
                }
            ]
        }),
    );
    assert!(!is_error, "{sloppy}");
    assert_eq!(sloppy["status"], "blocked", "{sloppy}");
    assert!(
        diagnostic_codes(&sloppy).contains(&"quote_mismatch".to_owned()),
        "{sloppy}"
    );

    // D. Legacy raw-OCR fallback compatibility: old OCR quote resolves & passes R2.
    let legacy_quote = "$C_n^r + C_r^{n+1} = C_{n+1}^{r+1}$";
    let (loc_legacy, is_error) = call(
        &f.server,
        "material.locate",
        json!({ "artifact_id": f.snap1, "quote": legacy_quote }),
    );
    assert!(!is_error, "{loc_legacy}");
    assert_eq!(loc_legacy["status"], "ok", "{loc_legacy}");
    assert!(
        diagnostic_codes(&loc_legacy).contains(&"matched_raw_ocr_fallback".to_owned()),
        "{loc_legacy}"
    );
    let (staged_legacy, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "req-errata-legacy",
            "rationale": "兼容历史 OCR 引用",
            "proposed_objects": [
                {
                    "local_ref": "s3",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": f.snap1,
                        "md_byte_range": loc_legacy["md_byte_range"],
                        "granularity": "page",
                        "quote": legacy_quote,
                    }
                },
                {
                    "local_ref": "u3",
                    "kind": "source_unit",
                    "op": "create",
                    "payload": {
                        "kind": "statement",
                        "display_title": "历史引用兼容",
                        "parent": null,
                        "source_spans": ["$new:s3"],
                        "completeness": "complete",
                    }
                }
            ]
        }),
    );
    assert!(!is_error, "{staged_legacy}");
    assert_eq!(staged_legacy["status"], "staged", "{staged_legacy}");

    // E. Unmodified block AFTER the lengthened errata maps 1:1 to physical bytes and passes R2.
    let unmodified_quote = "第二页正文：由母函数展开可得范德蒙德恒等式。";
    let (loc_after, is_error) = call(
        &f.server,
        "material.locate",
        json!({ "artifact_id": f.snap1, "quote": unmodified_quote }),
    );
    assert!(!is_error, "{loc_after}");
    assert_eq!(loc_after["status"], "ok", "{loc_after}");
    assert_eq!(loc_after["page"]["page_number"], 2);
    let after_start = loc_after["md_byte_range"]["start"].as_u64().unwrap() as usize;
    let after_end = loc_after["md_byte_range"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&f.snap1_body[after_start..after_end], unmodified_quote);

    let (staged_after, is_error) = call(
        &f.server,
        "propose_changes",
        json!({
            "request_id": "req-errata-after",
            "rationale": "引用勘误块之后的未改动段落",
            "proposed_objects": [
                {
                    "local_ref": "s4",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": f.snap1,
                        "md_byte_range": loc_after["md_byte_range"],
                        "granularity": "page",
                        "quote": unmodified_quote,
                    }
                },
                {
                    "local_ref": "u4",
                    "kind": "source_unit",
                    "op": "create",
                    "payload": {
                        "kind": "statement",
                        "display_title": "范德蒙德恒等式",
                        "parent": null,
                        "source_spans": ["$new:s4"],
                        "completeness": "complete",
                    }
                }
            ]
        }),
    );
    assert!(!is_error, "{staged_after}");
    assert_eq!(staged_after["status"], "staged", "{staged_after}");
}
