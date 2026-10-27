#!/usr/bin/env bash
# Phase0 stub for the `claude_code` capability (docs/phase0-contract.md §4).
# Mathscape never bundles Claude Code; this script only checks for it and prints
# official install hints. It does not install anything yet.
set -euo pipefail

if command -v claude >/dev/null 2>&1; then
  printf '[claude_code] available: %s\n' "$(claude --version 2>&1 | head -n 1)"
  exit 0
fi

printf '[claude_code] missing: `claude` not found on PATH\n'
printf '[claude_code] hint: install Claude Code: see https://docs.anthropic.com/en/docs/claude-code (e.g. npm install -g @anthropic-ai/claude-code)\n'
exit 1
