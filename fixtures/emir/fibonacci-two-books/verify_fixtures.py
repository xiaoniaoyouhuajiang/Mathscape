#!/usr/bin/env python3
"""核对 emir-fib-two-books 夹具：① 数学（精确整数运算）；② objects.json 结构不变量。
任一断言失败即退出非零。用法：python3 verify_fixtures.py > verify_output.txt
"""
import hashlib, json, pathlib, re, sys
from fractions import Fraction

ROOT = pathlib.Path(__file__).resolve().parent
N = 60
ok_count = 0

def check(cond, msg):
    global ok_count
    if not cond:
        print("  ✗ 失败：" + msg); sys.exit(1)
    ok_count += 1

def section(t):
    print("\n" + "=" * 64 + "\n" + t + "\n" + "=" * 64)

# ---------------- 数学 ----------------
F = [0, 1]                       # 星港：F_0 = 0, F_1 = 1, F_{n+1} = F_n + F_{n-1}
while len(F) < N + 4: F.append(F[-1] + F[-2])
f = {1: 1, 2: 1}                 # 灯塔：f_1 = f_2 = 1, f_{n+2} = f_{n+1} + f_n，无 f_0
for n in range(1, N + 2): f[n + 2] = f[n + 1] + f[n]

def mat_mul(A, B):
    return [[A[0][0]*B[0][0]+A[0][1]*B[1][0], A[0][0]*B[0][1]+A[0][1]*B[1][1]],
            [A[1][0]*B[0][0]+A[1][1]*B[1][0], A[1][0]*B[0][1]+A[1][1]*B[1][1]]]
Q = [[1, 1], [1, 0]]
def mpow(n):
    R = [[1, 0], [0, 1]]
    for _ in range(n): R = mat_mul(R, Q)
    return R

section("1. 两本书的定义与“前几项”")
print("  星港 F_0..F_10 =", F[:11])
print("  灯塔 f_1..f_10 =", [f[k] for k in range(1, 11)])
check(F[:9] == [0, 1, 1, 2, 3, 5, 8, 13, 21], "星港列出的前几项")
check([f[k] for k in range(1, 9)] == [1, 1, 2, 3, 5, 8, 13, 21], "灯塔列出的前几项")
check(all(f[k] == F[k] for k in range(1, N + 1)), "符号对照 f_k = F_k (k≥1)")
check(0 not in f, "灯塔没有第 0 项")
print(f"  符号对照 f_k = F_k 对 k = 1..{N} 成立 ✓；灯塔的定义域不含 0 ✓（F_0 = {F[0]} 在灯塔中无对应）")

section("2. 矩阵幂：星港 引理 3.2.2 / 灯塔 命题 2.4.2 / 共享 e.qpow")
for n in range(1, N + 1):
    check(mpow(n) == [[F[n+1], F[n]], [F[n], F[n-1]]], f"星港 Q^{n}")
for n in range(1, N):
    check(mpow(n + 1) == [[f[n+2], f[n+1]], [f[n+1], f[n]]], f"灯塔 M^{n+1}")
print(f"  星港：Q^n = [[F_(n+1),F_n],[F_n,F_(n-1)]]，n = 1..{N} ✓（n=1 用到 F_0：Q^1 = {mpow(1)}）")
print(f"  灯塔：M^(n+1) = [[f_(n+2),f_(n+1)],[f_(n+1),f_n]]，n = 1..{N-1} ✓")
shared_cover_yi = sorted({n + 1 for n in range(1, N)})
check(shared_cover_yi[0] == 2 and 1 not in shared_cover_yi, "灯塔经 n_共享 = n_灯塔+1 只覆盖 n_共享 ≥ 2")
print("  binder 对照 n_共享 = n_灯塔 + 1：灯塔覆盖共享的 n ≥ 2，缺 n = 1 → specializes ✓")

section("3. Cassini：星港 定理 3.2.3 / 灯塔 命题 2.4.3 / 共享 e.cassini")
for n in range(1, N + 1):
    check(F[n+1]*F[n-1] - F[n]**2 == (-1)**n, f"星港 Cassini n={n}")
