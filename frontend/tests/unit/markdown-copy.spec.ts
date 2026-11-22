import { describe, expect, it } from "vitest";
import { joinBlocksMarkdown } from "../../src/reader/markdown-copy";

describe("markdown-copy pure helpers", () => {
  it("joins multiple Markdown blocks with blank lines while trimming trailing newlines and empty entries", () => {
    expect(
      joinBlocksMarkdown([
        "# 第一节 母函数\n\n",
        "   ",
        "设序列 $a_0, a_1, \\dots$ 的母函数为\r\n",
        "$$\nA(x) = \\sum_{n=0}^{\\infty} a_n x^n.\n$$\n",
      ]),
    ).toBe(
      "# 第一节 母函数\n\n设序列 $a_0, a_1, \\dots$ 的母函数为\n\n$$\nA(x) = \\sum_{n=0}^{\\infty} a_n x^n.\n$$",
    );
  });

  it("returns empty string when all blocks are blank", () => {
    expect(joinBlocksMarkdown(["", "\n\n", "  \t  "])).toBe("");
  });
});
