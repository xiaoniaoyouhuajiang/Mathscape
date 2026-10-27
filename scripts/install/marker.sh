#!/usr/bin/env bash
# Install Marker (marker-pdf 2.x) into a user-local virtualenv for Mathscape.
#
# Idempotent: re-running keeps an existing venv and skips pip when a
# supported marker-pdf is already present. Never touches system Python or
# global site-packages. See scripts/install/README.md and
# docs/phase0-contract.md §4.
#
# Usage: scripts/install/marker.sh [--check] [--dry-run] [--upgrade]
#   --check    only report status (same detection as CapabilityProbe), exit 0 if available
#   --dry-run  print what would happen, change nothing
#   --upgrade  reinstall/upgrade marker-pdf within the supported range
#
# Environment:
#   MATHSCAPE_MARKER_VENV  venv directory
#                          (default: ${XDG_DATA_HOME:-$HOME/.local/share}/mathscape/marker/venv)
#   PYTHON                 base interpreter used to create the venv (default: python3)
#   MARKER_PIP_SPEC        pip requirement (default: "marker-pdf>=2,<3")
#   PIP_INDEX_URL etc.     honoured by pip as usual (mirrors for restricted networks)
set -euo pipefail

MODE=install
UPGRADE=0
for arg in "$@"; do
  case "$arg" in
    --check) MODE=check ;;
    --dry-run) MODE=dry-run ;;
    --upgrade) UPGRADE=1 ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "marker.sh: unknown argument: $arg" >&2; exit 2 ;;
  esac
done

DATA_HOME="${XDG_DATA_HOME:-${HOME:?HOME is not set}/.local/share}"
VENV="${MATHSCAPE_MARKER_VENV:-$DATA_HOME/mathscape/marker/venv}"
PYTHON="${PYTHON:-python3}"
PIP_SPEC="${MARKER_PIP_SPEC:-marker-pdf>=2,<3}"
REQUIRED_MAJOR=2
VERSION_SCRIPT="import importlib.metadata as m; print(m.version('marker-pdf'), end='')"

say() { printf '[marker] %s\n' "$*"; }
hint() { printf '[marker] hint: %s\n' "$*"; }

installed_version() {
  [ -x "$VENV/bin/python" ] || return 1
  "$VENV/bin/python" -c "$VERSION_SCRIPT" 2>/dev/null
}

is_supported() {
  local version="$1"
  [ "${version%%.*}" = "$REQUIRED_MAJOR" ] && [ -x "$VENV/bin/marker_single" ]
}

print_usage_hint() {
  say "marker_single: $VENV/bin/marker_single"
  hint "export MATHSCAPE_MARKER_EXECUTABLE=\"$VENV/bin/marker_single\""
  hint "Mathscape also detects this default venv automatically (CapabilityProbe)."
}

current="$(installed_version || true)"

if [ "$MODE" = check ]; then
  if [ -n "$current" ] && is_supported "$current"; then
    say "available: marker-pdf $current"
    print_usage_hint
    exit 0
  elif [ -n "$current" ]; then
    say "version: found marker-pdf $current, Mathscape requires ${REQUIRED_MAJOR}.x"
    hint "re-run: scripts/install/marker.sh --upgrade"
    exit 1
  fi
  say "missing: no Marker venv at $VENV"
  hint "install: scripts/install/marker.sh"
  exit 1
fi

if [ -n "$current" ] && is_supported "$current" && [ "$UPGRADE" -eq 0 ]; then
  say "already installed: marker-pdf $current (nothing to do)"
  print_usage_hint
  exit 0
fi

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  say "error: $PYTHON not found"
  hint "install Python 3.10+ (e.g. from python.org, Homebrew 'brew install python', or your package manager), then re-run; or set PYTHON=/path/to/python3"
  exit 1
fi

if ! "$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)'; then
  say "error: $("$PYTHON" --version 2>&1) is too old; Marker needs Python 3.10+"
  hint "set PYTHON=/path/to/python3.10+ and re-run"
  exit 1
fi

if [ "$MODE" = dry-run ]; then
  [ -x "$VENV/bin/python" ] || say "would create venv: $PYTHON -m venv $VENV"
  say "would run: $VENV/bin/python -m pip install --upgrade pip"
  say "would run: $VENV/bin/python -m pip install --upgrade '$PIP_SPEC'"
  exit 0
fi

if [ ! -x "$VENV/bin/python" ]; then
  say "creating venv at $VENV"
  mkdir -p "$(dirname "$VENV")"
  if ! "$PYTHON" -m venv "$VENV"; then
    say "error: could not create venv"
    hint "on Debian/Ubuntu install the venv module: sudo apt install python3-venv (system package; outside this script)"
    exit 1
  fi
else
  say "reusing venv at $VENV"
fi

say "installing $PIP_SPEC (this downloads PyTorch and models; it can take a while)"
if ! "$VENV/bin/python" -m pip install --upgrade pip >/dev/null \
  || ! "$VENV/bin/python" -m pip install --upgrade "$PIP_SPEC"; then
  say "error: pip install failed"
  hint "check network access to PyPI, or set PIP_INDEX_URL to a mirror (e.g. https://pypi.tuna.tsinghua.edu.cn/simple) and re-run"
  hint "the venv is kept; re-running this script resumes safely"
  exit 1
fi

current="$(installed_version || true)"
if [ -z "$current" ] || ! is_supported "$current"; then
  say "error: installation finished but marker-pdf ${REQUIRED_MAJOR}.x / marker_single was not found (got '${current:-none}')"
  exit 1
fi

say "installed: marker-pdf $current"
print_usage_hint
