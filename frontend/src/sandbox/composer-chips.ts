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
import { domainKindLabel } from "../graph/taxonomy";
import { citeChipParts, scanIds, shortCite, type CiteIndex, type CiteResolver, type CiteStatus, type CiteTarget } from "./cite";

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

// ---------------------------------------------------------------------------
// @ mentions: suggestions from the current landscape

/** The map nodes the `@` menu offers: one per domain id, and only ids
 * `scanIds` recognizes (canonical `mathscape:` or bare 32/64 hex), because
 * the inserted id has to chip exactly like a pasted one. */
export function mentionTargets(index: CiteIndex | null | undefined): CiteTarget[] {
  if (!index) return [];
  const seen = new Map<string, CiteTarget>();
  for (const targets of index.values()) {
    for (const target of targets) {
      if (seen.has(target.domainId)) continue;
      const hits = scanIds(target.domainId);
      if (hits.length === 1 && hits[0].start === 0 && hits[0].end === target.domainId.length) seen.set(target.domainId, target);
    }
  }
  return [...seen.values()];
}

export interface MentionSuggestion {
  target: CiteTarget;
  /** Visible label (falls back to the short id). */
  label: string;
  /** Type label (中文 kind) for the second line. */
  kind: string;
  /** `kind/head…tail`, the same short id chips show on hover. */
  shortId: string;
}

/**
 * Filter and rank `targets` for the text typed after `@`. Every
 * whitespace-separated keyword must hit the label, the short or full id, or
 * the type (raw `domain_kind` or its 中文 label), case-insensitively. Label
 * prefix hits rank first, then label substring, type, id; ties by label.
 */
export function mentionSuggestions(targets: readonly CiteTarget[], query: string, limit = 20): MentionSuggestion[] {
  const keywords = query.toLowerCase().split(/\s+/).filter(Boolean);
  const scored: { item: MentionSuggestion; score: number }[] = [];
  for (const target of targets) {
    const item: MentionSuggestion = {
      target,
      label: target.label || shortCite(target.domainId),
      kind: target.domainKind ? domainKindLabel(target.domainKind) : "",
      shortId: shortCite(target.domainId),
    };
    const label = item.label.toLowerCase();
    const kinds = `${target.domainKind ?? ""} ${item.kind}`.toLowerCase();
    const ids = `${item.shortId} ${target.domainId}`.toLowerCase();
    let score = 0;
    let ok = true;
    for (const word of keywords) {
      if (label.startsWith(word)) score += 0;
      else if (label.includes(word)) score += 1;
      else if (kinds.includes(word)) score += 2;
      else if (ids.includes(word)) score += 3;
      else {
        ok = false;
        break;
      }
    }
    if (ok) scored.push({ item, score });
  }
  scored.sort((a, b) => a.score - b.score || a.item.label.localeCompare(b.item.label, "zh-Hans-CN") || a.item.target.domainId.localeCompare(b.item.target.domainId));
  return scored.slice(0, limit).map((entry) => entry.item);
}

/** An open `@` mention: the `@` at `start` and the query up to the caret. */
export interface MentionTrigger {
  start: number;
  query: string;
}

const MENTION_QUERY_MAX = 40;

/**
 * The `@` mention the caret is in, given the composer's expanded text and
 * caret offset. The `@` must start the text or follow a character that is
 * not part of an email-ish word (so `a@b.com` does not open the menu; `看看@`
 * does). The query runs to the caret, stays on one line, has no other `@`,
 * does not span a chip (`blocked` = expanded ranges of chips) and is short.
 */
export function mentionTrigger(text: string, caret: number, blocked: readonly [number, number][] = []): MentionTrigger | null {
  if (caret < 1 || caret > text.length) return null;
  const from = Math.max(0, caret - MENTION_QUERY_MAX - 1);
  const start = text.lastIndexOf("@", caret - 1);
  if (start < from || start < 0) return null;
  const query = text.slice(start + 1, caret);
  if (/[\n@]/.test(query) || /\s\s/.test(query)) return null;
  const before = start > 0 ? text[start - 1] : "";
  if (before && /[A-Za-z0-9._%+-]/.test(before)) return null;
  if (blocked.some(([a, b]) => a < caret && b > start)) return null;
  return { start, query };
}
