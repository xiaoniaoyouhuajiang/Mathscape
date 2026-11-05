//! Canonical markdown ensure behind `material.read`, end to end through the
//! MCP server with a fake `marker_single` (no network, no models).

#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lopdf::{Document, Object, Stream, content::Content, dictionary};
use mathscape_agent::{CommandOutput, CommandRunner, SystemCommandRunner};
use mathscape_artifact::{ArtifactMetadata, ArtifactPool};
use mathscape_domain::{
    ActorRef, InclusivePageRange, MediaType, RightsRecord, RightsStatus, StableId,
};
use mathscape_mcp::{EnsurePolicy, McpServer, McpWorkspace};
use mathscape_store_sqlite::{
    CanonicalEnsureClaim, CanonicalEnsureClaimOutcome, CanonicalEnsureLimits,
    CanonicalEnsureOutcome, SqliteArtifactStore, unix_millis_now,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const FAKE_MARKER: &str = r#"#!/bin/sh
here="$(dirname "$0")"
source_path="$1"
output_root=''
page_range=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        --page_range) page_range="$2"; shift 2 ;;
        *) shift ;;
    esac
done
echo "$page_range" >> "$here/calls.log"
if [ -f "$here/slow" ]; then sleep "$(cat "$here/slow")"; fi
if [ -f "$here/fail" ]; then echo 'fake marker failure' >&2; exit 3; fi
source_name="${source_path##*/}"
source_stem="${source_name%.*}"
mkdir -p "$output_root/$source_stem"
printf '{0}----------------\n\n# 扫描页 %s\n' "$page_range" > "$output_root/$source_stem/$source_stem.md"
printf '{"page_stats":[{"page_id":0}]}\n' > "$output_root/$source_stem/${source_stem}_meta.json"
"#;

/// Marker env seam: `MATHSCAPE_MARKER_EXECUTABLE` points at the fake; real
/// processes run (the fake python prints a supported version).
struct FakeMarkerEnv {
    executable: Option<PathBuf>,
    inner: SystemCommandRunner,
}

impl CommandRunner for FakeMarkerEnv {
    fn run(&self, program: &Path, args: &[&str]) -> io::Result<CommandOutput> {
        if self.executable.is_none() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        self.inner.run(program, args)
    }
    fn env_var(&self, key: &str) -> Option<OsString> {
        match key {
            "MATHSCAPE_MARKER_EXECUTABLE" => self.executable.clone().map(Into::into),
            _ => None,
        }
    }
    fn is_file(&self, path: &Path) -> bool {
        self.executable.is_some() && path.is_file()
    }
}

