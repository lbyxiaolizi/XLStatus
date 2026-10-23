"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  StatusBadge,
  buttonClass,
  formatDate,
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

interface SessionInfo {
  id: string;
  user_id: string;
  username: string;
  role: string;
  ip?: string | null;
  user_agent?: string | null;
  expires_at?: string;
  created_at?: string;
  is_current?: boolean;
}

export function SessionsCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const cardRef = useLoadOnVisible(() => void loadSessions());

  async function loadSessions() {
    setSessionsLoading(true);
    const response = await apiClient.listSessions(200, 0);
    setSessionsLoading(false);
    if (response.success && response.data) {
      setSessions(((response.data.sessions as SessionInfo[]) ?? []).filter((session) => session.id));
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelSession(session: SessionInfo) {
    const label = session.is_current
      ? copy.settingsPage.currentSessionLabel
      : copy.settingsPage.userSessionLabel.replace("{username}", String(session.username));
    if (!(await dialogs.confirm({ message: copy.settingsPage.revokeSessionConfirm.replace("{label}", label), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteSession(session.id, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.sessionRevoked);
      await loadSessions();
      if (session.is_current) {
        localStorage.removeItem("session_token");
        localStorage.removeItem("user");
        window.location.href = "/login";
      }
    } else {
      setError(responseError(response));
    }
  }

  // Bans created here are not mirrored into the WAF card state; that card picks
  // them up on its next load or refresh.
  async function banSessionIp(session: SessionInfo) {
    const ip = session.ip?.trim();
    if (!ip) {
      setError(copy.settingsPage.sessionNoBannableIp);
      return;
    }
    if (!(await dialogs.confirm({ message: copy.settingsPage.banIpConfirm.replace("{ip}", String(ip)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createWafBans({
      ips: [ip],
      reason: DEFAULT_WAF_BAN_REASON,
      minutes: DEFAULT_WAF_BAN_MINUTES,
    }, totpCode);
    if (response.success && response.data) {
      const count = Array.isArray(response.data.bans) ? response.data.bans.length : 1;
      showToast(copy.settingsPage.wafBansCreated.replace("{count}", String(count)));
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">{copy.settingsPage.activeSessions}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadSessions()} disabled={sessionsLoading}>
            {sessionsLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} />
        <div className="grid gap-3">
          {sessionsLoading && sessions.length === 0 ? (
            <SettingsListSkeleton />
          ) : sessions.length === 0 ? (
            <p className="text-sm font-medium text-[var(--text-muted)]">{copy.settingsPage.noActiveSessions}</p>
          ) : (
            sessions.map((session) => (
              <div key={session.id} className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5 xl:grid-cols-[minmax(0,1fr)_14rem_auto] xl:items-center">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="break-words text-sm font-semibold text-[var(--text-main)]">{session.username}</span>
                    <StatusBadge tone={session.role === "admin" ? "yellow" : "gray"}>{session.role}</StatusBadge>
                    {session.is_current ? <StatusBadge tone="green">{copy.settingsPage.current}</StatusBadge> : null}
                  </div>
                  <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-muted)]">{session.id}</div>
                  <div className="mt-1 break-all text-xs text-[var(--text-muted)]">
                    {session.ip || "IP N/A"} / {session.user_agent || "UA N/A"}
                  </div>
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  <div>{copy.settingsPage.createdLabel.replace("{date}", String(formatDate(session.created_at)))}</div>
                  <div>{copy.settingsPage.expiresLabel.replace("{date}", String(formatDate(session.expires_at)))}</div>
                </div>
                <div className="flex flex-wrap gap-2 xl:justify-end">
                  <button type="button" className={buttonClass("secondary")} onClick={() => void run(() => banSessionIp(session))} disabled={busy || !session.ip}>
                    {copy.settingsPage.banIp}
                  </button>
                  <button type="button" className={buttonClass("danger")} onClick={() => void run(() => deletePanelSession(session))} disabled={busy}>
                    {copy.settingsPage.revoke}
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
