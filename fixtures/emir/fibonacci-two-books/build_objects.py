#!/usr/bin/env python3
"""从 materials/*.md 与下方声明生成 objects.json（示例夹具 v0.2，草稿）。

v0.2 按用户 2026-11-13 拍板的 (a)(b)(c) 更新：
  (a) 共享对象写法以 mathlib 为基线，对象上带 conventions 与 mathlib_baseline；
  (b) Debt 是地图对象（kind=debt），挂在对齐或实体上，有状态；
  (c) 最小 DialectScope（kind=dialect_scope），让“乙从第 1 项起编”可被机器读到。
并按“每个 change set 围绕一个目标对象”重新分组。

id 规则按现行代码推算（未经服务端核对）：
  * change_set_id = "mathscape:change-set/" + sha256(investigation_id + "\\0" + request_id)
  * 新建项 id = "mathscape:<kind>/" + sha256(change_set_id + "\\0" + local_ref)
    （source-unit / source-relation 为现行前缀；semantic-entity / alignment / debt / dialect-scope 为本草案提议）
  * span id = "mathscape:source-span/" + sha256(json(["markdown-source-span-1", artifact_id, {start,end}]))
artifact_id 用 materials 文件字节的 sha256 作占位。
"""
import hashlib, json, re, pathlib

ROOT = pathlib.Path(__file__).resolve().parent
INVESTIGATION = "fixture-investigation/emir-fib-two-books"
KIND_PREFIX = {"source_unit": "source-unit", "source_relation": "source-relation",
               "semantic_entity": "semantic-entity", "alignment": "alignment",
               "debt": "debt", "dialect_scope": "dialect-scope"}

def sha(b: bytes) -> str: return hashlib.sha256(b).hexdigest()
def change_set_id(req): return "mathscape:change-set/" + sha(f"{INVESTIGATION}\0{req}".encode())
def gen_id(kind, cs, lr): return f"mathscape:{kind}/" + sha(f"{cs}\0{lr}".encode())
def span_id(aid, s, e):
    return "mathscape:source-span/" + sha(json.dumps(["markdown-source-span-1", aid, {"start": s, "end": e}],
                                                     separators=(",", ":"), ensure_ascii=False).encode())

MARK = re.compile(rb"<!-- span:([A-Za-z0-9_.\-]+) -->\n(.*?)\n<!-- /span -->", re.S)
def load_material(name):
    data = (ROOT / "materials" / name).read_bytes()
    aid = "mathscape:artifact/sha256/" + sha(data)
    return aid, {m.group(1).decode(): {"artifact_id": aid, "start": m.start(2), "end": m.end(2),
                                       "quote": data[m.start(2):m.end(2)].decode(), "id": span_id(aid, m.start(2), m.end(2))}
                 for m in MARK.finditer(data)}
ART_JIA, SP_JIA = load_material("jia.md")
ART_YI, SP_YI = load_material("yi.md")
SP = {**SP_JIA, **SP_YI}

REQ = {k: f"fixture-req/{k}" for k in ("src-jia", "src-yi", "fib", "qpow", "cassini", "puzzle")}
CS = {k: change_set_id(v) for k, v in REQ.items()}
STAGED = {}          # local_ref -> staged id（全调查）

def new(lr): return f"$new:{lr}"
def ref(lr): return STAGED[lr]   # 引用其他 change set 的暂存对象（形成依赖边）

def finish(cs):
    ids = {}
    for o in cs["proposed_objects"]:
        lr = o["local_ref"]
        ids[lr] = SP[lr]["id"] if o["kind"] == "source_span" else gen_id(KIND_PREFIX[o["kind"]], cs["change_set_id"], lr)
    for r in cs.get("proposed_relations", []):
        ids[r["local_ref"]] = gen_id("source-relation", cs["change_set_id"], r["local_ref"])
    cs["staged_ids"] = ids
    STAGED.update(ids)
    return cs

def span_obj(lr):
    s = SP[lr]
    return {"local_ref": lr, "kind": "source_span", "op": "create",
            "payload": {"artifact_id": s["artifact_id"], "md_byte_range": {"start": s["start"], "end": s["end"]},
                        "granularity": "inline", "quote": s["quote"]}}
