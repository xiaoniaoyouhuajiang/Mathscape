#!/usr/bin/env bash
# Phase0 stub for the `codex` capability (docs/phase0-contract.md §4).
# Mathscape never bundles the Codex CLI; this script only checks for it and prints
# official install hints. It does not install anything yet.
set -euo pipefail

if command -v codex >/dev/null 2>&1; then
  printf '[codex] available: %s\n' "$(codex --version 2>&1 | head -n 1)"
  exit 0
fi

printf '[codex] missing: `codex` not found on PATH\n'
printf '[codex] hint: install the Codex CLI: npm install -g @openai/codex (or see https://github.com/openai/codex)\n'
exit 1