for n in range(1, N):
    lhs_yi = f[n]*f[n+2] - f[n+1]**2
    check(lhs_yi == (-1)**(n+1), f"灯塔 Cassini n={n}")
    m = n + 1   # binder map
    check(lhs_yi == F[m+1]*F[m-1] - F[m]**2 and (-1)**(n+1) == (-1)**m, f"灯塔 n={n} ↔ 共享 n={m}")
print(f"  星港：F_(n+1)F_(n-1) − F_n² = (−1)^n，n = 1..{N} ✓")
print(f"  灯塔：f_n f_(n+2) − f_(n+1)² = (−1)^(n+1)，n = 1..{N-1} ✓")
print("  灯塔第 n 条 = 共享第 n+1 条（两边逐项相等）✓")
print(f"  共享 n = 1：F_2·F_0 − F_1² = {F[2]}·{F[0]} − {F[1]}² = {F[2]*F[0]-F[1]**2}（= 星港 注 3.2.4，用到 F_0，灯塔无法写出）")
check(F[2]*F[0] - F[1]**2 == -1, "注 3.2.4")
# 反例：v1 的“逻辑等价 / 定义等价”为何不成立
print("  v1→v2 理由：若把灯塔的式子硬套到 n = 0 以得到 n_共享 = 1，需要 f_0，灯塔未定义 → 不能写成 logically_equivalent ✓")

section("4. 剪拼悖论：灯塔 习题 2.4.7 / 共享题型 e.puzzle（k = n + 1）")
def shoelace(P):
    s = sum(Fraction(P[i][0]) * P[(i+1) % len(P)][1] - Fraction(P[(i+1) % len(P)][0]) * P[i][1] for i in range(len(P)))
    return abs(s) / 2
for n in range(3, 31):
    s, a, b = f[n+1], f[n-1], f[n]        # 边长、三角形短直角边、梯形长底=高
    A = [(0, 0), (s, 0), (0, a)]; B = [(s, 0), (s, a), (0, a)]
    C = [(0, a), (a, a), (b, s), (0, s)]; D = [(a, a), (s, a), (s, s), (b, s)]
    areas = [shoelace(X) for X in (A, B, C, D)]
    check(b - a > 0, f"n={n} 梯形不退化")
    check(sum(areas) == s * s, f"n={n} 四块面积和 = f_(n+1)²")
    check(areas[0] == Fraction(s * a, 2) and areas[2] == Fraction((a + b) * b, 2), f"n={n} 三角形/梯形面积公式")
    rect = f[n] * f[n+2]
    check(f[n+2] == s + b, f"n={n} 长方形长边 = f_(n+1)+f_n")
    diff = rect - s * s
    check(diff == (-1)**(n+1), f"n={n} 面积差")
    check((diff == 1) == (n % 2 == 1), f"n={n} 奇数→缝，偶数→重叠")
    # 剪法：长条 y∈[0,a]（f_(n-1)×f_(n+1)）沿对角线剪；长条 y∈[a,s]（f_n×f_(n+1)）
    # 从长边 y=s 上距左端 f_n 的点 (b,s) 剪到对边 y=a 上距同侧端点 f_(n-1) 的点 (a,a)
    check(a + b == s, f"n={n} 两个长条 f_(n-1)+f_n = f_(n+1)")
    check(C[1] == (a, a) and C[2] == (b, s), f"n={n} 梯形切线端点与题面一致")
    sides = lambda P: sorted((P[i][0]-P[i-1][0])**2 + (P[i][1]-P[i-1][1])**2 for i in range(len(P)))
    check(sides(C) == sides(D), f"n={n} 两个梯形全等")
    # 拼成 b×(s+b)：下方边界 (0,b)→(b,a)→(s+b,0)（C 与 A），上方边界 (0,b)→(s,b−a)→(s+b,0)（B 与 D）
    A2 = [(b, 0), (s+b, 0), (b, a)]; B2 = [(0, b), (s, b), (s, b-a)]
    C2 = [(0, 0), (b, 0), (b, a), (0, b)]; D2 = [(s+b, b), (s, b), (s, b-a), (s+b, 0)]
    for X, Y in ((A, A2), (B, B2), (C, C2), (D, D2)):
        check(sides(X) == sides(Y) and shoelace(X) == shoelace(Y), f"n={n} 拼块刚体对应")
    par = [(0, b), (b, a), (s+b, 0), (s, b-a)]
    v = lambda P, Q: (Q[0]-P[0], Q[1]-P[1])
    check(v(par[0], par[1]) == v(par[3], par[2]) and v(par[1], par[2]) == v(par[0], par[3]), f"n={n} 中间区域是平行四边形")
    check(shoelace(par) == 1, f"n={n} 平行四边形面积 = 1")
    upper_at_b = Fraction(b) - Fraction(a * b, s)           # 上方边界（B 斜边）在 x=b 处的高度
    gap = a < upper_at_b                                    # 下方拼块顶点在上方边界之下 → 缝；之上 → 重叠
    check(gap == (diff == 1) == (n % 2 == 1), f"n={n} 奇数 n 缝 / 偶数 n 重叠（几何判定）")
    k = n + 1
    check(gap == (k % 2 == 0), f"k={k} 偶数缝 / 奇数重叠")
    check(F[k] == s and F[k-1] * F[k+1] == rect and F[k+1]*F[k-1] - F[k]**2 == (-1)**k == diff, f"n={n} ↔ k={k}")
