/**
 * Tiny toast/notification helper. Replaces silent `try { } catch { }` paths
 * with surfaced messages so users see failures (font load, settings write,
 * sibling-folder scan, etc.) instead of wondering why something didn't take.
 *
 * Usage:
 *   import { toastError, toastWarn, toastInfo } from "./toast";
 *   toastError(`Failed to read backup: ${err}`);
 *
 * Auto-dismiss defaults: error 8s, warn 6s, info 4s. Pass 0 for sticky.
 * Newest toast appears at the top of the stack.
 */
import { store } from "./stores.svelte";

let nextId = 1;

function push(kind: "error" | "warn" | "info", message: string, timeoutMs: number): void {
  const id = nextId++;
  store.toasts = [{ id, kind, message, timeoutMs }, ...store.toasts];
  if (timeoutMs > 0) {
    setTimeout(() => dismissToast(id), timeoutMs);
  }
}

export function toastError(message: string, timeoutMs = 8000): void {
  push("error", message, timeoutMs);
}

export function toastWarn(message: string, timeoutMs = 6000): void {
  push("warn", message, timeoutMs);
}

export function toastInfo(message: string, timeoutMs = 4000): void {
  push("info", message, timeoutMs);
}

export function dismissToast(id: number): void {
  store.toasts = store.toasts.filter((t) => t.id !== id);
}
