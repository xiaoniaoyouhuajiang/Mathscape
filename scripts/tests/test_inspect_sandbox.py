import importlib.util
import io
import json
import sqlite3
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


SCRIPT_PATH = Path(__file__).resolve().parent.parent / "inspect-sandbox.py"
spec = importlib.util.spec_from_file_location("inspect_sandbox", SCRIPT_PATH)
assert spec and spec.loader
inspect_sandbox = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inspect_sandbox)


class InspectSandboxTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.ws = Path(self.tmp.name) / "workspace"
        self.sdir = self.ws / ".mathscape" / "sandbox" / "sessions" / "sbx-test-001"
        (self.sdir / "workspace").mkdir(parents=True)

        # Managed markdown artifact for byte-range verification
        sha = "a" * 64
        self.artifact_id = f"mathscape:artifact/sha256/{sha}"
        md_bytes = "**定理 8** 线性循环数列的母函数。".encode("utf-8")
        obj_file = self.sdir / "workspace" / "objects" / "sha256" / sha[:2] / sha[2:4] / sha
        obj_file.parent.mkdir(parents=True)
        obj_file.write_bytes(md_bytes)
        self.md_len = len(md_bytes)

        (self.sdir / "session.json").write_text(
            json.dumps(
                {
                    "sandbox_session_id": "sbx-test-001",
                    "state": "active",
                    "backend": "codex",
                    "active_thread_id": "d1",
                    "dialogue_threads": [
                        {
                            "thread_id": "d1",
                            "created_at": "2026-11-25T10:00:00+08:00",
                            "harness": {
                                "backend": "codex",
                                "tool_schema_version": "mathscape-mcp/21",
                                "skill_version": "0.7.5",
                                "last_turn_at": "2026-11-25T10:05:00+08:00",
                            },
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        (self.sdir / "transcript.jsonl").write_text(
            "\n".join(
                [
                    json.dumps({"ts": "2026-11-25T10:01:00+08:00", "role": "user", "text": "发起提案", "thread_id": "d1"}),
                    json.dumps(
                        {
                            "ts": "2026-11-25T10:05:00+08:00",
                            "role": "agent",
                            "text": "已暂存 v1",
                            "round_id": "round-1",
                            "outcome": "completed",
                            "thread_id": "d1",
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        (self.sdir / "events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "round_id": "round-1",
                            "seq": 1,
                            "type": "tool_call",
                            "payload": {
                                "call_id": "c1",
                                "server": "mathscape",
                                "tool": "propose_changes",
                                "args_summary": "target=e.thm",
                            },
                        }
                    ),
                    json.dumps(
                        {
                            "round_id": "round-1",
                            "seq": 2,
                            "type": "tool_result",
                            "payload": {
                                "call_id": "c1",
                                "duration_ms": 42,
                                "status": "error",
                                "raw": {"result": {"content": [{"type": "text", "text": '{"error":{"code":"invalid_arg"}}'}]}},
                            },
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        db_path = self.sdir / "workspace" / "mathscape.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.executescript(
            """
            CREATE TABLE proposed_change_sets (
                change_set_id TEXT PRIMARY KEY,
                investigation_id TEXT NOT NULL,
                active_version INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE proposed_change_set_versions (
                change_set_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                round_id TEXT NOT NULL,
                proposal_json TEXT NOT NULL,
                result_json TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE change_set_decisions (
                change_set_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                kind TEXT NOT NULL,
                decided_by TEXT NOT NULL,
                commit_id TEXT,
                reason TEXT,
                decided_at TEXT NOT NULL
            );
            CREATE TABLE map_objects (
                object_id TEXT PRIMARY KEY,
                object_shape TEXT NOT NULL,
                entity_kind TEXT NOT NULL,
                current_revision TEXT NOT NULL
            );
            CREATE TABLE map_object_revisions (
                object_id TEXT NOT NULL,
                revision_id TEXT NOT NULL,
                lifecycle TEXT NOT NULL,
                payload_json TEXT NOT NULL
            );
            CREATE TABLE artifact_locations (
                artifact_id TEXT NOT NULL,
                uri TEXT NOT NULL,
                priority INTEGER NOT NULL
            );
            """
        )
        conn.execute(
            "INSERT INTO artifact_locations VALUES (?, ?, 0)",
            (self.artifact_id, f"pool:sha256/{sha}"),
        )
        conn.execute(
            "INSERT INTO map_objects VALUES ('mathscape:semantic-entity/th1', 'node', 'SemanticEntity', 'sha256:rev1')"
        )
        conn.execute(
            "INSERT INTO map_object_revisions VALUES ('mathscape:semantic-entity/th1', 'sha256:rev1', 'active', ?)",
            (json.dumps({"payload": {"plain_label": "理论：母函数", "semantic_kind": "theory"}}),),
        )
        proposal = {
            "existing_refs": [{"id": "mathscape:semantic-entity/th1", "revision": "sha256:rev1"}],
            "target_object": "e.thm8",
            "rationale": "测试提案",
            "proposed_objects": [
                {
                    "local_ref": "s.thm8",
                    "kind": "source_span",
                    "op": "create",
                    "payload": {
                        "artifact_id": self.artifact_id,
                        "granularity": "paragraph",
                        "md_byte_range": {"start": 0, "end": self.md_len},
                        "quote": "**定理 8** 线性循环数列的母函数。",
                    },
                }
            ],
            "proposed_relations": [],
        }
        result = {
            "status": "staged",
            "diagnostics": [{"level": "info", "code": "mathlib_baseline_unchecked", "local_ref": "e.thm8", "message": "未核对"}],
            "compiled_operations": [{"operation": "create_source_span", "draft": {"page": {"pdf_page_index": 64}}}],
        }
        conn.execute(
            "INSERT INTO proposed_change_sets VALUES ('mathscape:change-set/cs123456', 'sandbox:sbx-test-001', 1, '2026-11-25', '2026-11-25')"
        )
        conn.execute(
            "INSERT INTO proposed_change_set_versions VALUES ('mathscape:change-set/cs123456', 1, 'round-1', ?, ?, 'staged', '2026-11-25')",
            (json.dumps(proposal, ensure_ascii=False), json.dumps(result, ensure_ascii=False)),
        )
        conn.commit()
        conn.close()
        self.db_path = db_path

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def run_cli(self, *args: str) -> str:
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = inspect_sandbox.main(["--workspace", str(self.ws), *args])
        self.assertEqual(rc, 0)
        return buf.getvalue()

    def test_list_thread_and_proposal_stay_strictly_readonly(self) -> None:
        before_files = sorted(p.name for p in self.db_path.parent.iterdir())

        list_json = json.loads(self.run_cli("list", "--json"))
        self.assertEqual(list_json[0]["sandbox_session_id"], "sbx-test-001")
        self.assertEqual(list_json[0]["recent_change_sets"][0]["state"], "staged")

        thread_out = self.run_cli("thread", "d1")
        self.assertIn("mathscape.propose_changes", thread_out)
        self.assertIn("invalid_arg", thread_out)

        prop_json = json.loads(self.run_cli("proposal", "cs123456", "--json"))
        ver = prop_json["inspected_versions"][0]
        self.assertEqual(ver["span_verification"][0]["status"], "exact_match")
        self.assertEqual(ver["existing_refs_resolved"][0]["label"], "理论：母函数")

        after_files = sorted(p.name for p in self.db_path.parent.iterdir())
        self.assertEqual(before_files, after_files)

    def test_closed_wal_database_leaves_no_sidecars_even_in_readonly_dir(self) -> None:
        conn = sqlite3.connect(self.db_path)
        self.assertEqual(conn.execute("PRAGMA journal_mode = WAL").fetchone()[0], "wal")
        conn.close()
        db_dir = self.db_path.parent
        before_files = sorted(p.name for p in db_dir.iterdir())
        self.assertNotIn("mathscape.sqlite3-wal", before_files)

        db_dir.chmod(0o555)
        try:
            prop_json = json.loads(self.run_cli("proposal", "cs123456", "--json"))
            self.assertEqual(prop_json["inspected_versions"][0]["span_verification"][0]["status"], "exact_match")
            self.run_cli("list")
        finally:
            db_dir.chmod(0o755)
        self.assertEqual(before_files, sorted(p.name for p in db_dir.iterdir()))

    def test_partially_written_transcript_line_is_skipped(self) -> None:
        with (self.sdir / "transcript.jsonl").open("a", encoding="utf-8") as f:
            f.write('{"thread_id": "d1", "role": "user", "te')
        self.assertIn("mathscape.propose_changes", self.run_cli("thread", "d1"))

    def test_span_pages_follow_staged_ids_not_order(self) -> None:
        span = lambda ref: {
            "local_ref": ref,
            "kind": "source_span",
            "op": "create",
            "payload": {"artifact_id": self.artifact_id, "md_byte_range": {"start": 0, "end": self.md_len}},
        }
        with inspect_sandbox.open_readonly_db(self.db_path) as conn:
            checks = inspect_sandbox.verify_source_spans(
                conn,
                self.sdir / "workspace",
                self.ws,
                [span("s.a"), span("s.b")],
                [
                    {"operation": "create_source_span", "object_id": "span/b", "draft": {"page": {"pdf_page_index": 2}}},
                    {"operation": "create_source_span", "object_id": "span/a", "draft": {"page": {"pdf_page_index": 1}}},
                ],
                {"s.a": "span/a", "s.b": "span/b"},
            )
        self.assertEqual([c["pdf_page_index"] for c in checks], [1, 2])


if __name__ == "__main__":
    unittest.main()
