import { describe, expect, it } from "vitest";
import { wrapGraphLabel } from "../../src/graph/label-wrap";
import { texToPlain } from "../../src/graph/tex-plain";

const units = (text: string) => wrapGraphLabel(text).split(/[\s\u200b]+/u);

describe("graph label line-break opportunities", () => {
  it("keeps Latin words, references and formulae whole while allowing CJK wrapping", () => {
    const parts = units(texToPlain("前向证书：在共享 Cassini 中取 $F_{n+1}F_{n-1}-F_n^2=(-1)^{n+1}$，即得命题 2.4.3"));
    expect(parts).toContain("Cassini");
    expect(parts).toContain("Fₙ₊₁Fₙ₋₁-Fₙ²=(-1)ⁿ⁺¹，");
    expect(parts).toContain("2.4.3");
    expect(parts).toContain("向");
  });

  it("cannot orphan a closing bracket, comma or full stop, even after whitespace", () => {
    const parts = units("命题 2.4.3 Cassini（灯塔的写法 ）；精确说法：Fibonacci 数列（F₀=0 , F₁=1）。");
    expect(parts).toContain("Cassini");
    expect(parts).toContain("法）；");
    expect(parts).toContain("（F₀=0,");
    expect(parts).toContain("F₁=1）。");
    expect(parts.every(part => !/^[，。、；：！？）》」』】〕,.;:!?%)\]}]/u.test(part))).toBe(true);
    expect(parts.every(part => !/[（《「『【〔(\[{]$/u.test(part))).toBe(true);
  });

  it("preserves Chinese subscripts in formulas, English spaces and Unicode characters", () => {
    expect(units("取 n=n_灯塔+1，并用 fₖ=Fₖ 改写")).toContain("n=n_灯塔+1，");
    expect(wrapGraphLabel("Linear recurrence 𝔽ₙ")).toBe("Linear recurrence 𝔽ₙ");
    expect(wrapGraphLabel("标题\nCassini identity")).toBe("标\u200b题\nCassini identity");
  });
});
