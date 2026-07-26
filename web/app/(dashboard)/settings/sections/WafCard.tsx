"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import { apiClient } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useBusy } from "@/app/lib/use-busy";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import {
  DEFAULT_WAF_BAN_MINUTES,
  DEFAULT_WAF_BAN_REASON,
  SectionFeedback,
  SettingsListSkeleton,
} from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

interface WafBan {
  id: string;
  ip: string;
  reason: string;
  failed_count: number;
  banned_until?: string;
  created_at?: string;
  updated_at?: string;
}

export function WafCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [wafBans, setWafBans] = useState<WafBan[]>([]);
  const [wafLoading, setWafLoading] = useState(true);
  const [wafBanDraft, setWafBanDraft] = useState({
    ips: "",
    reason: DEFAULT_WAF_BAN_REASON,
    minutes: String(DEFAULT_WAF_BAN_MINUTES),
  });
  const cardRef = useLoadOnVisible(() => void loadWafBans());

  async function loadWafBans() {
    setWafLoading(true);
    const response = await apiClient.listWafBans(200, 0);
    setWafLoading(false);
    if (response.success && response.data) {
      setWafBans(((response.data.bans as WafBan[]) ?? []).filter((ban) => ban.id));
    } else {
      setError(responseError(response));
    }
  }

  async function createPanelWafBan() {
    const ips = [wafBanDraft.ips];
    if (ips.every((value) => !value.trim())) {
      setError(copy.settingsPage.wafEnterIp);
      return;
    }
    const minutes = Number.parseInt(wafBanDraft.minutes, 10);
    if (!Number.isFinite(minutes) || minutes <= 0) {
      setError(copy.settingsPage.wafMinutesPositive);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createWafBans({
      ips,
      reason: wafBanDraft.reason,
      minutes,
    }, totpCode);
    if (response.success && response.data) {
      const count = Array.isArray(response.data.bans) ? response.data.bans.length : ips.length;
      showToast(copy.settingsPage.wafBansCreated.replace("{count}", String(count)));
      setWafBanDraft((current) => ({ ...current, ips: "" }));
      await loadWafBans();
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelWafBan(ban: WafBan) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.unbanConfirm.replace("{ip}", String(ban.ip)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteWafBan(ban.id, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.wafBanRemoved.replace("{ip}", String(ban.ip)));
      await loadWafBans();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.wafBans}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadWafBans()} disabled={wafLoading}>
            {wafLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} />
        <div className="mb-4 grid gap-3 border-b-2 border-black pb-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,0.8fr)_9rem_auto] xl:items-end">
          <Field label={copy.settingsPage.ipList}>
            <input
              className={inputClass}
              value={wafBanDraft.ips}
              onChange={(event) => setWafBanDraft((current) => ({ ...current, ips: event.target.value }))}
              placeholder="1.2.3.4, 2001:db8::1"
            />
          </Field>
          <Field label={copy.settingsPage.reason}>
            <input
              className={inputClass}
              value={wafBanDraft.reason}
              onChange={(event) => setWafBanDraft((current) => ({ ...current, reason: event.target.value }))}
            />
          </Field>
          <Field label={copy.settingsPage.minutes}>
            <input
              className={inputClass}
              type="number"
              min="1"
              max="43200"
              value={wafBanDraft.minutes}
              onChange={(event) => setWafBanDraft((current) => ({ ...current, minutes: event.target.value }))}
            />
          </Field>
          <button type="button" className={buttonClass("danger")} onClick={() => void run(createPanelWafBan)} disabled={wafLoading || busy}>
            {copy.settingsPage.bulkBan}
          </button>
        </div>
        <div className="grid gap-3">
          {wafLoading && wafBans.length === 0 ? (
            <SettingsListSkeleton />
          ) : wafBans.length === 0 ? (
            <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noActiveBans}</p>
          ) : (
            wafBans.map((ban) => (
              <div key={ban.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_12rem_auto] xl:items-center">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="break-all text-sm font-black">{ban.ip}</span>
                    <StatusBadge tone="red">{copy.settingsPage.failedCount.replace("{count}", String(ban.failed_count))}</StatusBadge>
                  </div>
                  <div className="mt-1 break-words text-xs font-bold text-[var(--text-muted)]">{ban.reason}</div>
                  <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{ban.id}</div>
                </div>
                <div className="text-xs font-bold text-[var(--text-muted)]">
                  <div>{copy.settingsPage.unbanLabel.replace("{date}", String(formatDate(ban.banned_until)))}</div>
                  <div>{copy.settingsPage.updatedLabel.replace("{date}", String(formatDate(ban.updated_at || ban.created_at)))}</div>
                </div>
                <button type="button" className={buttonClass("danger")} onClick={() => void run(() => deletePanelWafBan(ban))} disabled={busy}>
                  {copy.settingsPage.remove}
                </button>
              </div>
            ))
          )}
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
