#!/usr/bin/env python3
"""Read-only debugger for Mathscape sandbox sessions, threads, and proposals.

Inspects a sandbox session without mutating any file. A database whose app is
closed (no `-wal` file) is opened `immutable=1`, so no WAL/SHM sidecars appear;
a live one is opened `mode=ro` next to the sidecars the app already keeps.
Combines four local sources in one view:
  1. `session.json`    — threads, active thread, backend, skill & MCP schema version
  2. `harness.jsonl`   — per-round launch config (skill path, Lean profile root)
  3. `transcript.jsonl` + `events.jsonl` — user/agent turns & tool-call timeline
     (including failed/blocked `propose_changes` calls that never created a version)
  4. `workspace/mathscape.sqlite3` — `proposed_change_sets`, all versions (`v1..vN`),
     validator diagnostics/debts, decisions, resolved `existing_refs`, and byte-exact
     `source_span` quote checks against managed Markdown artifacts.

Examples:
  python3 scripts/inspect-sandbox.py list
  python3 scripts/inspect-sandbox.py thread d7
  python3 scripts/inspect-sandbox.py proposal latest
  python3 scripts/inspect-sandbox.py proposal a8cdc8ba --all-versions
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DEV_WORKSPACE = REPO_ROOT / ".mathscape-dev" / "workspace"


@contextmanager
def open_readonly_db(db_path: Path) -> Iterator[sqlite3.Connection]:
    if not db_path.is_file():
        raise FileNotFoundError(f"SQLite database not found: {db_path}")
    db_path = db_path.resolve()
    # WAL databases opened `mode=ro` create (and keep) -wal/-shm sidecars and fail in
    # read-only directories. Without a -wal file nobody is writing, so `immutable=1`
    # reads the file as-is.
    live = db_path.with_name(db_path.name + "-wal").exists()
    uri = f"{db_path.as_uri()}?{'mode=ro' if live else 'immutable=1'}"
    conn = sqlite3.connect(uri, uri=True)
    try:
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA query_only = ON")
        yield conn
    finally:
        conn.close()


def resolve_workspace_root(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit).expanduser().resolve()
    env_root = os.environ.get("MATHSCAPE_WORKSPACE_ROOT")
    if env_root:
        return Path(env_root).expanduser().resolve()
    return DEFAULT_DEV_WORKSPACE


def list_session_dirs(workspace_root: Path) -> list[Path]:
    sessions_dir = workspace_root / ".mathscape" / "sandbox" / "sessions"
    if not sessions_dir.is_dir():
        return []
    dirs = [p for p in sessions_dir.iterdir() if p.is_dir() and (p / "session.json").is_file()]
    dirs.sort(key=session_last_activity_key, reverse=True)
    return dirs


def session_last_activity_key(session_dir: Path) -> float:
    candidates = [
        session_dir / "transcript.jsonl",
        session_dir / "session.json",
        session_dir / "workspace" / "mathscape.sqlite3",
    ]
    return max((p.stat().st_mtime for p in candidates if p.exists()), default=0.0)


def resolve_session_dir(workspace_root: Path, session_selector: str | None, session_dir_arg: str | None) -> Path:
    if session_dir_arg:
        p = Path(session_dir_arg).expanduser().resolve()
        if not (p / "session.json").is_file():
            raise SystemExit(f"Not a sandbox session directory (missing session.json): {p}")
        return p
    dirs = list_session_dirs(workspace_root)
    if not dirs:
        raise SystemExit(f"No sandbox sessions found under {workspace_root / '.mathscape/sandbox/sessions'}")
    if not session_selector or session_selector == "latest":
        return dirs[0]
    exact = [d for d in dirs if d.name == session_selector]
    if len(exact) == 1:
        return exact[0]
    matches = [d for d in dirs if session_selector in d.name]
    if len(matches) == 1:
        return matches[0]
    if not matches:
        available = ", ".join(d.name for d in dirs)
        raise SystemExit(f"No sandbox session matching {session_selector!r} (available: {available})")
    ambiguous = ", ".join(d.name for d in matches)
    raise SystemExit(f"Ambiguous sandbox session prefix {session_selector!r}: {ambiguous}")


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    out: list[dict[str, Any]] = []
    with path.open(encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                # The app may be appending this line right now.
                continue
    return out


def read_events_for_rounds(events_path: Path, round_ids: set[str]) -> dict[str, list[dict[str, Any]]]:
    by_round: dict[str, list[dict[str, Any]]] = {rid: [] for rid in round_ids}
    if not events_path.is_file() or not round_ids:
        return by_round
    # Pre-filter raw lines by round_id substring so multi-ten-MB logs scan in milliseconds.
    with events_path.open(encoding="utf-8", errors="replace") as f:
        for line in f:
            if not any(rid in line for rid in round_ids):
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            rid = obj.get("round_id")
            if rid in by_round:
                by_round[rid].append(obj)
    for evs in by_round.values():
        evs.sort(key=lambda e: e.get("seq", 0))
    return by_round


def short_id(identifier: str | None, keep: int = 10) -> str:
    if not identifier:
        return "-"
    if "/" in identifier:
        prefix, tail = identifier.rsplit("/", 1)
        if len(tail) > keep + 4:
            return f"{prefix}/{tail[:keep]}…"
    if identifier.startswith("sha256:") and len(identifier) > 7 + keep:
        return f"sha256:{identifier[7:7 + keep]}…"
    return identifier


def extract_entity_label(payload_wrapper: dict[str, Any]) -> str:
    p = payload_wrapper.get("payload", payload_wrapper)
    if not isinstance(p, dict):
        return "-"
    if p.get("plain_label"):
        return str(p["plain_label"])
    if p.get("display_title"):
        return str(p["display_title"])
    if p.get("label"):
        return str(p["label"])
    if p.get("lean_name"):
        return str(p["lean_name"])
    labels = p.get("preferred_labels")
    if isinstance(labels, list) and labels:
        first = labels[0]
        if isinstance(first, dict) and first.get("label"):
            return str(first["label"])
    return "-"


def has_table(conn: sqlite3.Connection, table_name: str) -> bool:
    row = conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
        (table_name,),
    ).fetchone()
    return row is not None


def fetch_recent_change_sets(conn: sqlite3.Connection, limit: int = 10) -> list[dict[str, Any]]:
    if not has_table(conn, "proposed_change_sets"):
        return []
    has_decisions = has_table(conn, "change_set_decisions")
    decision_join = (
        """
        LEFT JOIN (
            SELECT change_set_id, version, kind, decided_at
            FROM change_set_decisions
            WHERE rowid IN (SELECT MAX(rowid) FROM change_set_decisions GROUP BY change_set_id, version)
        ) d
          ON cs.change_set_id = d.change_set_id AND cs.active_version = d.version
        """
        if has_decisions
        else ""
    )
    decision_cols = "d.kind AS decision_kind, d.decided_at" if has_decisions else "NULL AS decision_kind, NULL AS decided_at"
    rows = conn.execute(
        f"""
        SELECT cs.change_set_id, cs.investigation_id, cs.active_version, cs.created_at, cs.updated_at,
               v.round_id, v.state, v.proposal_json, v.result_json,
               {decision_cols}
        FROM proposed_change_sets cs
        LEFT JOIN proposed_change_set_versions v
          ON cs.change_set_id = v.change_set_id AND cs.active_version = v.version
        {decision_join}
        ORDER BY cs.rowid DESC
        LIMIT ?
        """,
        (limit,),
    ).fetchall()

    items: list[dict[str, Any]] = []
    for r in rows:
        prop = json.loads(r["proposal_json"]) if r["proposal_json"] else {}
        res = json.loads(r["result_json"]) if r["result_json"] else {}
        diags = res.get("diagnostics") or []
        items.append(
            {
                "change_set_id": r["change_set_id"],
                "active_version": r["active_version"],
                "round_id": r["round_id"],
                "state": r["decision_kind"] or r["state"] or res.get("status") or "unknown",
                "version_state": r["state"],
                "decision": r["decision_kind"],
                "target_object": prop.get("target_object"),
                "rationale": prop.get("rationale", ""),
                "object_count": len(prop.get("proposed_objects") or []),
                "relation_count": len(prop.get("proposed_relations") or []),
                "diagnostic_codes": [d.get("code") for d in diags if isinstance(d, dict)],
                "created_at": r["created_at"],
                "updated_at": r["updated_at"],
            }
        )
    return items


# ---------------------------------------------------------------------------
# Subcommand: list
# ---------------------------------------------------------------------------


def cmd_list(args: argparse.Namespace) -> int:
    workspace = resolve_workspace_root(args.workspace)
    dirs = list_session_dirs(workspace) if not args.session_dir else [resolve_session_dir(workspace, None, args.session_dir)]
    if not dirs:
        print(f"No sandbox sessions found in {workspace}", file=sys.stderr)
        return 1

    payload_out: list[dict[str, Any]] = []
    for sdir in dirs:
        meta = read_json(sdir / "session.json")
        transcript = read_jsonl(sdir / "transcript.jsonl")
        turns_by_thread: dict[str, int] = {}
        for entry in transcript:
            tid = entry.get("thread_id", "?")
            turns_by_thread[tid] = turns_by_thread.get(tid, 0) + 1

        threads_summary = []
        for th in meta.get("dialogue_threads") or []:
            tid = th.get("thread_id")
            harness = th.get("harness") or {}
            threads_summary.append(
                {
                    "thread_id": tid,
                    "active": tid == meta.get("active_thread_id"),
                    "messages": turns_by_thread.get(tid, 0),
                    "backend": harness.get("backend") or meta.get("backend"),
                    "tool_schema_version": harness.get("tool_schema_version"),
                    "skill_version": harness.get("skill_version"),
                    "last_turn_at": harness.get("last_turn_at") or th.get("created_at"),
                }
            )

        db_path = sdir / "workspace" / "mathscape.sqlite3"
        change_sets: list[dict[str, Any]] = []
        if db_path.is_file():
            with open_readonly_db(db_path) as conn:
                change_sets = fetch_recent_change_sets(conn, limit=args.limit)

        payload_out.append(
            {
                "sandbox_session_id": meta.get("sandbox_session_id", sdir.name),
                "path": str(sdir),
                "state": meta.get("state"),
                "backend": meta.get("backend"),
                "active_thread_id": meta.get("active_thread_id"),
                "threads": threads_summary,
                "recent_change_sets": change_sets,
            }
        )

    if args.json:
        print(json.dumps(payload_out, ensure_ascii=False, indent=2))
        return 0

    for idx, item in enumerate(payload_out):
        if idx > 0:
            print("\n" + "=" * 78)
        print(
            f"Sandbox: {item['sandbox_session_id']}  "
            f"[state={item['state']}, backend={item['backend']}, active_thread={item['active_thread_id']}]"
        )
        print(f"Path:    {item['path']}")
        print("\nThreads:")
        for th in item["threads"]:
            star = "*" if th["active"] else " "
            schema = th["tool_schema_version"] or "-"
            skill = f"skill@{th['skill_version']}" if th["skill_version"] else "-"
            print(
                f"  {star} {th['thread_id']:<4} msgs={th['messages']:<3} "
                f"schema={schema:<18} {skill:<12} last={th['last_turn_at']}"
            )
        print(f"\nRecent Change Sets (top {len(item['recent_change_sets'])}):")
        if not item["recent_change_sets"]:
            print("  (none)")
        for i, cs in enumerate(item["recent_change_sets"], start=1):
            diags = f" diags={cs['diagnostic_codes']}" if cs["diagnostic_codes"] else ""
            rationale = cs["rationale"].replace("\n", " ")
            if len(rationale) > 58:
                rationale = rationale[:58] + "…"
            print(
                f"  #{i:<2} {short_id(cs['change_set_id'], 12)}  v{cs['active_version']} "
                f"[{cs['state']:<9}] objs={cs['object_count']} rels={cs['relation_count']}{diags}"
            )
            if rationale:
                print(f"      target={cs['target_object']} | {rationale}")
    return 0


# ---------------------------------------------------------------------------
# Subcommand: thread
# ---------------------------------------------------------------------------


def extract_tool_result_detail(result_event: dict[str, Any] | None) -> str:
    if not result_event:
        return ""
    payload = result_event.get("payload") or {}
    status = payload.get("status", "")
    raw = payload.get("raw")
    if not isinstance(raw, dict):
        return ""
    if raw.get("error"):
        return str(raw["error"])[:240]
    res = raw.get("result")
    text = ""
    if isinstance(res, dict):
        content = res.get("content")
        if isinstance(content, list) and content and isinstance(content[0], dict):
            text = str(content[0].get("text") or "")
    elif isinstance(res, str):
        text = res
    if not text:
        return ""
    if status in ("error", "blocked") or '"error"' in text or '"blocking_codes"' in text:
        try:
            parsed = json.loads(text)
            if isinstance(parsed, dict):
                if "error" in parsed:
                    return f"error={json.dumps(parsed['error'], ensure_ascii=False)[:220]}"
                if "diagnostics" in parsed:
                    codes = [f"{d.get('level')}:{d.get('code')}({d.get('local_ref')})" for d in parsed["diagnostics"]]
                    return f"change_set={short_id(parsed.get('change_set_id'))} v{parsed.get('version')} diagnostics={codes}"
        except json.JSONDecodeError:
            pass
        return text[:220]
    if status == "staged":
        try:
            parsed = json.loads(text)
            if isinstance(parsed, dict) and parsed.get("change_set_id"):
                codes = [d.get("code") for d in (parsed.get("diagnostics") or []) if isinstance(d, dict)]
                return f"change_set={short_id(parsed.get('change_set_id'))} v{parsed.get('version')} info_diags={codes}"
        except json.JSONDecodeError:
            pass
    return ""


def summarize_round_events(events: list[dict[str, Any]]) -> dict[str, Any]:
    calls: list[dict[str, Any]] = []
    results_by_id: dict[str, dict[str, Any]] = {}
    usage_last: dict[str, Any] | None = None
    outcome: str | None = None

    for ev in events:
        etype = ev.get("type")
        payload = ev.get("payload") or {}
        if etype == "tool_result" and payload.get("call_id"):
            results_by_id[str(payload["call_id"])] = ev
        elif etype == "usage":
            usage_last = {
                "input_tokens": payload.get("input_tokens"),
                "cached_input_tokens": payload.get("cached_input_tokens"),
                "output_tokens": payload.get("output_tokens"),
            }
        elif etype == "turn_end":
            outcome = payload.get("outcome")

    for ev in events:
        if ev.get("type") != "tool_call":
            continue
        payload = ev.get("payload") or {}
        call_id = str(payload.get("call_id") or "")
        res_ev = results_by_id.get(call_id)
        res_payload = (res_ev or {}).get("payload") or {}
        calls.append(
            {
                "seq": ev.get("seq"),
                "server": payload.get("server"),
                "tool": payload.get("tool"),
                "args_summary": payload.get("args_summary", ""),
                "status": res_payload.get("status", "pending"),
                "duration_ms": res_payload.get("duration_ms"),
                "detail": extract_tool_result_detail(res_ev),
            }
        )

    return {"tool_calls": calls, "usage": usage_last, "outcome": outcome}


def cmd_thread(args: argparse.Namespace) -> int:
    workspace = resolve_workspace_root(args.workspace)
    sdir = resolve_session_dir(workspace, args.session, args.session_dir)
    meta = read_json(sdir / "session.json")
    thread_id = args.thread_id or meta.get("active_thread_id")
    if not thread_id:
        raise SystemExit("No thread_id specified and session has no active_thread_id")

    thread_meta = next((t for t in (meta.get("dialogue_threads") or []) if t.get("thread_id") == thread_id), None)
    if not thread_meta:
        known = ", ".join(str(t.get("thread_id")) for t in (meta.get("dialogue_threads") or []))
        raise SystemExit(f"Unknown thread {thread_id!r} in {sdir.name} (known: {known})")

    transcript = [e for e in read_jsonl(sdir / "transcript.jsonl") if e.get("thread_id") == thread_id]
    round_ids = {str(e["round_id"]) for e in transcript if e.get("round_id")}
    events_by_round = read_events_for_rounds(sdir / "events.jsonl", round_ids) if not args.no_tools else {}

    harness_by_round = {
        str(h["round_id"]): h for h in read_jsonl(sdir / "harness.jsonl") if h.get("round_id") in round_ids
    }

    # Pair turns / enrich agent messages with round tool calls
    enriched_turns: list[dict[str, Any]] = []
    for entry in transcript:
        item = dict(entry)
        rid = entry.get("round_id")
        if rid and rid in events_by_round:
            item["round_events"] = summarize_round_events(events_by_round[rid])
        if rid and rid in harness_by_round:
            h = harness_by_round[rid]
            item["skill"] = h.get("skill")
            item["lean_profile_root"] = h.get("lean_profile_root")
        enriched_turns.append(item)

    if args.tail and args.tail > 0:
        enriched_turns = enriched_turns[-args.tail :]

    out = {
        "sandbox_session_id": meta.get("sandbox_session_id", sdir.name),
        "thread": thread_meta,
        "turn_count": len(enriched_turns),
        "turns": enriched_turns,
    }
    if args.json:
        print(json.dumps(out, ensure_ascii=False, indent=2))
        return 0

    harness = thread_meta.get("harness") or {}
    print(
        f"Session: {out['sandbox_session_id']}  |  Thread: {thread_id}  "
        f"|  schema={harness.get('tool_schema_version')}  skill={harness.get('skill_version')}"
    )
    print("=" * 78)
    for idx, turn in enumerate(enriched_turns, start=1):
        role = str(turn.get("role", "?")).upper()
        ts = turn.get("ts", "")
        rid = f"  round={turn['round_id']}" if turn.get("round_id") else ""
        print(f"\n[{idx}] {role} @ {ts}{rid}")
        if turn.get("lean_profile_root"):
            lpr = turn["lean_profile_root"]
            root_val = lpr.get("root") if isinstance(lpr, dict) else lpr
            print(f"    env: lean_profile_root={root_val}")
        round_ev = turn.get("round_events")
        if round_ev and round_ev.get("tool_calls"):
            print(f"    Tool Calls ({len(round_ev['tool_calls'])}):")
            for tc in round_ev["tool_calls"]:
                dur = f"{tc['duration_ms']}ms" if tc["duration_ms"] is not None else "-"
                args_str = str(tc["args_summary"] or "").replace("\n", " ")
                if len(args_str) > 96 and not args.full:
                    args_str = args_str[:96] + "…"
                flag = "!!" if tc["status"] in ("error", "blocked") else "  "
                print(f"    {flag} #{tc['seq']:<3} [{tc['status']:<9} {dur:>7}] {tc['server']}.{tc['tool']}({args_str})")
                if tc["detail"]:
                    print(f"           -> {tc['detail']}")
            if round_ev.get("usage"):
                u = round_ev["usage"]
                print(
                    f"    Tokens: in={u.get('input_tokens')} (cached={u.get('cached_input_tokens')}) "
                    f"out={u.get('output_tokens')}"
                )
        print("-" * 40)
        text = str(turn.get("text") or "")
        if not args.full and len(text) > 1600:
            text = text[:1600] + f"\n… [truncated {len(text) - 1600} chars; pass --full for complete text]"
        for line in text.splitlines() or [""]:
            print(f"  {line}")
    return 0


# ---------------------------------------------------------------------------
# Subcommand: proposal
# ---------------------------------------------------------------------------


def resolve_change_set_id(conn: sqlite3.Connection, selector: str) -> str:
    rows = [
        r["change_set_id"]
        for r in conn.execute("SELECT change_set_id FROM proposed_change_sets ORDER BY rowid DESC").fetchall()
    ]
    if not rows:
        raise SystemExit("No proposed_change_sets found in sandbox SQLite database.")
    if selector in ("latest", "last"):
        return rows[0]
    if selector.startswith("#") and selector[1:].isdigit():
        idx = int(selector[1:]) - 1
        if 0 <= idx < len(rows):
            return rows[idx]
        raise SystemExit(f"Index {selector} out of range (1..{len(rows)})")
    exact = [cid for cid in rows if cid == selector]
    if len(exact) == 1:
        return exact[0]
    matches = [cid for cid in rows if selector in cid]
    if len(matches) == 1:
        return matches[0]
    if not matches:
        raise SystemExit(f"No change_set matching {selector!r}")
    raise SystemExit(f"Ambiguous change_set selector {selector!r}: {', '.join(matches[:5])}")


def resolve_existing_refs(conn: sqlite3.Connection, refs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    resolved: list[dict[str, Any]] = []
    for ref in refs:
        oid = ref.get("id")
        rev = ref.get("revision")
        row = conn.execute(
            """
            SELECT o.entity_kind, o.current_revision, r.lifecycle, r.payload_json
            FROM map_objects o
            JOIN map_object_revisions r ON o.object_id = r.object_id AND o.current_revision = r.revision_id
            WHERE o.object_id = ?
            """,
            (oid,),
        ).fetchone()
        if not row:
            resolved.append({"id": oid, "revision": rev, "found": False})
            continue
        p = json.loads(row["payload_json"])
        inner = p.get("payload", p)
        resolved.append(
            {
                "id": oid,
                "revision": rev,
                "found": True,
                "revision_matches_head": rev == row["current_revision"],
                "entity_kind": row["entity_kind"],
                "semantic_kind": inner.get("semantic_kind") if isinstance(inner, dict) else None,
                "lifecycle": row["lifecycle"],
                "label": extract_entity_label(p),
            }
        )
    return resolved


def find_artifact_bytes(conn: sqlite3.Connection, sandbox_workspace: Path, mother_workspace: Path, artifact_id: str) -> bytes | None:
    sha = artifact_id.rsplit("/", 1)[-1]
    locs = conn.execute("SELECT uri FROM artifact_locations WHERE artifact_id = ? ORDER BY priority", (artifact_id,)).fetchall()
    candidates: list[Path] = []
    for loc in locs:
        uri = str(loc["uri"] or "")
        if uri.startswith("file://"):
            candidates.append(Path(uri.removeprefix("file://")))
        elif uri.startswith("pool:sha256/"):
            h = uri.split("/", 1)[1]
            candidates.append(sandbox_workspace / "objects" / "sha256" / h[:2] / h[2:4] / h)
            candidates.append(mother_workspace / "objects" / "sha256" / h[:2] / h[2:4] / h)
    if len(sha) == 64:
        candidates.append(sandbox_workspace / "objects" / "sha256" / sha[:2] / sha[2:4] / sha)
        candidates.append(mother_workspace / "objects" / "sha256" / sha[:2] / sha[2:4] / sha)
    for cand in candidates:
        if cand.is_file():
            return cand.read_bytes()
    return None


def verify_source_spans(
    conn: sqlite3.Connection,
    sandbox_workspace: Path,
    mother_workspace: Path,
    proposed_objects: list[dict[str, Any]],
    compiled_ops: list[dict[str, Any]],
    staged_ids: dict[str, str] | None = None,
) -> list[dict[str, Any]]:
    span_ops = [op for op in compiled_ops if op.get("operation") == "create_source_span"]
    span_objs = [o for o in proposed_objects if o.get("kind") == "source_span"]
    span_ops_by_id = {str(op["object_id"]): op for op in span_ops if op.get("object_id")}

    def compiled_span_op(i: int, obj: dict[str, Any]) -> dict[str, Any]:
        object_id = obj.get("target_id") or (staged_ids or {}).get(str(obj.get("local_ref")))
        if object_id:
            return span_ops_by_id.get(str(object_id)) or {}
        # Older results without staged ids: fall back to proposal order.
        return span_ops[i] if i < len(span_ops) else {}

    artifact_cache: dict[str, bytes | None] = {}
    checks: list[dict[str, Any]] = []
    for i, obj in enumerate(span_objs):
        p = obj.get("payload") or {}
        aid = str(p.get("artifact_id") or "")
        br = p.get("md_byte_range") or {}
        start, end = br.get("start"), br.get("end")
        quote = p.get("quote", "")
        if aid not in artifact_cache:
            artifact_cache[aid] = find_artifact_bytes(conn, sandbox_workspace, mother_workspace, aid)
        raw = artifact_cache[aid]
        page_info = (compiled_span_op(i, obj).get("draft") or {}).get("page") or {}
        pdf_page = page_info.get("pdf_page_index")

        if raw is None:
            status = "artifact_missing"
            actual = None
        elif not isinstance(start, int) or not isinstance(end, int) or start < 0 or end > len(raw) or start > end:
            status = "out_of_bounds"
            actual = None
        else:
            actual = raw[start:end].decode("utf-8", errors="replace")
            status = "exact_match" if actual == quote else "quote_mismatch"

        checks.append(
            {
                "local_ref": obj.get("local_ref"),
                "artifact_id": aid,
                "byte_range": [start, end],
                "granularity": p.get("granularity"),
                "pdf_page_index": pdf_page,
                "status": status,
                "quote": quote,
                "actual_slice": actual if status == "quote_mismatch" else None,
            }
        )
    return checks


def format_proposed_object_brief(obj: dict[str, Any]) -> list[str]:
    kind = obj.get("kind")
    op = obj.get("op")
    lref = obj.get("local_ref")
    p = obj.get("payload") or {}
    lines = [f"  - [{kind}] {lref} (op={op})"]
    if kind == "source_unit":
        lines.append(
            f"      unit_kind={p.get('kind')} completeness={p.get('completeness')} "
            f"parent={p.get('parent')} title={p.get('display_title')!r} spans={p.get('source_spans')}"
        )
    elif kind == "dialect_scope":
        syms = [f"{s.get('symbol')}:{s.get('role')}" for s in (p.get("local_symbols") or [])]
        convs = [f"{c.get('key')}({c.get('subject')})={c.get('value')}" for c in (p.get("conventions") or [])]
        lines.append(
            f"      label={p.get('label')!r} anchor={p.get('anchor_unit')} parent_scope={p.get('parent_scope')}"
        )
        lines.append(f"      conventions={convs}")
        lines.append(f"      local_symbols={syms}")
    elif kind == "semantic_entity":
        sk = p.get("semantic_kind")
        label = p.get("plain_label") or extract_entity_label({"payload": p})
        mb = p.get("mathlib_baseline") or {}
        lines.append(
            f"      semantic_kind={sk} label={label!r} home_theory={short_id(p.get('home_theory'))} "
            f"mathlib={mb.get('decl')} ({mb.get('relation_hint')}, {mb.get('status')})"
        )
        decl = p.get("declaration")
        if isinstance(decl, dict):
            guards = [f"{g.get('guard_id')}:{g.get('label')}" for g in (decl.get("guards") or [])]
            gp = (decl.get("guard_partition") or {}).get("status")
            excl = [f"{e.get('kind')}:{(e.get('case') or {}).get('text', '')[:50]}" for e in (decl.get("exclusions") or [])]
            lines.append(
                f"      decl_kind={decl.get('declaration_kind')} concept={short_id(decl.get('concept'))} "
                f"uses={[short_id(u) for u in (decl.get('uses_entities') or [])]}"
            )
            if guards or gp:
                lines.append(f"      guards={guards} partition={gp}")
            if decl.get("statement"):
                lines.append(f"      statement={(decl['statement'] or {}).get('text')}")
            for br in decl.get("branches") or []:
                deg = " [degenerate]" if br.get("is_degenerate") else ""
                lines.append(
                    f"      branch {br.get('branch_id')}{deg} guards={br.get('guards')}: "
                    f"{(br.get('conclusion') or {}).get('text')}"
                )
            if excl:
                lines.append(f"      exclusions={excl}")
        ps = p.get("problem_schema")
        if isinstance(ps, dict):
            lines.append(f"      task={(ps.get('task') or {}).get('text')}")
    elif kind == "representation_core":
        label = p.get("plain_label") or extract_entity_label({"payload": p})
        ops = [o.get("name") for o in (p.get("operations") or [])]
        lines.append(
            f"      label={label!r} represented_object={short_id(p.get('represented_object'))} operations={ops}"
        )
        lines.append(f"      carrier={(p.get('carrier') or {}).get('text')}")
    elif kind == "alignment":
        lost = [li.get("short") or li.get("claim") for li in (p.get("lost_information") or [])]
        lines.append(
            f"      {p.get('source')} --[{p.get('relation')} / {p.get('status')}]--> {short_id(p.get('target'))} "
            f"target_branches={p.get('target_branches')} scope={(p.get('source_context') or {}).get('dialect_scope')}"
        )
        if lost:
            lines.append(f"      lost_information={lost}")
    elif kind == "representation_binding":
        smap = [
            f"{m.get('local_symbol')} -> {(m.get('target_expr') or {}).get('text')}"
            for m in (p.get("symbol_map") or [])
        ]
        lines.append(
            f"      scope={p.get('dialect_scope')} -> repr={p.get('representation')} status={p.get('status')}"
        )
        lines.append(f"      symbol_map={smap}")
    return lines


def cmd_proposal(args: argparse.Namespace) -> int:
    workspace = resolve_workspace_root(args.workspace)
    sdir = resolve_session_dir(workspace, args.session, args.session_dir)
    sbx_ws = sdir / "workspace"
    db_path = sbx_ws / "mathscape.sqlite3"

    with open_readonly_db(db_path) as conn:
        cs_id = resolve_change_set_id(conn, args.selector)
        cs_row = conn.execute("SELECT * FROM proposed_change_sets WHERE change_set_id = ?", (cs_id,)).fetchone()
        ver_rows = conn.execute(
            "SELECT * FROM proposed_change_set_versions WHERE change_set_id = ? ORDER BY version ASC",
            (cs_id,),
        ).fetchall()
        decisions = [
            dict(r)
            for r in conn.execute(
                "SELECT version, kind, decided_by, commit_id, reason, decided_at FROM change_set_decisions WHERE change_set_id = ? ORDER BY rowid ASC",
                (cs_id,),
            ).fetchall()
        ]

        version_summaries = []
        selected_versions = []
        target_ver = args.version or int(cs_row["active_version"])
        for vr in ver_rows:
            vnum = int(vr["version"])
            prop = json.loads(vr["proposal_json"])
            res = json.loads(vr["result_json"])
            diags = res.get("diagnostics") or []
            version_summaries.append(
                {
                    "version": vnum,
                    "state": vr["state"],
                    "result_status": res.get("status"),
                    "round_id": vr["round_id"],
                    "created_at": vr["created_at"],
                    "diagnostics": diags,
                }
            )
            if args.all_versions or vnum == target_ver:
                selected_versions.append(
                    {
                        "version": vnum,
                        "state": vr["state"],
                        "round_id": vr["round_id"],
                        "created_at": vr["created_at"],
                        "existing_refs_resolved": resolve_existing_refs(conn, prop.get("existing_refs") or []),
                        "span_verification": verify_source_spans(
                            conn,
                            sbx_ws,
                            workspace,
                            prop.get("proposed_objects") or [],
                            res.get("compiled_operations") or [],
                            res.get("staged_ids") or {},
                        ),
                        "proposal": prop,
                        "result": res,
                    }
                )

    if not selected_versions:
        avail = [v["version"] for v in version_summaries]
        raise SystemExit(f"Version {target_ver} not found for {cs_id} (available: {avail})")

    report = {
        "sandbox_session_id": sdir.name,
        "change_set_id": cs_id,
        "active_version": cs_row["active_version"],
        "decisions": decisions,
        "version_history": version_summaries,
        "inspected_versions": selected_versions,
    }

    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0

    print(f"Change Set: {cs_id}")
    print(f"Sandbox:    {sdir.name}  |  active_version=v{cs_row['active_version']}  |  decisions={decisions or 'pending'}")
    print("\nVersion History:")
    for vh in version_summaries:
        codes = [f"{d.get('level')}:{d.get('code')}({d.get('local_ref')})" for d in vh["diagnostics"]]
        print(
            f"  v{vh['version']} [{vh['state']:<10} / status={vh['result_status']:<7}] "
            f"round={vh['round_id']} @ {vh['created_at']}  diags={codes}"
        )

    for sv in selected_versions:
        prop = sv["proposal"]
        res = sv["result"]
        print("\n" + "=" * 78)
        print(f"INSPECTING VERSION v{sv['version']}  (state={sv['state']}, target_object={prop.get('target_object')})")
        print(f"Rationale:  {prop.get('rationale')}")
        if prop.get("unresolved"):
            print("Unresolved:")
            for u in prop["unresolved"]:
                print(f"  - {u}")

        if args.raw:
            print("\nRaw proposal_json:")
            print(json.dumps(prop, ensure_ascii=False, indent=2))
            continue

        print("\nExisting Refs:")
        for er in sv["existing_refs_resolved"]:
            if not er["found"]:
                print(f"  ✗ {er['id']} (NOT FOUND)")
            else:
                head_ok = "✓head" if er["revision_matches_head"] else "!stale_rev"
                sk = f"/{er['semantic_kind']}" if er["semantic_kind"] else ""
                print(f"  - [{er['entity_kind']}{sk}] {er['label']} ({short_id(er['id'], 12)}) [{head_ok}]")

        print("\nValidator Diagnostics:")
        for d in res.get("diagnostics") or []:
            print(f"  - [{d.get('level', '?').upper():<5}] {d.get('code')} on {d.get('local_ref')}: {d.get('message')}")
        if not (res.get("diagnostics") or []):
            print("  (none)")

        print("\nSource Span Byte-Range Audit:")
        for sp in sv["span_verification"]:
            mark = "✓" if sp["status"] == "exact_match" else "✗"
            q_preview = sp["quote"].replace("\n", " ⏎ ")
            if len(q_preview) > 84 and not args.full:
                q_preview = q_preview[:84] + "…"
            page_tag = f"pdf_p0={sp['pdf_page_index']}" if sp["pdf_page_index"] is not None else "pdf_p0=?"
            print(
                f"  {mark} {sp['local_ref']:<14} [{sp['status']}] {short_id(sp['artifact_id'], 10)} "
                f"bytes={sp['byte_range']} gran={sp['granularity']} {page_tag}"
            )
            print(f"      quote: {q_preview}")
            if sp["actual_slice"] is not None:
                print(f"      actual: {sp['actual_slice']!r}")

        non_span_objs = [o for o in (prop.get("proposed_objects") or []) if o.get("kind") != "source_span"]
        print(f"\nProposed Objects ({len(non_span_objs)} non-span + {len(sv['span_verification'])} spans):")
        for obj in non_span_objs:
            for line in format_proposed_object_brief(obj):
                print(line)

        rels = prop.get("proposed_relations") or []
        print(f"\nProposed Relations ({len(rels)}):")
        for rel in rels:
            print(
                f"  - [{rel.get('kind')}] {rel.get('local_ref')}: "
                f"{rel.get('source')} --[{rel.get('relation')}]--> {rel.get('target')} "
                f"spans={rel.get('evidence_spans')}"
            )
    return 0


def build_parser() -> argparse.ArgumentParser:
    root_common = argparse.ArgumentParser(add_help=False)
    root_common.add_argument(
        "-w",
        "--workspace",
        dest="_root_workspace",
        default=None,
        help="Mother workspace root (defaults to $MATHSCAPE_WORKSPACE_ROOT or .mathscape-dev/workspace)",
    )
    root_common.add_argument(
        "-s",
        "--session",
        dest="_root_session",
        default=None,
        help="Sandbox session ID or substring (defaults to most recently active session)",
    )
    root_common.add_argument(
        "--session-dir",
        dest="_root_session_dir",
        default=None,
        help="Explicit path to a sandbox session directory",
    )
    root_common.add_argument(
        "--json",
        dest="_root_json",
        action="store_true",
        default=False,
        help="Output structured JSON",
    )

    sub_common = argparse.ArgumentParser(add_help=False)
    sub_common.add_argument("-w", "--workspace", default=None, help=argparse.SUPPRESS)
    sub_common.add_argument("-s", "--session", default=None, help=argparse.SUPPRESS)
    sub_common.add_argument("--session-dir", default=None, help=argparse.SUPPRESS)
    sub_common.add_argument("--json", action="store_true", default=False, help=argparse.SUPPRESS)

    parser = argparse.ArgumentParser(
        description="Read-only inspector for Mathscape sandbox dialogues, tool calls, and proposed change sets.",
        parents=[root_common],
    )
    sub = parser.add_subparsers(dest="subcommand")

    p_list = sub.add_parser("list", parents=[sub_common], help="List sandbox sessions, threads, and recent proposals")
    p_list.add_argument("-n", "--limit", type=int, default=10, help="Max recent change sets per session (default: 10)")

    p_thread = sub.add_parser("thread", parents=[sub_common], help="Inspect a dialogue thread and its MCP tool-call timeline")
    p_thread.add_argument("thread_id", nargs="?", help="Thread ID such as d7 (defaults to session's active_thread_id)")
    p_thread.add_argument("--tail", type=int, default=0, help="Show only the last N messages")
    p_thread.add_argument("--no-tools", action="store_true", help="Skip reading events.jsonl tool calls")
    p_thread.add_argument("--full", action="store_true", help="Do not truncate long messages or tool arguments")

    p_prop = sub.add_parser("proposal", parents=[sub_common], help="Inspect a proposed change set, diagnostics, and span quotes")
    p_prop.add_argument(
        "selector",
        nargs="?",
        default="latest",
        help="'latest' (default), '#N' index from list, or change-set ID / substring",
    )
    p_prop.add_argument("-v", "--version", type=int, help="Inspect a specific version (defaults to active_version)")
    p_prop.add_argument("--all-versions", action="store_true", help="Inspect every version v1..vN of the change set")
    p_prop.add_argument("--raw", action="store_true", help="Print full raw proposal_json")
    p_prop.add_argument("--full", action="store_true", help="Do not truncate quotes")

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    args.workspace = getattr(args, "workspace", None) or getattr(args, "_root_workspace", None)
    args.session = getattr(args, "session", None) or getattr(args, "_root_session", None)
    args.session_dir = getattr(args, "session_dir", None) or getattr(args, "_root_session_dir", None)
    args.json = bool(getattr(args, "json", False) or getattr(args, "_root_json", False))
    if args.subcommand in (None, "list"):
        if not hasattr(args, "limit"):
            args.limit = 10
        return cmd_list(args)
    if args.subcommand == "thread":
        return cmd_thread(args)
    if args.subcommand == "proposal":
        return cmd_proposal(args)
    parser.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
