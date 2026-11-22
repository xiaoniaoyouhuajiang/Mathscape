import { invoke } from "@tauri-apps/api/core";
import { eventBus } from "../core/events";
import { errorMessage } from "../core/ipc";
import { t } from "../i18n";
import { gapLabel, pagesLabel, type PageRange } from "./canonical";

/**
 * 「解析这几页」 on a gap of the 「规范解析正文」 view.
 *
 * A gap is usually far larger than what a reader needs next (tens of pages,
 * about half a minute of Marker each), so the card offers the few pages at
 * the end the reader arrived from, and the original PDF pages to read while
 * Marker runs. The backend (`parse_canonical_pages`) is the same ensure path
 * an agent's `material.read` uses: only pages the view lacks are parsed, one
 * parse per PDF and one Marker per workspace at a time.
 */
export const GAP_PARSE_PAGES = 4;
/** A gap this much longer than one step is still parsed whole. */
const GAP_WHOLE_SLACK = 2;

export type GapEnd = "whole" | "head" | "tail";
export interface GapParseAction { end: GapEnd; pages: PageRange }

/**
 * The whole gap when it is small, otherwise the pages at each of its ends.
 * Whole reaches a little past one step so that an end never leaves one or two
 * pages behind (a Marker start and a snapshot of their own), and the two ends
 * never offer the same page.
 */
export function gapParseActions(gap: PageRange, chunk = GAP_PARSE_PAGES): GapParseAction[] {
  if (gap.to - gap.from + 1 <= chunk + GAP_WHOLE_SLACK) return [{ end: "whole", pages: gap }];
  const head = { from: gap.from, to: gap.from + chunk - 1 };
  return [
    { end: "head", pages: head },
    { end: "tail", pages: { from: Math.max(gap.to - chunk + 1, head.to + 1), to: gap.to } },
  ];
}

export type GapParseStatus =
  | "expanded" | "reused" | "blocked_budget" | "capability_unavailable" | "failed" | "lineage_mismatch";

export interface GapParseReport {
  status: GapParseStatus;
  requested: PageRange;
  missing: PageRange[];
  marker_ranges?: PageRange[];
  waited_ms: number;
  timed_out?: boolean;
  max_pages_per_request?: number;
  message: string;
}

/** A live parse of the workspace, whoever started it, on the local clock. */
export interface ParseLease { pdfId: string; pages: PageRange; startedAt: number }

interface WireLeases {
  now_ms: number;
  leases: { source_pdf_artifact_id: string; pages: PageRange; started_at_ms: number }[];
}

export async function canonicalParseLeases(): Promise<ParseLease[]> {
  const wire = await invoke<WireLeases>("get_canonical_parse_leases");
  const now = Date.now();
  return wire.leases.map(lease => ({
    pdfId: lease.source_pdf_artifact_id,
    pages: lease.pages,
    startedAt: now - Math.max(0, wire.now_ms - lease.started_at_ms),
  }));
}

export interface GapParseFailure {
  status: GapParseStatus | "error";
  timedOut: boolean;
  maxPages?: number;
  /** Backend wording, kept for 「技术详情」. */
  detail: string;
}

export interface GapParseJob {
  canonicalId: string;
  pdfId: string;
  pages: PageRange;
  startedAt: number;
  /** Set once the call ended without the pages; cleared by retry or dismiss. */
  failure?: GapParseFailure;
}

export interface GapParseEvent { canonicalId: string; filled: boolean }

// Outlives the reader document: a parse keeps running while the reader shows
// the PDF page or another file, and its card is restored on return.
const jobs = new Map<string, GapParseJob>();
const listeners = new Set<(event: GapParseEvent) => void>();

function publish(event: GapParseEvent): void {
  for (const listener of [...listeners]) listener(event);
}

export function gapParseJob(canonicalId: string): GapParseJob | undefined {
  return jobs.get(canonicalId);
}

