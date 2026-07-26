"use client";

// Transient success/notice feedback. Inline notices previously stayed on
// screen until the next action replaced them, and feedback for an action in
// one settings card was invisible while looking at another. Toasts stack
// bottom-right, dismiss themselves, and are announced politely.
//
// Rule of thumb for adopters: pure confirmations ("saved", "deleted") become
// toasts; anything the user must read or copy (generated tokens, temp URLs)
// stays inline where it cannot vanish mid-read.

import { useSyncExternalStore } from "react";

export interface ToastItem {
  id: number;
  tone: "green" | "yellow";
  message: string;
}

const TOAST_TTL_MS = 4500;
const MAX_TOASTS = 4;
const EMPTY_TOASTS: ToastItem[] = [];

let toasts: ToastItem[] = EMPTY_TOASTS;
let nextToastId = 1;
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) listener();
}

export function showToast(message: string, tone: ToastItem["tone"] = "green"): void {
  if (typeof window === "undefined" || !message) return;
  const id = nextToastId++;
  toasts = [...toasts.slice(-(MAX_TOASTS - 1)), { id, tone, message }];
  emitChange();
  window.setTimeout(() => dismissToast(id), TOAST_TTL_MS);
}

export function dismissToast(id: number): void {
  if (!toasts.some((toast) => toast.id === id)) return;
  toasts = toasts.filter((toast) => toast.id !== id);
  emitChange();
}

function subscribe(callback: () => void): () => void {
  listeners.add(callback);
  return () => listeners.delete(callback);
}

function getSnapshot(): ToastItem[] {
  return toasts;
}

function getServerSnapshot(): ToastItem[] {
  return EMPTY_TOASTS;
}

const toneClasses: Record<ToastItem["tone"], string> = {
  green: "bg-[var(--accent-color)] text-[var(--btn-text)]",
  yellow: "bg-[var(--accent-bg)] text-[var(--text-main)]",
};

export function Toaster() {
  const items = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  return (
    // The live region stays mounted even when empty: screen readers only
    // announce changes inside an existing region, not regions that appear
    // already holding content. Above dialogs (z-60) so confirmations issued
    // from a modal remain visible; pointer-events scoped to the cards so the
    // empty region never blocks clicks.
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none fixed bottom-4 right-4 z-[70] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
    >
      {items.map((toast) => (
        <div
          key={toast.id}
          className={`pointer-events-auto flex items-start justify-between gap-3 border-2 border-black px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal)] ${toneClasses[toast.tone]}`}
        >
          <span className="min-w-0 break-words">{toast.message}</span>
          <button
            type="button"
            onClick={() => dismissToast(toast.id)}
            aria-label="dismiss"
            className="shrink-0 border-2 border-black bg-[var(--bg-card)] px-1.5 text-xs font-black text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)]"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
