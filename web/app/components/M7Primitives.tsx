"use client";

import { type HTMLAttributes, ReactNode, useEffect, useId, useRef } from "react";
import { acquireBodyScrollLock } from "@/app/lib/body-scroll-lock";
import type { ApiResponse } from "@/lib/api";
import { formatLocaleDate, getTranslations } from "@/lib/i18n";

export {
  useStoredUser,
  readStoredUser,
  isAdmin,
  useBoldTheme,
  setStoredUser,
  clearStoredUser,
} from "@/app/lib/client-store";
export type { StoredUser } from "@/app/lib/client-store";

export function responseError(response: ApiResponse<unknown>): string {
  const copy = getTranslations();
  const suffix = response.request_id ? ` (${response.request_id})` : "";
  if (response.status === 401) return `${copy.common.authRequired}${suffix}`;
  if (response.status === 403) return `${copy.common.permissionDenied}${suffix}`;
  if (response.status === 404) return `${copy.common.backendNotFound}${suffix}`;
  return `${response.error || copy.common.requestFailed}${suffix}`;
}

export function formatDate(value?: string | number | null): string {
  if (!value) return getTranslations().common.never;
  const date = typeof value === "number" ? new Date(value * 1000) : new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return formatLocaleDate(date);
}

export function formatPercent(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return getTranslations().common.notAvailable;
  return `${value.toFixed(1)}%`;
}

export function formatMs(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return getTranslations().common.notAvailable;
  return `${value} ms`;
}

export function formatBytes(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return getTranslations().common.notAvailable;
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

export type BadgeTone = "green" | "red" | "yellow" | "gray" | "blue" | "pink";

export function StatusBadge({
  tone,
  children,
}: {
  tone: BadgeTone;
  children: ReactNode;
}) {
  const classes: Record<BadgeTone, string> = {
    green: "border-emerald-500/25 bg-[var(--status-ok-bg)] text-[var(--status-ok-text)]",
    red: "border-rose-500/25 bg-[var(--status-danger-bg)] text-[var(--status-danger-text)]",
    yellow: "border-amber-500/25 bg-[var(--status-warn-bg)] text-[var(--status-warn-text)]",
    gray: "border-[var(--border-color)] bg-[var(--bg-muted)] text-[var(--text-muted)]",
    blue: "border-indigo-500/25 bg-[var(--status-info-bg)] text-[var(--status-info-text)]",
    pink: "border-fuchsia-500/25 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300",
  };

  const dotColors: Record<BadgeTone, string> = {
    green: "bg-emerald-500",
    red: "bg-rose-500",
    yellow: "bg-amber-500",
    gray: "bg-zinc-400",
    blue: "bg-indigo-500",
    pink: "bg-fuchsia-500",
  };

  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium leading-5 tracking-tight ${classes[tone]}`}
    >
      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${dotColors[tone]}`} aria-hidden="true" />
      <span>{children}</span>
    </span>
  );
}

export function InlineError({ message }: { message: string | null | undefined }) {
  if (!message) return null;
  return (
    <div
      role="alert"
      className="flex items-start gap-2.5 rounded-xl border border-rose-500/30 bg-[var(--status-danger-bg)] px-4 py-3 text-sm font-medium text-[var(--status-danger-text)] break-words"
    >
      <span className="mt-1 h-2 w-2 shrink-0 rounded-full bg-rose-500" aria-hidden="true" />
      <div className="min-w-0 flex-1">{message}</div>
    </div>
  );
}

export function InlineNotice({
  tone = "blue",
  children,
}: {
  tone?: "blue" | "yellow" | "green" | "pink" | undefined;
  children: ReactNode;
}) {
  const classes = {
    blue: "border-indigo-500/25 bg-[var(--status-info-bg)] text-[var(--status-info-text)]",
    yellow: "border-amber-500/25 bg-[var(--status-warn-bg)] text-[var(--status-warn-text)]",
    green: "border-emerald-500/25 bg-[var(--status-ok-bg)] text-[var(--status-ok-text)]",
    pink: "border-fuchsia-500/25 bg-fuchsia-500/10 text-fuchsia-600 dark:text-fuchsia-300",
  };

  return (
    <div role="status" className={`rounded-xl border px-4 py-3 text-sm font-medium ${classes[tone]}`}>
      {children}
    </div>
  );
}

export function EmptyState({
  title,
  detail,
  action,
}: {
  title: string;
  detail?: string | undefined;
  action?: ReactNode | undefined;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-[var(--border-strong)] bg-[var(--bg-subtle)] px-6 py-12 text-center">
      <p className="text-sm font-semibold text-[var(--text-main)]">{title}</p>
      {detail ? <p className="mx-auto mt-1.5 max-w-xl text-xs leading-relaxed text-[var(--text-muted)]">{detail}</p> : null}
      {action ? <div className="mt-4">{action}</div> : null}
    </div>
  );
}

