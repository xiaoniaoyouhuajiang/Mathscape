#!/usr/bin/env python3
"""Export small, exact raw ACP records from disposable-fixture runs."""
import hashlib
import json
from pathlib import Path
import sys


root = Path(sys.argv[1])
destination = Path(sys.argv[2])
manifest = []
for source in sorted(root.glob("c*.jsonl")):
    selected = []
    for line in source.read_text().splitlines(keepends=True):
        record = json.loads(line)
        message = json.loads(record["raw"]) if "raw" in record else record.get("message", {})
        update = message.get("params", {}).get("update", {})
        if (record.get("check") == "model" or
            update.get("sessionUpdate") in ["tool_call", "tool_call_update", "agent_message_chunk"] or
            message.get("method") in ["session/prompt", "session/request_permission", "session/cancel"] or
            ("result" in message and "stopReason" in message["result"])):
            selected.append(line)
    if not selected:
        continue
    output = destination / source.name
    output.write_text("".join(selected))
    summary = Path(str(source) + ".summary.json")
    manifest.append({"file": source.name, "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                     "export_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
                     "outcome": json.loads(summary.read_text()) if summary.exists() else None})
    trace = Path(str(source) + ".execve")
    if trace.exists():
        output.with_suffix(".execve").write_text(trace.read_text())
(destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(f"Exported {len(manifest)} ACP excerpts; total bytes: {sum(p.stat().st_size for p in destination.iterdir())}")
