/**
 * 地图标签里的 TeX → 可读的 Unicode 纯文字。
 *
 * Cytoscape 的标签只能画纯文字，`$k\ge 4$` 这种原样画出来既难读，截断时还会
 * 切在公式中间。这里只做“看得懂”的近似：常见命令换成符号，简单的上下标换成
 * Unicode 上下标，其余去掉花括号保留内容。不追求排版正确，卡片上的公式仍走 KaTeX。
 */

const SYMBOLS: Record<string, string> = {
  ge: "≥", geq: "≥", geqslant: "≥", le: "≤", leq: "≤", leqslant: "≤", ne: "≠", neq: "≠",
  pm: "±", mp: "∓", cdot: "·", times: "×", div: "÷", to: "→", rightarrow: "→", leftarrow: "←",
  mapsto: "↦", Rightarrow: "⇒", implies: "⇒", iff: "⇔", Leftrightarrow: "⇔", infty: "∞",
  in: "∈", notin: "∉", subset: "⊂", subseteq: "⊆", cup: "∪", cap: "∩", forall: "∀", exists: "∃",
  sum: "∑", prod: "∏", int: "∫", approx: "≈", equiv: "≡", sim: "∼", cong: "≅", circ: "∘",
  ldots: "…", dots: "…", cdots: "⋯", mid: "∣", nmid: "∤", lfloor: "⌊", rfloor: "⌋",
  lceil: "⌈", rceil: "⌉", langle: "⟨", rangle: "⟩", emptyset: "∅", partial: "∂", nabla: "∇",
  alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ε", varepsilon: "ε", zeta: "ζ", eta: "η",
  theta: "θ", iota: "ι", kappa: "κ", lambda: "λ", mu: "μ", nu: "ν", xi: "ξ", pi: "π", rho: "ρ",
  sigma: "σ", tau: "τ", upsilon: "υ", phi: "φ", varphi: "φ", chi: "χ", psi: "ψ", omega: "ω",
  Gamma: "Γ", Delta: "Δ", Theta: "Θ", Lambda: "Λ", Xi: "Ξ", Pi: "Π", Sigma: "Σ", Phi: "Φ", Psi: "Ψ", Omega: "Ω",
  "{": "{", "}": "}", "%": "%", "$": "$", "&": "&", "#": "#", "_": "_",
};

const SPACES: Record<string, string> = { ",": " ", ";": " ", ":": " ", "!": "", quad: " ", qquad: " ", " ": " " };
/** 只留内容的命令：`\text{…}` → `…`。 */
const UNWRAP = new Set(["text", "mathrm", "mathit", "mathbf", "mathsf", "mathtt", "operatorname", "textbf", "textit", "boldsymbol", "mbox"]);
const DROP = new Set(["left", "right", "big", "Big", "bigl", "bigr", "Bigl", "Bigr", "displaystyle", "limits"]);
const BLACKBOARD: Record<string, string> = { N: "ℕ", Z: "ℤ", Q: "ℚ", R: "ℝ", C: "ℂ", P: "ℙ" };

const SUB: Record<string, string> = {
  "0": "₀", "1": "₁", "2": "₂", "3": "₃", "4": "₄", "5": "₅", "6": "₆", "7": "₇", "8": "₈", "9": "₉",
  "+": "₊", "-": "₋", "=": "₌", "(": "₍", ")": "₎", a: "ₐ", e: "ₑ", h: "ₕ", i: "ᵢ", j: "ⱼ", k: "ₖ",
  l: "ₗ", m: "ₘ", n: "ₙ", o: "ₒ", p: "ₚ", r: "ᵣ", s: "ₛ", t: "ₜ", u: "ᵤ", v: "ᵥ", x: "ₓ",
};
const SUP: Record<string, string> = {
  "0": "⁰", "1": "¹", "2": "²", "3": "³", "4": "⁴", "5": "⁵", "6": "⁶", "7": "⁷", "8": "⁸", "9": "⁹",
  "+": "⁺", "-": "⁻", "=": "⁼", "(": "⁽", ")": "⁾", a: "ᵃ", b: "ᵇ", c: "ᶜ", d: "ᵈ", e: "ᵉ", f: "ᶠ",
  g: "ᵍ", h: "ʰ", i: "ⁱ", j: "ʲ", k: "ᵏ", l: "ˡ", m: "ᵐ", n: "ⁿ", o: "ᵒ", p: "ᵖ", r: "ʳ", s: "ˢ",
  t: "ᵗ", u: "ᵘ", v: "ᵛ", w: "ʷ", x: "ˣ", y: "ʸ", z: "ᶻ", T: "ᵀ",
};