def unit(lr, kind, title, spans, parent=None):
    return {"local_ref": lr, "kind": "source_unit", "op": "create",
            "payload": {"kind": kind, "display_title": title, "parent": new(parent) if parent else None,
                        "source_spans": [new(s) for s in spans], "completeness": "complete"}}
def rel(lr, relation, src, tgt, ev):
    return {"local_ref": lr, "kind": "source_relation", "op": "create", "relation": relation,
            "source": new(src), "target": new(tgt), "evidence_spans": [new(e) for e in ev]}
def conv(key, subject, value, text, spans, explicitness="explicit", local=True):
    return {"key": key, "subject": subject, "value": value, "text": text, "explicitness": explicitness,
            "evidence_spans": [new(s) if local else ref(s) for s in spans]}

# ======================= 来源层 + 最小 DialectScope =======================
cs_jia = finish({
    "request_id": REQ["src-jia"], "change_set_id": CS["src-jia"], "version": 1,
    "target_object": "jia.u.sec",
    "contract_status": "来源项为现行契约 ✅；dialect_scope 为本草案新增 🆕",
    "rationale": "登记《星港数列讲义》（虚构）§3.2，并记下本节的编号约定（下标从 0 起）。",
    "proposed_objects": [span_obj(s) for s in SP_JIA] + [
        unit("jia.u.sec", "section", "§3.2 Fibonacci 数列（甲）", ["jia.s.sec"]),
        unit("jia.u.def", "definition", "定义 3.2.1 Fibonacci 数列（F₀=0, F₁=1）", ["jia.s.def"], "jia.u.sec"),
        unit("jia.u.lem", "statement", "引理 3.2.2 Qⁿ 的矩阵形式", ["jia.s.lem"], "jia.u.sec"),
        unit("jia.u.lem_pf", "proof", "引理 3.2.2 的证明（归纳）", ["jia.s.lem_pf"], "jia.u.sec"),
        unit("jia.u.thm", "statement", "定理 3.2.3 Cassini 恒等式", ["jia.s.thm"], "jia.u.sec"),
        unit("jia.u.thm_pf", "proof", "定理 3.2.3 的证明（取行列式）", ["jia.s.thm_pf"], "jia.u.sec"),
        unit("jia.u.rem", "remark", "注 3.2.4 n=1 的情形用到 F₀", ["jia.s.rem"], "jia.u.sec"),
        {"local_ref": "ds.jia", "kind": "dialect_scope", "op": "create",
         "payload": {"label": "《星港》§3.2 的约定", "anchor_unit": new("jia.u.sec"), "applies_to": "anchor_subtree",
                     "parent_scope": None, "introduced_by_unit": new("jia.u.def"),
                     "conventions": [conv("sequence_index_origin", "F", 0, "数列 $(F_n)_{n\\ge 0}$，从第 0 项 $F_0=0$ 起编", ["jia.s.def"])]}},
    ],
    "proposed_relations": [
        rel("jia.r.lem_proof", "proves", "jia.u.lem_pf", "jia.u.lem", ["jia.s.lem_pf"]),
        rel("jia.r.thm_proof", "proves", "jia.u.thm_pf", "jia.u.thm", ["jia.s.thm_pf"]),
        rel("jia.r.thm_cites_lem", "cites", "jia.u.thm_pf", "jia.u.lem", ["jia.s.thm_pf"]),
        rel("jia.r.rem_explains", "explains", "jia.u.rem", "jia.u.thm", ["jia.s.rem"]),
    ], "unresolved": []})

