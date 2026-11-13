import { describe, expect, it } from "vitest";
import {
  convertBareSubscripts,
  glueShortMath,
  graphLabelText,
  texMathToPlain,
  texToPlain,
  truncatePlain,
} from "../../src/graph/tex-plain";

/** 短公式去空格后的对照（k = n+1 → k=n+1）。 */
const g = (text: string) => text.replace(/ /g, "");

describe("texToPlain (graph labels)", () => {
  it("turns common commands into symbols and spaces relations", () => {
    expect(texToPlain("对一切 $k\\ge 4$，面积差恰为 $\\pm 1$")).toBe(`对一切 ${g("k ≥ 4")}，面积差恰为 ±1`);
    expect(texMathToPlain("n \\to \\infty")).toBe("n → ∞");
    expect(texMathToPlain("x \\in \\mathbb{Z}")).toBe("x ∈ ℤ");
    expect(texMathToPlain("\\frac{1}{2}")).toBe("1/2");
    expect(texMathToPlain("\\frac{a+b}{2}")).toBe("(a+b)/2");
    expect(texMathToPlain("\\sqrt{5}")).toBe("√5");
    expect(texMathToPlain("\\text{若 } n")).toBe("若 n");
  });

  it("uses Unicode sub/superscripts when every character has one", () => {
    expect(texToPlain("$F_{k+1}F_{k-1}-F_k^2=(-1)^k$")).toBe(g("Fₖ₊₁Fₖ₋₁-Fₖ² = (-1)ᵏ"));
    expect(texToPlain("$f_k = F_k$")).toBe(g("fₖ = Fₖ"));
    expect(texToPlain("$(F_n)_{n\\ge 0}$")).toBe(`(Fₙ)_(${g("n ≥ 0")})`);
    expect(texToPlain("$Q^{n+1}$")).toBe("Qⁿ⁺¹");
  });

  it("converts bare f_k = F_k and glues k = n+1 with non-breaking spaces", () => {
    expect(texToPlain("前向证书：在共享公式中取 n = n_灯塔 + 1，并用 f_k = F_k 改写，即得灯塔的命题 2.4.2"))
      .toBe(`前向证书：在共享公式中取 ${g("n = n_灯塔 + 1")}，并用 ${g("fₖ = Fₖ")} 改写，即得灯塔的命题 2.4.2`);
    expect(texToPlain("说的就是它：灯塔的 n 对应题型的 k = n+1"))
      .toBe(`说的就是它：灯塔的 n 对应题型的 ${g("k = n+1")}`);
    expect(texToPlain("k = n+1").includes(" ")).toBe(false);
    expect(texToPlain("k = n+1")).toBe(g("k = n+1"));
    expect(convertBareSubscripts("f_k = F_k（k ≥ 1）")).toBe("fₖ = Fₖ（k ≥ 1）");
    expect(convertBareSubscripts("index_shift 和 n_灯塔")).toBe("index_shift 和 n_灯塔");
    expect(glueShortMath("灯塔的 n 对应题型的 k = n+1")).toBe(`灯塔的 n 对应题型的 ${g("k = n+1")}`);
  });

  it("leaves text without TeX untouched", () => {
    expect(texToPlain("价格 \\$5")).toBe("价格 $5");
  });

  it("reads an unterminated formula to the end (label cut mid-formula)", () => {
    expect(texToPlain("对一切 $k\\ge 4$，面积差恰为 $\\pm …")).toBe(`对一切 ${g("k ≥ 4")}，面积差恰为 ±…`);
    expect(texToPlain("证明 $f_k = F_k$ 对一切 $k\\ge…")).toBe(`证明 ${g("fₖ = Fₖ")} 对一切 k ≥…`);
  });
});

describe("truncatePlain", () => {
  it("counts characters, not code units, and adds an ellipsis", () => {
    expect(truncatePlain("Fₖ₊₁Fₖ₋₁", 20)).toBe("Fₖ₊₁Fₖ₋₁");
    expect(truncatePlain("一二三四五六", 4)).toBe("一二三…");
    expect(Array.from(truncatePlain("𝔽𝔽𝔽𝔽𝔽", 3))).toHaveLength(3);
  });
});

describe("graphLabelText", () => {
  it("recovers the full text from the summary, converts, then truncates on plain text", () => {
    const label = "一笔账：对一切 $k\\ge 4$，面积差恰为 $\\pm …";
    const summary = "还欠着 · 对一切 $k\\ge 4$，面积差恰为 $\\pm 1$，即 $F_{k+1}F_{k-1}-F_k^2=(-1)^k$";
    const text = graphLabelText(label, summary);
    expect(text.startsWith(`一笔账：对一切 ${g("k ≥ 4")}，面积差恰为 ±1，即 Fₖ₊₁`)).toBe(true);
    expect(text).not.toMatch(/[$\\]/);
    expect(graphLabelText(label, summary, 100)).toBe(`一笔账：对一切 ${g("k ≥ 4")}，面积差恰为 ±1，即 ${g("Fₖ₊₁Fₖ₋₁-Fₖ² = (-1)ᵏ")}`);
  });

  it("falls back to converting the label when the summary does not help", () => {
    expect(graphLabelText("一笔账：证明 $f_k = F_k$ 对一切 $k\\ge…", "挂在这个对象上"))
      .toBe(`一笔账：证明 ${g("fₖ = Fₖ")} 对一切 k ≥…`);
    expect(graphLabelText("概念：Fibonacci 数列", null)).toBe("概念：Fibonacci 数列");
  });
});