/** 关系符号两边补一个空格：`k≥4` → `k ≥ 4`。 */
const RELATIONS = /\s*([=≥≤≠<>≈≡→↦⇒⇔∈∉⊂⊆])\s*/g;

/** 读一个 TeX 参数：`{…}`（配平）或单个字符 / 单个命令。返回 [参数原文, 下一个位置]。 */
function readArg(src: string, at: number): [string, number] {
  let i = at;
  while (src[i] === " ") i++;
  if (i >= src.length) return ["", i];
  if (src[i] === "{") {
    let depth = 0;
    for (let j = i; j < src.length; j++) {
      if (src[j] === "{") depth++;
      else if (src[j] === "}" && --depth === 0) return [src.slice(i + 1, j), j + 1];
    }
    return [src.slice(i + 1), src.length];
  }
  if (src[i] === "\\") {
    const m = /^\\([A-Za-z]+|.)/.exec(src.slice(i));
    if (m) return [m[0], i + m[0].length];
  }
  const ch = Array.from(src.slice(i))[0];
  return [ch, i + ch.length];
}

function script(body: string, table: Record<string, string>, mark: "_" | "^"): string {
  const chars = Array.from(body.replace(/\s+/g, ""));
  if (chars.length && chars.every((ch) => table[ch] !== undefined)) return chars.map((ch) => table[ch]).join("");
  return chars.length === 1 ? `${mark}${body}` : `${mark}(${body})`;
}

/** 公式内部（不含 `$`）→ 纯文字。 */
export function texMathToPlain(src: string): string {
  let out = "";
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    if (ch === "\\") {
      const m = /^\\([A-Za-z]+|.)/.exec(src.slice(i));
      if (!m) { i++; continue; }
      const name = m[1];
      i += m[0].length;
      if (name in SYMBOLS) out += SYMBOLS[name];
      else if (name in SPACES) out += SPACES[name];
      else if (DROP.has(name)) { /* 去掉 */ }
      else if (UNWRAP.has(name)) { const [arg, next] = readArg(src, i); out += texMathToPlain(arg); i = next; }
      else if (name === "mathbb") { const [arg, next] = readArg(src, i); out += BLACKBOARD[arg.trim()] ?? texMathToPlain(arg); i = next; }
      else if (name === "frac" || name === "dfrac" || name === "tfrac") {
        const [a, n1] = readArg(src, i);
        const [b, n2] = readArg(src, n1);
        const wrap = (s: string) => (Array.from(s).length > 1 && /[^\w.]/u.test(s) ? `(${s})` : s);
        out += `${wrap(texMathToPlain(a))}/${wrap(texMathToPlain(b))}`;
        i = n2;
      } else if (name === "sqrt") {
        const [a, next] = readArg(src, i);
        const inner = texMathToPlain(a);
        out += Array.from(inner).length > 1 ? `√(${inner})` : `√${inner}`;
        i = next;
      } else out += name; // 不认识的命令：留下名字，比留下反斜杠好读
      continue;
    }
    if (ch === "_" || ch === "^") {
      const [arg, next] = readArg(src, i + 1);
      out += script(texMathToPlain(arg), ch === "_" ? SUB : SUP, ch);
      i = next;
      continue;
    }
    if (ch === "{" || ch === "}") { i++; continue; }
    if (ch === "~") { out += " "; i++; continue; }
    out += ch;
    i++;
  }
  // `\pm 1` 读作 “±1”：± ∓ 后面的空格去掉。
  return out.replace(RELATIONS, " $1 ").replace(/([±∓])\s+/g, "$1").replace(/\s+/g, " ").trim();
}