cs_yi = finish({
    "request_id": REQ["src-yi"], "change_set_id": CS["src-yi"], "version": 1,
    "target_object": "yi.u.sec",
    "contract_status": "来源项为现行契约 ✅；dialect_scope 为本草案新增 🆕",
    "rationale": "登记《灯塔组合数学初步》（虚构）§2.4，并记下本节约定：下标从 1 起、不用第 0 项；“兔子数列”是本书对 Fibonacci 数列的叫法。",
    "proposed_objects": [span_obj(s) for s in SP_YI] + [
        unit("yi.u.sec", "section", "§2.4 兔子数列（乙）", ["yi.s.sec"]),
        unit("yi.u.def", "definition", "定义 2.4.1 兔子数列（f₁=f₂=1，无第 0 项）", ["yi.s.def"], "yi.u.sec"),
        unit("yi.u.lem", "statement", "命题 2.4.2 Mⁿ⁺¹ 的矩阵形式", ["yi.s.lem"], "yi.u.sec"),
        unit("yi.u.thm", "statement", "命题 2.4.3 Cassini（乙的写法）", ["yi.s.thm"], "yi.u.sec"),
        unit("yi.u.ex", "problem", "习题 2.4.7 剪拼悖论", ["yi.s.ex"], "yi.u.sec"),
        unit("yi.u.hint", "hint", "习题 2.4.7 的提示", ["yi.s.hint"], "yi.u.sec"),
        {"local_ref": "ds.yi", "kind": "dialect_scope", "op": "create",
         "payload": {"label": "《灯塔》§2.4 的约定", "anchor_unit": new("yi.u.sec"), "applies_to": "anchor_subtree",
                     "parent_scope": None, "introduced_by_unit": new("yi.u.def"),
                     "conventions": [
                         conv("sequence_index_origin", "f", 1, "本书的下标一律从 1 开始，不使用第 0 项", ["yi.s.def"]),
                         conv("terminology", "兔子数列", "Fibonacci 数列", "本节把 Fibonacci 数列叫作“兔子数列”", ["yi.s.def"], "inferred")]}},
    ],
    "proposed_relations": [
        rel("yi.r.hint", "hints", "yi.u.hint", "yi.u.ex", ["yi.s.hint"]),
        rel("yi.r.hint_cites_thm", "cites", "yi.u.hint", "yi.u.thm", ["yi.s.hint"]),
    ], "unresolved": ["命题 2.4.2、2.4.3 在本摘录中没有证明；不补写。"]})

# ======================= 语义层 =======================
def text(t, spans, note=None, local=False):
    d = {"text": t, "format": "markdown+tex", "evidence_spans": [new(s) if local else ref(s) for s in spans]}
    if note: d["note"] = note
    return d

def entity(lr, kind, labels, plain, home="e.theory", conventions=(), mathlib=None, ext_key=None, ext=None, home_local=True, **extra):
    p = {"semantic_kind": kind, "home_theory": (ref(home) if home in STAGED else new(home)) if home else None,
         "preferred_labels": labels, "plain_label": plain,
         "conventions": list(conventions), "mathlib_baseline": mathlib}
    if ext_key: p[ext_key] = ext
    p.update(extra)
    return {"local_ref": lr, "kind": "semantic_entity", "op": "create", "payload": p}

def eccv(text_, value=0):  # 共享对象上的约定注记（a）
    return {"key": "sequence_index_origin", "subject": "F", "value": value, "text": text_, "basis": "mathlib"}

LABEL = {  # 前端白话标签模板（这里预先算出，供夹具测试；以前端模板为准）
    "expresses": "说的就是它", "overlaps_with": "部分重合", "specializes": "特例"}

def align(lr, src_unit, target, relation, book, spans, *, param_map=None, term_map=None,
          required=(), lost=(), status="retrieved_candidate", target_local=True, plain=""):
    ds = {"甲": "ds.jia", "乙": "ds.yi"}[book]
    label = LABEL[relation]
    if relation == "specializes" and param_map:
        label += f"：{book}的第 n 条 ＝ 共享的第 n+1 条"
    elif relation == "overlaps_with" and lost:
        label += "：" + lost[0]["short"]
    elif param_map and param_map["source"] != param_map["target"]:
        label += f"：{book}的 n 对应题型的 k = n+1"
    p = {"source": ref(src_unit), "target": new(target) if target_local else ref(target), "relation": relation,
         "source_context": {"dialect_scope": ref(ds)},
         "target_context": ref("e.theory") if "e.theory" in STAGED else new("e.theory"),
         "parameter_map": [param_map] if param_map else [],
         "term_index_map": [term_map] if term_map else [],
         "required_assumptions": list(required),
         "lost_information": [{k: v for k, v in l.items()} for l in lost],
         "evidence_spans": [ref(s) for s in spans], "status": status,
         "map_label": label, "plain_note": plain}
    return {"local_ref": lr, "kind": "alignment", "op": "create", "payload": p}

