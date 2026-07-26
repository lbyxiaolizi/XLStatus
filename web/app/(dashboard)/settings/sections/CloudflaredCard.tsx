"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  InlineError,
  buttonClass,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import { apiClient, type CloudflaredStatusResponse } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { MaintenanceCapability, SectionFeedback, nullableText } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function CloudflaredCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [cloudflaredStatus, setCloudflaredStatus] = useState<CloudflaredStatusResponse | null>(null);
  const [cloudflaredToken, setCloudflaredToken] = useState("");
  const [cloudflaredLoading, setCloudflaredLoading] = useState(true);
  const cardRef = useLoadOnVisible(() => void loadCloudflaredStatus());

  async function loadCloudflaredStatus() {
    setCloudflaredLoading(true);
    const response = await apiClient.getCloudflaredStatus();
    setCloudflaredLoading(false);
    if (response.success && response.data) {
      setCloudflaredStatus(response.data);
    } else {
      setError(responseError(response));
    }
  }

  async function saveCloudflaredToken() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setCloudflaredLoading(true);
    const response = await apiClient.saveCloudflaredToken(nullableText(cloudflaredToken), totpCode);
    setCloudflaredLoading(false);
    if (response.success && response.data) {
      setCloudflaredStatus(response.data.status);
      setCloudflaredToken("");
      setNotice(copy.settingsPage.cloudflaredTokenSaved);
    } else {
      setError(responseError(response));
    }
  }

  async function runCloudflared(action: "start" | "stop") {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setCloudflaredLoading(true);
    const response =
      action === "start"
        ? await apiClient.startCloudflared(totpCode)
        : await apiClient.stopCloudflared(totpCode);
    setCloudflaredLoading(false);
    if (response.success && response.data) {
      setCloudflaredStatus(response.data.status);
      setNotice(action === "start" ? copy.settingsPage.cloudflaredStarted : copy.settingsPage.cloudflaredStopped);
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.cloudflareTunnel}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadCloudflaredStatus()} disabled={cloudflaredLoading}>
            {cloudflaredLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} notice={notice} />
        <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
          <div className="grid gap-3 md:grid-cols-3">
            <MaintenanceCapability label={copy.settingsPage.runningStatus} enabled={cloudflaredStatus ? Boolean(cloudflaredStatus.running) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.tokenLabel} enabled={cloudflaredStatus ? Boolean(cloudflaredStatus.token_configured) : undefined} copy={copy} />
            <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
              <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">PID</div>
              <div className="mt-2 text-sm font-black">{cloudflaredStatus?.pid ?? "N/A"}</div>
            </div>
          </div>
          <div className="flex flex-wrap gap-2 lg:justify-end">
            <button type="button" className={buttonClass("primary")} onClick={() => void runCloudflared("start")} disabled={cloudflaredLoading}>
              {copy.settingsPage.start}
            </button>
            <button type="button" className={buttonClass("danger")} onClick={() => void runCloudflared("stop")} disabled={cloudflaredLoading}>
              {copy.settingsPage.stop}
            </button>
          </div>
        </div>
        <div className="mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
          <Field label="Tunnel token">
            <input
              className={inputClass}
              value={cloudflaredToken}
              onChange={(event) => setCloudflaredToken(event.target.value)}
              type="password"
              autoComplete="off"
            />
          </Field>
          <button type="button" className={buttonClass("secondary")} onClick={() => void saveCloudflaredToken()} disabled={cloudflaredLoading}>
            {copy.settingsPage.saveToken}
          </button>
        </div>
        {cloudflaredStatus?.last_error ? (
          <InlineError message={cloudflaredStatus.last_error} />
        ) : null}
        {cloudflaredStatus?.logs?.length ? (
          <pre className="mt-3 max-h-48 overflow-auto border-2 border-black bg-black p-3 text-xs font-bold text-green-300 shadow-[var(--shadow-brutal-sm)]">
            {cloudflaredStatus.logs.slice(-20).join("\n")}
          </pre>
        ) : null}
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
