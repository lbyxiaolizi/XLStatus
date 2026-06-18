"use client";

import { ReactNode, useEffect, useState } from "react";
import type { ApiResponse } from "@/lib/api";
import { formatLocaleDate, t } from "@/lib/i18n";

export interface StoredUser {
  id: string;
  username: string;
  role: string;
}

declare global {
  interface Window {
    applyBoldTheme?: () => void;
  }
}

export function useStoredUser(): StoredUser | null {
  const [user, setUser] = useState<StoredUser | null>(null);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setUser(readStoredUser());
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, []);

  return user;
}

export function readStoredUser(): StoredUser | null {
  if (typeof window === "undefined") {
    return null;
  }

  const userStr = window.localStorage.getItem("user");
  if (!userStr) {
    return null;
  }

  try {
    return JSON.parse(userStr) as StoredUser;
  } catch {
    return null;
  }
}

export function isAdmin(user: StoredUser | null): boolean {
  return user?.role?.toLowerCase() === "admin";
}

export function responseError(response: ApiResponse<unknown>): string {
  const suffix = response.request_id ? ` (${response.request_id})` : "";
  if (response.status === 401) return `${t.common.authRequired}${suffix}`;
  if (response.status === 403) return `${t.common.permissionDenied}${suffix}`;
  if (response.status === 404) return `${t.common.backendNotFound}${suffix}`;
  return `${response.error || t.common.requestFailed}${suffix}`;
}

export function formatDate(value?: string | number | null): string {
  if (!value) return t.common.never;
  const date = typeof value === "number" ? new Date(value * 1000) : new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return formatLocaleDate(date);
}

export function formatPercent(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return t.common.notAvailable;
  return `${value.toFixed(1)}%`;
}

export function formatMs(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return t.common.notAvailable;
  return `${value} ms`;
}

export function formatBytes(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return t.common.notAvailable;
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export function compactId(value?: string | null): string {
  if (!value) return "-";
  return value.length > 13 ? `${value.slice(0, 8)}...${value.slice(-4)}` : value;
}

export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

export function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

export function asNumber(value: unknown, fallback = 0): number {
  return typeof value === "number" && !Number.isNaN(value) ? value : fallback;
}

export function useBoldTheme(): ["light" | "dark", (mode: "light" | "dark") => void] {
  const [mode, setModeState] = useState<"light" | "dark">("light");

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setModeState(localStorage.getItem("darkMode") === "true" ? "dark" : "light");
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, []);

  function setMode(nextMode: "light" | "dark") {
    localStorage.setItem("darkMode", nextMode === "dark" ? "true" : "false");
    window.applyBoldTheme?.();
    setModeState(nextMode);
  }

  return [mode, setMode];
}

export function StatusBadge({
  tone,
  children,
}: {
  tone: "green" | "red" | "yellow" | "gray" | "blue" | "pink";
  children: ReactNode;
}) {
  const classes = {
    green: "bg-[var(--accent-color)] text-[var(--btn-text)]",
    red: "bg-[var(--btn-bg)] text-[var(--btn-text)]",
    yellow: "bg-[var(--accent-bg)] text-[var(--text-main)]",
    gray: "bg-[var(--bg-card)] text-[var(--text-muted)]",
    blue: "bg-[var(--btn-bg)] text-[var(--btn-text)]",
    pink: "bg-[var(--accent-color)] text-[var(--btn-text)]",
  };

  return (
    <span
      className={`inline-flex items-center border-2 border-black px-2.5 py-1 text-xs font-black uppercase tracking-wide shadow-[2px_2px_0_0_#000] ${classes[tone]}`}
    >
      {children}
    </span>
  );
}

export function InlineError({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <div className="border-2 border-black bg-[var(--btn-bg)] px-4 py-3 text-sm font-bold text-[var(--btn-text)] shadow-[var(--shadow-brutal-sm)]">
      {message}
    </div>
  );
}

export function InlineNotice({
  tone = "blue",
  children,
}: {
  tone?: "blue" | "yellow" | "green" | "pink";
  children: ReactNode;
}) {
  const classes = {
    blue: "bg-[var(--btn-bg)] text-[var(--btn-text)]",
    yellow: "bg-[var(--accent-bg)] text-[var(--text-main)]",
    green: "bg-[var(--accent-color)] text-[var(--btn-text)]",
    pink: "bg-[var(--accent-bg)] text-[var(--text-main)]",
  };

  return (
    <div className={`border-2 border-black px-4 py-3 text-sm font-bold shadow-[var(--shadow-brutal-sm)] ${classes[tone]}`}>
      {children}
    </div>
  );
}