def debt(lr, kind, attached, obligation, quantifier, failure_mode, failure_note, spans, attached_local=True, hint=None):
    p = {"debt_kind": kind, "attached_to": new(attached) if attached_local else ref(attached), "status": "open",
         "obligation": obligation, "quantifier": quantifier, "failure_mode": failure_mode,
         "failure_note": failure_note, "evidence_spans": [ref(s) for s in spans]}
    if hint: p["payoff_hint"] = hint
    return {"local_ref": lr, "kind": "debt", "op": "create", "payload": p}

TERM_SAME = {"source": "f_k", "target": "F_k", "index_shift": 0, "valid_for": "k ≥ 1",
             "note": "数列项下标对齐：乙的第 k 项就是共享的第 k 项（数列本身没有错开）"}
TERM_ID_JIA = {"source": "F_k", "target": "F_k", "index_shift": 0, "valid_for": "k ≥ 0", "note": "逐字相同"}
LOST_F0 = {"short": "乙没有第 0 项", "claim": "共享定义含第 0 项 F₀ = 0；乙不使用第 0 项，乙的任何陈述都不能谈论 F₀。",
           "direction": "target_has_source_lacks"}

# --- cs.fib：目标 = 概念星 e.fib（附带它所在的星区与精确定义） ---
cs_fib = {"request_id": REQ["fib"], "change_set_id": CS["fib"], "version": 2, "target_object": "e.fib",
          "supersedes": {"change_set_id": CS["fib"], "version": 1, "proposal_fingerprint": "sha256:<v1 指纹，由服务端给出>"},
          "contract_status": "🆕 语义层契约草案",
          "rationale": "建立“Fibonacci 数列”这颗概念星、它所在的星区和精确定义（写法同 mathlib Nat.fib），并把两本书的定义分别连上。乙的定义只部分重合：它没有第 0 项。",
          "proposed_objects": [
    entity("e.theory", "theory", [{"lang": "zh", "label": "整数数列与二阶线性递推（初等）"}], "星区：线性递推", home=None,
           conventions=[{"key": "baseline", "subject": "*", "value": "mathlib", "text": "本星区的共享写法以 mathlib 为参照基线", "basis": "mathlib"}]),
    entity("e.fib", "concept", [{"lang": "zh", "label": "Fibonacci 数列"}, {"lang": "en", "label": "Fibonacci sequence"}],
           "概念星：Fibonacci 数列",
           description="“每项是前两项之和、以 1, 1 开头”的那一串数这个概念本身；精确写法见 e.fib_def。"),
    entity("e.fib_def", "declaration", [{"lang": "zh", "label": "Fibonacci 数列的定义"}],
           "精确说法：Fibonacci 数列（F₀ = 0 起编，同 mathlib）",
           conventions=[eccv("F₀ = 0，F₁ = 1，从第 0 项起编（同 mathlib Nat.fib）。采用这一写法只为与基线一致，不表示哪本书更正统。")],
           mathlib={"decl": "Nat.fib", "module": "Mathlib.Data.Nat.Fib.Basic", "relation_hint": "exact_realization",
                    "note": "mathlib 取值 ℕ，共享对象取值 ℤ（便于写减法），差一个自然数到整数的强制转换", "status": "unchecked"},
           ext_key="declaration", ext={
               "declaration_kind": "definition", "binders": [{"name": "n", "type": "ℕ", "role": "index"}],
               "assumptions": [], "statement": None,
               "definiens": text(r"$F:\mathbb N\to\mathbb Z$，$F_0=0$，$F_1=1$，$F_{n+1}=F_n+F_{n-1}\ (n\ge 1)$", ["jia.s.def"]),
               "uses_entities": [], "concept": new("e.fib")}),
    align("a.jia.def.fib", "jia.u.def", "e.fib", "expresses", "甲", ["jia.s.def"], plain="甲的定义 3.2.1 说的就是这颗概念星。"),
    align("a.yi.def.fib", "yi.u.def", "e.fib", "expresses", "乙", ["yi.s.def"], plain="乙的“兔子数列”也是这颗星：名字不同，概念相同。"),
    align("a.jia.def", "jia.u.def", "e.fib_def", "expresses", "甲", ["jia.s.def"], term_map=TERM_ID_JIA,
          plain="甲的定义与共享定义逐字一致。"),
    align("a.yi.def", "yi.u.def", "e.fib_def", "overlaps_with", "乙", ["yi.s.def"], term_map=TERM_SAME,
          lost=[LOST_F0], status="partially_aligned",
          required=[{"text": "f_k = F_k（k ≥ 1）", "debt": new("d.yi.f_eq_F")}],
          plain="从第 1 项起两边逐项相同；乙没有第 0 项。乙定义的是共享数列在 k ≥ 1 上的部分，不是特例。"),
    debt("d.yi.f_eq_F", "alignment_debt", "a.yi.def", "证明 $f_k = F_k$ 对一切 $k\\ge 1$ 成立（两者 k = 1, 2 取值相同、递推相同，对 k 归纳）",
         "∀ k ≥ 1", "statement", "若不成立，乙的命题 2.4.2、2.4.3 与习题 2.4.7 三条对齐都不再适用", ["yi.s.def", "jia.s.def"]),
], "proposed_relations": [], "unresolved": []}
finish(cs_fib)