print("  n = 3..30：剪法（两长条、梯形切点距端点 f_n / f_(n-1)）与坐标一致，两梯形全等，拼块刚体对应 ✓")
print("  n = 3..30：中间区域是沿对角线的细长平行四边形，面积恰为 1；奇数 n（偶数 k）为缝，偶数 n（奇数 k）为重叠 ✓")
print("  n = 3..30：四块面积和 = f_(n+1)²；长方形 f_n × f_(n+2)；差 = (−1)^(n+1) ✓；奇数 n 多一条缝、偶数 n 重叠 ✓")
print(f"  (3) n = 5：正方形 {f[6]}×{f[6]} = {f[6]**2}，三角形直角边 {f[6]}, {f[4]}，梯形两底 {f[4]}, {f[5]} 高 {f[5]}，"
      f"拼成 {f[5]}×{f[7]} = {f[5]*f[7]}，差 +1（缝）")
check((f[6], f[5], f[7]) == (8, 5, 13), "n=5 即 8×8→5×13")
print(f"  与引导动画（verify.py）一致：灯塔 n=5 ↔ 共享 k=6 ↔ 动画 8×8→5×13 ✓；灯塔 n=4：5×5→{f[4]}×{f[6]}={f[4]*f[6]}（−1）✓；灯塔 n=7：{f[8]}×{f[8]}→{f[7]}×{f[9]}={f[7]*f[9]}（+1）✓")
check(f[4]*f[6] == 24 and f[7]*f[9] == 442 and f[8] == 21, "与动画同族数字")
print("  k = 3 时梯形两底 F_1 = F_2 = 1，退化 → 题型取 k ≥ 4 ✓")
check(F[1] == F[2], "k=3 退化")

doc0 = json.loads((ROOT / "objects.json").read_text(encoding="utf-8"))
task = [o for c in doc0["change_sets"] for o in c["proposed_objects"] if o["local_ref"] == "e.puzzle"][0]["payload"]["problem_schema"]["task"]["text"]
def _strings(x):
    if isinstance(x, str): yield x
    elif isinstance(x, dict):
        for v in x.values(): yield from _strings(v)
    elif isinstance(x, list):
        for v in x: yield from _strings(v)
