/**
 * Themed confirmation dialog helper. Drop-in replacement for the OS-native
 * `ask()` / `confirm()` popups so confirmations match the app theme.
 *
 * Usage:
 *   if (await confirmDialog({ message: "Delete this?", danger: true })) { ... }
 *
 * Resolves true on confirm, false on cancel/escape/backdrop.
 */
import { store } from "./stores.svelte";

export interface ConfirmOptions {
  message: string;
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Style the confirm button as destructive (red). */
  danger?: boolean;
}

export function confirmDialog(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    store.confirmRequest = {
      title: opts.title ?? "Confirm",
      message: opts.message,
      confirmLabel: opts.confirmLabel ?? "Confirm",
      cancelLabel: opts.cancelLabel ?? "Cancel",
      danger: opts.danger ?? false,
      resolve,
    };
  });
}
