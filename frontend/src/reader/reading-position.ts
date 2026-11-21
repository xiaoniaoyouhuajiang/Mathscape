/** Scroll position helpers shared by the snapshot reader and the 「规范解析正文」 view. */

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
  top: number,
  scrollerOf: (target: HTMLElement) => HTMLElement,
  listenOn: HTMLElement,
): { apply: () => void; dispose: () => void } {
  let released = false;
  const release = () => { released = true; };
  const events = ["wheel", "touchmove", "pointerdown", "keydown"] as const;
  events.forEach(name => listenOn.addEventListener(name, release, { capture: true, passive: true }));
  return {
    apply: () => {
      if (released) return;
      const target = find();
      if (!target) return;
      const scroller = scrollerOf(target);
      scroller.scrollTop += viewportTopOf(target, scroller) - top;
    },
    dispose: () => {
      released = true;
      events.forEach(name => listenOn.removeEventListener(name, release, { capture: true }));
    },
  };
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
  chip.hidden = true;
  const back = document.createElement("button");
  back.type = "button";
  back.className = "reader-return-btn";
  back.textContent = `↩ 回到出发处 · ${options.label}`;
  back.title = "滚回刚才定位到的那一块";
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "reader-return-dismiss";
  dismiss.textContent = "×";
  dismiss.title = "不再显示";
  dismiss.setAttribute("aria-label", "不再显示回到出发处");
  chip.append(back, dismiss);
  container.append(chip);

  let dismissed = false;
  let frame = 0;
  const update = () => {
    frame = 0;
    if (dismissed) return;
    const target = find();
    chip.hidden = !target || isInViewport(target, scrollerOf(target));
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
  return chars.length > limit ? chars.slice(0, limit).join("") + "…" : text || "定位块";
}