check(not any(c in t for t in _strings(doc0) for c in "\t\b\f\r\v"), "TeX 字符串里没有被误转义的控制字符（如 \\times → 制表符）")
check("$k$ 为偶数时" in task and "缝" in task and "$k$ 为奇数时" in task and "重叠" in task and task.index("偶数") < task.index("奇数"), "题型 task 写明 k 偶数缝 / k 奇数重叠")
yi = (ROOT / "materials" / "灯塔组合数学初步.md").read_text(encoding="utf-8")
check("距端点 $f_n$ 处" in yi and "距同侧端点 $f_{n-1}$ 处" in yi and "面积为 1" in yi, "灯塔习题含剪法与缝/重叠说明")
print("  共享题型 task：“k 为偶数时有缝，k 为奇数时重叠”与几何判定一致 ✓；灯塔习题已写明剪法与面积为 1 的平行四边形 ✓")

section("4b. 教程分镜里出现的数字（tutorial-storyboard.md 镜 5、6、9、11）")
check([F[k] for k in range(7)] == [0, 1, 1, 2, 3, 5, 8] and [f[k] for k in range(1, 7)] == [1, 1, 2, 3, 5, 8], "镜 5 数表")
check(f[1]*f[3] - f[2]**2 == 1 == F[3]*F[1] - F[2]**2, "镜 6：灯塔第 1 条 = 共享第 2 条 = 1")
check(f[2]*f[4] - f[3]**2 == -1 == F[4]*F[2] - F[3]**2, "镜 6：灯塔第 2 条 = 共享第 3 条 = −1")
check(F[2]*F[0] - F[1]**2 == -1, "镜 6：共享第 1 条 = −1，要用 F_0")
check(f[6] == F[6] == 8, "镜 9：灯塔 n=5 的正方形边长 f_6 = F_6 = 8（k = 6）")
print("  镜 5 数表、镜 6 三组具体数字、镜 9 的 8×8 ↔ k = 6 ✓")

# ---------------- 结构不变量 ----------------
section("5. objects.json 结构不变量（契约草案的确定性校验）")
doc = doc0
sha = lambda b: hashlib.sha256(b).hexdigest()
inv = doc["investigation_id"]
files = {v["artifact_id"]: (ROOT / v["file"]).read_bytes() for v in doc["artifacts"].values()}
for aid, data in files.items():
    check(aid == "mathscape:artifact/sha256/" + sha(data), "artifact_id = 文件 sha256")
    check("自行编写".encode() in data[:400], "材料开头的注释标明是自行编写的")

PREFIX = doc["id_rules"]["kinds"]
CSS = doc["change_sets"]
all_staged = {}
for cs in CSS:
    check(cs["change_set_id"] == "mathscape:change-set/" + sha(f"{inv}\0{cs['request_id']}".encode()), "change_set_id 推算")
    for lr, sid in cs["staged_ids"].items():
        check(sid not in all_staged, f"id 唯一 {lr}")
        all_staged[sid] = (cs["change_set_id"], lr)
kind_of = {}
for cs in CSS:
    for o in cs["proposed_objects"]:
        kind_of[cs["staged_ids"][o["local_ref"]]] = o["kind"]
    for r in cs["proposed_relations"]:
        kind_of[cs["staged_ids"][r["local_ref"]]] = "source_relation"

ALIGN_REL = {"notation_for", "expresses", "exact_realization", "definitionally_equivalent", "logically_equivalent",
             "specializes", "generalizes", "instance_of", "representation_of", "overlaps_with", "incompatible_with"}
