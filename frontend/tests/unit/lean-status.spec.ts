import { describe, expect, it } from "vitest";
import { leanStatusView } from "../../src/artifact/lean-status";
import type { LeanRuntimeStatus } from "../../src/artifact/types";

const ready: LeanRuntimeStatus = {
  root: "/Users/me/.mathscape/lean/profiles",
  source: "workspace_parent",
  root_exists: true,
  problem: null,
  profiles: [{
    id: "lean4-4.33.1-mathlib-4.33.1",
    state: "ready",
    lean_toolchain: "leanprover/lean4:v4.33.1",
    mathlib_reference: "v4.33.1",
    detected_mathlib_revision: "0123456789abcdef",
    fingerprint_status: "verified",
    fingerprint_digest: "sha256:abcdef0123456789",
    problem: null,
  }],
};

describe("lean status card", () => {
  it("shows a verified profile as usable with versions and fingerprint", () => {
    const view = leanStatusView(ready);
    expect(view.ready).toBe(true);
    expect(view.state).toBe("可用");
    expect(view.summary).toBe("1 个 profile，1 个可用");
    expect(view.profiles[0].title).toBe("lean4-4.33.1-mathlib-4.33.1（可用）");
    expect(view.profiles[0].versions).toContain("Mathlib v4.33.1");
    expect(view.profiles[0].fingerprint).toBe("指纹已验证 sha256:abcde");
    expect(view.problem).toContain("上级目录");
  });

  it("does not call an unverified fingerprint usable", () => {
    const view = leanStatusView({
      ...ready,
      profiles: [{ ...ready.profiles[0], fingerprint_status: "unverified" }],
    });
    expect(view.ready).toBe(false);
    expect(view.state).toBe("未就绪");
  });

  it("reports a missing root and an absent snapshot field", () => {
    expect(leanStatusView({ ...ready, root_exists: false, profiles: [], problem: "Lean profile 目录不存在" }).state)
      .toBe("目录缺失");
    expect(leanStatusView(undefined).state).toBe("未配置");
    expect(leanStatusView({ root: null, source: null, root_exists: false, profiles: [], problem: "x" }).problem).toBe("x");
  });
});
