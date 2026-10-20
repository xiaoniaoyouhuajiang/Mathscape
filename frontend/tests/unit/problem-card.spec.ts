import { describe, expect, it } from "vitest";
import { problemCardMarkdown } from "../../src/ink/problem-card";

describe("problem card Markdown", () => {
  it("renders paragraphs, emphasis, inline and display math", () => {
    const html = problemCardMarkdown(String.raw`## 求极限

设 **数列** 满足 $a_n=\frac{1}{n}$。

\[
\lim_{n\to\infty} a_n=0
\]`);
    expect(html).toContain("<h2>求极限</h2>");
    expect(html).toContain("<strong>数列</strong>");
    expect(html).toContain('class="katex"');
    expect(html).toContain('class="katex-display"');
    expect(html).toContain("<mfrac>");
    expect(html).not.toContain("katex-error");
  });

  it("keeps source HTML inert and does not load images or navigate links", () => {
    const html = problemCardMarkdown('<script>alert(1)</script>\n\n![图](https://example.com/image.png) [参考](https://example.com)');
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<a ");
    expect(html).toContain("参考");
  });
});