# --- cs.qpow：目标 = 矩阵幂公式 ---
cs_qpow = finish({"request_id": REQ["qpow"], "change_set_id": CS["qpow"], "version": 1, "target_object": "e.qpow",
    "contract_status": "🆕 语义层契约草案",
    "rationale": "建立共享的矩阵幂公式，并连上甲的引理 3.2.2（说的就是它）和乙的命题 2.4.2（特例：乙的第 n 条 = 共享的第 n+1 条）。",
    "proposed_objects": [
    entity("e.qpow", "declaration", [{"lang": "zh", "label": "Fibonacci 矩阵幂公式"}],
           "精确说法：Q 的 n 次方里是三个相邻的 Fibonacci 数（n ≥ 1）", home_local=False,
           conventions=[eccv("项下标按 F₀ = 0 约定（同 mathlib）")],
           mathlib={"decl": None, "module": None, "relation_hint": None,
                    "note": "未在 mathlib 文档中定位到同形引理；名称留空，等本机核对入口或用户补名", "status": "unchecked"},
           ext_key="declaration", ext={
               "declaration_kind": "lemma", "binders": [{"name": "n", "type": "ℕ", "role": "universal"}],
               "assumptions": [text(r"$n\ge 1$", ["jia.s.lem"])],
               "statement": text(r"$\begin{pmatrix}1&1\\1&0\end{pmatrix}^n=\begin{pmatrix}F_{n+1}&F_n\\F_n&F_{n-1}\end{pmatrix}$", ["jia.s.lem"]),
               "definiens": None, "uses_entities": [ref("e.fib_def")]}),
    align("a.jia.lem", "jia.u.lem", "e.qpow", "expresses", "甲", ["jia.s.lem"], term_map=TERM_ID_JIA,
          param_map={"source": "n", "target": "n", "role": "statement_parameter", "map": "n ↦ n", "source_range": "n ≥ 1", "target_range": "n ≥ 1"}),
    align("a.yi.lem", "yi.u.lem", "e.qpow", "specializes", "乙", ["yi.s.lem"], term_map=TERM_SAME,
          param_map={"source": "n", "target": "n", "role": "statement_parameter", "map": "乙的第 n 条 ＝ 共享的第 n+1 条",
                     "source_range": "n_乙 ≥ 1", "target_range": "n_共享 ≥ 2",
                     "note": "n_共享 = n_乙 + 1。移动的是命题参数（第几条公式），不是数列：项下标 f_k = F_k 对齐"},
          required=[{"text": "f_k = F_k（k ≥ 1）", "debt": ref("d.yi.f_eq_F")}],
          lost=[{"short": "缺共享的 n = 1", "claim": "共享公式在 n = 1 时 Q¹ = [[F₂, F₁], [F₁, F₀]] 用到 F₀，乙的命题 2.4.2 不覆盖这一条。", "direction": "target_has_source_lacks"}]),
    debt("d.yi.lem.forward", "alignment_debt", "a.yi.lem",
         "前向证书：在共享公式中取 n = n_乙 + 1，并用 f_k = F_k 改写，即得乙的命题 2.4.2", "∀ n_乙 ≥ 1", "statement",
         "若不成立，这条“特例”对齐作废", ["yi.s.lem", "jia.s.lem"]),
], "proposed_relations": [], "unresolved": []})

