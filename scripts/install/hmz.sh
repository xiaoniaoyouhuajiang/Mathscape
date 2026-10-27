#!/usr/bin/env bash
# Phase0 stub for the `hmz` capability (docs/phase0-contract.md §4).
# Mathscape never bundles hmz; this script only checks for it and prints
# official install hints. It does not install anything yet.
set -euo pipefail

if command -v hmz >/dev/null 2>&1; then
  printf '[hmz] available: %s\n' "$(hmz --version 2>&1 | head -n 1)"
  exit 0
fi

printf '[hmz] missing: `hmz` not found on PATH\n'
printf '[hmz] hint: hmz is an optional adapter; nothing in Mathscape requires it. Install it per its own docs if you want the hmz backend.\n'
exit 1