AGENT_ALIGN_STATUS = {"unmatched", "retrieved_candidate", "context_checked", "partially_aligned", "ambiguous", "disputed", "incompatible"}
AGENT_DEBT_STATUS = {"open", "claimed_paid"}
DEBT_KINDS = {"parse_debt", "scope_debt", "typing_debt", "assumption_debt", "alignment_debt", "formalization_debt", "proof_debt"}
CONV_KEYS = {"sequence_index_origin", "notation", "terminology", "baseline", "other"}
count = {}
deps_found = {}
for cs in CSS:
    cid = cs["change_set_id"]
    local = {o["local_ref"]: o for o in cs["proposed_objects"] + cs["proposed_relations"]}
    used = set(); deps = set()
    def resolve(ref, want=None):
        if ref.startswith("$new:"):
            lr = ref[5:]; check(lr in local, f"dangling_new_ref $new:{lr}"); used.add(lr)
            sid = cs["staged_ids"][lr]
        else:
            check(ref in all_staged, f"dangling_staged_ref {ref}")
            check(all_staged[ref][0] != cid, "跨 change set 引用必须指向其他 change set")
            deps.add(all_staged[ref][0]); sid = ref
        if want: check(kind_of[sid] in want, f"{ref} 种类应为 {want}，实为 {kind_of[sid]}")
        return sid
    def refs_in(x):
        b = json.dumps(x, ensure_ascii=False)
        return re.findall(r'"(\$new:[^"]+)"', b) + re.findall(r'"(mathscape:[a-z-]+/[0-9a-f]{64})"', b)
    check(cs["target_object"] in local, "target_object 属于本 change set")
    sem = [o for o in cs["proposed_objects"] if o["kind"] == "semantic_entity"]
    if sem:
        tgt = local[cs["target_object"]]
        check(tgt["kind"] == "semantic_entity", "语义 change set 的目标是共享实体")
        # 一个目标：除目标和它的 home_theory / 被目标 definiens 引用的概念外，不再新建其他“主角”实体
        extra = [o["local_ref"] for o in sem if o["local_ref"] != cs["target_object"]]
        allowed = {tgt["payload"]["home_theory"][5:] if (tgt["payload"]["home_theory"] or "").startswith("$new:") else None}
        allowed |= {o["local_ref"] for o in sem if o["payload"].get("declaration", {}) and o["payload"]["declaration"].get("concept") == "$new:" + cs["target_object"]}
        check(set(extra) <= allowed, f"{cs['target_object']} change set 只围绕一个目标：多余实体 {set(extra) - allowed}")
    for o in cs["proposed_objects"]:
        lr, p, k = o["local_ref"], o["payload"], o["kind"]
        count[k] = count.get(k, 0) + 1
        check(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.\-]{0,63}", lr) is not None, f"invalid_local_ref {lr}")
        check("trust" not in p, f"{lr} forbidden_trust_claim")
        check(o["op"] == "create", f"{lr} 夹具只含 create")
        if k == "source_span":
            data = files[p["artifact_id"]]; r = p["md_byte_range"]
            check(0 <= r["start"] < r["end"] <= len(data) and r["end"] - r["start"] <= 4096, f"{lr} 范围")
            check(data[r["start"]:r["end"]].decode("utf-8") == p["quote"], f"{lr} quote_mismatch")
            ident = json.dumps(["markdown-source-span-1", p["artifact_id"], {"start": r["start"], "end": r["end"]}],
                               separators=(",", ":"), ensure_ascii=False).encode()
            check(cs["staged_ids"][lr] == "mathscape:source-span/" + sha(ident), f"{lr} span id 推算")
            continue
        check(cs["staged_ids"][lr] == f"mathscape:{PREFIX[k]}/" + sha(f"{cid}\0{lr}".encode()), f"{lr} id 推算")
        if k == "source_unit":
            check(len(p["source_spans"]) >= 1, f"{lr} 至少回链一个 span")
            for x in p["source_spans"]: resolve(x, {"source_span"})
            if p["parent"]: resolve(p["parent"], {"source_unit"})
            check(p["kind"] in {"section", "definition", "statement", "proof", "problem", "hint", "solution", "example", "remark", "method", "notation", "narrative"}, f"{lr} unit kind（无 theorem）")
        elif k == "dialect_scope":
            resolve(p["anchor_unit"], {"source_unit"}); resolve(p["introduced_by_unit"], {"source_unit"})
            check(p["applies_to"] == "anchor_subtree" and p["parent_scope"] is None, f"{lr} 最小 DialectScope 字段")
            check(p["conventions"], f"{lr} 至少一条约定")
            for c in p["conventions"]:
                check(c["key"] in CONV_KEYS and c["evidence_spans"], f"{lr} 约定 key / 证据")
                check(c["explicitness"] in {"explicit", "inherited", "conventional", "inferred"}, f"{lr} explicitness")
                for x in c["evidence_spans"]: resolve(x, {"source_span"})
        elif k == "semantic_entity":
            check(p["semantic_kind"] in {"concept", "declaration", "theory", "problem_schema"}, f"{lr} kind_not_supported")
            check(not any(x in p for x in ("page", "source_spans")), f"{lr} 共享实体不占有来源（不变量 9）")
            if p["home_theory"]: resolve(p["home_theory"], {"semantic_entity"})
            else: check(p["semantic_kind"] == "theory", f"{lr} 非 theory 必须有 home_theory")
            check("conventions" in p and "mathlib_baseline" in p, f"{lr} 带 conventions / mathlib_baseline 字段（决定 a）")
            for c in p["conventions"]: check(c["key"] in CONV_KEYS and c.get("basis") == "mathlib", f"{lr} 约定注明 mathlib 基线")
            if p["semantic_kind"] in ("declaration", "problem_schema"):
                check(any(c["key"] == "sequence_index_origin" and c["value"] == 0 for c in p["conventions"]), f"{lr} 注明 F₀=0 约定")
                mb = p["mathlib_baseline"]
                # 决定 4：基线三态 unchecked / confirmed / not_found，Agent 只能写 unchecked；题型可以不写基线
                check(mb is None if p["semantic_kind"] == "problem_schema" else mb is not None, f"{lr} mathlib_baseline 是否应出现")
                check(mb is None or mb["status"] == "unchecked", f"{lr} mathlib_baseline.status 只能是 unchecked")
            ext = p.get("declaration") or p.get("problem_schema")
            check((p["semantic_kind"] in ("declaration", "problem_schema")) == (ext is not None), f"{lr} 扩展字段")
            if ext:
                for x in refs_in(ext): resolve(x)
        elif k == "alignment":
            check(p["relation"] in ALIGN_REL, f"{lr} 关系在 EMIR 集合内")
            resolve(p["source"], {"source_unit"}); resolve(p["target"], {"semantic_entity"})
            resolve(p["target_context"], {"semantic_entity"})
            resolve(p["source_context"]["dialect_scope"], {"dialect_scope"})
            check(p["evidence_spans"], f"{lr} 有证据片段")
            for x in p["evidence_spans"]: resolve(x, {"source_span"})
            check(p["status"] in AGENT_ALIGN_STATUS, f"{lr} forbidden_alignment_status")
            for tm in p["term_index_map"]:
                check(tm["index_shift"] == 0, f"{lr} 数列项下标对齐（index_shift = 0）")
            for pm in p["parameter_map"]:
                check(pm["role"] in {"statement_parameter", "problem_parameter"}, f"{lr} 参数映射角色")
            for ra in p["required_assumptions"]:
                if "debt" in ra: resolve(ra["debt"], {"debt"})
            for l in p["lost_information"]:
                if "see_also" in l: resolve(l["see_also"])
            lab = p["map_label"]
            check(not any(w in lab for w in ("平移", "错开", "滑动")), f"{lr} 白话标签不说“数列错开”")
            if p["relation"] == "specializes":
                check(lab.startswith("特例：") and "第 n 条 ＝ 共享的第 n+1 条" in lab, f"{lr} 特例标签")
                check(p["lost_information"] and p["parameter_map"], f"{lr} 特例需参数映射与对不上的部分（审核项）")
            if p["relation"] == "overlaps_with":
                check(lab.startswith("部分重合"), f"{lr} 部分重合标签")
            if p["relation"] == "expresses":
                check(lab.startswith("说的就是它"), f"{lr} 说的就是它标签")
        elif k == "debt":
            check(p["debt_kind"] in DEBT_KINDS, f"{lr} debt_kind")
            check(p["status"] in AGENT_DEBT_STATUS, f"{lr} forbidden_debt_status")
            resolve(p["attached_to"], {"alignment", "semantic_entity"})
            check(p["failure_mode"] in {"theorem", "statement", "route"} and p["quantifier"], f"{lr} 量词 / 失败方式")
            for x in p["evidence_spans"]: resolve(x, {"source_span"})
    for r in cs["proposed_relations"]:
        count["source_relation"] = count.get("source_relation", 0) + 1
        check(r["relation"] in {"proves", "solves", "hints", "explains", "cites", "applies_method"}, f"{r['local_ref']} 关系种类")
        resolve(r["source"], {"source_unit"}); resolve(r["target"], {"source_unit"})
        for x in r["evidence_spans"]: resolve(x, {"source_span"})
    for o in cs["proposed_objects"]:
        if o["kind"] == "source_span": check(o["local_ref"] in used, f"{o['local_ref']} unused_span")
    check(sorted(deps) == cs["depends_on_change_sets"], f"{cs['target_object']} 依赖边与派生字段一致")
    deps_found[cid] = deps