# --- cs.cassini：目标 = Cassini 恒等式（教程核心） ---
cs_cas = finish({"request_id": REQ["cassini"], "change_set_id": CS["cassini"], "version": 2, "target_object": "e.cassini",
    "supersedes": {"change_set_id": CS["cassini"], "version": 1, "proposal_fingerprint": "sha256:<v1 指纹，由服务端给出>"},
    "contract_status": "🆕 语义层契约草案",
    "rationale": "建立共享 Cassini 恒等式（n ≥ 1，mathlib Int 版本的 n ≥ 1 部分），甲的定理 3.2.3 说的就是它；乙的命题 2.4.3 是特例：乙的第 n 条 = 共享的第 n+1 条，缺共享的 n = 1 那一条（它要用 F₀，即甲的注 3.2.4）。",
    "proposed_objects": [
    entity("e.cassini", "declaration", [{"lang": "zh", "label": "Cassini 恒等式"}, {"lang": "en", "label": "Cassini's identity"}],
           "精确说法：Cassini 恒等式（对一切 n ≥ 1）", home_local=False,
           conventions=[eccv("项下标按 F₀ = 0 约定（同 mathlib）")],
           mathlib={"decl": "Int.fib_succ_mul_fib_pred_sub_fib_sq", "module": "Mathlib.Data.Int.Fib.Lemmas",
                    "relation_hint": "specializes",
                    "note": "mathlib 版本对一切整数 n 成立（Int.fib 延拓到负下标，右端为 (−1)^|n|）；共享对象取其 n ≥ 1 的部分",
                    "status": "unchecked"},
           ext_key="declaration", ext={
               "declaration_kind": "theorem", "binders": [{"name": "n", "type": "ℕ", "role": "universal"}],
               "assumptions": [text(r"$n\ge 1$", ["jia.s.thm"])],
               "statement": text(r"$F_{n+1}F_{n-1}-F_n^2=(-1)^n$", ["jia.s.thm"], note="在 ℤ 中计算"),
               "definiens": None, "uses_entities": [ref("e.fib_def")]}),
    align("a.jia.thm", "jia.u.thm", "e.cassini", "expresses", "甲", ["jia.s.thm"], term_map=TERM_ID_JIA,
          param_map={"source": "n", "target": "n", "role": "statement_parameter", "map": "n ↦ n", "source_range": "n ≥ 1", "target_range": "n ≥ 1"}),
    align("a.yi.thm", "yi.u.thm", "e.cassini", "specializes", "乙", ["yi.s.thm"], term_map=TERM_SAME,
          param_map={"source": "n", "target": "n", "role": "statement_parameter", "map": "乙的第 n 条 ＝ 共享的第 n+1 条",
                     "source_range": "n_乙 ≥ 1", "target_range": "n_共享 ≥ 2",
                     "note": "移动的是第几条恒等式，不是数列：项下标 f_k = F_k 对齐"},
          required=[{"text": "f_k = F_k（k ≥ 1）", "debt": ref("d.yi.f_eq_F")}],
          lost=[{"short": "缺共享的 n = 1", "claim": "共享 Cassini 在 n = 1 时 F₂F₀ − F₁² = −1（即甲的注 3.2.4）在乙里写不出来，因为乙没有第 0 项。", "direction": "target_has_source_lacks",
                 "see_also": ref("jia.u.rem")}],
          plain="乙的 Cassini 看上去和甲不一样（乘积的下标、指数都不同），其实乙的第 n 条就是共享的第 n+1 条，只少了共享 n = 1 那一条。"),
    debt("d.yi.thm.forward", "alignment_debt", "a.yi.thm",
         r"前向证书：在共享 Cassini 中取 n = n_乙 + 1，得 $F_{n+2}F_n-F_{n+1}^2=(-1)^{n+1}$，再用 $f_k=F_k$ 即得乙的命题 2.4.3",
         "∀ n_乙 ≥ 1", "statement", "若不成立，这条“特例”对齐作废", ["yi.s.thm", "jia.s.thm"]),
], "proposed_relations": [], "unresolved": []})