struct Harness {
    _dir: TempDir,
    root: PathBuf,
    bin: PathBuf,
    pdf: String,
    other_pdf: String,
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

/// `pages` empty pages: no text layer anywhere.
fn scanned_pdf(pages: usize) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for _ in 0..pages {
        let content = Content { operations: vec![] };
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
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

/// One line of text per page: every page has a native text layer.
fn text_pdf(lines: &[&str]) -> Vec<u8> {
    use lopdf::content::Operation;
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
    for line in lines {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 18.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(*line)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id =
            document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
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
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn import_pdf(pool: &mut ArtifactPool, bytes: Vec<u8>, name: &str, pages: u32) -> String {
    let metadata = ArtifactMetadata::new(
        None,
        MediaType::parse("application/pdf").unwrap(),
        Some(name.to_owned()),
        Some(pages),
        RightsRecord::new(RightsStatus::NotAssessed, vec![], None).unwrap(),
    );
    pool.import_bytes(&bytes, metadata, ActorRef::parse("test").unwrap())
        .unwrap()
        .stable_id()
        .to_string()
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let mut pool = ArtifactPool::create(&root).unwrap();
    let pdf = import_pdf(&mut pool, scanned_pdf(20), "scan-book.pdf", 20);
    let other_pdf = import_pdf(&mut pool, scanned_pdf(3), "other-scan.pdf", 3);
    drop(pool);
    let bin = dir.path().join("marker-runtime").join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_executable(&bin.join("python"), "#!/bin/sh\nprintf '2.1.0'\n");
    write_executable(&bin.join("marker_single"), FAKE_MARKER);
    Harness {
        _dir: dir,
        root,
        bin,
        pdf,
        other_pdf,
    }
}

fn test_policy() -> EnsurePolicy {
    EnsurePolicy {
        sync_timeout: Duration::from_secs(30),
        marker_timeout: Duration::from_secs(20),
        // Short lease + fast heartbeat: a parse outlives several TTLs.
        lease_ttl: Duration::from_millis(800),
        heartbeat: Duration::from_millis(150),
        poll_interval: Duration::from_millis(50),
        ..EnsurePolicy::default()
    }
}

fn server_at(root: &Path, executable: Option<PathBuf>, policy: EnsurePolicy) -> McpServer {
    McpServer::new(
        McpWorkspace::open(root)
            .unwrap()
            .with_capability_runner(Box::new(FakeMarkerEnv {
                executable,
                inner: SystemCommandRunner::default(),
            }))
            .with_ensure_policy(policy),
    )
}

fn pdf_summary(h: &Harness) -> String {
    let landscape = h.store().landscape_snapshot().unwrap();
    let projection = mathscape_query::project_landscape(
        mathscape_query::LandscapeProjectionInput::from_snapshot(&landscape),
    )
    .unwrap();
    projection
        .nodes()
        .iter()
        .find(|node| node.entity_id().as_str() == h.pdf)
        .expect("pdf node in the landscape")
        .summary_ref()
        .unwrap_or("")
        .to_owned()
}

impl Harness {
    fn server(&self, policy: EnsurePolicy) -> McpServer {
        self.server_with(Some(self.bin.join("marker_single")), policy)
    }

    fn server_with(&self, executable: Option<PathBuf>, policy: EnsurePolicy) -> McpServer {
        server_at(&self.root, executable, policy)
    }

    fn no_live_lease(&self) -> bool {
        self.store()
            .active_canonical_ensures(unix_millis_now())
            .unwrap()
            .is_empty()
    }

    fn marker_calls(&self) -> Vec<String> {
        fs::read_to_string(self.bin.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn store(&self) -> SqliteArtifactStore {
        let pool = ArtifactPool::open(&self.root).unwrap();
        SqliteArtifactStore::open(&pool.workspace().database_path()).unwrap()
    }

    fn object_bytes(&self, id: &str) -> Vec<u8> {
        let pool = ArtifactPool::open(&self.root).unwrap();
        let path = pool.managed_path(&StableId::parse(id).unwrap()).unwrap();
        fs::read(path).unwrap()
    }
}

fn read(server: &McpServer, artifact_id: &str, from: u32, to: u32) -> Value {
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "material.read",
                "arguments": {
                    "artifact_id": artifact_id,
                    "locator": { "pdf_pages": { "from": from, "to": to } },
                },
            },
        }))
        .unwrap();
    let result = &response["result"];
    assert_eq!(result["isError"], false, "{response}");
    result["structuredContent"].clone()
}

fn codes(result: &Value) -> Vec<String> {
    result["diagnostics"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item["code"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn uncovered_scanned_read_ensures_then_serves_and_covered_reads_never_rerun() {
    let h = harness();
    let server = h.server(test_policy());
    let pdf_bytes = h.object_bytes(&h.pdf);

    let first = read(&server, &h.pdf, 3, 4);
    assert_eq!(first["status"], "ok", "{first}");
    assert_eq!(first["content_type"], "markdown");
    assert_eq!(
        first["locator"],
        json!({ "pdf_pages": { "from": 3, "to": 4 } })
    );
    assert!(
        first["text"].as_str().unwrap().contains("扫描页 2-3"),
        "{first}"
    );
    let ensure = &first["details"]["ensure"];
    assert_eq!(ensure["status"], "expanded", "{first}");
    assert_eq!(ensure["requested"], json!({ "from": 3, "to": 4 }));
    assert_eq!(ensure["missing"], json!([{ "from": 3, "to": 4 }]));
    assert_eq!(ensure["marker_range"], json!({ "from": 3, "to": 4 }));
    assert!(
        ensure["idempotency_key"]
            .as_str()
            .unwrap()
            .ends_with("|3-4|mathscape.marker.math-ocr.v1|marker-2.1.0|ensure-v1"),
        "{ensure}"
    );
    assert_eq!(
        first["canonical_id"],
        format!(
            "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-2.1.0/{}",
            h.pdf
        )
    );
    assert_eq!(first["served_artifact_id"], ensure["markdown_artifact_id"]);
    // Post-hoc scene record: this call blocked on parsing pages 3–4.
    assert_eq!(first["details"]["map_signal"]["label"], "解析中");
    assert_eq!(first["details"]["map_signal"]["phase"], "done");
    assert_eq!(
        first["details"]["map_signal"]["pages"],
        json!({ "from": 3, "to": 4 })
    );
    assert!(codes(&first).contains(&"canonical_ensure_expanded".to_owned()));
    assert_eq!(h.marker_calls(), ["2-3"]);

    // Receipt, registry head and released lease.
    let store = h.store();
    let receipt = store
        .canonical_ensure_receipt(ensure["ensure_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "succeeded");
    assert_eq!(
        receipt.markdown_artifact_id.as_deref(),
        ensure["markdown_artifact_id"].as_str()
    );
    let registry = store.list_canonical_markdown().unwrap();
    let view = registry
        .iter()
        .find(|view| view.source_pdf_artifact_id.as_str() == h.pdf)
        .expect("registry row for the ensured pdf");
    assert_eq!(
        view.head_artifact_id.as_ref().map(StableId::as_str),
        ensure["markdown_artifact_id"].as_str()
    );
    assert!(
        store
            .active_canonical_ensures(unix_millis_now())
            .unwrap()
            .is_empty()
    );

    // Covered: same range and a contained sub-range reuse, no rerun.
    for (from, to) in [(3, 4), (4, 4)] {
        let again = read(&server, &h.pdf, from, to);
        assert_eq!(again["status"], "ok", "{again}");
        assert!(codes(&again).contains(&"served_canonical_markdown".to_owned()));
        assert!(again["details"].get("ensure").is_none());
        assert_eq!(again["served_artifact_id"], first["served_artifact_id"]);
    }
    assert_eq!(
        h.marker_calls(),
        ["2-3"],
        "covered reads must not rerun Marker"
    );

    // Overlap-only coverage (3–4 vs request 4–5) is not served; the whole
    // request is parsed again as one new snapshot (no stitching).
    let overlap = read(&server, &h.pdf, 4, 5);
    assert_eq!(overlap["status"], "ok", "{overlap}");
    assert_eq!(overlap["details"]["ensure"]["status"], "expanded");
    assert_eq!(
        overlap["details"]["ensure"]["missing"],
        json!([{ "from": 5, "to": 5 }])
    );
    assert_eq!(
        overlap["details"]["ensure"]["marker_range"],
        json!({ "from": 4, "to": 5 })
    );
    assert_ne!(overlap["served_artifact_id"], first["served_artifact_id"]);
    assert_eq!(h.marker_calls(), ["2-3", "3-4"]);

    // The old snapshot is still served for its own range; PDF bytes untouched.
    let old = read(&server, &h.pdf, 3, 4);
    assert_eq!(old["status"], "ok");
    assert!(old["text"].as_str().unwrap().contains("扫描页 2-3"));
    assert_eq!(h.object_bytes(&h.pdf), pdf_bytes);
    let first_md = first["served_artifact_id"].as_str().unwrap();
    assert!(
        String::from_utf8(h.object_bytes(first_md))
            .unwrap()
            .contains("扫描页 2-3")
    );
}

#[test]
fn missing_marker_is_capability_unavailable_and_overlap_is_still_refused() {
    let h = harness();
    // Seed one snapshot for pages 2–3 with Marker present.
    let seeded = read(&h.server(test_policy()), &h.pdf, 2, 3);
    assert_eq!(seeded["status"], "ok", "{seeded}");

    let server = h.server_with(None, test_policy());
    let result = read(&server, &h.pdf, 1, 3);
    assert_eq!(result["status"], "capability_unavailable", "{result}");
    assert!(result.get("text").is_none());
    assert!(result.get("served_artifact_id").is_none());
    assert_eq!(result["details"]["state"], "missing");
    assert_eq!(
        result["details"]["ensure"]["status"],
        "capability_unavailable"
    );
    assert_eq!(
        result["details"]["ensure"]["missing"],
        json!([{ "from": 1, "to": 1 }])
    );
    assert_eq!(result["partial"], true);
    let codes = codes(&result);
    assert!(codes.contains(&"pdf_no_text_layer".to_owned()), "{codes:?}");
    assert!(codes.contains(&"canonical_ensure_capability_unavailable".to_owned()));
    assert_eq!(h.marker_calls(), ["1-2"]);
}

#[test]
fn oversized_requests_and_the_rolling_budget_block_without_running_marker() {
    let h = harness();
    let server = h.server(EnsurePolicy {
        max_pages_per_request: 3,
        window_pages: 4,
        ..test_policy()
    });
    let too_big = read(&server, &h.pdf, 1, 4);
    assert_eq!(too_big["status"], "capability_unavailable", "{too_big}");
    assert_eq!(too_big["details"]["ensure"]["status"], "blocked_budget");
    assert_eq!(too_big["details"]["ensure"]["max_pages_per_request"], 3);
    assert!(h.marker_calls().is_empty());

    assert_eq!(read(&server, &h.pdf, 1, 3)["status"], "ok");
    let over_window = read(&server, &h.pdf, 5, 6);
    assert_eq!(
        over_window["details"]["ensure"]["status"], "blocked_budget",
        "{over_window}"
    );
    assert!(codes(&over_window).contains(&"canonical_ensure_blocked_budget".to_owned()));
    assert_eq!(h.marker_calls(), ["0-2"]);
    // Already covered pages are served even with the budget spent.
    assert_eq!(read(&server, &h.pdf, 2, 3)["status"], "ok");
}

fn foreign_claim(pdf: &str, id: &str, from: u32, to: u32) -> CanonicalEnsureClaim {
    let pdf = StableId::parse(pdf).unwrap();
    CanonicalEnsureClaim {
        ensure_id: id.to_owned(),
        idempotency_key: format!("{id}-key"),
        canonical_id: StableId::parse(format!(
            "mathscape:canonical-markdown/v1/mathscape.marker.math-ocr.v1/marker-2.1.0/{pdf}"
        ))
        .unwrap(),
        source_pdf_artifact_id: pdf,
        parser_profile: "mathscape.marker.math-ocr.v1".into(),
        parser_policy_version: "marker-2.1.0".into(),
        pages: InclusivePageRange::new(from, to).unwrap(),
        owner: "another-process".into(),
    }
}

fn claim_foreign(h: &Harness, pdf: &str, id: &str, pages: (u32, u32), lease_ms: i64) {
    let limits = CanonicalEnsureLimits {
        max_running: 1,
        window_pages: 1000,
        window_ms: 3_600_000,
        lease_ms,
    };
    let outcome = h
        .store()
        .claim_canonical_ensure(
            &foreign_claim(pdf, id, pages.0, pages.1),
            limits,
            unix_millis_now(),
        )
        .unwrap();
    assert_eq!(outcome, CanonicalEnsureClaimOutcome::Acquired);
}

#[test]
fn a_read_queues_behind_a_live_lease_on_the_same_pdf_then_parses_in_the_same_call() {
    let h = harness();
    claim_foreign(&h, &h.pdf, "held", (7, 8), 600_000);
    // The other owner finishes (without covering pages 1–1) after a moment.
    let database = ArtifactPool::open(&h.root)
        .unwrap()
        .workspace()
        .database_path();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        SqliteArtifactStore::open(&database)
            .unwrap()
            .finish_canonical_ensure(
                "held",
                CanonicalEnsureOutcome::Failed,
                None,
                None,
                Some("other owner gave up"),
                unix_millis_now(),
            )
            .unwrap();
    });
    let server = h.server(test_policy());
    let started = Instant::now();
    let result = read(&server, &h.pdf, 1, 1);
    releaser.join().unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(600),
        "the read waited"
    );
    // Never blocked_lock / started: one call, one ok body.
    assert_eq!(result["status"], "ok", "{result}");
    let text = result["text"].as_str().unwrap();
    assert!(text.contains("扫描页 0"), "{text}");
    let ensure = &result["details"]["ensure"];
    assert_eq!(ensure["status"], "expanded", "{ensure}");
    assert_eq!(ensure["waited_for"]["ensure_id"], "held");
    assert_eq!(ensure["waited_for"]["pages"], json!({ "from": 7, "to": 8 }));
    assert!(ensure["waited_ms"].as_u64().unwrap() >= 600);
    assert_eq!(h.marker_calls(), ["0"]);
    assert!(h.no_live_lease());
}

#[test]
fn a_dead_owner_lease_expires_and_a_read_on_another_pdf_takes_the_slot() {
    let h = harness();
    // Another process claimed the only Marker slot and died (no heartbeat).
    claim_foreign(&h, &h.pdf, "dead", (7, 8), 600);
    let server = h.server(test_policy());
    let result = read(&server, &h.other_pdf, 1, 1);
    assert_eq!(result["status"], "ok", "{result}");
    let ensure = &result["details"]["ensure"];
    assert_eq!(ensure["status"], "expanded", "{ensure}");
    assert_eq!(ensure["waited_for"]["ensure_id"], "dead");
    // A slot wait on another PDF is not a parse of this PDF.
    assert_eq!(
        result["details"]["map_signal"]["pages"],
        json!({ "from": 1, "to": 1 })
    );
    assert_eq!(
        h.store()
            .canonical_ensure_receipt("dead")
            .unwrap()
            .unwrap()
            .status,
        "abandoned"
    );
    assert!(h.no_live_lease());
}

#[test]
fn queueing_past_the_sync_deadline_fails_without_leaving_a_lease() {
    let h = harness();
    claim_foreign(&h, &h.pdf, "held", (7, 8), 600_000);
    let server = h.server(EnsurePolicy {
        sync_timeout: Duration::from_millis(600),
        ..test_policy()
    });
    let result = read(&server, &h.pdf, 1, 1);
    assert_eq!(result["status"], "capability_unavailable", "{result}");
    let ensure = &result["details"]["ensure"];
    assert_eq!(ensure["status"], "failed", "{ensure}");
    assert_eq!(ensure["timed_out"], true);
    assert!(ensure.get("ensure_id").is_none(), "nothing was claimed");
    assert!(codes(&result).contains(&"canonical_ensure_failed".to_owned()));
    assert_eq!(result["details"]["map_signal"]["phase"], "failed");
    assert!(h.marker_calls().is_empty());
    // Only the other owner's lease is live; this read left none behind.
    let live = h
        .store()
        .active_canonical_ensures(unix_millis_now())
        .unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].ensure_id, "held");
}

#[test]
fn concurrent_reads_of_one_pdf_share_one_parse_and_the_map_says_parsing_meanwhile() {
    let h = harness();
    fs::write(h.bin.join("slow"), b"2").unwrap();
    let spawn_read = |delay: Duration| {
        let root = h.root.clone();
        let exe = h.bin.join("marker_single");
        let pdf = h.pdf.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let server = server_at(&root, Some(exe), test_policy());
            read(&server, &pdf, 9, 10)
        })
    };
    let first = spawn_read(Duration::ZERO);
    // While Marker runs (and heartbeats keep an 800 ms lease alive for 2 s),
    // the landscape PDF node says 「解析中」.
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.no_live_lease() {
        assert!(Instant::now() < deadline, "ensure never claimed");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        pdf_summary(&h).contains("解析中 第9–10页"),
        "{}",
        pdf_summary(&h)
    );
    let second = spawn_read(Duration::ZERO);
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        pdf_summary(&h).contains("解析中 第9–10页"),
        "the heartbeat keeps the signal past the lease TTL"
    );
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    for result in [&first, &second] {
        assert_eq!(result["status"], "ok", "{result}");
        assert!(result["text"].as_str().unwrap().contains("扫描页 8-9"));
        assert_eq!(result["details"]["map_signal"]["phase"], "done");
    }
    assert_eq!(first["details"]["ensure"]["status"], "expanded");
    // The second read queued on the same lease and reused its snapshot.
    let waited = &second["details"]["ensure"];
    assert_eq!(waited["status"], "reused", "{waited}");
    assert_eq!(
        waited["waited_for"]["ensure_id"],
        first["details"]["ensure"]["ensure_id"]
    );
    assert!(codes(&second).contains(&"canonical_ensure_reused".to_owned()));
    assert_eq!(second["served_artifact_id"], first["served_artifact_id"]);
    assert_eq!(h.marker_calls(), ["8-9"], "one Marker run for both reads");
    assert!(h.no_live_lease());
    assert!(!pdf_summary(&h).contains("解析中"), "the signal clears");
}

