#!/usr/bin/env bash
set -euo pipefail
: "${SPIKE_ROOT:?set SPIKE_ROOT to the disposable run directory}"
: "${OPENCODE_BINARY:?set OPENCODE_BINARY to the native opencode executable}"
: "${MATHSCAPE_BINARY:?set MATHSCAPE_BINARY to the built mathscape executable}"
# Model traffic shares the network namespace; shell is rejected by ACP policy.
args=(--die-with-parent --new-session --unshare-pid --unshare-ipc --unshare-uts
  --ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/sbin /sbin
  --symlink usr/lib /lib --symlink usr/lib64 /lib64
  --proc /proc --dev /dev --tmpfs /tmp --remount-ro /tmp --dir /etc
  --ro-bind /etc/ssl /etc/ssl --ro-bind /etc/resolv.conf /etc/resolv.conf
  --ro-bind /etc/hosts /etc/hosts --ro-bind /etc/nsswitch.conf /etc/nsswitch.conf
  --ro-bind /etc/passwd /etc/passwd
  --ro-bind "$OPENCODE_BINARY" "$OPENCODE_BINARY"
  --ro-bind "$MATHSCAPE_BINARY" "$MATHSCAPE_BINARY"
  --ro-bind "$SPIKE_ROOT/workspace" "$SPIKE_ROOT/workspace"
  --bind "$SPIKE_ROOT/workspace/scratch" "$SPIKE_ROOT/workspace/scratch"
  --ro-bind "$SPIKE_ROOT/round.txt" "$SPIKE_ROOT/round.txt"
  --setenv HOME "$SPIKE_ROOT/workspace/scratch/runtime"
  --setenv XDG_CONFIG_HOME "$SPIKE_ROOT/workspace/scratch/runtime/config"
  --setenv XDG_DATA_HOME "$SPIKE_ROOT/workspace/scratch/runtime/data"
  --setenv XDG_CACHE_HOME "$SPIKE_ROOT/workspace/scratch/runtime/cache"
  --setenv XDG_STATE_HOME "$SPIKE_ROOT/workspace/scratch/runtime/state"
  --setenv TMPDIR "$SPIKE_ROOT/workspace/scratch/runtime/tmp"
  --chdir "$SPIKE_ROOT/workspace")
exec /usr/bin/bwrap "${args[@]}" -- "$OPENCODE_BINARY" acp "$@"