/** Cytoscape 按 `\\s` 折行，而 JS 的 `\\s` 包含 NBSP，所以短公式改成去掉空格（k=n+1），不能靠不断行空格。 */
const BARE_SUB = /(?<![A-Za-z0-9])([A-Za-z])_((?:[A-Za-z]|\d+))(?![A-Za-z0-9])/g;
const MATH_ATOM = String.raw`\p{L}[\p{L}\p{N}_₀-₉₊₋₌₍₎ₐ-ₓ⁰-⁹⁺⁻⁼⁽⁾ᵃ-ᶻᴬ-ᵂ]*|\d+|\(-?\d+\)[₀-₉⁺⁻ᵏⁿᵐⁱʲ]*|…`;
const MATH_OP = String.raw`[=≥≤≠≈≡±+−–]`;
const MATH_RUN = new RegExp(`(?:${MATH_ATOM})(?:\\s+${MATH_OP}\\s+(?:${MATH_ATOM}))+`, "gu");

/** 无 `$` 的 `f_k` / `F_k` / `F_0` 也换成 Unicode 下标（只处理单字母或纯数字下标）。 */
export function convertBareSubscripts(text: string): string {
  return text.replace(BARE_SUB, (all, base: string, sub: string) => {
    const chars = Array.from(sub);
    if (chars.every((ch) => SUB[ch] !== undefined)) return base + chars.map((ch) => SUB[ch]).join("");
    return all;
  });
}

/** 把短公式里运算符两侧的空格去掉，整段公式不会在等号处被 Cytoscape 折行。 */
export function glueShortMath(text: string): string {
  return text.replace(MATH_RUN, (run) => run.replace(/\s+/g, ""));
}

/**
 * 一段文字里的 `$…$` / `$$…$$` / `\(…\)` 换成纯文字。被截断、没有收尾的
 * `$…`（投影标签可能已经切在公式中间）当作公式读到结尾。没有 `$` 的
 * `f_k = F_k`、`k = n+1` 也会转成下标并去掉空格，避免在等号处折行。
 */
export function texToPlain(text: string): string {
  let out = text;
  if (/[$\\]/.test(text)) {
    out = "";
    let i = 0;
    while (i < text.length) {
      if (text.startsWith("\\(", i)) {
        const end = text.indexOf("\\)", i + 2);
        const stop = end < 0 ? text.length : end;
        out += texMathToPlain(text.slice(i + 2, stop));
        i = end < 0 ? text.length : end + 2;
        continue;
      }
      if (text.startsWith("\\$", i)) {
        out += "$";
        i += 2;
        continue;
      }
      if (text[i] === "$") {
        const fence = text.startsWith("$$", i) ? "$$" : "$";
        const end = text.indexOf(fence, i + fence.length);
        const stop = end < 0 ? text.length : end;
        let body = text.slice(i + fence.length, stop);
        // 截断的公式：末尾的省略号留在公式外面。
        let tail = "";
        if (end < 0 && body.endsWith("…")) { body = body.slice(0, -1); tail = "…"; }
        out += texMathToPlain(body) + tail;
        i = end < 0 ? text.length : end + fence.length;
        continue;
      }
      out += text[i];
      i++;
    }
    out = out.replace(/ +…/g, "…").replace(/ {2,}/g, " ");
  }
  return glueShortMath(convertBareSubscripts(out));
}

/** 按字符（不是 UTF-16 码元）截断纯文字，超长时末尾加 “…”。 */
export function truncatePlain(text: string, max: number): string {
  const chars = Array.from(text);
  if (chars.length <= max) return text;
  return `${chars.slice(0, Math.max(1, max - 1)).join("").trimEnd()}…`;
}

/**
 * 地图节点 / 连线的显示标签：投影给的标签可能已经在公式中间截断（例如
 * `一笔账：对一切 $k\ge 4$，面积差恰为 $\pm …`）。如果摘要里有完整原文，
 * 先找回完整原文，再整体转成纯文字、在纯文字上重新截断。
 */
export function graphLabelText(label: string, summary?: string | null, max = 28): string {
  if (label.endsWith("…") && summary) {
    const head = label.slice(0, -1).trimEnd();
    const colon = head.indexOf("：");
    const prefix = colon >= 0 ? head.slice(0, colon + 1) : "";
    const body = head.slice(prefix.length);
    const at = body ? summary.indexOf(body) : -1;
    if (at >= 0) return prefix + truncatePlain(texToPlain(summary.slice(at)), max);
  }
  return texToPlain(label);
}
