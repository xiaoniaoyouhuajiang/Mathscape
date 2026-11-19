/** Pure reconciliation and grouping for InkPad local drafts.
 *
 * Safety invariants:
 * - Any draft with `pendingCount > 0` is NEVER auto-pruned and NEVER included in `cleanableStaleKeys`.
 * - The currently mounted `activeKey` is NEVER auto-pruned and NEVER included in `cleanableStaleKeys`.
 * - For a given `documentId`, older `writerEpoch` duplicates with `pendingCount === 0` are marked
 *   for safe auto-pruning once superseded by a newer epoch.
 */

export interface PadDraftSummary {
  key: string;
  documentId: string;
  title: string;
  writerEpoch: number;
  updatedAt: string;
  pendingCount: number;
  needsReauth: boolean;
}

export interface PadDraftReconciliation {
  /** Superseded older-epoch entries with zero pending operations that can be auto-pruned. */
  pruneKeys: string[];
  /** Non-active read-only (`needsReauth`) entries with zero pending operations eligible for one-tap cleanup. */
  cleanableStaleKeys: string[];
  /** Usable / resumable drafts (`!needsReauth`), newest first. */
  active: PadDraftSummary[];
  /** Revoked / expired drafts that still carry unsynced local operations (`pendingCount > 0`). */
  pendingOffline: PadDraftSummary[];
  /** Remaining read-only history shells (`needsReauth && pendingCount === 0`), newest first. */
  readonlyHistory: PadDraftSummary[];
}

function compareByEpochDesc(a: PadDraftSummary, b: PadDraftSummary): number {
  if (a.writerEpoch !== b.writerEpoch) return b.writerEpoch - a.writerEpoch;
  const updated = b.updatedAt.localeCompare(a.updatedAt);
  if (updated !== 0) return updated;
  if (a.needsReauth !== b.needsReauth) return a.needsReauth ? 1 : -1;
  return a.key.localeCompare(b.key);
}

function compareByUpdatedDesc(a: PadDraftSummary, b: PadDraftSummary): number {
  const updated = b.updatedAt.localeCompare(a.updatedAt);
  if (updated !== 0) return updated;
  if (a.writerEpoch !== b.writerEpoch) return b.writerEpoch - a.writerEpoch;
  return a.key.localeCompare(b.key);
}

export function reconcilePadDrafts(
  entries: readonly PadDraftSummary[],
  activeKey?: string,
): PadDraftReconciliation {
  const byDoc = new Map<string, PadDraftSummary[]>();
  for (const entry of entries) {
    const bucket = byDoc.get(entry.documentId);
    if (bucket) bucket.push(entry);
    else byDoc.set(entry.documentId, [entry]);
  }

  const pruneKeys: string[] = [];
  const kept: PadDraftSummary[] = [];

  for (const group of byDoc.values()) {
    group.sort(compareByEpochDesc);
    const canonical = group[0];
    kept.push(canonical);
    for (let i = 1; i < group.length; i++) {
      const older = group[i];
      if (older.pendingCount === 0 && older.key !== activeKey) {
        pruneKeys.push(older.key);
      } else {
        kept.push(older);
      }
    }
  }

  const active: PadDraftSummary[] = [];
  const pendingOffline: PadDraftSummary[] = [];
  const readonlyHistory: PadDraftSummary[] = [];
  const cleanableStaleKeys: string[] = [];

  for (const entry of kept) {
    if (!entry.needsReauth) {
      active.push(entry);
    } else if (entry.pendingCount > 0) {
      pendingOffline.push(entry);
    } else {
      readonlyHistory.push(entry);
      if (entry.key !== activeKey) {
        cleanableStaleKeys.push(entry.key);
      }
    }
  }

  active.sort(compareByUpdatedDesc);
  pendingOffline.sort(compareByUpdatedDesc);
  readonlyHistory.sort(compareByUpdatedDesc);

  return {
    pruneKeys,
    cleanableStaleKeys,
    active,
    pendingOffline,
    readonlyHistory,
  };
}

export function formatPadDraftLabel(entry: PadDraftSummary): string {
  if (!entry.needsReauth) {
    return entry.pendingCount > 0
      ? `${entry.title} · 最近手稿（待同步 ${entry.pendingCount}）`
      : `${entry.title} · 最近手稿`;
  }
  if (entry.pendingCount > 0) {
    return `${entry.title} · 待同步 ${entry.pendingCount}（需导出恢复）`;
  }
  return `${entry.title} · 仅本机历史（需电脑重新推送）`;
}
