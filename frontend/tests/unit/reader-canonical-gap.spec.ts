import { describe, expect, it } from "vitest";
import { elapsedLabel, failureText, gapCardState, gapParseActions, mentionsHuggingFace, type GapParseJob, type ParseLease } from "../../src/reader/canonical-gap";

const job = (from: number, to: number, extra: Partial<GapParseJob> = {}): GapParseJob =>
  ({ canonicalId: "c", pdfId: "pdf", pages: { from, to }, startedAt: 1000, ...extra });
const lease = (from: number, to: number, pdfId = "pdf"): ParseLease => ({ pdfId, pages: { from, to }, startedAt: 500 });

describe("gapParseActions", () => {
  const pages = (from: number, to: number) => gapParseActions({ from, to }).map(action => `${action.end} ${action.pages.from}-${action.pages.to}`);

  it("offers a small gap whole, up to two pages past one step", () => {
    expect(pages(7, 7)).toEqual(["whole 7-7"]);
    expect(pages(3, 6)).toEqual(["whole 3-6"]);
    expect(pages(7, 11)).toEqual(["whole 7-11"]);
    expect(pages(3, 8)).toEqual(["whole 3-8"]);
  });

  it("offers four pages at each end of a larger gap", () => {
    expect(gapParseActions({ from: 21, to: 44 })).toEqual([
      { end: "head", pages: { from: 21, to: 24 } },
      { end: "tail", pages: { from: 41, to: 44 } },
    ]);
    expect(pages(1, 8)).toEqual(["head 1-4", "tail 5-8"]);
  });

  it("never offers the same page at both ends", () => {
    expect(pages(1, 7)).toEqual(["head 1-4", "tail 5-7"]);
  });

  it("never leaves one or two pages behind", () => {
    for (let length = 1; length <= 40; length += 1) {
      for (const action of gapParseActions({ from: 1, to: length })) {
        const left = length - (action.pages.to - action.pages.from + 1);
        expect(left === 0 || left >= 3, `${length} pages, ${action.end}`).toBe(true);
      }
    }
  });
});

describe("gapCardState", () => {
  const gap = { from: 3, to: 12 };

  it("is idle, and busy while another gap of the view is being parsed", () => {
    expect(gapCardState(gap, "pdf", undefined, [])).toEqual({ kind: "idle", busy: false });
    expect(gapCardState(gap, "pdf", job(15, 16), [])).toEqual({ kind: "idle", busy: true });
    expect(gapCardState(gap, "pdf", job(15, 16, { failure: { status: "failed", timedOut: false, detail: "" } }), []))
      .toEqual({ kind: "idle", busy: false });
  });

  it("follows its own parse from asked to running", () => {
    expect(gapCardState(gap, "pdf", job(9, 12), [])).toEqual({ kind: "starting", pages: { from: 9, to: 12 }, startedAt: 1000 });
    expect(gapCardState(gap, "pdf", job(9, 12), [lease(9, 12)]))
      .toEqual({ kind: "running", pages: { from: 9, to: 12 }, startedAt: 500, own: true });
  });

  it("waits behind a parse that holds Marker", () => {
    expect(gapCardState(gap, "pdf", job(9, 12), [lease(20, 23)])).toMatchObject({ kind: "queued", samePdf: true });
    expect(gapCardState(gap, "pdf", job(9, 12), [lease(1, 4, "other")])).toMatchObject({ kind: "queued", samePdf: false });
  });

  it("shows pages someone else is parsing in this gap", () => {
    expect(gapCardState(gap, "pdf", undefined, [lease(5, 8)]))
      .toEqual({ kind: "running", pages: { from: 5, to: 8 }, startedAt: 500, own: false });
    expect(gapCardState(gap, "pdf", undefined, [lease(5, 8, "other")])).toEqual({ kind: "idle", busy: false });
  });

  it("keeps a failure on the gap it was asked for", () => {
    const failure = { status: "failed" as const, timedOut: false, detail: "boom" };
    expect(gapCardState(gap, "pdf", job(9, 12, { failure }), [])).toEqual({ kind: "failed", pages: { from: 9, to: 12 }, failure });
  });
});

describe("failure wording", () => {
  const pages = { from: 9, to: 12 };

  it("says what happened without the backend's wording", () => {
    expect(failureText({ status: "failed", timedOut: true, detail: "" }, pages)).toContain("超时");
    expect(failureText({ status: "failed", timedOut: false, detail: "Traceback" }, pages)).not.toContain("Traceback");
    expect(failureText({ status: "blocked_budget", timedOut: false, maxPages: 12, detail: "" }, pages)).toContain("12");
    expect(failureText({ status: "capability_unavailable", timedOut: false, detail: "" }, pages)).toContain("Marker");
    expect(failureText({ status: "lineage_mismatch", timedOut: false, detail: "" }, pages)).toContain("版本");
  });

  it("recognises a Hugging Face failure in Marker's output", () => {
    expect(mentionsHuggingFace("Warning: You are sending unauthenticated requests to the HF Hub.")).toBe(true);
    expect(mentionsHuggingFace("huggingface_hub.errors.LocalEntryNotFoundError")).toBe(true);
    expect(mentionsHuggingFace("CUDA out of memory")).toBe(false);
  });
});

describe("elapsedLabel", () => {
  it("counts minutes and seconds", () => {
    expect(elapsedLabel(0, 0)).toBe("0:00");
    expect(elapsedLabel(0, 65_400)).toBe("1:05");
    expect(elapsedLabel(10_000, 0)).toBe("0:00");
  });
});
