// Shared formatting + parsing helpers.
//
// These were previously copy-pasted across servers / server detail / public
// status pages, and the copies had drifted (e.g. one formatRate forgot the
// null guard). Keep the single source of truth here and import everywhere.

import { formatBytes } from "@/app/components/M7Primitives";

// Coerce an unknown value to a finite number, or undefined. Accepts numeric
// strings. Backend Option<f64>/Option<i64> fields arrive as null → undefined.
export function optionalNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

// Bytes-per-second label, e.g. "1.2 MB/s". Null/undefined/NaN → "N/A".
export function formatRate(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return `${formatBytes(value)}/s`;
}

// Humanized duration from a seconds value (accepts number | null | numeric
// string). Null/invalid → "N/A".
export function durationLabel(value: unknown): string {
  const seconds = optionalNumber(value);
  if (seconds === undefined) return "N/A";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

// Clamp a percentage into [0, 100]. Null/undefined/NaN → 0.
export function clampPercent(value?: number | null): number {
  if (value === undefined || value === null || Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

// Single-pass min/max over a numeric array. Avoids `Math.max(...arr)`, which
// allocates an args list and throws on very large arrays (arg-count limit) —
// a real risk for 30-day chart series.
export function maxOf(values: number[], floor = -Infinity): number {
  let max = floor;
  for (const value of values) if (value > max) max = value;
  return max;
}

export function minOf(values: number[], ceil = Infinity): number {
  let min = ceil;
  for (const value of values) if (value < min) min = value;
  return min;
}

// Read a browser cookie by name. SSR-safe (returns null when document absent).
export function getCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  return (
    document.cookie
      .split("; ")
      .find((row) => row.startsWith(`${name}=`))
      ?.split("=")[1] ?? null
  );
}

// True if the string contains any ASCII control character (0x00-0x1F or 0x7F).
function hasControlCharacter(value: string): boolean {
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i);
    if (code <= 0x1f || code === 0x7f) return true;
  }
  return false;
}

// Validate a `return_to` redirect target. Only same-origin absolute paths are
// allowed; anything suspicious (protocol-relative, backslashes, control chars,
// or pointing back at /login) falls back to /dashboard.
export function sanitizeReturnTo(value: string | null): string {
  if (
    !value ||
    !value.startsWith("/") ||
    value.startsWith("//") ||
    value.includes("\\") ||
    hasControlCharacter(value) ||
    value.startsWith("/login")
  ) {
    return "/dashboard";
  }
  return value;
}
