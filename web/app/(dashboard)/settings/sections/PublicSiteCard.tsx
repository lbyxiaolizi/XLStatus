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
import { apiClient, type SystemSettingsResponse } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { SectionFeedback, nullableText } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function PublicSiteCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
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
      setNotice(enabled ? copy.settingsPage.publicSiteEnabled : copy.settingsPage.publicSiteDisabled);
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
      setNotice(enabled ? copy.settingsPage.publicServerDetailsShown : copy.settingsPage.publicServerDetailsHidden);
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
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({
      public_site_name: siteName,
      public_logo_url: nullableText(publicBranding.logoUrl),
      public_favicon_url: nullableText(publicBranding.faviconUrl),
      public_theme_color: nullableText(publicBranding.themeColor),
      public_background_url: nullableText(publicBranding.backgroundUrl),
      public_custom_head: null,
      public_custom_body: null,
    }, totpCode);
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      setPublicBranding((current) => ({ ...current, customHead: "", customBody: "" }));
      setNotice(copy.settingsPage.publicBrandingSaved);
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.publicStatusPage}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadSystemSettings(true)} disabled={settingsLoading}>
            {settingsLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} notice={notice} />
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge tone={systemSettings ? (systemSettings.public_site_enabled ? "green" : "red") : "gray"}>
              {systemSettings ? (systemSettings.public_site_enabled ? copy.settingsPage.public : copy.settingsPage.private) : copy.common.notAvailable}
            </StatusBadge>
            <span className="text-sm font-bold text-[var(--text-muted)]">
              {copy.settingsPage.publicSiteToggleHint}
            </span>
          </div>
          <label className="inline-flex cursor-pointer items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input
              type="checkbox"
              checked={systemSettings?.public_site_enabled === true}
              onChange={(event) => void updatePublicSiteEnabled(event.target.checked)}
              disabled={settingsLoading || !systemSettings}
            />
            {copy.settingsPage.anonymousAccess}
          </label>
        </div>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t-2 border-black pt-3">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge tone={systemSettings?.public_server_details_enabled ? "green" : "gray"}>
              {systemSettings ? (systemSettings.public_server_details_enabled ? copy.settingsPage.showDetails : copy.settingsPage.hideDetails) : copy.common.notAvailable}
            </StatusBadge>
            <span className="text-sm font-bold text-[var(--text-muted)]">
              {copy.settingsPage.publicDetailsToggleHint}
            </span>
          </div>
          <label className="inline-flex cursor-pointer items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input
              type="checkbox"
              checked={systemSettings?.public_server_details_enabled === true}
              onChange={(event) => void updatePublicServerDetailsEnabled(event.target.checked)}
              disabled={settingsLoading || !systemSettings}
            />
            {copy.settingsPage.serverDetails}
          </label>
        </div>
        <div className="mt-4 grid gap-3 border-t-2 border-black pt-4 lg:grid-cols-2">
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
            <button type="button" className={buttonClass("primary")} onClick={() => void savePublicBranding()} disabled={settingsLoading}>
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
