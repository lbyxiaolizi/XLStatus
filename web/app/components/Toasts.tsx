"use client";

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
  green: "border-emerald-500/30 bg-[var(--bg-card)] text-[var(--text-main)]",
  yellow: "border-amber-500/30 bg-[var(--bg-card)] text-[var(--text-main)]",
};

const dotClasses: Record<ToastItem["tone"], string> = {
  green: "bg-emerald-500",
  yellow: "bg-amber-500",
};

export function Toaster() {
  const items = useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);

  return (
    <div
      role="status"
      aria-live="polite"
      className="pointer-events-none fixed bottom-4 right-4 z-[70] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2"
    >
      {items.map((toast) => (
        <div
          key={toast.id}
          className={`pointer-events-auto flex items-start justify-between gap-3 rounded-xl border px-3.5 py-2.5 text-xs font-medium shadow-[var(--shadow-md)] backdrop-blur-md ${toneClasses[toast.tone]}`}
        >
          <div className="flex min-w-0 items-center gap-2">
            <span className={`h-2 w-2 shrink-0 rounded-full ${dotClasses[toast.tone]}`} aria-hidden="true" />
            <span className="min-w-0 break-words">{toast.message}</span>
          </div>
          <button
            type="button"
            onClick={() => dismissToast(toast.id)}
            aria-label="dismiss"
            className="shrink-0 rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-1.5 py-0.5 text-xs font-medium text-[var(--text-muted)] transition hover:text-[var(--text-main)]"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}

export const ToastViewport = Toaster;
