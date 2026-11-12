/**
 * 「反馈给 agent」：拼一段引用插入对话输入框，**从不自动发送**，也不改提案状态
 * （docs/semantic-write-contract.md S9.2）。
 */
import { eventBus } from "../core/events";

export interface FeedbackTarget {
  /** 审核项 / 对齐线 / 一笔账 / 约定卡 / 共享对象 */
  noun: string;
  /** 提案标题与版本，例如「Cassini 恒等式」v2；已接受对象可省略。 */
  proposal?: { title: string; version: number } | null;
  /** 提案里的 local_ref，或对象的白话名。 */
  name: string;
  /** 白话说明，例如对齐线的白话标签。 */
  detail?: string | null;
  id?: string | null;
  /** 审核项的码，例如 warn:relation_detail_missing。 */
  code?: string | null;
}

const shortId = (id: string): string => id.length > 40 ? `${id.slice(0, 32)}…` : id;

export function feedbackQuote(target: FeedbackTarget): string {
  const head = [
    target.proposal ? `提案「${target.proposal.title}」v${target.proposal.version}` : null,
    `${target.noun === "审核项" ? "" : target.noun + " "}${target.name}${target.detail ? `（${target.detail}）` : ""}`.trim(),
  ].filter(Boolean).join(" · ");
  const lines = [`> [${target.code ? "审核项" : target.noun}] ${head}`];
  const meta = [target.id ? `id: ${shortId(target.id)}` : null, target.code ? `码: ${target.code}` : null].filter(Boolean);
  if (meta.length) lines.push(`> ${meta.join("  · ")}`);
  return `${lines.join("\n")}\n`;
}

/** 请对话框插入引用。返回 false 表示当前没有可写的对话输入框。 */
export function sendFeedbackToComposer(target: FeedbackTarget): boolean {
  const text = feedbackQuote(target);
  let accepted = false;
  eventBus.emit("insert-agent-feedback", { text, accept: () => { accepted = true; } });
  return accepted;
}

export function feedbackButton(target: FeedbackTarget, onResult: (inserted: boolean) => void): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "semantic-feedback-button";
  button.textContent = "反馈给 agent";
  button.title = "把这一项的引用插入对话输入框（不会自动发送）";
  button.onclick = (event) => {
    event.stopPropagation();
    onResult(sendFeedbackToComposer(target));
  };
  return button;
}
