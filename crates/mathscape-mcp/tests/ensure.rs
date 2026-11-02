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
    CanonicalEnsureClaim, CanonicalEnsureClaimOutcome, CanonicalEnsureLimits, SqliteArtifactStore,
    unix_millis_now,
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
if [ -f "$here/slow" ]; then sleep 2; fi
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
        wait: Duration::from_secs(30),
        marker_timeout: Duration::from_secs(20),
        ..EnsurePolicy::default()
    }
}

impl Harness {
    fn server(&self, policy: EnsurePolicy) -> McpServer {
        self.server_with(Some(self.bin.join("marker_single")), policy)
    }

    fn server_with(&self, executable: Option<PathBuf>, policy: EnsurePolicy) -> McpServer {
        McpServer::new(
            McpWorkspace::open(&self.root)
                .unwrap()
                .with_capability_runner(Box::new(FakeMarkerEnv {
                    executable,
                    inner: SystemCommandRunner::default(),
                }))
                .with_ensure_policy(policy),
        )
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

    fn wait_until_idle(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self
            .store()
            .active_canonical_ensures(unix_millis_now())
            .unwrap()
            .is_empty()
        {
            assert!(Instant::now() < deadline, "ensure never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
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
    assert!(first["details"].get("map_signal").is_none());
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

#[test]
fn live_leases_block_the_same_pdf_and_the_global_marker_slot() {
    let h = harness();
    let limits = CanonicalEnsureLimits {
        max_running: 1,
        window_pages: 1000,
        window_ms: 3_600_000,
        lease_ms: 600_000,
    };
    let outcome = h
        .store()
        .claim_canonical_ensure(
            &foreign_claim(&h.pdf, "held", 7, 8),
            limits,
            unix_millis_now(),
        )
        .unwrap();
    assert_eq!(outcome, CanonicalEnsureClaimOutcome::Acquired);
    let server = h.server(test_policy());

    let same_pdf = read(&server, &h.pdf, 1, 1);
    assert_eq!(same_pdf["status"], "capability_unavailable", "{same_pdf}");
    assert_eq!(same_pdf["details"]["ensure"]["status"], "blocked_lock");
    assert_eq!(
        same_pdf["details"]["ensure"]["lease"]["pages"],
        json!({ "from": 7, "to": 8 })
    );
    assert_eq!(
        same_pdf["details"]["map_signal"],
        json!({ "kind": "parsing", "label": "解析中", "node_id": h.pdf, "pages": { "from": 7, "to": 8 } })
    );

    let other_pdf = read(&server, &h.other_pdf, 1, 1);
    assert_eq!(
        other_pdf["details"]["ensure"]["status"], "blocked_lock",
        "{other_pdf}"
    );
    assert!(other_pdf["details"].get("map_signal").is_none());
    assert!(h.marker_calls().is_empty());
}

#[test]
fn slow_marker_reports_started_then_locks_then_serves() {
    let h = harness();
    fs::write(h.bin.join("slow"), b"").unwrap();
    let server = h.server(EnsurePolicy {
        wait: Duration::ZERO,
        ..test_policy()
    });
    let started = read(&server, &h.pdf, 9, 10);
    assert_eq!(started["status"], "capability_unavailable", "{started}");
    assert_eq!(started["details"]["ensure"]["status"], "started");
    assert_eq!(
        started["details"]["map_signal"],
        json!({ "kind": "parsing", "label": "解析中", "node_id": h.pdf, "pages": { "from": 9, "to": 10 } })
    );
    let leases = h
        .store()
        .active_canonical_ensures(unix_millis_now())
        .unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].pages, InclusivePageRange::new(9, 10).unwrap());
    let landscape = h.store().landscape_snapshot().unwrap();
    assert_eq!(landscape.canonical_ensures().len(), 1);
    // Map signal: the PDF node of the landscape projection says 「解析中」.
    let projection = mathscape_query::project_landscape(
        mathscape_query::LandscapeProjectionInput::from_snapshot(&landscape),
    )
    .unwrap();
    let pdf_node = projection
        .nodes()
        .iter()
        .find(|node| node.entity_id().as_str() == h.pdf)
        .expect("pdf node in the landscape");
    assert!(
        pdf_node.summary_ref().unwrap().contains("解析中 第9–10页"),
        "{:?}",
        pdf_node.summary_ref()
    );

    let locked = read(&server, &h.pdf, 9, 10);
    assert_eq!(
        locked["details"]["ensure"]["status"], "blocked_lock",
        "{locked}"
    );
    assert_eq!(locked["details"]["map_signal"]["label"], "解析中");

    h.wait_until_idle();
    let idle = h.store().landscape_snapshot().unwrap();
    let projection = mathscape_query::project_landscape(
        mathscape_query::LandscapeProjectionInput::from_snapshot(&idle),
    )
    .unwrap();
    assert!(
        projection
            .nodes()
            .iter()
            .all(|node| !node.summary_ref().unwrap_or("").contains("解析中")),
        "the signal clears when the lease is released"
    );
    let served = read(&server, &h.pdf, 9, 10);
    assert_eq!(served["status"], "ok", "{served}");
    assert!(codes(&served).contains(&"served_canonical_markdown".to_owned()));
    assert_eq!(h.marker_calls(), ["8-9"]);
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
