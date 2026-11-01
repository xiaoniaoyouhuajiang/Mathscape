// Pure helpers for Mathscape stable ids (`mathscape:<kind>/<hash>`) and the
// opaque hex ids the stores hand out (32-hex note document_id, 64-hex hashes).
// Shared by discovery run labels, the review workspace and sandbox cite chips,
// so every surface shortens an id the same head…tail way.

export interface ParsedMathscapeId {
  /** `source-unit`, `artifact`, `ink-snapshot`, … */
  kind: string;
  /** Everything after `mathscape:<kind>/`. */
  hash: string;
}

const MATHSCAPE_ID = /^mathscape:([a-z0-9-]+)\/(.+)$/;

export function parseMathscapeId(value: string): ParsedMathscapeId | null {
  const match = MATHSCAPE_ID.exec(value);
  return match ? { kind: match[1], hash: match[2] } : null;
}

export interface ShortIdOptions {
  /** Ids up to this length are returned unchanged. */
  max?: number;
  head?: number;
  tail?: number;
}

/** `head…tail` once the id is longer than `max` (default 34 / 17 / 12, the
 * discovery run id style). Never loses the tail, so two ids that share a
 * prefix stay distinguishable. */
export function shortStableId(value: string, options: ShortIdOptions = {}): string {
  const { max = 34, head = 17, tail = 12 } = options;
  return value.length > max ? `${value.slice(0, head)}…${value.slice(-tail)}` : value;
}
