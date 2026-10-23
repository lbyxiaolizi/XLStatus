"use client";

import { InlineError, StatusBadge } from "@/app/components/M7Primitives";
import { sanitizeDownloadFilename } from "@/app/lib/security";
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
  link.download = sanitizeDownloadFilename(filename, "download");
  document.body.appendChild(link);
  link.click();
  link.remove();
  window.URL.revokeObjectURL(url);
}

// Errors live per card: one raised here must not be attributed to another card
// further down the page. Success confirmations go through toasts instead, so
// this slot only carries errors, which must stay on screen until resolved.
export function SectionFeedback({ error }: { error: string | null }) {
  if (!error) return null;
  return (
    <div className="mb-4">
      <InlineError message={error} />
    </div>
  );
}

export function MaintenanceCapability({
  label,
  enabled,
  copy,
}: {
  label: string;
  enabled?: boolean | undefined;
  copy: Translations;
}) {
  return (
    <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
      <div className="text-xs font-medium text-[var(--text-muted)]">{label}</div>
      <div className="mt-2">
        <StatusBadge tone={enabled ? "green" : "gray"}>
          {enabled === undefined ? copy.common.notAvailable : enabled ? copy.settingsPage.supported : copy.settingsPage.unsupported}
        </StatusBadge>
      </div>
    </div>
  );
}

export function SettingsListSkeleton({ rows = 3 }: { rows?: number | undefined }) {
  return (
    <div className="grid gap-3" aria-busy="true">
      {Array.from({ length: rows }).map((_, index) => (
        <div key={index} className="h-20 animate-pulse rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]" />
      ))}
    </div>
  );
}