export function Skeleton({ className = "" }: { className?: string | undefined }) {
  return <div className={`animate-pulse rounded-md bg-[var(--bg-muted)] ${className}`} aria-hidden />;
}

export function SkeletonCard({ lines = 4 }: { lines?: number | undefined }) {
  return (
    <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-xs)]">
      <Skeleton className="h-5 w-1/2" />
      <Skeleton className="mt-2.5 h-3.5 w-3/4" />
      <div className="mt-4 grid grid-cols-2 gap-3">
        {Array.from({ length: lines }).map((_, index) => (
          <Skeleton key={index} className="h-8" />
        ))}
      </div>
    </div>
  );
}

export function TableSkeleton({
  columns = 5,
  rows = 5,
}: {
  columns?: number | undefined;
  rows?: number | undefined;
}) {
  const gridStyle = { gridTemplateColumns: `repeat(${columns}, minmax(8rem, 1fr))` };
  return (
    <div
      className="overflow-hidden rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]"
      aria-busy="true"
      aria-label={getTranslations().common.loading}
    >
      <div className="min-w-max">
        <div className="grid border-b border-[var(--border-color)] bg-[var(--bg-subtle)]" style={gridStyle}>
          {Array.from({ length: columns }).map((_, index) => (
            <div key={index} className="p-3.5">
              <Skeleton className="h-3.5 w-20" />
            </div>
          ))}
        </div>
        {Array.from({ length: rows }).map((_, row) => (
          <div key={row} className="grid border-b border-[var(--border-color)] last:border-b-0" style={gridStyle}>
            {Array.from({ length: columns }).map((_, column) => (
              <div key={column} className="p-3.5">
                <Skeleton className={`h-4 ${column === 0 ? "w-28" : "w-20"}`} />
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: string | undefined;
  error?: string | undefined;
  children: ReactNode;
}) {
  return (
    <label className="block space-y-1.5">
      <span className="block text-xs font-medium tracking-tight text-[var(--text-main)]">
        {label}
      </span>
      {children}
      {hint ? <span className="block text-xs text-[var(--text-muted)]">{hint}</span> : null}
      {error ? <span className="block text-xs font-medium text-[var(--status-danger-text)]">{error}</span> : null}
    </label>
  );
}

const DIALOG_FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

export function useDialogFocusTrap(onClose: () => void) {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const onCloseRef = useRef(onClose);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const previousFocus = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    const releaseScrollLock = acquireBodyScrollLock();

    const focusableElements = () => Array.from(
      dialog.querySelectorAll<HTMLElement>(DIALOG_FOCUSABLE_SELECTOR),
    ).filter((element) => {
      const style = window.getComputedStyle(element);
      return element.getClientRects().length > 0
        && !element.hidden
        && style.display !== "none"
        && style.visibility !== "hidden";
    });

    if (!dialog.contains(document.activeElement)) {
      const preferred = dialog.querySelector<HTMLElement>("[data-dialog-initial-focus]")
        ?? dialog.querySelector<HTMLElement>("input:not([disabled]):not([type='hidden']), select:not([disabled]), textarea:not([disabled])")
        ?? focusableElements()[0]
        ?? dialog;
      preferred.focus();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab") return;

      const focusable = focusableElements();
      if (focusable.length === 0) {
        event.preventDefault();
        dialogRef.current?.focus();
        return;
      }
      const activeIndex = focusable.indexOf(document.activeElement as HTMLElement);
      const nextIndex = event.shiftKey ? activeIndex - 1 : activeIndex + 1;
      if (activeIndex === -1 || nextIndex < 0 || nextIndex >= focusable.length) {
        event.preventDefault();
        const target = focusable[event.shiftKey ? focusable.length - 1 : 0];
        target?.focus();
      }
    }

    dialog.addEventListener("keydown", handleKeyDown);
    return () => {
      dialog.removeEventListener("keydown", handleKeyDown);
      releaseScrollLock();
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, []);

  return dialogRef;
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
  const titleId = useId();
  const dialogRef = useDialogFocusTrap(onClose);

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/55 backdrop-blur-xs px-4 py-6 sm:py-12">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="w-full max-w-3xl overflow-hidden rounded-2xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-lg)] outline-none"
      >
        <div className="flex items-center justify-between gap-3 border-b border-[var(--border-color)] bg-[var(--bg-subtle)] px-5 py-3.5">
          <h2 id={titleId} className="min-w-0 flex-1 break-words text-base font-semibold tracking-tight text-[var(--text-main)]">{title}</h2>
          <button type="button" onClick={onClose} className={`${buttonClass("secondary")} shrink-0`}>
            {getTranslations().common.close}
          </button>
        </div>
        <div className="px-5 py-5">{children}</div>
      </div>
    </div>
  );
}

export function PageShell({
  children,
  wide = false,
}: {
  children: ReactNode;
  wide?: boolean | undefined;
}) {
  return (
    <main
      className={`mx-auto w-full ${
        wide ? "max-w-[90rem]" : "max-w-7xl"
      } px-4 py-6 sm:px-6 lg:px-8 lg:py-8`}
    >
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
  eyebrow?: string | undefined;
  detail?: string | undefined;
  actions?: ReactNode | undefined;
}) {
  return (
    <div className="mb-6 grid gap-4 border-b border-[var(--border-color)] pb-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
      <div>
        {eyebrow ? (
          <p className="mb-2 inline-flex items-center gap-1.5 rounded-full border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 text-[11px] font-medium tracking-wide text-[var(--text-muted)] uppercase">
            <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-color)]" aria-hidden="true" />
            {eyebrow}
          </p>
        ) : null}
        <h1 className="text-2xl font-semibold tracking-tight text-[var(--text-main)] sm:text-3xl">
          {title}
        </h1>
        {detail ? <p className="mt-1.5 max-w-3xl text-sm text-[var(--text-muted)]">{detail}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}

export function BrutalCard({
  children,
  className = "",
  accent = false,
  ...props
}: {
  children: ReactNode;
  className?: string | undefined;
  accent?: boolean | undefined;
} & Omit<HTMLAttributes<HTMLDivElement>, "children" | "className">) {
  return (
    <div
      {...props}
      className={`rounded-xl border border-[var(--border-color)] ${accent ? "bg-[var(--bg-subtle)]" : "bg-[var(--bg-card)]"} p-5 shadow-[var(--shadow-xs)] ${className}`}
    >
      {children}
    </div>
  );
}

export function KpiCard({
  label,
  value,
  hint,
  badge,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode | undefined;
  badge?: ReactNode | undefined;
}) {
  return (
    <div className="flex flex-col justify-between rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-4 shadow-[var(--shadow-xs)]">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-[var(--text-muted)]">{label}</span>
        {badge}
      </div>
      <div className="mt-2 font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
        {value}
      </div>
      {hint ? <div className="mt-1.5 text-xs text-[var(--text-muted)]">{hint}</div> : null}
    </div>
  );
}

export function ProgressBar({
  value,
  tone = "emerald",
}: {
  value?: number | null | undefined;
  tone?: "emerald" | "indigo" | "amber" | "rose" | undefined;
}) {
  const safe = typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.min(100, value))
    : 0;
  const fillColor = safe >= 90
    ? "bg-rose-500"
    : safe >= 75
      ? "bg-amber-500"
      : tone === "indigo"
        ? "bg-indigo-500"
        : tone === "amber"
          ? "bg-amber-500"
          : tone === "rose"
            ? "bg-rose-500"
            : "bg-emerald-500";
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-muted)]">
      <div
        className={`h-full rounded-full transition-all duration-300 ${fillColor}`}
        style={{ width: `${safe}%` }}
      />
    </div>
  );
}

