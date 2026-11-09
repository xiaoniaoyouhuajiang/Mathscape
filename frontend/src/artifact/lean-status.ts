import type { LeanProfileSummary, LeanRuntimeStatus } from "./types";

/**
 * Source-layer card for Lean profiles. Profiles are an environment /
 * external library, not workspace sources: the card only shows the recorded
 * `profile.json` state. It never builds, upgrades, or re-probes a profile,
 * and a ready profile says nothing about whether a proof is faithful.
 */
export interface LeanStatusView {
  state: string;
  ready: boolean;
  summary: string;
  profiles: LeanProfileLine[];
  problem: string;
}

export interface LeanProfileLine {
  id: string;
  ready: boolean;
  title: string;
  versions: string;
  fingerprint: string;
  problem: string | null;
}

const SOURCE_LABEL: Record<string, string> = {
  environment: "环境变量 MATHSCAPE_LEAN_PROFILE_ROOT",
  workspace: "工作区 .mathscape/lean/profiles",
  workspace_parent: "上级目录 .mathscape/lean/profiles",
  current_directory: "当前目录 .mathscape/lean/profiles",
  platform_data: "系统数据目录",
};

const STATE_LABEL: Record<string, string> = {
  ready: "可用",
  incomplete: "不完整",
  unverified: "未验证",
  mismatch: "版本不符",
};

const FINGERPRINT_LABEL: Record<string, string> = {
  verified: "指纹已验证",
  unverified: "指纹未验证",
  incomplete: "指纹不完整",
};

export function leanProfileLine(profile: LeanProfileSummary): LeanProfileLine {
  const ready = profile.state === "ready" && profile.fingerprint_status === "verified";
  const state = profile.state ? STATE_LABEL[profile.state] ?? profile.state : "记录不可读";
  const versions = [
    profile.lean_toolchain ? `Lean ${profile.lean_toolchain}` : null,
    profile.mathlib_reference ? `Mathlib ${profile.mathlib_reference}` : null,
    profile.detected_mathlib_revision ? `rev ${profile.detected_mathlib_revision.slice(0, 12)}` : null,
  ].filter(Boolean).join(" · ") || "版本未知";
  const fingerprintLabel = profile.fingerprint_status
    ? FINGERPRINT_LABEL[profile.fingerprint_status] ?? profile.fingerprint_status
    : "无指纹";
  const digest = profile.fingerprint_digest ? ` ${profile.fingerprint_digest.slice(0, 12)}` : "";
  return {
    id: profile.id,
    ready,
    title: `${profile.id}（${state}）`,
    versions,
    fingerprint: `${fingerprintLabel}${digest}`,
    problem: profile.problem,
  };
}

export function leanStatusView(status: LeanRuntimeStatus | undefined | null): LeanStatusView {
  if (!status || status.root === null) {
    return {
      state: "未配置",
      ready: false,
      summary: "—",
      profiles: [],
      problem: status?.problem ?? "未找到 Lean profile 目录。",
    };
  }
  const profiles = status.profiles.map(leanProfileLine);
  const readyCount = profiles.filter(line => line.ready).length;
  const ready = readyCount > 0;
  const state = !status.root_exists
    ? "目录缺失"
    : profiles.length === 0
      ? "无 profile"
      : ready ? "可用" : "未就绪";
  const source = status.source ? SOURCE_LABEL[status.source] ?? status.source : "未知来源";
  return {
    state,
    ready,
    summary: profiles.length === 0
      ? "尚无已登记的 profile"
      : `${profiles.length} 个 profile，${readyCount} 个可用`,
    profiles,
    problem: status.problem ?? `${source}：${status.root}`,
  };
}

export interface LeanStatusElements {
  state: HTMLElement;
  summary: HTMLElement;
  profiles: HTMLElement;
  problem: HTMLElement;
}

export function renderLeanStatus(elements: LeanStatusElements, status: LeanRuntimeStatus | undefined): void {
  const view = leanStatusView(status);
  elements.state.textContent = view.state;
  elements.state.classList.toggle("is-ready", view.ready);
  elements.summary.textContent = view.summary;
  elements.problem.textContent = view.problem;
  const items = view.profiles.map(line => {
    const item = document.createElement("li");
    item.className = "lean-profile-item";
    item.classList.toggle("is-ready", line.ready);
    item.dataset.profileId = line.id;
    const title = document.createElement("strong");
    title.textContent = line.title;
    const versions = document.createElement("span");
    versions.textContent = line.versions;
    const fingerprint = document.createElement("span");
    fingerprint.className = "lean-fingerprint";
    fingerprint.textContent = line.fingerprint;
    item.append(title, versions, fingerprint);
    if (line.problem) {
      const problem = document.createElement("span");
      problem.textContent = line.problem;
      item.append(problem);
    }
    return item;
  });
  elements.profiles.replaceChildren(...items);
}
