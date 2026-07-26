"use client";

import { InlineError, InlineNotice, StatusBadge } from "@/app/components/M7Primitives";
import type { Translations } from "@/lib/i18n";

// Ban defaults shared by the WAF form and the one-click ban on the sessions
// card. The reason is an API payload value, not rendered copy, so it stays
// untranslated to keep stored ban records stable.
export const DEFAULT_WAF_BAN_REASON = "manual WAF ban";
export const DEFAULT_WAF_BAN_MINUTES = 30;

// Runtime-only values (API base URL, local-time defaults) are stable for the
// lifetime of the page, so the external store never notifies subscribers.
export function subscribeRuntimeValue(): () => void {
  return () => {};
}

export function splitSettingList(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
    .filter((item) => {
      if (seen.has(item)) return false;
      seen.add(item);
      return true;
    });
}

export function nullableText(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

export function saveBlob(blob: Blob, filename: string) {
  const url = window.URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename || "download";
  document.body.appendChild(link);
  link.click();
  link.remove();
  window.URL.revokeObjectURL(url);
}

// Feedback lives per card: an error raised by one card must not be attributed
// to another one further down the page.
export function SectionFeedback({
  error,
  notice,
}: {
  error: string | null;
  notice: string | null;
}) {
  if (!error && !notice) return null;
  return (
    <div className="mb-4 space-y-3">
      <InlineError message={error} />
      {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
    </div>
  );
}

export function MaintenanceCapability({
  label,
  enabled,
  copy,
}: {
  label: string;
  enabled?: boolean;
  copy: Translations;
}) {
  return (
    <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2">
        <StatusBadge tone={enabled ? "green" : "gray"}>
          {enabled === undefined ? copy.common.notAvailable : enabled ? copy.settingsPage.supported : copy.settingsPage.unsupported}
        </StatusBadge>
      </div>
    </div>
  );
}

export function SettingsListSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="grid gap-3" aria-busy="true">
      {Array.from({ length: rows }).map((_, index) => (
        <div key={index} className="h-20 animate-pulse border-2 border-black bg-[var(--accent-bg)] shadow-[var(--shadow-brutal-sm)]" />
      ))}
    </div>
  );
}
