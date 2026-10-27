#!/usr/bin/env bash
# Phase0 stub for the `lean` capability (docs/phase0-contract.md §4).
# Mathscape never bundles Lean; this script only checks for it and prints
# official install hints. It does not install anything yet.
set -euo pipefail

if command -v lean >/dev/null 2>&1; then
  printf '[lean] available: %s\n' "$(lean --version 2>&1 | head -n 1)"
  exit 0
fi

printf '[lean] missing: `lean` not found on PATH\n'
printf '[lean] hint: install elan (user-local): https://lean-lang.org/install/ — curl https://elan.lean-lang.org/elan-init.sh -sSf | sh\n'
exit 1