# --- cs.puzzle：目标 = 剪拼题型 ---
cs_puz = finish({"request_id": REQ["puzzle"], "change_set_id": CS["puzzle"], "version": 1, "target_object": "e.puzzle",
    "contract_status": "🆕 语义层契约草案",
    "rationale": "建立“64 = 65 剪拼族”题型（参数 k ≥ 4，动画里的 8×8 是 k = 6），连上乙的习题 2.4.7（乙的 n 对应 k = n+1），并把动画第 2 幕的那笔账记为地图上的 Debt。",
    "proposed_objects": [
    entity("e.puzzle", "problem_schema", [{"lang": "zh", "label": "Fibonacci 剪拼悖论（参数化题型）"}],
           "题型：64 = 65 这一族剪拼题", home_local=False,
           conventions=[eccv("参数 k 是正方形边长 F_k 的项下标，按 F₀ = 0 约定")],
           mathlib=None,  # 题型不在 mathlib 中：不写基线
           ext_key="problem_schema", ext={
               "parameters": [{"name": "k", "type": "ℕ", "role": "parameter"}],
               "assumptions": [text(r"$k\ge 4$", ["yi.s.ex"], note="k = 3 时梯形退化为矩形")],
               "task": text(r"边长 $F_k$ 的正方形平行于一边分成 $F_{k-2}\times F_k$ 与 $F_{k-1}\times F_k$ 两个长条：前者沿对角线剪成两个直角三角形（直角边 $F_k, F_{k-2}$），后者从长边上距端点 $F_{k-1}$ 处剪到对边上距同侧端点 $F_{k-2}$ 处，得两个全等直角梯形（两底 $F_{k-2}, F_{k-1}$，高 $F_{k-1}$），拼成看上去是 $F_{k-1}\times F_{k+1}$ 的图形；说明面积差 $F_{k+1}F_{k-1}-F_k^2=(-1)^k$：$k$ 为偶数时四块之间沿对角线留出一条面积为 1 的细长平行四边形缝，$k$ 为奇数时四块沿对角线重叠出一条面积为 1 的细长平行四边形。", ["yi.s.ex"]),
               "known_instances": [{"k": 6, "label": "8×8 → 5×13（引导动画用的那一张）"}],
               "uses_entities": [ref("e.fib_def"), ref("e.cassini")]}),
    align("a.yi.ex", "yi.u.ex", "e.puzzle", "expresses", "乙", ["yi.s.ex"], term_map=TERM_SAME,
          param_map={"source": "n", "target": "k", "role": "problem_parameter", "map": "乙的 n 对应题型的 k = n+1", "source_range": "n ≥ 3", "target_range": "k ≥ 4",
                     "note": "k = n + 1。正方形边长 f_{n+1} = F_{n+1} = F_k；数列项下标不变"},
          required=[{"text": "f_k = F_k（k ≥ 1）", "debt": ref("d.yi.f_eq_F")}],
          plain="乙的习题 2.4.7 就是这一族题：乙写 n = 5 的那一张，在题型里是 k = 6，也就是动画里的 8×8。"),
    debt("d.puzzle.all_k", "proof_debt", "e.puzzle", r"对一切 $k\ge 4$，面积差恰为 $\pm 1$，即 $F_{k+1}F_{k-1}-F_k^2=(-1)^k$",
         "∀ k ≥ 4", "statement", "不还清，就只剩 k = 6 一个个例", ["yi.s.ex"],
         hint="引导动画第 2 幕开出、第 4 幕由 det→Cassini 还清的那笔账。第一条 PR 没有 D 对象：Agent 可把状态提议为 claimed_paid 并以 e.cassini 为证据，paid 须用户在审核页确认。"),
], "proposed_relations": [], "unresolved": []})

CHANGE_SETS = [cs_jia, cs_yi, cs_fib, cs_qpow, cs_cas, cs_puz]

# 依赖边（服务端会自行计算；这里派生出来供审阅与测试）
owner = {sid: cs["change_set_id"] for cs in CHANGE_SETS for sid in cs["staged_ids"].values()}
for cs in CHANGE_SETS:
    blob = json.dumps(cs["proposed_objects"] + cs["proposed_relations"], ensure_ascii=False)
    deps = sorted({owner[s] for s in re.findall(r'"(mathscape:[a-z-]+/[0-9a-f]{64})"', blob)
                   if s in owner and owner[s] != cs["change_set_id"]} - {None})
    # span 引用若本 cs 未自带 CreateSourceSpan，同样形成依赖（§4.2.5）
    cs["depends_on_change_sets"] = deps

