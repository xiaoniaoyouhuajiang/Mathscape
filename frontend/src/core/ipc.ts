import { invoke } from "@tauri-apps/api/core";

/**
 * Error thrown by `safeInvoke` when a command returns a structured error
 * (`{code, message, details}`). `code` is stable and meant for branching;
 * `message` is user-facing.
 */
export class CommandError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly details?: Record<string, unknown>,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

function structured(error: unknown): { code: string; message: string; details?: Record<string, unknown> } | undefined {
  if (error === null || typeof error !== "object" || error instanceof Error) return undefined;
  const value = error as Record<string, unknown>;
  if (typeof value.code !== "string" || typeof value.message !== "string") return undefined;
  const details = value.details !== null && typeof value.details === "object" ? value.details as Record<string, unknown> : undefined;
  return { code: value.code, message: value.message, details };
}

/**
 * Extract a human-readable message from an arbitrary error payload.
 */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  const value = structured(error);
  if (value) return value.message;
  if (error !== null && typeof error === "object" && typeof (error as { message?: unknown }).message === "string") {
    return (error as { message: string }).message;
  }
  return "发生了未知错误";
}

/** Stable error code of a command failure, if the command returned one. */
export function errorCode(error: unknown): string | undefined {
  return error instanceof CommandError ? error.code : structured(error)?.code;
}

/**
 * Strongly typed wrapper around Tauri's invoke command, with consistent
 * error wrapping if needed.
 */
export async function safeInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    const value = structured(error);
    if (value) throw new CommandError(value.code, value.message, value.details);
    throw new Error(errorMessage(error));
  }
}
