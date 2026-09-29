import { describe, expect, it } from "vitest";
import {
  HIGHLIGHT_MARKERS,
  markdownPatchSource,
  removePatchMarkers,
} from "../../src/reader/markdown";

describe("markdown patch and anchor markers", () => {
  it("removes all patch and anchor markers completely", () => {
    const rawText = "This is normal text without any markers.";
    const textWithMarkers =
      "This is " +
      HIGHLIGHT_MARKERS.new.start +
      "inserted text" +
      HIGHLIGHT_MARKERS.new.end +
      " and " +
      HIGHLIGHT_MARKERS.old.start +
      "deleted text" +
      HIGHLIGHT_MARKERS.old.end +
      " and " +
      HIGHLIGHT_MARKERS.anchor.start +
      "anchored text" +
      HIGHLIGHT_MARKERS.anchor.end +
      ".";

    const cleaned = removePatchMarkers(textWithMarkers);
    expect(cleaned).toBe("This is inserted text and deleted text and anchored text.");
    expect(removePatchMarkers(rawText)).toBe(rawText);
  });

  it("injects markers wrapping plain text changes", () => {
    const before = "Hello, ";
    const changed = "beautiful world";
    const after = "!";

    const patched = markdownPatchSource(before, changed, after, "new");
    expect(patched).toBe(
      `Hello, ${HIGHLIGHT_MARKERS.new.start}beautiful world${HIGHLIGHT_MARKERS.new.end}!`,
    );
  });

  it("places markers inside inline math delimiters ($...$)", () => {
    const before = "Given that ";
    const formula = "$x^2 + y^2 = z^2$";
    const after = " holds true.";

    const patched = markdownPatchSource(before, formula, after, "new");
    // Markers must go INSIDE the $ delimiters, not outside
    expect(patched).toBe(
      `Given that $${HIGHLIGHT_MARKERS.new.start}x^2 + y^2 = z^2${HIGHLIGHT_MARKERS.new.end}$ holds true.`,
    );

    // Verify roundtrip cleanup restores the original formula intact
    expect(removePatchMarkers(patched)).toBe("Given that $x^2 + y^2 = z^2$ holds true.");
  });

  it("places markers inside display math delimiters ($$...$$)", () => {
    const before = "Consider the following formula:\n\n";
    const formula = "$$\\int_0^1 f(x) dx = 1$$";
    const after = "\n\nwhich completes the proof.";

    const patched = markdownPatchSource(before, formula, after, "old");
    // Markers must go INSIDE the $$ delimiters
    expect(patched).toBe(
      `Consider the following formula:\n\n$$${HIGHLIGHT_MARKERS.old.start}\\int_0^1 f(x) dx = 1${HIGHLIGHT_MARKERS.old.end}$$\n\nwhich completes the proof.`,
    );

    expect(removePatchMarkers(patched)).toBe(
      "Consider the following formula:\n\n$$\\int_0^1 f(x) dx = 1$$\n\nwhich completes the proof.",
    );
  });

  it("places markers inside LaTeX bracket delimiters (\\[...\\] and \\(...\\))", () => {
    const before = "Evaluate ";
    const formula1 = "\\[E = mc^2\\]";
    const patched1 = markdownPatchSource(before, formula1, "", "anchor");
    expect(patched1).toBe(
      `Evaluate \\[${HIGHLIGHT_MARKERS.anchor.start}E = mc^2${HIGHLIGHT_MARKERS.anchor.end}\\]`,
    );

    const formula2 = "\\(\\alpha + \\beta\\)";
    const patched2 = markdownPatchSource(before, formula2, "", "new");
    expect(patched2).toBe(
      `Evaluate \\(${HIGHLIGHT_MARKERS.new.start}\\alpha + \\beta${HIGHLIGHT_MARKERS.new.end}\\)`,
    );
  });
});
