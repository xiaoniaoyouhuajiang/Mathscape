import { describe, expect, it } from "vitest";
import {
  extractAnchorPhrase,
  findSpanMatchesInPage,
  normalizeSearchText,
  readPageTextStrings,
} from "../../src/reader/pdf-text-find";

describe("pdf-text-find pure helpers", () => {
  it("normalizes whitespace, width variants, and case", () => {
    expect(normalizeSearchText("  Theorem  １  Generating\nFunction ")).toBe("theorem1generatingfunction");
  });

  it("extracts a clean human phrase from Markdown with LaTeX and page markers", () => {
    const md = "{1}------------------------\n\n**定理 1（无穷数列的母函数）** 设 $\\{a_n\\}_{n=0}^\\infty$ 为数列，有 $$A(x) = \\sum a_n x^n$$。";
    const phrase = extractAnchorPhrase(md);
    expect(phrase).toContain("定理 1");
    expect(phrase).not.toContain("$");
    expect(phrase).not.toContain("*");
  });

  it("matches phrases across split PDF textLayer spans", () => {
    const spans = ["第二章", " ", "无穷数列", "的", "母函数", "；以及另一处母函数定义"];
    const hits = findSpanMatchesInPage(spans, "数列的母函数");
    expect(hits).toEqual([{ spanIndices: [2, 3, 4] }]);

    const multi = findSpanMatchesInPage(spans, "母函数");
    expect(multi).toEqual([{ spanIndices: [4] }, { spanIndices: [5] }]);
  });
});

describe("readPageTextStrings", () => {
  it("reads every chunk without ReadableStream async iteration (WebKitGTK)", async () => {
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue({ items: [{ str: "Trace-based" }, { type: "beginMarkedContent" }] });
        controller.enqueue({ items: [{ str: " JIT" }] });
        controller.close();
      },
    });
    Object.defineProperty(stream, Symbol.asyncIterator, { value: undefined });
    const page = { streamTextContent: () => stream } as unknown as Parameters<typeof readPageTextStrings>[0];

    const strs = await readPageTextStrings(page);
    expect(strs).toEqual(["Trace-based", "", " JIT"]);
    expect(findSpanMatchesInPage(strs, "trace")).toHaveLength(1);
  });
});
