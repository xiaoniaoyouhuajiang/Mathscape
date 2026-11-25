#!/usr/bin/env python3
"""从 materials/*.md 生成阶段二双书黄金样例 objects.json（S2 完整 + S3 前半）。"""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent
INVESTIGATION = "fixture-investigation/emir-generating-functions-two-books"
KIND_PREFIX = {
    "source_unit": "source-unit",
    "source_relation": "source-relation",
    "semantic_entity": "semantic-entity",
    "alignment": "alignment",
    "debt": "debt",
    "dialect_scope": "dialect-scope",
    "representation_core": "representation-core",
    "representation_binding": "representation-binding",
}


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def change_set_id(req: str) -> str:
    return "mathscape:change-set/" + sha(f"{INVESTIGATION}\0{req}".encode())


def gen_id(kind: str, cs: str, lr: str) -> str:
    return f"mathscape:{kind}/" + sha(f"{cs}\0{lr}".encode())


def span_id(aid: str, s: int, e: int) -> str:
    payload = json.dumps(
        ["markdown-source-span-1", aid, {"start": s, "end": e}],
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode()
    return "mathscape:source-span/" + sha(payload)


MARK = re.compile(rb"<!-- span:([A-Za-z0-9_.\-]+) -->\n(.*?)\n<!-- /span -->", re.S)


def load_material(name: str):
    data = (ROOT / "materials" / name).read_bytes()
    aid = "mathscape:artifact/sha256/" + sha(data)
    spans = {
        m.group(1).decode(): {
            "artifact_id": aid,
            "start": m.start(2),
            "end": m.end(2),
            "quote": data[m.start(2) : m.end(2)].decode(),
            "id": span_id(aid, m.start(2), m.end(2)),
        }
        for m in MARK.finditer(data)
    }
    return aid, spans


ART_JIA, SP_JIA = load_material("星港母函数讲义.md")
ART_YI, SP_YI = load_material("灯塔母函数初步.md")
SP = {**SP_JIA, **SP_YI}

REQ_NAMES = ("src-jia", "src-yi", "rep-seq", "rep-fps", "geom-fin", "geom-inf")
REQ = {k: f"fixture-req/{k}" for k in REQ_NAMES}
CS = {k: change_set_id(v) for k, v in REQ.items()}
STAGED: dict[str, str] = {}


def new(lr: str) -> str:
    return f"$new:{lr}"


def ref(lr: str) -> str:
    return STAGED[lr]


def finish(cs: dict) -> dict:
    ids: dict[str, str] = {}
    for o in cs["proposed_objects"]:
        lr = o["local_ref"]
        if o["kind"] == "source_span":
            ids[lr] = SP[lr]["id"]
        else:
            ids[lr] = gen_id(KIND_PREFIX[o["kind"]], cs["change_set_id"], lr)
    for r in cs.get("proposed_relations", []):
        ids[r["local_ref"]] = gen_id(
            "source-relation", cs["change_set_id"], r["local_ref"]
        )
    cs["staged_ids"] = ids
    STAGED.update(ids)
    return cs


def span_obj(lr: str) -> dict:
    s = SP[lr]
    return {
        "local_ref": lr,
        "kind": "source_span",
        "op": "create",
        "payload": {
            "artifact_id": s["artifact_id"],
            "md_byte_range": {"start": s["start"], "end": s["end"]},
            "granularity": "inline",
            "quote": s["quote"],
        },
    }


def unit(lr: str, kind: str, title: str, spans: list[str], parent: str | None = None) -> dict:
    return {
        "local_ref": lr,
        "kind": "source_unit",
        "op": "create",
        "payload": {
            "kind": kind,
            "display_title": title,
            "parent": new(parent) if parent else None,
            "source_spans": [new(s) for s in spans],
            "completeness": "complete",
        },
    }


def rel(lr: str, relation: str, src: str, tgt: str, ev: list[str]) -> dict:
    return {
        "local_ref": lr,
        "kind": "source_relation",
        "op": "create",
        "relation": relation,
        "source": new(src),
        "target": new(tgt),
        "evidence_spans": [new(e) for e in ev],
    }


cs_jia = finish(
    {
        "request_id": REQ["src-jia"],
        "change_set_id": CS["src-jia"],
        "version": 1,
        "target_object": "jia.u.sec",
        "rationale": "登记《星港母函数讲义》§2.1，并记录形式幂级数约定与局部符号表（a_n, A(x), [x^n]）。",
        "proposed_objects": [span_obj(s) for s in SP_JIA]
        + [
            unit("jia.u.sec", "section", "§2.1 普通母函数与有限几何和", ["jia.s.sec"]),
            unit(
                "jia.u.def",
                "definition",
                "定义 2.1.1 普通母函数（形式幂级数 R[[x]]）",
                ["jia.s.def"],
                "jia.u.sec",
            ),
            unit(
                "jia.u.thm",
                "statement",
                "定理 2.1.2 有限几何级数（x≠1 与 x=1 双分支）",
                ["jia.s.thm"],
                "jia.u.sec",
            ),
            unit(
                "jia.u.cor",
                "statement",
                "推论 2.1.3 形式逆元与实变数发散排除",
                ["jia.s.cor"],
                "jia.u.sec",
            ),
            {
                "local_ref": "ds.jia",
                "kind": "dialect_scope",
                "op": "create",
                "payload": {
                    "label": "《星港》§2.1 的形式母函数约定",
                    "anchor_unit": new("jia.u.sec"),
                    "applies_to": "anchor_subtree",
                    "parent_scope": None,
                    "introduced_by_unit": new("jia.u.def"),
                    "conventions": [
                        {
                            "key": "sequence_index_origin",
                            "subject": "a",
                            "value": 0,
                            "text": "数列 $(a_n)_{n \\ge 0}$ 从第 0 项 $a_0$ 起算",
                            "explicitness": "explicit",
                            "evidence_spans": [new("jia.s.def")],
                        },
                        {
                            "key": "notation",
                            "subject": "A(x)",
                            "value": "formal_power_series",
                            "text": "$A(x) \\in R[[x]]$ 为形式幂级数，不讨论实数收敛性",
                            "explicitness": "explicit",
                            "evidence_spans": [new("jia.s.def")],
                        },
                    ],
                    "local_symbols": [
                        {
                            "symbol": "a_n",
                            "role": "sequence",
                            "meaning": "下标从 0 起的系数数列通项",
                            "domain": {"text": "$n \\in \\mathbb{N},\\ a_n \\in R$"},
                            "evidence_spans": [new("jia.s.def")],
                        },
                        {
                            "symbol": "A(x)",
                            "role": "series",
                            "meaning": "数列 (a_n) 的普通母函数（形式幂级数）",
                            "domain": {"text": "$R[[x]]$"},
                            "evidence_spans": [new("jia.s.def")],
                        },
                        {
                            "symbol": "[x^n]",
                            "role": "operator",
                            "meaning": "提取形式幂级数的第 n 次项系数",
                            "domain": {"text": "$R[[x]] \\to R$"},
                            "evidence_spans": [new("jia.s.def")],
                        },
                    ],
                },
            },
        ],
        "proposed_relations": [
            rel("jia.r.cor_explains", "explains", "jia.u.cor", "jia.u.def", ["jia.s.cor"]),
        ],
        "unresolved": [],
    }
)

cs_yi = finish(
    {
        "request_id": REQ["src-yi"],
        "change_set_id": CS["src-yi"],
        "version": 1,
        "target_object": "yi.u.sec",
        "rationale": "登记《灯塔母函数初步》§1.3，并记录从 1 起算及实收敛生成函数局部符号表（b_m, g(t), r）。",
        "proposed_objects": [span_obj(s) for s in SP_YI]
        + [
            unit("yi.u.sec", "section", "§1.3 生成函数与等比求和", ["yi.s.sec"]),
            unit(
                "yi.u.def",
                "definition",
                "定义 1.3.1 实收敛区间上的生成函数 g(t)",
                ["yi.s.def"],
                "yi.u.sec",
            ),
            unit(
                "yi.u.prop",
                "statement",
                "命题 1.3.2 有限等比和公式（仅写 t≠1）",
                ["yi.s.prop"],
                "yi.u.sec",
            ),
            unit(
                "yi.u.rem",
                "remark",
                "注 1.3.3 |t|<1 收敛与 |t|≥1 发散边界",
                ["yi.s.rem"],
                "yi.u.sec",
            ),
            {
                "local_ref": "ds.yi",
                "kind": "dialect_scope",
                "op": "create",
                "payload": {
                    "label": "《灯塔》§1.3 的收敛生成函数约定",
                    "anchor_unit": new("yi.u.sec"),
                    "applies_to": "anchor_subtree",
                    "parent_scope": None,
                    "introduced_by_unit": new("yi.u.def"),
                    "conventions": [
                        {
                            "key": "sequence_index_origin",
                            "subject": "b",
                            "value": 1,
                            "text": "正项标号数列 $(b_m)_{m \\ge 1}$ 一律从第 1 项起算",
                            "explicitness": "explicit",
                            "evidence_spans": [new("yi.s.def")],
                        },
                        {
                            "key": "other",
                            "subject": "g(t)",
                            "value": "analytic_interval",
                            "text": "生成函数 $g(t)$ 定义为开区间 $(-r, r)$ 上的绝对收敛实函数",
                            "explicitness": "explicit",
                            "evidence_spans": [new("yi.s.def")],
                        },
                    ],
                    "local_symbols": [
                        {
                            "symbol": "b_m",
                            "role": "sequence",
                            "meaning": "下标从 1 起的实系数数列通项",
                            "domain": {"text": "$m \\ge 1,\\ b_m \\in \\mathbb{R}$"},
                            "evidence_spans": [new("yi.s.def")],
                        },
                        {
                            "symbol": "g(t)",
                            "role": "function",
                            "meaning": "收敛区间 (-r, r) 上由幂级数定义的生成函数",
                            "domain": {"text": "$(-r, r) \\to \\mathbb{R}$"},
                            "evidence_spans": [new("yi.s.def")],
                        },
                        {
                            "symbol": "r",
                            "role": "constant",
                            "meaning": "保证幂级数绝对收敛的正半径参数",
                            "domain": {"text": "$r > 0$"},
                            "evidence_spans": [new("yi.s.def")],
                        },
                    ],
                },
            },
        ],
        "proposed_relations": [
            rel("yi.r.rem_explains", "explains", "yi.u.rem", "yi.u.prop", ["yi.s.rem"]),
        ],
        "unresolved": [],
    }
)

cs_rep_seq = finish(
    {
        "request_id": REQ["rep-seq"],
        "change_set_id": CS["rep-seq"],
        "version": 1,
        "target_object": "r.seq",
        "rationale": "建立母函数理论与数列表示核 R_seq，并将《星港》（0 起算）与《灯塔》（1 起算补零）各自的约定卡绑定到 R_seq。",
        "proposed_objects": [
            {
                "local_ref": "e.theory",
                "kind": "semantic_entity",
                "op": "create",
                "payload": {
                    "semantic_kind": "theory",
                    "preferred_labels": [
                        {"lang": "zh", "label": "母函数与形式幂级数"},
                        {"lang": "en", "label": "Generating functions and formal power series"},
                    ],
                    "aliases": ["普通母函数", "生成函数", "OGF", "Formal power series"],
                    "plain_label": "母函数与形式幂级数",
                    "description": "数列、截断几何和与形式幂级数之间的代数与分析边界专题。",
                },
            },
            {
                "local_ref": "r.seq",
                "kind": "representation_core",
                "op": "create",
                "payload": {
                    "home_theory": new("e.theory"),
                    "preferred_labels": [
                        {"lang": "zh", "label": "自然数下标数列表示核"},
                        {"lang": "en", "label": "Nat-indexed sequence representation"},
                    ],
                    "aliases": ["数列表示", "ℕ → R"],
                    "plain_label": "数列表示核 ℕ → R",
                    "carrier": {
                        "text": "以自然数 $\\mathbb{N}$ 为下标的系数函数空间 $\\mathbb{N} \\to R$",
                        "note": "Lean: ℕ → R",
                    },
                    "binders": [{"name": "R", "type": "CommRing", "role": "parameter"}],
                    "operations": [
                        {
                            "name": "term",
                            "signature": {"text": "$f(n)$：读取第 $n \\in \\mathbb{N}$ 项"},
                            "note": "逐项求值算子",
                        }
                    ],
                    "invariants_and_domain": [
                        {"text": "两个数列相等当且仅当对一切 $n \\in \\mathbb{N}$ 有 $f(n) = g(n)$。"}
                    ],
                    "conventions": [
                        {
                            "key": "sequence_index_origin",
                            "subject": "f",
                            "value": 0,
                            "text": "共享数列表示默认从 $n = 0$ 起算",
                            "basis": "mathlib",
                        }
                    ],
                    "description": "母函数编码前的离散数列承载表示。",
                },
            },
            {
                "local_ref": "rb.jia.seq",
                "kind": "representation_binding",
                "op": "create",
                "payload": {
                    "dialect_scope": ref("ds.jia"),
                    "representation": new("r.seq"),
                    "symbol_map": [
                        {
                            "local_symbol": "a_n",
                            "target_expr": {"text": "$f(n) \\in R$（$n \\ge 0$）"},
                            "role": "sequence",
                            "note": "星港从 0 起算，直接恒等对应",
                        }
                    ],
                    "evidence_spans": [ref("jia.s.def")],
                    "status": "context_checked",
                    "map_label": "绑定为数列表示（0 起算直连）",
                },
            },
            {
                "local_ref": "rb.yi.seq",
                "kind": "representation_binding",
                "op": "create",
                "payload": {
                    "dialect_scope": ref("ds.yi"),
                    "representation": new("r.seq"),
                    "symbol_map": [
                        {
                            "local_symbol": "b_m",
                            "target_expr": {"text": "$f(m) \\in \\mathbb{R}$（$m \\ge 1$），补 $f(0) = 0$"},
                            "role": "sequence",
                            "note": "灯塔无第 0 项，嵌入 ℕ → ℝ 时显式置零第 0 项",
                        }
                    ],
                    "index_and_domain_adaptations": [
                        {
                            "text": "将 $(b_m)_{m \\ge 1}$ 延拓到 $\\mathbb{N}$：规定 $f(0) = 0$，对 $m \\ge 1$ 取 $f(m) = b_m$。"
                        }
                    ],
                    "evidence_spans": [ref("yi.s.def")],
                    "status": "partially_aligned",
                    "map_label": "绑定为数列表示（首项补 f(0)=0）",
                },
            },
        ],
        "proposed_relations": [],
        "unresolved": [],
    }
)

cs_rep_fps = finish(
    {
        "request_id": REQ["rep-fps"],
        "change_set_id": CS["rep-fps"],
        "version": 1,
        "target_object": "r.fps",
        "rationale": "建立形式幂级数表示核 R_fps，绑定两本书的母函数记号，并把《灯塔》实收敛半径要求挂为表示绑定上的账目。",
        "proposed_objects": [
            {
                "local_ref": "e.ogf_concept",
                "kind": "semantic_entity",
                "op": "create",
                "payload": {
                    "semantic_kind": "concept",
                    "home_theory": ref("e.theory"),
                    "preferred_labels": [
                        {"lang": "zh", "label": "普通母函数"},
                        {"lang": "en", "label": "Ordinary generating function"},
                    ],
                    "aliases": ["生成函数", "OGF"],
                    "plain_label": "普通母函数",
                    "description": "将下标数列编码为未定元幂次的级数对象。",
                },
            },
            {
                "local_ref": "r.fps",
                "kind": "representation_core",
                "op": "create",
                "payload": {
                    "home_theory": ref("e.theory"),
                    "preferred_labels": [
                        {"lang": "zh", "label": "形式幂级数表示核"},
                        {"lang": "en", "label": "Formal power series representation"},
                    ],
                    "aliases": ["FPS 表示", "R[[X]]", "PowerSeries"],
                    "plain_label": "形式幂级数表示核 R[[X]]",
                    "carrier": {
                        "text": "交换环 $R$ 上的形式幂级数环 $R[[X]]$",
                        "note": "Lean: PowerSeries R",
                    },
                    "represented_object": new("e.ogf_concept"),
                    "binders": [{"name": "R", "type": "CommRing", "role": "parameter"}],
                    "operations": [
                        {
                            "name": "coeff",
                            "signature": {
                                "text": "$[X^n] F(X)$：提取第 $n$ 次项系数",
                                "note": "PowerSeries.coeff R n",
                            },
                            "note": "系数投影",
                        },
                        {
                            "name": "mul",
                            "signature": {
                                "text": "$(F \\cdot G)(X)$：Cauchy 卷积乘法",
                                "note": "HMul.hMul",
                            },
                            "note": "对应数列卷积",
                        },
                    ],
                    "invariants_and_domain": [
                        {
                            "text": "纯代数形式比较：两级数相等当且仅当各次项系数全相等，不涉及将实数代入 $X$ 的分析收敛性。"
                        }
                    ],
                    "mathlib_baseline": {
                        "decl": "PowerSeries",
                        "module": "Mathlib.RingTheory.PowerSeries.Basic",
                        "status": "unchecked",
                        "note": "mathlib 中一元数据形式幂级数环",
                    },
                    "uses_entities": [new("e.ogf_concept")],
                    "description": "母函数的标准代数承载表示，阶段三将在此与 R_seq 之间架设 OGF 双向运输边 T。",
                },
            },
            {
                "local_ref": "rb.jia.fps",
                "kind": "representation_binding",
                "op": "create",
                "payload": {
                    "dialect_scope": ref("ds.jia"),
                    "representation": new("r.fps"),
                    "symbol_map": [
                        {
                            "local_symbol": "A(x)",
                            "target_expr": {"text": "$F \\in R[[X]]$"},
                            "role": "series",
                            "note": "未定元 x 对应形式未定元 X",
                        },
                        {
                            "local_symbol": "[x^n]",
                            "target_expr": {"text": "$\\mathrm{coeff}_R(n) : R[[X]] \\to R$"},
                            "role": "operator",
                            "note": "系数提取算子一一对应",
                        },
                    ],
                    "evidence_spans": [ref("jia.s.def")],
                    "status": "context_checked",
                    "map_label": "绑定为形式幂级数表示",
                },
            },
            {
                "local_ref": "rb.yi.fps",
                "kind": "representation_binding",
                "op": "create",
                "payload": {
                    "dialect_scope": ref("ds.yi"),
                    "representation": new("r.fps"),
                    "symbol_map": [
                        {
                            "local_symbol": "g(t)",
                            "target_expr": {
                                "text": "$F(X) = \\sum_{m \\ge 1} b_m X^m \\in \\mathbb{R}[[X]]$（$[X^0]F = 0$）"
                            },
                            "role": "series",
                            "note": "剥离收敛区间 (-r, r) 后提升为常数项为 0 的形式幂级数",
                        }
                    ],
                    "index_and_domain_adaptations": [
                        {
                            "text": "首项从 $m=1$ 起算对应常数项 $[X^0]F(X) = 0$；把实自变量 $t \\in (-r, r)$ 视作形式未定元 $X$。"
                        }
                    ],
                    "lost_information": [
                        {
                            "short": "原书要求 (-r, r) 内绝对收敛",
                            "claim": "灯塔要求存在 r > 0 使 ∑ b_m t^m 在 (-r, r) 内绝对收敛；形式幂级数表示核不保留解析收敛域",
                            "direction": "source_has_target_lacks",
                        }
                    ],
                    "evidence_spans": [ref("yi.s.def")],
                    "status": "partially_aligned",
                    "map_label": "绑定为形式幂级数（剥离收敛半径）",
                },
            },
            {
                "local_ref": "d.yi.analytic_radius",
                "kind": "debt",
                "op": "create",
                "payload": {
                    "debt_kind": "assumption_debt",
                    "attached_to": new("rb.yi.fps"),
                    "status": "open",
                    "obligation": "若需要把形式幂级数结论代回实数 $t$ 求值，必须验证点 $t$ 落在《灯塔》要求的绝对收敛区间 $(-r, r)$ 内。",
                    "quantifier": "∀ t ∈ (-r, r), r > 0",
                    "failure_mode": "theorem",
                    "failure_note": "在收敛半径之外（如 |t| ≥ 1）直接代值会导致发散级数伪等式。",
                    "evidence_spans": [ref("yi.s.def"), ref("yi.s.rem")],
                },
            },
        ],
        "proposed_relations": [],
        "unresolved": [],
    }
)

cs_geom_fin = finish(
    {
        "request_id": REQ["geom-fin"],
        "change_set_id": CS["geom-fin"],
        "version": 1,
        "target_object": "e.geom_fin",
        "rationale": "建立带完整守卫、双分支（x≠1 常规分式与 x=1 退化常数）与分母排除说明的有限几何级数定理，并分别用分支感知对齐连接两本书。",
        "proposed_objects": [
            {
                "local_ref": "e.geom_fin",
                "kind": "semantic_entity",
                "op": "create",
                "payload": {
                    "semantic_kind": "declaration",
                    "home_theory": ref("e.theory"),
                    "preferred_labels": [
                        {"lang": "zh", "label": "有限几何级数求和公式"},
                        {"lang": "en", "label": "Finite geometric series sum"},
                    ],
                    "aliases": ["有限等比求和", "geom_sum_eq"],
                    "plain_label": "有限几何级数求和（含 x=1 退化分支）",
                    "mathlib_baseline": {
                        "decl": "geom_sum_eq",
                        "module": "Mathlib.Algebra.GeomSum",
                        "status": "unchecked",
                        "note": "mathlib 中 geom_sum_eq 对应 x ≠ 1 分支，geom_sum_one 对应 x = 1 退化分支",
                    },
                    "declaration": {
                        "declaration_kind": "theorem",
                        "binders": [
                            {"name": "N", "type": "ℕ", "role": "universal"},
                            {"name": "x", "type": "ℝ", "role": "universal"},
                        ],
                        "assumptions": [{"text": "$N \\ge 1$"}],
                        "statement": {
                            "text": "有限几何和 $\\sum_{k=0}^{N-1} x^k$ 按 $x \\neq 1$ 与 $x = 1$ 分两支给出精确值。",
                            "evidence_spans": [ref("jia.s.thm")],
                        },
                        "guards": [
                            {
                                "guard_id": "regular_x_ne_1",
                                "label": "常规公比（x ≠ 1）",
                                "predicate": {
                                    "text": "$x \\neq 1$",
                                    "evidence_spans": [ref("jia.s.thm"), ref("yi.s.prop")],
                                },
                                "evidence_spans": [ref("jia.s.thm")],
                            },
                            {
                                "guard_id": "degenerate_x_eq_1",
                                "label": "单位公比退化（x = 1）",
                                "predicate": {
                                    "text": "$x = 1$",
                                    "evidence_spans": [ref("jia.s.thm")],
                                },
                                "evidence_spans": [ref("jia.s.thm")],
                            },
                        ],
                        "guard_partition": {
                            "status": "partition",
                            "note": "在实数域 ℝ 上 x ≠ 1 与 x = 1 互斥且穷尽全部公比取值。",
                        },
                        "branches": [
                            {
                                "branch_id": "fraction_form",
                                "label": "分式闭合式",
                                "guards": ["regular_x_ne_1"],
                                "is_degenerate": False,
                                "conclusion": {
                                    "text": "$\\sum_{k=0}^{N-1} x^k = \\frac{1 - x^N}{1 - x}$"
                                },
                                "evidence_spans": [ref("jia.s.thm"), ref("yi.s.prop")],
                            },
                            {
                                "branch_id": "constant_sum",
                                "label": "退化常数和",
                                "guards": ["degenerate_x_eq_1"],
                                "is_degenerate": True,
                                "conclusion": {"text": "$\\sum_{k=0}^{N-1} 1^k = N$"},
                                "evidence_spans": [ref("jia.s.thm")],
                            },
                        ],
                        "exclusions": [
                            {
                                "case": {"text": "在 $x = 1$ 处代入分式 $\\frac{1 - x^N}{1 - x}$"},
                                "kind": "undefined",
                                "reason": "分母 1 - x = 0 使实数除法无定义，必须切入 constant_sum 退化分支。",
                                "evidence_spans": [ref("jia.s.thm")],
                            }
                        ],
                        "uses_entities": [],
                    },
                },
            },
            {
                "local_ref": "a.jia.thm",
                "kind": "alignment",
                "op": "create",
                "payload": {
                    "source": ref("jia.u.thm"),
                    "target": new("e.geom_fin"),
                    "relation": "expresses",
                    "source_context": {"dialect_scope": ref("ds.jia")},
                    "target_context": ref("e.theory"),
                    "target_branches": ["fraction_form", "constant_sum"],
                    "evidence_spans": [ref("jia.s.thm")],
                    "status": "context_checked",
                    "plain_note": "星港定理 2.1.2 同时完整陈述了 x ≠ 1 分式分支与 x = 1 退化常数分支。",
                },
            },
            {
                "local_ref": "a.yi.prop",
                "kind": "alignment",
                "op": "create",
                "payload": {
                    "source": ref("yi.u.prop"),
                    "target": new("e.geom_fin"),
                    "relation": "specializes",
                    "source_context": {"dialect_scope": ref("ds.yi")},
                    "target_context": ref("e.theory"),
                    "target_branches": ["fraction_form"],
                    "parameter_map": [
                        {
                            "source": "t",
                            "target": "x",
                            "role": "statement_parameter",
                            "map": "灯塔的实公比 t（t ≠ 1）对应共享定理常规分支的 x",
                            "source_range": "t ∈ ℝ, t ≠ 1",
                            "target_range": "x ∈ ℝ",
                        }
                    ],
                    "lost_information": [
                        {
                            "short": "漏写 t = 1 退化分支（和为 N）",
                            "claim": "灯塔命题 1.3.2 假设 t ≠ 1，未写出 t = 1 时和等于 N 的退化分支 constant_sum",
                            "direction": "target_has_source_lacks",
                        }
                    ],
                    "evidence_spans": [ref("yi.s.prop")],
                    "status": "partially_aligned",
                },
            },
        ],
        "proposed_relations": [],
        "unresolved": [],
    }
)

cs_geom_inf = finish(
    {
        "request_id": REQ["geom-inf"],
        "change_set_id": CS["geom-inf"],
        "version": 1,
        "target_object": "e.geom_inf_fps",
        "rationale": "建立形式几何级数逆元恒等式及其 |x| ≥ 1 实分析发散排除边界，并关联两本书的推论与注记。",
        "proposed_objects": [
            {
                "local_ref": "e.geom_inf_fps",
                "kind": "semantic_entity",
                "op": "create",
                "payload": {
                    "semantic_kind": "declaration",
                    "home_theory": ref("e.theory"),
                    "preferred_labels": [
                        {"lang": "zh", "label": "形式几何级数乘法逆元"},
                        {"lang": "en", "label": "Formal geometric series inverse"},
                    ],
                    "aliases": ["(1 - X)⁻¹", "形式等比级数"],
                    "plain_label": "形式几何级数逆元 (1 - X)∑Xⁿ = 1",
                    "declaration": {
                        "declaration_kind": "theorem",
                        "concept": ref("e.ogf_concept"),
                        "binders": [{"name": "R", "type": "CommRing", "role": "parameter"}],
                        "statement": {
                            "text": "在形式幂级数环 $R[[X]]$ 中，$(1 - X)\\sum_{n=0}^{\\infty} X^n = 1$。",
                            "evidence_spans": [ref("jia.s.cor")],
                        },
                        "exclusions": [
                            {
                                "case": {"text": "实变数 $|x| \\ge 1$ 处的无穷级数求值 $\\sum_{n=0}^{\\infty} x^n$"},
                                "kind": "diverges",
                                "reason": "当 |x| ≥ 1 时实级数发散，不能将形式逆元恒等式当作实数值收敛等式使用。",
                                "evidence_spans": [ref("jia.s.cor"), ref("yi.s.rem")],
                            }
                        ],
                        "uses_entities": [ref("e.ogf_concept")],
                    },
                },
            },
            {
                "local_ref": "a.jia.cor",
                "kind": "alignment",
                "op": "create",
                "payload": {
                    "source": ref("jia.u.cor"),
                    "target": new("e.geom_inf_fps"),
                    "relation": "expresses",
                    "source_context": {"dialect_scope": ref("ds.jia")},
                    "target_context": ref("e.theory"),
                    "evidence_spans": [ref("jia.s.cor")],
                    "status": "context_checked",
                },
            },
            {
                "local_ref": "a.yi.rem",
                "kind": "alignment",
                "op": "create",
                "payload": {
                    "source": ref("yi.u.rem"),
                    "target": new("e.geom_inf_fps"),
                    "relation": "overlaps_with",
                    "source_context": {"dialect_scope": ref("ds.yi")},
                    "target_context": ref("e.theory"),
                    "required_assumptions": [
                        {
                            "text": "实数代值需要满足 |t| < 1 收敛条件",
                            "debt": ref("d.yi.analytic_radius"),
                        }
                    ],
                    "lost_information": [
                        {
                            "short": "仅在 |t| < 1 实收敛域成立，非形式环恒等式",
                            "claim": "灯塔注 1.3.3 讨论的是 |t| < 1 上的实级数极限，而共享定理是 R[[X]] 上的无条件形式乘法逆元",
                            "direction": "both" if False else "source_has_target_lacks",
                        }
                    ],
                    "evidence_spans": [ref("yi.s.rem")],
                    "status": "partially_aligned",
                },
            },
        ],
        "proposed_relations": [],
        "unresolved": [],
    }
)

doc = {
    "fixture": "emir-generating-functions-two-books",
    "fixture_version": "0.1",
    "label": "阶段二（S2 完整 + S3 前半）双书母函数与退化边界黄金夹具",
    "investigation_id": INVESTIGATION,
    "artifacts": {
        "jia": {
            "file": "materials/星港母函数讲义.md",
            "artifact_id": ART_JIA,
            "role": "canonical Markdown 快照（占位）",
        },
        "yi": {
            "file": "materials/灯塔母函数初步.md",
            "artifact_id": ART_YI,
            "role": "canonical Markdown 快照（占位）",
        },
    },
    "id_rules": {
        "change_set_id": "mathscape:change-set/sha256(investigation_id \\0 request_id)",
        "new_object": "mathscape:<kind>/sha256(change_set_id \\0 local_ref)",
        "kinds": KIND_PREFIX,
    },
    "change_sets": [cs_jia, cs_yi, cs_rep_seq, cs_rep_fps, cs_geom_fin, cs_geom_inf],
}

(ROOT / "objects.json").write_text(
    json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
)
print(
    f"Wrote objects.json with {len(doc['change_sets'])} change sets and {len(STAGED)} staged objects."
)
