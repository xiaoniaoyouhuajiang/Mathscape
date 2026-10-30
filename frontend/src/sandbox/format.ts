/** Pure display helpers for the sandbox panel (kept DOM-free for unit tests). */

export type SessionState = "active" | "exported" | "discarded";

export const STATE_LABEL: Record<SessionState, string> = {
  active: "进行中",
  exported: "已导出",
  discarded: "已丢弃",
};

/**
 * Compact timestamp for one-line summaries: `2026-10-30T22:56:46.409+08:00`
 * → `10-30 22:56`. The backend already emits local-offset RFC 3339, so this
 * only trims; anything unparseable is returned unchanged.
 */
export function shortTimestamp(value: string): string {
  const match = /^\d{4}-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(value);
  return match ? `${match[1]}-${match[2]} ${match[3]}:${match[4]}` : value;
}

/** Readable full timestamp: `2026-10-30 22:56:46`. */
export function fullTimestamp(value: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})/.exec(value);
  return match ? `${match[1]} ${match[2]}` : value;
}

/** Details default: live sessions open, discarded ones collapsed. */
export function sessionOpenByDefault(state: SessionState): boolean {
  return state !== "discarded";
}
