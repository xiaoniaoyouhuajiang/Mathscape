/**
 * 「减少动效」: a UI preference (this device only, localStorage).
 *
 * `system` (default) follows `prefers-reduced-motion`; `on` / `off` override
 * it. The map scene player reads `reducedMotion()` per segment: reduced =
 * instant state changes (no clump tween, camera glide, typewriter or
 * one-by-one reveals) while every state is still shown.
 */
export type MotionPreference = "system" | "on" | "off";

export const MOTION_KEY = "mathscape:reduce-motion";
export const MOTION_CHANGED = "mathscape:motion-changed";
const REDUCED_CLASS = "reduce-motion";

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function motionPreference(): MotionPreference {
  const value = storage()?.getItem(MOTION_KEY);
  return value === "on" || value === "off" ? value : "system";
}

function systemReduced(): boolean {
  try {
    return globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
  } catch {
    return false;
  }
}

export function reducedMotion(preference: MotionPreference = motionPreference()): boolean {
  return preference === "on" || (preference === "system" && systemReduced());
}

/** Mirror the effective value on `<html>` (CSS animations key off it). */
export function applyMotionClass(): void {
  globalThis.document?.documentElement.classList.toggle(REDUCED_CLASS, reducedMotion());
}

export function setMotionPreference(preference: MotionPreference): void {
  const store = storage();
  try {
    if (preference === "system") store?.removeItem(MOTION_KEY);
    else store?.setItem(MOTION_KEY, preference);
  } catch {
    /* preference only */
  }
  applyMotionClass();
  globalThis.dispatchEvent?.(new CustomEvent(MOTION_CHANGED, { detail: preference }));
}

/** Keep the `<html>` class in sync with the OS setting while `system`. */
export function watchMotion(): void {
  applyMotionClass();
  try {
    globalThis.matchMedia?.("(prefers-reduced-motion: reduce)").addEventListener("change", applyMotionClass);
  } catch {
    /* old engines: the class is set once */
  }
}
