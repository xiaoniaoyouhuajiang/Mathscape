/**
 * Composer chips: the pure half (no DOM).
 *
 * The dialogue input shows ids the current landscape (or this session's tool
 * results) knows as short-label chips, the same chips A-PR3 renders in reply
 * bubbles. A chip keeps the full id; sending expands every chip back to it,
 * so the agent always receives the exact string the user pasted or typed.
 *
 * Recognition is `scanIds` + `CiteResolver` (cite.ts): the landscape index
 * plus the per-round tool-context index. No NLP or fuzzy matching. An id
 * that resolves to nothing (`unknown`) stays plain text in the composer.
 */
import { citeChipParts, scanIds, type CiteResolver, type CiteStatus } from "./cite";

export interface ComposerChip {
  /** Full id as written (what the agent receives). */
  id: string;
  status: Exclude<CiteStatus, "unknown">;
  text: string;
  title: string;
  statusClass: string;
  /** Projection node a click selects, when the map has a target. */
  jumpNode?: string;
}

export type ComposerSegment = { kind: "text"; text: string } | { kind: "chip"; chip: ComposerChip };

/** Chip for one id, or null when the indexes do not know it (stays text). */
export function composerChip(raw: string, resolve: CiteResolver | undefined): ComposerChip | null {
  if (!resolve) return null;
  const cite = resolve(raw);
  if (cite.status === "unknown") return null;
  const parts = citeChipParts(cite);
  // A label-only resolution without a label renders as the grey unknown chip
  // in replies; in the composer that is just text.
  if (parts.statusClass === "is-unknown") return null;
  return { id: raw, status: cite.status, text: parts.text, title: parts.title, statusClass: parts.statusClass, jumpNode: parts.jumpNode };
}

export interface TokenizeOptions {
  /**
   * Caret offset in `text` while the user is typing: an id ending exactly
   * here may still be growing (a 32-hex prefix of a 64-hex hash, a canonical
   * id mid-hash), so it stays text until a delimiter follows or the caret
   * leaves.
   */
  openAt?: number;
  /** Start offsets of ids that already are chips: `openAt` never dissolves
   * an existing chip (deleting the space after one must not un-chip it). */
  chipStarts?: ReadonlySet<number>;
}

/** Split `text` into plain runs and chips for every known id. */
export function tokenizeComposer(text: string, resolve: CiteResolver | undefined, options: TokenizeOptions = {}): ComposerSegment[] {
  const out: ComposerSegment[] = [];
  let at = 0;
  const pushText = (value: string) => {
    if (!value) return;
    const last = out[out.length - 1];
    if (last?.kind === "text") last.text += value;
    else out.push({ kind: "text", text: value });
  };
  for (const match of scanIds(text)) {
    if (options.openAt !== undefined && match.end === options.openAt && !options.chipStarts?.has(match.start)) continue;
    const chip = composerChip(match.raw, resolve);
    if (!chip) continue;
    pushText(text.slice(at, match.start));
    out.push({ kind: "chip", chip });
    at = match.end;
  }
  pushText(text.slice(at));
  return out;
}

/** What `send` hands the agent: chips expanded back to their full ids. */
export function expandComposer(segments: readonly ComposerSegment[]): string {
  return segments.map((segment) => (segment.kind === "chip" ? segment.chip.id : segment.text)).join("");
}