#[test]
fn marker_past_the_sync_deadline_is_stopped_failed_and_unlocked() {
    let h = harness();
    fs::write(h.bin.join("slow"), b"20").unwrap();
    let server = h.server(EnsurePolicy {
        sync_timeout: Duration::from_secs(1),
        ..test_policy()
    });
    let started = Instant::now();
    let result = read(&server, &h.pdf, 1, 2);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the call is bounded: {:?}",
        started.elapsed()
    );
    assert_eq!(result["status"], "capability_unavailable", "{result}");
    let ensure = &result["details"]["ensure"];
    assert_eq!(ensure["status"], "failed", "{ensure}");
    assert_eq!(ensure["timed_out"], true, "{ensure}");
    let receipt = h
        .store()
        .canonical_ensure_receipt(ensure["ensure_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "failed");
    assert!(h.no_live_lease(), "no stuck running lease");
    assert!(!pdf_summary(&h).contains("解析中"));
}

#[test]
fn marker_failure_is_reported_and_recorded_without_new_artifacts() {
    let h = harness();
    fs::write(h.bin.join("fail"), b"").unwrap();
    let server = h.server(test_policy());
    let before = h.store().read_catalog_snapshot().unwrap().artifacts().len();
    let failed = read(&server, &h.pdf, 1, 2);
    assert_eq!(failed["status"], "capability_unavailable", "{failed}");
    let ensure = &failed["details"]["ensure"];
    assert_eq!(ensure["status"], "failed");
    assert!(ensure.get("timed_out").is_none());
    assert_eq!(failed["details"]["map_signal"]["phase"], "failed");
    assert!(codes(&failed).contains(&"canonical_ensure_failed".to_owned()));
    let receipt = h
        .store()
        .canonical_ensure_receipt(ensure["ensure_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "failed");
    assert!(receipt.error.is_some());
    assert_eq!(
        h.store().read_catalog_snapshot().unwrap().artifacts().len(),
        before
    );
    assert!(
        h.store()
            .active_canonical_ensures(unix_millis_now())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn search_never_triggers_ensure() {
    let h = harness();
    let server = h.server(test_policy());
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "workspace.search", "arguments": { "query": "scan-book" } },
        }))
        .unwrap();
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(h.marker_calls().is_empty(), "search must never run Marker");
}

fn grep(server: &McpServer, artifact_id: &str, pattern: &str) -> Value {
    let response = server
        .handle_message(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "material.grep",
                "arguments": { "artifact_ids": [artifact_id], "pattern": pattern },
            },
        }))
        .unwrap();
    let result = &response["result"];
    assert_eq!(result["isError"], false, "{response}");
    result["structuredContent"].clone()
}