revision_history = [
    {"change_set_id": CS["fib"], "version": 1, "state_after_v2": "superseded",
     "summary": "v1 把乙的定义写成 definitionally_equivalent。用户在对话里指出“乙没有第 0 项”，双方同意后 Agent 用 supersedes 提交 v2。",
     "diff_v1_to_v2": [{"local_ref": "a.yi.def", "v1": {"relation": "definitionally_equivalent", "lost_information": []},
                        "v2": {"relation": "overlaps_with", "lost_information": [LOST_F0["claim"]], "status": "partially_aligned"},
                        "why": "定义等价要求两边定义出同一个对象；共享定义在 n = 0 有值，乙没有。"}],
     "dependents_note": "cs.qpow / cs.cassini / cs.puzzle 在 v2 之后才提交，所以没有触发 dependency_superseded。"},
    {"change_set_id": CS["cassini"], "version": 1, "state_after_v2": "superseded",
     "summary": "v1 把乙的 Cassini 写成 logically_equivalent；用户反馈后改为 specializes。",
     "diff_v1_to_v2": [{"local_ref": "a.yi.thm", "v1": {"relation": "logically_equivalent", "map_label": "说的就是它"},
                        "v2": {"relation": "specializes", "map_label": "特例：乙的第 n 条 ＝ 共享的第 n+1 条"},
                        "why": "乙的陈述只推出共享陈述 n ≥ 2 的部分，推不出 n = 1。"}],
     "id_note": "local_ref 不变，v1、v2 中这条对齐的 staged id 相同。"},
]

doc = {
    "fixture": "emir-fib-two-books", "fixture_version": "0.3",
    "label": "示例材料（虚构教材）派生的 EMIR 对象；语义层第一条 PR 的测试与教程夹具",
    "decisions_applied": {"a": "共享对象写法以 mathlib 为基线，对象上带 conventions 与 mathlib_baseline",
                          "b": "Debt 是地图对象（kind=debt），挂在对齐或实体上，status ∈ open / claimed_paid / paid / waived / obsolete",
                          "c": "最小 DialectScope（kind=dialect_scope）：anchor_unit + conventions"},
    "investigation_id": INVESTIGATION,
    "artifacts": {"jia": {"file": "materials/jia.md", "artifact_id": ART_JIA, "role": "canonical Markdown 快照（占位）"},
                  "yi": {"file": "materials/yi.md", "artifact_id": ART_YI, "role": "canonical Markdown 快照（占位）"}},
    "id_rules": {"change_set_id": "mathscape:change-set/sha256(investigation_id \\0 request_id)",
                 "new_object": "mathscape:<kind>/sha256(change_set_id \\0 local_ref)",
                 "kinds": KIND_PREFIX,
                 "source_span": "mathscape:source-span/sha256(json([\"markdown-source-span-1\", artifact_id, {start,end}]))",
                 "note": "id 不透明；可读名称只放在 display_title / preferred_labels / plain_label / label 中。"},
    "trust_policy": {"on_propose": "payload 不写 trust；服务端填默认值 provenance=located, parse=raw, segmentation=proposed, typing=unchecked, proof=none, alignment=candidate, curation=raw",
                     "on_accept": "只写进地图（lifecycle=active）；trust 各轴、Alignment.status、Debt.status 都不变。"},
    "change_sets": CHANGE_SETS,
    "revision_history": revision_history,
    "map_debts": [{"local_ref": o["local_ref"], "id": cs["staged_ids"][o["local_ref"]], "change_set_id": cs["change_set_id"],
                   "debt_kind": o["payload"]["debt_kind"], "attached_to": o["payload"]["attached_to"], "status": o["payload"]["status"],
                   "obligation": o["payload"]["obligation"]}
                  for cs in CHANGE_SETS for o in cs["proposed_objects"] if o["kind"] == "debt"],
    "beyond_current_contract": [
        "kind = semantic_entity / alignment / debt / dialect_scope 的 proposed_objects（现行 §4.2.3 为 ❌，见 docs/semantic-write-contract.md §S4）",
        "来源 change set 中附带的 dialect_scope",
        "plain_label / plain_note / map_label（白话展示字段；map_label 以前端模板为准，夹具里预先算出供测试）",
        "depends_on_change_sets 为派生字段，服务端自行计算，不由 Agent 提交"],
}
(ROOT / "objects.json").write_text(json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
n = sum(len(c["proposed_objects"]) + len(c["proposed_relations"]) for c in CHANGE_SETS)
print(f"objects.json written: {len(CHANGE_SETS)} change sets, {n} items")