export function EmptyState({
  title,
  detail,
}: {
  title: string;
  detail?: string;
}) {
  return (
    <div className="border-2 border-black bg-[var(--bg-card)] px-4 py-12 text-center shadow-[var(--shadow-brutal)]">
      <p className="text-lg font-black uppercase text-[var(--text-main)]">{title}</p>
      {detail ? <p className="mx-auto mt-2 max-w-2xl text-sm font-bold text-[var(--text-muted)]">{detail}</p> : null}
    </div>
  );
}

export function Field({
  label,
  error,
  children,
}: {
  label: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-xs font-black uppercase tracking-wide text-[var(--text-main)]">
        {label}
      </span>
      {children}
      {error ? <span className="mt-1 block text-xs font-bold text-[var(--accent-color)]">{error}</span> : null}
    </label>
  );
}

export function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/50 px-4 py-8">
      <div className="w-full max-w-3xl border-4 border-black bg-[var(--bg-card)] shadow-[10px_10px_0_0_#000]">
        <div className="flex items-center justify-between border-b-4 border-black bg-[var(--accent-bg)] px-5 py-4">
          <h2 className="text-xl font-black uppercase text-[var(--text-main)]">{title}</h2>
          <button type="button" onClick={onClose} className={buttonClass("secondary")}>
            {t.common.close}
          </button>
        </div>
        <div className="px-5 py-5">{children}</div>
      </div>
    </div>
  );
}

export function PageShell({ children }: { children: ReactNode }) {
  return (
    <main className="mx-auto w-full max-w-7xl px-4 py-6 sm:px-6 lg:px-8 lg:py-8">
      {children}
    </main>
  );
}

export function PageHeader({
  title,
  eyebrow,
  detail,
  actions,
}: {
  title: string;
  eyebrow?: string;
  detail?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-6 grid gap-4 border-b-4 border-black pb-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
      <div>
        {eyebrow ? (
          <p className="mb-2 inline-block border-2 border-black bg-[var(--accent-bg)] px-3 py-1 text-xs font-black uppercase tracking-wide shadow-[var(--shadow-brutal-sm)]">
            {eyebrow}
          </p>
        ) : null}
        <h1 className="text-3xl font-black uppercase tracking-tight text-[var(--text-main)] sm:text-5xl">
          {title}
        </h1>
        {detail ? <p className="mt-2 max-w-3xl text-sm font-bold text-[var(--text-muted)]">{detail}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap gap-2">{actions}</div> : null}
    </div>
  );
}

export function BrutalCard({
  children,
  className = "",
  accent = false,
}: {
  children: ReactNode;
  className?: string;
  accent?: boolean;
}) {
  return (
    <div
      className={`border-2 border-black ${accent ? "bg-[var(--accent-bg)]" : "bg-[var(--bg-card)]"} p-4 shadow-[var(--shadow-brutal)] ${className}`}
    >
      {children}
    </div>
  );
}

export function buttonClass(variant: "primary" | "secondary" | "danger" | "good" = "secondary") {
  const base =
    "inline-flex min-h-10 items-center justify-center border-2 border-black px-4 py-2 text-sm font-black uppercase tracking-wide shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[var(--shadow-brutal)] disabled:pointer-events-none disabled:opacity-50";
  if (variant === "primary") return `${base} bg-black text-white`;
  if (variant === "danger") return `${base} bg-[var(--btn-bg)] text-[var(--btn-text)]`;
  if (variant === "good") return `${base} bg-[var(--accent-color)] text-[var(--btn-text)]`;
  return `${base} bg-[var(--bg-card)] text-[var(--text-main)]`;
}

export const inputClass =
  "mt-1 block w-full border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-bold text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] outline-none placeholder:text-[var(--text-muted)] focus:-translate-x-0.5 focus:-translate-y-0.5 focus:shadow-[var(--shadow-brutal)]";

export const selectClass =
  "mt-1 block w-full border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-bold text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] outline-none focus:-translate-x-0.5 focus:-translate-y-0.5 focus:shadow-[var(--shadow-brutal)]";

export const textareaClass =
  "mt-1 block w-full border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-sm font-mono text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] outline-none placeholder:text-[var(--text-muted)] focus:-translate-x-0.5 focus:-translate-y-0.5 focus:shadow-[var(--shadow-brutal)]";

export const tableWrapClass =
  "overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]";

export const thClass =
  "border-b-2 border-black bg-[var(--accent-bg)] px-4 py-3 text-left text-xs font-black uppercase tracking-wide";

export const tdClass = "border-b border-black/15 px-4 py-3 text-sm font-bold";