export function onGapParseChange(listener: (event: GapParseEvent) => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function dismissGapParse(canonicalId: string): void {
  if (jobs.get(canonicalId)?.failure) jobs.delete(canonicalId);
}

/** Starts the parse unless this view already has one running. */
export function startGapParse(canonicalId: string, pdfId: string, pages: PageRange): boolean {
  const running = jobs.get(canonicalId);
  if (running && !running.failure) return false;
  const job: GapParseJob = { canonicalId, pdfId, pages, startedAt: Date.now() };
  jobs.set(canonicalId, job);
  void (async () => {
    let filled = false;
    try {
      const report = await invoke<GapParseReport>("parse_canonical_pages", {
        request: { canonical_id: canonicalId, first_page: pages.from, last_page: pages.to },
      });
      filled = report.status === "expanded" || Boolean(report.marker_ranges?.length);
      if (report.status === "expanded" || report.status === "reused") {
        if (jobs.get(canonicalId) === job) jobs.delete(canonicalId);
      } else {
        job.failure = {
          status: report.status,
          timedOut: Boolean(report.timed_out),
          maxPages: report.max_pages_per_request,
          detail: report.message,
        };
      }
    } catch (error) {
      job.failure = { status: "error", timedOut: false, detail: errorMessage(error) };
    }
    // New snapshots are new map nodes; the reader stays open on its document.
    if (filled) eventBus.emit("source-map-changed", { reason: "canonical-pages-parsed", preserveReader: true });
    publish({ canonicalId, filled });
  })();
  return true;
}

export type GapCardState =
  | { kind: "idle"; busy: boolean }
  /** This reader asked; Marker has not taken the lease yet. */
  | { kind: "starting"; pages: PageRange; startedAt: number }
  | { kind: "queued"; pages: PageRange; behind: ParseLease; samePdf: boolean }
  /** `own` false: an agent or another window is parsing these pages. */
  | { kind: "running"; pages: PageRange; startedAt: number; own: boolean }
  | { kind: "failed"; pages: PageRange; failure: GapParseFailure };

const overlaps = (a: PageRange, b: PageRange): boolean => a.from <= b.to && b.from <= a.to;

export function gapCardState(
  gap: PageRange,
  pdfId: string,
  job: GapParseJob | undefined,
  leases: readonly ParseLease[],
): GapCardState {
  const mine = job && overlaps(job.pages, gap) ? job : undefined;
  const live = leases.find(lease => lease.pdfId === pdfId && overlaps(lease.pages, gap));
  if (live) {
    const own = Boolean(job && !job.failure && overlaps(job.pages, live.pages));
    if (own || !mine || mine.failure) return { kind: "running", pages: live.pages, startedAt: live.startedAt, own };
  }
  if (mine?.failure) return { kind: "failed", pages: mine.pages, failure: mine.failure };
  if (mine) {
    const behind = leases.find(lease => lease.pdfId === pdfId) ?? leases[0];
    return behind
      ? { kind: "queued", pages: mine.pages, behind, samePdf: behind.pdfId === pdfId }
      : { kind: "starting", pages: mine.pages, startedAt: mine.startedAt };
  }
  return { kind: "idle", busy: Boolean(job && !job.failure) };
}

/** Marker reached for the Hugging Face hub in this run (its stderr says so). */
export function mentionsHuggingFace(detail: string): boolean {
  return /hugging\s*face|\bhf[ _]hub\b|huggingface_hub|\bHF_TOKEN\b/i.test(detail);
}

export function failureText(failure: GapParseFailure, pages: PageRange): string {
  const range = pagesLabel(pages);
  switch (failure.status) {
    case "capability_unavailable": return t("reader.gap.failed.noMarker");
    case "blocked_budget": return failure.maxPages
      ? t("reader.gap.failed.tooMany", { max: failure.maxPages })
      : t("reader.gap.failed.budget");
    case "lineage_mismatch": return t("reader.gap.failed.lineage");
    case "error": return t("reader.gap.failed.error", { range, detail: failure.detail });
    default: return failure.timedOut
      ? t("reader.gap.failed.timedOut", { range })
      : t("reader.gap.failed.marker", { range });
  }
}

export function elapsedLabel(startedAt: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export interface GapCardModel {
  gap: PageRange;
  /** Parsed text right above / below the gap: which end continues what. */
  hasBefore: boolean;
  hasAfter: boolean;
  state: GapCardState;
  onParse: (pages: PageRange) => void;
  onDismiss: () => void;
  /** Zero-based PDF page to show while the pages are not parsed. */
  onOpenPdf: (pageIndex: number) => void;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text = ""): HTMLElementTagNameMap[K] {
  const result = document.createElement(tag);
  result.className = className;
  result.textContent = text;
  return result;
}

function button(className: string, text: string, onClick: () => void): HTMLButtonElement {
  const result = node("button", className, text);
  result.type = "button";
  result.onclick = onClick;
  return result;
}

/** Fills `card` for `model`; called again whenever the parse state changes. */
export function renderGapCard(card: HTMLElement, model: GapCardModel): void {
  const { gap, state } = model;
  card.className = "canonical-gap";
  card.setAttribute("role", "group");
  card.setAttribute("aria-label", gapLabel({ kind: "gap", pages: gap }));
  card.dataset.pages = `${gap.from}-${gap.to}`;
  card.dataset.state = state.kind;
  const active = state.kind === "starting" || state.kind === "queued" || state.kind === "running";
  const blocked = active || (state.kind === "idle" && state.busy);

  const row = (action: GapParseAction): HTMLElement => {
    const actions = node("div", "canonical-gap-actions");
    actions.dataset.end = action.end;
    const continues = action.end === "head" ? model.hasBefore : action.end === "tail" ? model.hasAfter : false;
    if (continues) {
      actions.append(node("span", "canonical-gap-caption", t(action.end === "head" ? "reader.gap.afterPrevious" : "reader.gap.beforeNext")));
    }
    const parse = button("canonical-gap-parse", t("reader.gap.parse", { range: pagesLabel(action.pages) }), () => model.onParse(action.pages));
    parse.disabled = blocked;
    parse.title = blocked ? t("reader.gap.parseBusyTitle") : t("reader.gap.parseTitle", { pages: action.pages.to - action.pages.from + 1 });
    // The page next to what is being read: the gap's last page from below, its first from above.
    const page = action.end === "tail" ? action.pages.to : action.pages.from;
    const pdf = button("canonical-gap-pdf", t("reader.gap.viewPdf", { page }), () => model.onOpenPdf(page - 1));
    pdf.title = t("reader.gap.viewPdfTitle");
    actions.append(parse, pdf);
    return actions;
  };

  const actions = gapParseActions(gap);
  const children: HTMLElement[] = [];
  const head = actions.find(action => action.end === "head");
  if (head) children.push(row(head));
  children.push(node("p", "canonical-gap-label", gapLabel({ kind: "gap", pages: gap })));

  const status = node("div", "canonical-gap-status");
  status.setAttribute("role", "status");
  if (state.kind === "starting") {
    status.append(node("span", "", t("reader.gap.starting", { range: pagesLabel(state.pages) })));
  } else if (state.kind === "queued") {
    status.append(node("span", "", state.samePdf
      ? t("reader.gap.queuedSamePdf", { range: pagesLabel(state.pages), behind: pagesLabel(state.behind.pages) })
      : t("reader.gap.queuedOtherPdf", { range: pagesLabel(state.pages) })));
  } else if (state.kind === "running") {
    const elapsed = node("span", "canonical-gap-elapsed", elapsedLabel(state.startedAt));
    elapsed.dataset.startedAt = String(state.startedAt);
    const line = node("span", "canonical-gap-progress");
    line.append(
      t(state.own ? "reader.gap.running" : "reader.gap.runningElsewhere", { range: pagesLabel(state.pages) }),
      t("reader.gap.elapsed"),
      elapsed,
      t("reader.gap.pace"),
    );
    status.append(line);
  } else if (state.kind === "failed") {
    card.classList.add("is-failed");
    status.append(node("span", "canonical-gap-error", failureText(state.failure, state.pages)));
    if (mentionsHuggingFace(state.failure.detail)) {
      status.append(node("span", "canonical-gap-hint", t("reader.gap.hfHint")));
    }
    const retry = button("canonical-gap-retry", t("reader.gap.retry"), () => model.onParse(state.pages));
    const dismiss = button("canonical-gap-dismiss", t("reader.gap.dismiss"), model.onDismiss);
    status.append(retry, dismiss);
    if (state.failure.status !== "error" && state.failure.detail) {
      const details = node("details", "canonical-gap-detail");
      details.append(node("summary", "", t("reader.gap.detail")), node("pre", "", state.failure.detail));
      status.append(details);
    }
  }
  if (status.childElementCount) children.push(status);

  for (const action of actions) if (action.end !== "head") children.push(row(action));
  card.replaceChildren(...children);
}

/** Advances the 「已用时」 of every running card under `root`. */
export function tickGapCards(root: HTMLElement, now = Date.now()): void {
  for (const elapsed of root.querySelectorAll<HTMLElement>(".canonical-gap-elapsed[data-started-at]")) {
    elapsed.textContent = elapsedLabel(Number(elapsed.dataset.startedAt), now);
  }
}
