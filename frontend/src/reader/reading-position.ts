/** Scroll position helpers shared by the snapshot reader and the 「规范解析正文」 view. */

import { t } from "../i18n";

/** Nearest ancestor that actually scrolls; `fallback` when none does yet. */
export function scrollParentOf(element: HTMLElement, fallback: HTMLElement): HTMLElement {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && node.scrollHeight > node.clientHeight) return node;
    if (node === fallback) break;
  }
  return fallback;
}

/** Distance from the top of the scroll viewport to the top of `element`. */
export function viewportTopOf(element: HTMLElement, scroller: HTMLElement): number {
  return element.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
}

export function isInViewport(element: HTMLElement, scroller: HTMLElement): boolean {
  const rect = element.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  return rect.height > 0 && rect.bottom > view.top + 4 && rect.top < view.bottom - 4;
}

/**
 * Holds `target` at `top` px below the scroll viewport's top while late layout
 * (images, formula fonts) still shifts the page, and lets go for good as soon
 * as the reader scrolls on their own.
 */
export function pinToViewportTop(
  find: () => HTMLElement | null,
  top: number | ((target: HTMLElement, scroller: HTMLElement) => number),
  scrollerOf: (target: HTMLElement) => HTMLElement,
  listenOn: HTMLElement,
): { apply: () => void; watch: (content: HTMLElement) => void; dispose: () => void } {
  let released = false;
  let observer: ResizeObserver | undefined;
  const events = ["wheel", "touchmove", "pointerdown", "keydown"] as const;
  const release = () => {
    released = true;
    observer?.disconnect();
    observer = undefined;
  };
  events.forEach(name => listenOn.addEventListener(name, release, { capture: true, passive: true }));
  const apply = () => {
    if (released) return;
    const target = find();
    if (!target) return;
    const scroller = scrollerOf(target);
    const wanted = typeof top === "number" ? top : top(target, scroller);
    scroller.scrollTop += viewportTopOf(target, scroller) - wanted;
  };
  return {
    apply,
    // WebKit has no scroll anchoring: content that grows above the target
    // (images, markers) would push it down the screen without this.
    watch: (content) => {
      if (released || typeof ResizeObserver === "undefined") return;
      observer ??= new ResizeObserver(apply);
      observer.observe(content);
    },
    dispose: () => {
      release();
      events.forEach(name => listenOn.removeEventListener(name, release, { capture: true }));
    },
  };
}

/** Viewport offset that centres `target`, or keeps its top in view when it is taller than the viewport. */
export function centredTop(target: HTMLElement, scroller: HTMLElement): number {
  return Math.max(12, (scroller.clientHeight - target.getBoundingClientRect().height) / 2);
}

/**
 * 「回到出发处」: a chip pinned to the bottom of the reader that appears once the
 * block the reader arrived at has scrolled out of view, and brings it back.
 */
export function mountReturnPoint(options: {
  container: HTMLElement;
  /** Element whose scroll events (its own or its descendants') move the content. */
  listenOn: HTMLElement;
  label: string;
  find: () => HTMLElement | null;
  scrollerOf: (target: HTMLElement) => HTMLElement;
}): () => void {
  const { container, listenOn, find, scrollerOf } = options;
  const chip = document.createElement("div");
  chip.className = "reader-return-point";
  const back = document.createElement("button");
  back.type = "button";
  back.className = "reader-return-btn";
  back.textContent = t("reader.returnPoint.label", { label: options.label });
  back.title = t("reader.returnPoint.title");
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "reader-return-dismiss";
  dismiss.textContent = "×";
  dismiss.title = t("reader.returnPoint.dismissTitle");
  dismiss.setAttribute("aria-label", t("reader.returnPoint.dismissAriaLabel"));
  chip.append(back, dismiss);
  container.append(chip);

  let dismissed = false;
  let frame = 0;
  const update = () => {
    frame = 0;
    if (dismissed) return;
    const target = find();
    // Hidden by class, not removed from flow: its room at the end of the
    // document stays reserved, so it never sits on top of the last lines.
    const away = Boolean(target) && !isInViewport(target!, scrollerOf(target!));
    chip.classList.toggle("is-shown", away);
    chip.setAttribute("aria-hidden", String(!away));
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
  listenOn.addEventListener("scroll", schedule, { capture: true, passive: true });
  window.addEventListener("resize", schedule);
  back.onclick = () => {
    const target = find();
    if (!target) return;
    target.scrollIntoView({ behavior: "smooth", block: "center" });
    target.classList.remove("is-return-flash");
    void target.offsetWidth;
    target.classList.add("is-return-flash");
  };
  dismiss.onclick = () => { dismissed = true; chip.hidden = true; };
  return () => {
    dismissed = true;
    if (frame) cancelAnimationFrame(frame);
    listenOn.removeEventListener("scroll", schedule, { capture: true });
    window.removeEventListener("resize", schedule);
    chip.remove();
  };
}

/** Short plain-text label for a rendered block. */
export function blockLabel(element: HTMLElement, limit = 18): string {
  const copy = element.cloneNode(true) as HTMLElement;
  // KaTeX keeps a MathML twin of every formula; badges and pills are not block text.
  copy.querySelectorAll(".katex-mathml, button, .ink-block-marker, .block-action-pill, .unit-map-badge-row").forEach(node => node.remove());
  const text = (copy.textContent ?? "").replace(/\s+/g, " ").trim();
  const chars = [...text];
  return chars.length > limit ? chars.slice(0, limit).join("") + "…" : text || t("reader.blockLabelFallback");
}
