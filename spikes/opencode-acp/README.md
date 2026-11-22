# OpenCode ACP spike (not a production integration)

Findings: [中文报告](../../docs/spikes/opencode-acp.md).

Requirements: Python 3.10+, Rust workspace toolchain, `opencode` 1.19.6 on
PATH, `/usr/bin/bwrap`, and `strace`. No Python packages or new Cargo dependencies.
No model key is needed for the tested free models. Model availability/quotas can change.

## Reproduce

Use a **disposable** directory outside the repository. The probe deliberately tries
to change fixture files outside scratch and performs public `example.com` HEAD
requests in its network positive control. Never point it at a real workspace.

From the repo root:

```sh
cargo build -p mathscape-cli
ROOT=$(mktemp -d "$HOME/opencode-acp-XXXXXX")
mkdir -p "$ROOT/workspace/scratch/runtime/tmp" "$ROOT/evidence"
rustc --edition=2024 spikes/opencode-acp/workspace.rs \
  -L dependency=target/debug/deps \
  --extern mathscape_artifact="$(find target/debug/deps -name 'libmathscape_artifact-*.rlib' -print -quit)" \
  --extern mathscape_store_sqlite="$(find target/debug/deps -name 'libmathscape_store_sqlite-*.rlib' -print -quit)" \
  -o "$ROOT/workspace-init"
"$ROOT/workspace-init" "$ROOT/workspace"
printf 'acp-spike-round\n' > "$ROOT/round.txt"
printf 'SENTINEL_acp_7b9d6e2f_DO_NOT_LEAK\n' > "$ROOT/sentinel.txt"
printf 'ORIGINAL_WORKSPACE_CONTENT\n' > "$ROOT/workspace/protected.txt"
ln -s "$ROOT/sentinel.txt" "$ROOT/workspace/scratch/sentinel-link.txt"
for CONFIG in 1 2; do
  printf 'ORIGINAL_LINK_TARGET\n' > "$ROOT/workspace/protected-link-target-c$CONFIG.txt"
  ln -s "$ROOT/workspace/protected-link-target-c$CONFIG.txt" \
    "$ROOT/workspace/scratch/protected-link-c$CONFIG.txt"
done
```

`probe.py` writes workspace `opencode.json`, pins the requested model, and runs
the ACP client under execve tracing. Run **sequentially**, since the config and
fixtures are shared. Non-git workspaces currently match native file permissions
relative to `/`; this spike config includes both absolute and root-relative paths.
Do not reuse that config for projects with a different worktree root.

```sh
for MODEL in opencode/nemotron-3.5-lightning-free opencode/hy4-flash-free; do
  for CONFIG in 1 2; do
    python3 spikes/opencode-acp/probe.py --root "$ROOT" \
      --mathscape target/debug/mathscape --model "$MODEL" --config "$CONFIG" --case all
  done
  python3 spikes/opencode-acp/probe.py --root "$ROOT" \
    --mathscape target/debug/mathscape --model "$MODEL" --config 1 --case mcp
done
```

Other cases: `symlink`, `symlink-write`, `cancel`; individual `write`, `read`,
`network`, `workspace`, `scratch` cases can re-prompt actions a model skipped.
`injection` expects an existing `workspace/injection.txt` containing an untrusted
instruction, and asks the model to summarize it, not follow it.
`--tag=-retry1` avoids overwriting earlier logs.

For the **separate network positive control**, use `--config 2 --case network
--policy allow-once --tag=-positive-control`. This intentionally permits shell
execution and is NOT the default confinement configuration.

The lower-level client can exercise load against the same pinned workspace config:

```sh
python3 spikes/opencode-acp/client.py --cwd "$ROOT/workspace" \
  --log "$ROOT/evidence/load.jsonl" --model opencode/hy4-flash-free \
  --mathscape "$PWD/target/debug/mathscape" --round-file "$ROOT/round.txt" \
  --load --prompt 'Reply READY'
```

Use the model most recently written by `probe.py`. A mismatch fails closed before
the prompt. `--cancel-after 3` cancels an in-flight prompt. Client capabilities do
not offer host file/terminal access; OpenCode uses its own tools inside the child.

Each run saves the full request/response stream, verbatim updates, stderr, execve
trace, client output, and independent fixture summary. `collect.py "$ROOT/evidence"
spikes/opencode-acp/evidence` exports selected unmodified raw records plus source
hashes; this generated directory is git-ignored. Review logs for secrets before
sharing. `results.json` contains only compact selected fixture evidence from this run.

## Focused checks

```sh
python3 -m unittest spikes/opencode-acp/test_client.py
python3 -m py_compile spikes/opencode-acp/*.py
bash -n spikes/opencode-acp/bwrap.sh
```

The bwrap wrapper is a Linux feasibility experiment, not a portable hardened
launcher. It shares model network access with all children. Disabling shell and
other network-capable tools is mandatory unless a separate shell sandbox is added.
It mounts runtime binaries/libraries and a handful of system files read-only;
OpenCode's persistent runtime data is under scratch. `/dev`/`proc` remain runtime
mounts, and no claim is made about confinement of a compromised OpenCode binary.
