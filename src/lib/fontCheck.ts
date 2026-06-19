/**
 * Detects whether a named font is installed by comparing the rendered width
 * of a test string between the named font and a known-different baseline.
 * If the widths match, the browser fell back to the baseline → font absent.
 *
 * Standard fingerprinting trick — reliable for CJK fonts (the test string
 * includes characters whose width differs meaningfully between Meiryo and
 * monospace fallback).
 */
function isFontAvailable(fontName: string): boolean {
  const test = "比べるのなら……良くて—引き分けabcd";
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return true; // can't test → don't trigger false warning
  ctx.font = "16px monospace";
  const baseline = ctx.measureText(test).width;
  ctx.font = `16px "${fontName}", monospace`;
  const measured = ctx.measureText(test).width;
  return measured !== baseline;
}

/**
 * Returns the list of preferred fonts that the system DOES NOT have.
 * Currently we only flag a problem if BOTH Meiryo and Yu Gothic UI are
 * missing — either of those is sufficient for proper Japanese rendering.
 * Segoe UI alone is not enough because it lacks U+2014/U+2026 coverage.
 */
export function detectMissingCJKFonts(): string[] {
  const candidates = ["Meiryo", "Yu Gothic UI"];
  const missing = candidates.filter((f) => !isFontAvailable(f));
  // If at least one preferred CJK font is present, no warning needed.
  return missing.length === candidates.length ? missing : [];
}

const DISMISS_KEY = "eushully-editor:font-warning-dismissed";

export function isFontWarningDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissFontWarning(): void {
  try {
    localStorage.setItem(DISMISS_KEY, "1");
  } catch {
    // ignore — at worst the warning shows again next boot
  }
}
