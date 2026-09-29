import { invoke } from "@tauri-apps/api/core";

/**
 * Extract a human-readable message from an arbitrary error payload.
 */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "发生了未知错误";
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
    throw new Error(errorMessage(error));
  }
}
