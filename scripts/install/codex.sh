#!/usr/bin/env bash
# Phase0 stub for the `codex` capability (docs/phase0-contract.md §4).
# Mathscape never bundles the Codex CLI and does not log in or store a ChatGPT
# subscription. `CodexExecutor` runs `codex app-server` with the user's existing
# vendor login. This script only checks for the binary and prints install hints.
set -euo pipefail

if command -v codex >/dev/null 2>&1; then
  printf '[codex] available: %s\n' "$(codex --version 2>&1 | head -n 1)"
  exit 0
fi

printf '[codex] missing: `codex` not found on PATH\n'
printf '[codex] hint: install the Codex CLI: npm install -g @openai/codex (or see https://github.com/openai/codex)\n'
printf '[codex] hint: sign in with the Codex CLI itself. Mathscape only runs `codex app-server` and does not proxy that login.\n'
exit 1