#[test]
fn text_layer_pdf_read_serves_canonical_markdown_and_grep_never_runs_marker() {
    let h = harness();
    let mut pool = ArtifactPool::open(&h.root).unwrap();
    let text = import_pdf(
        &mut pool,
        text_pdf(&["Alpha text layer", "Bounded monotone text layer", "Omega"]),
        "text-book.pdf",
        3,
    );
    drop(pool);
    let server = h.server(test_policy());

    // Coarse locate on the text layer: page hit, no Marker run.
    let located = grep(&server, &text, "monotone");
    assert_eq!(located["status"], "ok", "{located}");
    let hit = &located["hits"][0];
    assert_eq!(hit["surface"], "pdf_text_layer");
    assert_eq!(
        hit["locator"],
        json!({ "pdf_pages": { "from": 2, "to": 2 } })
    );
    assert!(hit.get("canonical_id").is_none(), "{hit}");
    assert!(h.marker_calls().is_empty(), "grep must never run Marker");

    // Careful read of that page: canonical Marker markdown even though the
    // page has a text layer; never the raw extract.
    let page = read(&server, &text, 2, 2);
    assert_eq!(page["status"], "ok", "{page}");
    assert_eq!(page["content_type"], "markdown");
    let body = page["text"].as_str().unwrap();
    assert!(body.contains("扫描页 1"), "{body}");
    assert!(!body.contains("Bounded monotone"), "{body}");
    assert_eq!(page["details"]["ensure"]["status"], "expanded");
    assert!(page.get("page_spans").is_none());
    assert_eq!(h.marker_calls(), ["1"]);

    // A covered re-read reuses the snapshot.
    let again = read(&server, &text, 2, 2);
    assert!(codes(&again).contains(&"served_canonical_markdown".to_owned()));
    assert_eq!(h.marker_calls(), ["1"]);

    // Now that a canonical head exists, grep on the PDF id searches the head
    // markdown (canonical first), not the text layer: hits are md_byte_range
    // into the head snapshot, and the text-layer phrase no longer matches.
    let located = grep(&server, &text, "扫描页");
    assert_eq!(located["status"], "ok", "{located}");
    let hit = &located["hits"][0];
    assert!(hit.get("surface").is_none(), "{hit}");
    assert!(hit["locator"]["md_byte_range"].is_object(), "{hit}");
    assert_eq!(hit["artifact_id"], page["served_artifact_id"], "{hit}");
    assert!(hit["canonical_id"].is_string());
    assert_eq!(hit["partial"], true);
    assert_eq!(
        hit["coverage"]["covered"],
        json!([{ "from": 2, "to": 2 }]),
        "{located}"
    );
    assert!(codes(&located).contains(&"pdf_resolved_to_canonical".to_owned()));
    let quoted = grep(&server, &text, "monotone");
    assert_eq!(quoted["hits"], json!([]), "{quoted}");
    assert_eq!(h.marker_calls(), ["1"]);
}
