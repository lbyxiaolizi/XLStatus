"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  StatusBadge,
  buttonClass,
  inputClass,
  responseError,
  textareaClass,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import { apiClient, type SystemSettingsResponse } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { sanitizeAssetUrl, sanitizeCssVariableValue } from "@/app/lib/security";
import { useBusy } from "@/app/lib/use-busy";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { SectionFeedback, nullableText } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function PublicSiteCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [systemSettings, setSystemSettings] = useState<SystemSettingsResponse | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [publicBranding, setPublicBranding] = useState({
    siteName: "XLStatus",
    logoUrl: "",
    faviconUrl: "",
    themeColor: "",
    backgroundUrl: "",
    customHead: "",
    customBody: "",
  });
  const cardRef = useLoadOnVisible(() => void loadSystemSettings());

  async function loadSystemSettings(force = false) {
    setSettingsLoading(true);
    const response = await apiClient.getSettings({ bypassClientCache: force });
    setSettingsLoading(false);
    if (response.success && response.data) {
      const settings = response.data;
      setSystemSettings(settings);
      setPublicBranding({
        siteName: settings.public_site_name || "XLStatus",
        logoUrl: settings.public_logo_url || "",
        faviconUrl: settings.public_favicon_url || "",
        themeColor: settings.public_theme_color || "",
        backgroundUrl: settings.public_background_url || "",
        customHead: settings.public_custom_head || "",
        customBody: settings.public_custom_body || "",
      });
    } else {
      setError(responseError(response));
    }
  }

  async function updatePublicSiteEnabled(enabled: boolean) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({ public_site_enabled: enabled }, totpCode);
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      showToast(enabled ? copy.settingsPage.publicSiteEnabled : copy.settingsPage.publicSiteDisabled);
    } else {
      setError(responseError(response));
    }
  }

  async function updatePublicServerDetailsEnabled(enabled: boolean) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({ public_server_details_enabled: enabled }, totpCode);
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      showToast(enabled ? copy.settingsPage.publicServerDetailsShown : copy.settingsPage.publicServerDetailsHidden);
    } else {
      setError(responseError(response));
    }
  }

  async function savePublicBranding() {
    const siteName = publicBranding.siteName.trim();
    if (!siteName) {
      setError(copy.settingsPage.enterPublicSiteName);
      return;
    }
    const rawLogoUrl = nullableText(publicBranding.logoUrl);
    const safeLogoUrl = rawLogoUrl ? sanitizeAssetUrl(rawLogoUrl) : null;
    if (rawLogoUrl && !safeLogoUrl) {
      setError("Invalid logo URL: only http://, https://, or relative / paths are allowed.");
      return;
    }
    const rawFaviconUrl = nullableText(publicBranding.faviconUrl);
    const safeFaviconUrl = rawFaviconUrl ? sanitizeAssetUrl(rawFaviconUrl) : null;
    if (rawFaviconUrl && !safeFaviconUrl) {
      setError("Invalid favicon URL: only http://, https://, or relative / paths are allowed.");
      return;
    }
    const rawBackgroundUrl = nullableText(publicBranding.backgroundUrl);
    const safeBackgroundUrl = rawBackgroundUrl ? sanitizeAssetUrl(rawBackgroundUrl) : null;
    if (rawBackgroundUrl && !safeBackgroundUrl) {
      setError("Invalid background URL: only http://, https://, or relative / paths are allowed.");
      return;
    }
    const rawThemeColor = nullableText(publicBranding.themeColor);
    const safeThemeColor = rawThemeColor ? sanitizeCssVariableValue(rawThemeColor) : null;
    if (rawThemeColor && !safeThemeColor) {
      setError("Invalid theme color value.");
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({
      public_site_name: siteName,
      public_logo_url: safeLogoUrl,
      public_favicon_url: safeFaviconUrl,
      public_theme_color: safeThemeColor,
      public_background_url: safeBackgroundUrl,
      public_custom_head: null,
      public_custom_body: null,
    }, totpCode);
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      setPublicBranding((current) => ({ ...current, customHead: "", customBody: "" }));
      showToast(copy.settingsPage.publicBrandingSaved);
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">{copy.settingsPage.publicStatusPage}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadSystemSettings(true)} disabled={settingsLoading}>
            {settingsLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} />
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge tone={systemSettings ? (systemSettings.public_site_enabled ? "green" : "red") : "gray"}>
              {systemSettings ? (systemSettings.public_site_enabled ? copy.settingsPage.public : copy.settingsPage.private) : copy.common.notAvailable}
            </StatusBadge>
            <span className="text-sm font-medium text-[var(--text-muted)]">
              {copy.settingsPage.publicSiteToggleHint}
            </span>
          </div>
          <label className="inline-flex cursor-pointer items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={systemSettings?.public_site_enabled === true}
              onChange={(event) => void run(() => updatePublicSiteEnabled(event.target.checked))}
              disabled={settingsLoading || busy || !systemSettings}
            />
            {copy.settingsPage.anonymousAccess}
          </label>
        </div>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-color)] pt-3">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge tone={systemSettings?.public_server_details_enabled ? "green" : "gray"}>
              {systemSettings ? (systemSettings.public_server_details_enabled ? copy.settingsPage.showDetails : copy.settingsPage.hideDetails) : copy.common.notAvailable}
            </StatusBadge>
            <span className="text-sm font-medium text-[var(--text-muted)]">
              {copy.settingsPage.publicDetailsToggleHint}
            </span>
          </div>
          <label className="inline-flex cursor-pointer items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={systemSettings?.public_server_details_enabled === true}
              onChange={(event) => void run(() => updatePublicServerDetailsEnabled(event.target.checked))}
              disabled={settingsLoading || busy || !systemSettings}
            />
            {copy.settingsPage.serverDetails}
          </label>
        </div>
        <div className="mt-4 grid gap-3 border-t border-[var(--border-color)] pt-4 lg:grid-cols-2">
          <Field label={copy.settingsPage.siteName}>
            <input
              className={inputClass}
              value={publicBranding.siteName}
              onChange={(event) => setPublicBranding((current) => ({ ...current, siteName: event.target.value }))}
            />
          </Field>
          <Field label={copy.settingsPage.themeColor}>
            <input
              className={inputClass}
              value={publicBranding.themeColor}
              onChange={(event) => setPublicBranding((current) => ({ ...current, themeColor: event.target.value }))}
              placeholder="#16a34a"
            />
          </Field>
          <Field label={copy.settingsPage.logoUrl}>
            <input
              className={inputClass}
              value={publicBranding.logoUrl}
              onChange={(event) => setPublicBranding((current) => ({ ...current, logoUrl: event.target.value }))}
            />
          </Field>
          <Field label={copy.settingsPage.faviconUrl}>
            <input
              className={inputClass}
              value={publicBranding.faviconUrl}
              onChange={(event) => setPublicBranding((current) => ({ ...current, faviconUrl: event.target.value }))}
            />
          </Field>
          <Field label={copy.settingsPage.backgroundUrl}>
            <input
              className={inputClass}
              value={publicBranding.backgroundUrl}
              onChange={(event) => setPublicBranding((current) => ({ ...current, backgroundUrl: event.target.value }))}
            />
          </Field>
          <div className="flex items-end justify-end">
            <button type="button" className={buttonClass("primary")} onClick={() => void run(savePublicBranding)} disabled={settingsLoading || busy}>
              {copy.settingsPage.saveBranding}
            </button>
          </div>
          <Field label={copy.settingsPage.customHeadDisabled}>
            <textarea
              className={`${textareaClass} min-h-28`}
              value={publicBranding.customHead}
              readOnly
            />
          </Field>
          <Field label={copy.settingsPage.customBodyDisabled}>
            <textarea
              className={`${textareaClass} min-h-28`}
              value={publicBranding.customBody}
              readOnly
            />
          </Field>
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