# 依赖图无环（change_set_dependency_cycle）
state = {}
def dfs(u):
    state[u] = 1
    for v in deps_found[u]:
        check(state.get(v) != 1, "change_set_dependency_cycle")
        if v not in state: dfs(v)
    state[u] = 2
for u in deps_found:
    if u not in state: dfs(u)

blob = json.dumps(doc, ensure_ascii=False)
check('"same_as"' not in blob, "禁止 same_as")
check(not re.search(r'"(curation|alignment|proof)"\s*:\s*"(accepted|canonical|kernel_verified|proved)"', blob), "无自设可信终态")
check(not re.search(r'"status"\s*:\s*"(accepted|semantically_reviewed|paid|waived)"', blob), "无自设 accepted / paid 等状态")
# 不合并：同一共享实体被两本书分别对齐
tgt_books = {}
for cs in CSS:
    for o in cs["proposed_objects"]:
        if o["kind"] == "alignment":
            book = o["payload"]["source_context"]["dialect_scope"]
            t = o["payload"]["target"]
            t = cs["staged_ids"][t[5:]] if t.startswith("$new:") else t
            tgt_books.setdefault(t, set()).add(book)
names = {cs["staged_ids"][lr]: lr for cs in CSS for lr in cs["staged_ids"]}
multi = sorted(names[t] for t, b in tgt_books.items() if len(b) >= 2)
check(set(multi) >= {"e.fib", "e.fib_def", "e.qpow", "e.cassini"}, "同一共享实体被两本书分别对齐（不合并）")
# 灯塔的约定可被机器读到
ds_yi = [o for cs in CSS for o in cs["proposed_objects"] if o["local_ref"] == "ds.yi"][0]["payload"]
check(any(c["key"] == "sequence_index_origin" and c["value"] == 1 for c in ds_yi["conventions"]), "ds.yi 记下“从第 1 项起编”")
# 修订：v1 → v2 local_ref 不变
for h in doc["revision_history"]:
    cs = [c for c in CSS if c["change_set_id"] == h["change_set_id"]][0]
    check(cs["version"] == 2 and cs["supersedes"]["version"] == 1, "v2 替换 v1")
    for d in h["diff_v1_to_v2"]: check(d["local_ref"] in cs["staged_ids"], "改动项在 v2 中仍存在（同 id）")
check(len(doc["map_debts"]) == 4 and all(d["status"] == "open" for d in doc["map_debts"]), "4 个地图 Debt，均为 open")
print("  " + "、".join(f"{k} {v}" for k, v in sorted(count.items())))
print(f"  change set {len(CSS)} 个，每个围绕一个目标对象：" + "、".join(c["target_object"] for c in CSS))
print("  quote 与字节范围一致；全部 id 可按规则重算；引用闭合且种类正确；依赖边一致且无环；无 unused_span ✓")
print("  无 same_as；无自设可信终态；对齐状态 / Debt 状态都在 Agent 可设范围内；数列项下标映射 index_shift 全为 0 ✓")
print("  白话标签：特例 = “灯塔的第 n 条 ＝ 共享的第 n+1 条”；不出现“平移 / 错开 / 滑动” ✓")
print("  ds.yi 记下“从第 1 项起编”；共享对象都注明 F₀ = 0 约定与 mathlib 基线 ✓")
print("  被两本书分别对齐的共享实体：" + "、".join(multi))

print(f"\n全部 {ok_count} 条断言通过。")
