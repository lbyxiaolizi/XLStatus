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
  textareaClass,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import {
  apiClient,
  type OAuthAccount,
  type OAuthProvider,
  type TotpSetupResponse,
  type TotpStatusResponse,
} from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { sanitizeExternalHttpUrl } from "@/app/lib/security";
import { useBusy } from "@/app/lib/use-busy";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { SectionFeedback } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function SecurityCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [totpSetup, setTotpSetup] = useState<TotpSetupResponse | null>(null);
  const [totpCode, setTotpCode] = useState("");
  const [totpLoading, setTotpLoading] = useState(true);
  const [oauthProviders, setOauthProviders] = useState<OAuthProvider[]>([]);
  const [oauthBindings, setOauthBindings] = useState<OAuthAccount[]>([]);
  const cardRef = useLoadOnVisible(() => {
    void Promise.all([loadTotpStatus(), loadOAuthProviders(), loadOAuthBindings()]);
  });

  async function loadTotpStatus(force = false) {
    setTotpLoading(true);
    const response = await apiClient.getTotpStatus({ bypassClientCache: force });
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpStatus(response.data);
    } else {
      setError(responseError(response));
    }
  }

  async function loadOAuthProviders() {
    const response = await apiClient.listOAuthProviders();
    if (response.success && response.data) {
      setOauthProviders(response.data.providers ?? []);
    }
  }

  async function loadOAuthBindings() {
    const response = await apiClient.listOAuthBindings();
    if (response.success && response.data) {
      setOauthBindings(response.data.accounts ?? []);
    } else {
      setError(responseError(response));
    }
  }

  async function setupTotp() {
    setTotpLoading(true);
    const response = await apiClient.setupTotp();
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpSetup(response.data);
      setTotpStatus({ enabled: false, setup_pending: true });
      setTotpCode("");
      showToast(copy.settingsPage.totpSecretGenerated);
    } else {
      setError(responseError(response));
    }
  }

  async function enableTotp() {
    if (totpCode.trim().length !== 6) {
      setError(copy.settingsPage.totpEnterSixDigits);
      return;
    }
    setTotpLoading(true);
    const response = await apiClient.enableTotp(totpCode.trim());
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpStatus(response.data);
      setTotpSetup(null);
      setTotpCode("");
      showToast(copy.settingsPage.totpEnabled);
    } else {
      setError(responseError(response));
    }
  }

  async function disableTotp() {
    if (totpStatus?.enabled && totpCode.trim().length !== 6) {
      setError(copy.settingsPage.totpEnterCurrentSixDigits);
      return;
    }
    if (!(await dialogs.confirm({ message: copy.settingsPage.disableTotpConfirm, danger: true }))) return;
    setTotpLoading(true);
    const response = await apiClient.disableTotp(totpCode.trim());
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpStatus(response.data);
      setTotpSetup(null);
      setTotpCode("");
      showToast(copy.settingsPage.totpDisabled);
    } else {
      setError(responseError(response));
    }
  }

  async function unbindOAuthProvider(providerId: string, displayName: string) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.unbindOAuthConfirm.replace("{name}", String(displayName)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.unbindOAuthProvider(providerId, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.oauthUnbound.replace("{name}", String(displayName)));
      await loadOAuthBindings();
    } else {
      setError(responseError(response));
    }
  }

  async function bindOAuthProvider(providerId: string) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.startOAuthBind(providerId, "/settings", totpCode);
    const safeAuthUrl = sanitizeExternalHttpUrl(response.data?.authorization_url);
    if (response.success && safeAuthUrl) {
      window.location.href = safeAuthUrl;
    } else {
      setError(responseError(response));
    }
  }

  async function copyTotpUri() {
    if (!totpSetup?.otpauth_uri) return;
    try {
      await navigator.clipboard.writeText(totpSetup.otpauth_uri);
      showToast(copy.settingsPage.totpUriCopied);
    } catch {
      setError(copy.settingsPage.clipboardUriFailed);
    }
  }

  const oauthBindingMap = new Map(oauthBindings.map((binding) => [binding.provider, binding]));
  const oauthProviderIds = new Set(oauthProviders.map((provider) => provider.id));
  const staleOAuthBindings = oauthBindings.filter((binding) => !oauthProviderIds.has(binding.provider));

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">{copy.settingsPage.accountSecurity}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadTotpStatus(true)} disabled={totpLoading}>
            {totpLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} />
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,0.45fr)]">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <StatusBadge tone={totpStatus?.enabled ? "green" : "gray"}>
                {totpStatus ? (totpStatus.enabled ? copy.settingsPage.totpEnabledBadge : copy.settingsPage.totpDisabledBadge) : copy.common.notAvailable}
              </StatusBadge>
              {totpStatus?.setup_pending ? <StatusBadge tone="yellow">{copy.settingsPage.pendingVerification}</StatusBadge> : null}
            </div>
            {totpSetup ? (
              <div className="grid gap-3">
                <Field label="Secret">
                  <input className={inputClass} value={totpSetup.secret} readOnly />
                </Field>
                <Field label="otpauth URI">
                  <textarea className={`${textareaClass} min-h-24`} value={totpSetup.otpauth_uri} readOnly />
                </Field>
                <div className="flex flex-wrap gap-2">
                  <button type="button" className={buttonClass("secondary")} onClick={() => void copyTotpUri()}>
                    {copy.settingsPage.copyUri}
                  </button>
                </div>
              </div>
            ) : (
              <p className="text-sm font-medium text-[var(--text-muted)]">
                {copy.settingsPage.totpHint}
              </p>
            )}
          </div>
          <div className="grid content-end gap-3">
            <Field label={totpStatus?.enabled ? copy.settingsPage.currentCode : copy.settingsPage.enableCode}>
              <input
                className={inputClass}
                value={totpCode}
                onChange={(event) => setTotpCode(event.target.value.replace(/\D/g, "").slice(0, 6))}
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="123456"
              />
            </Field>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={buttonClass("primary")} onClick={() => void run(setupTotp)} disabled={totpLoading || busy}>
                {copy.settingsPage.generateSecret}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void run(enableTotp)} disabled={totpLoading || busy || !totpStatus?.setup_pending}>
                {copy.settingsPage.enable}
              </button>
              <button type="button" className={buttonClass("danger")} onClick={() => void run(disableTotp)} disabled={totpLoading || busy || !totpStatus?.enabled}>
                {copy.settingsPage.disable}
              </button>
            </div>
            {oauthProviders.length > 0 || staleOAuthBindings.length > 0 ? (
              <div className="grid gap-3 border-t border-[var(--border-color)] pt-3">
                {oauthProviders.map((provider) => {
                  const binding = oauthBindingMap.get(provider.id);
                  return (
                    <div key={provider.id} className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div className="min-w-0">
                          <p className="truncate text-sm font-semibold text-[var(--text-main)]">{provider.display_name}</p>
                          <p className="break-all text-xs text-[var(--text-muted)]">
                            {binding?.email || binding?.display_name || binding?.subject || provider.scopes.join(" ")}
                          </p>
                          {binding ? (
                            <p className="text-xs text-[var(--text-muted)]">
                              {copy.settingsPage.boundAt.replace("{date}", String(formatDate(binding.created_at)))}
                            </p>
                          ) : null}
                        </div>
                        <StatusBadge tone={binding ? "green" : "gray"}>{binding ? copy.settingsPage.bound : copy.settingsPage.notBound}</StatusBadge>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {binding ? (
                          <button
                            type="button"
                            className={buttonClass("danger")}
                            onClick={() => void run(() => unbindOAuthProvider(provider.id, provider.display_name))}
                            disabled={busy}
                          >
                            {copy.settingsPage.unbind}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className={buttonClass("secondary")}
                            onClick={() => void run(() => bindOAuthProvider(provider.id))}
                            disabled={busy}
                          >
                            {copy.settingsPage.bind}
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
                {staleOAuthBindings.map((binding) => (
                  <div key={binding.provider} className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-semibold text-[var(--text-main)]">{binding.provider_display_name}</p>
                        <p className="break-all text-xs text-[var(--text-muted)]">
                          {binding.email || binding.display_name || binding.subject}
                        </p>
                        <p className="text-xs text-[var(--text-muted)]">
                          {copy.settingsPage.configRemovedBoundAt.replace("{date}", String(formatDate(binding.created_at)))}
                        </p>
                      </div>
                      <StatusBadge tone="yellow">{copy.settingsPage.staleBinding}</StatusBadge>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <button
                        type="button"
                        className={buttonClass("danger")}
                        onClick={() => void run(() => unbindOAuthProvider(binding.provider, binding.provider_display_name))}
                        disabled={busy}
                      >
                        {copy.settingsPage.unbind}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