export function buttonClass(variant: "primary" | "secondary" | "danger" | "good" | "ghost" = "secondary") {
  const base =
    "inline-flex min-h-9 items-center justify-center gap-1.5 rounded-lg border px-3.5 py-1.5 text-xs font-medium tracking-tight transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--ring-color)] disabled:pointer-events-none disabled:opacity-50 cursor-pointer";
  if (variant === "primary") {
    return `${base} border-transparent bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)] hover:opacity-90`;
  }
  if (variant === "danger") {
    return `${base} border-rose-500/30 bg-[var(--status-danger-bg)] text-[var(--status-danger-text)] hover:bg-rose-500/20`;
  }
  if (variant === "good") {
    return `${base} border-emerald-500/30 bg-[var(--status-ok-bg)] text-[var(--status-ok-text)] hover:bg-emerald-500/20`;
  }
  if (variant === "ghost") {
    return `${base} border-transparent bg-transparent text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]`;
  }
  return `${base} border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]`;
}

export const inputClass =
  "mt-1 block w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 text-sm text-[var(--text-main)] shadow-[var(--shadow-xs)] outline-none transition placeholder:text-[var(--text-faint)] focus:border-[var(--accent-color)] focus:ring-2 focus:ring-[var(--ring-color)]";

export const selectClass =
  "mt-1 block w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 text-sm text-[var(--text-main)] shadow-[var(--shadow-xs)] outline-none transition focus:border-[var(--accent-color)] focus:ring-2 focus:ring-[var(--ring-color)]";

export const textareaClass =
  "mt-1 block w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 text-sm font-mono text-[var(--text-main)] shadow-[var(--shadow-xs)] outline-none transition placeholder:text-[var(--text-faint)] focus:border-[var(--accent-color)] focus:ring-2 focus:ring-[var(--ring-color)]";

export const tableWrapClass =
  "overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]";

export const thClass =
  "border-b border-[var(--border-color)] bg-[var(--bg-subtle)] px-4 py-3 text-left text-xs font-medium text-[var(--text-muted)]";

export const tdClass = "border-b border-[var(--border-color)] px-4 py-3 text-sm text-[var(--text-main)]";
