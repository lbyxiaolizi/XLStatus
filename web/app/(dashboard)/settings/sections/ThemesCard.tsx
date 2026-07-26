"use client";

import { useEffect, useRef, useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  InlineError,
  StatusBadge,
  buttonClass,
  inputClass,
  responseError,
  textareaClass,
} from "@/app/components/M7Primitives";
import { apiClient, type ImportThemeRequest, type ThemeDefinition } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import {
  cacheDashboardTheme,
  DASHBOARD_THEME_CACHE_KEY,
} from "@/app/lib/dashboard-theme";
import { readStoredUser } from "@/app/lib/client-store";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import type { Translations } from "@/lib/i18n";
import { SectionFeedback } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function ThemesCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [themes, setThemes] = useState<ThemeDefinition[]>([]);
  const [selectedPublicThemeId, setSelectedPublicThemeId] = useState("");
  const [selectedDashboardThemeId, setSelectedDashboardThemeId] = useState("");
  const [themeImportText, setThemeImportText] = useState(() => defaultThemeImportText());
  const [themeLoading, setThemeLoading] = useState(false);
  const [themesLoaded, setThemesLoaded] = useState(false);
  const [themeError, setThemeError] = useState<string | null>(null);
  const themesLoadRequestIdRef = useRef(0);
  const cardRef = useLoadOnVisible(() => void loadThemes());

  useEffect(() => {
    function handleThemeStorage(event: StorageEvent) {
      if (event.key !== DASHBOARD_THEME_CACHE_KEY) return;
      void loadThemes(true);
    }

    window.addEventListener("storage", handleThemeStorage);
    return () => {
      themesLoadRequestIdRef.current += 1;
      window.removeEventListener("storage", handleThemeStorage);
    };
  }, []);

  async function loadThemes(force = false, broadcast = false) {
    const requestUserId = readStoredUser()?.id;
    const requestId = themesLoadRequestIdRef.current + 1;
    themesLoadRequestIdRef.current = requestId;
    setThemeLoading(true);
    setThemeError(null);
    const response = await apiClient.listThemes({ bypassClientCache: force });
    if (requestId !== themesLoadRequestIdRef.current) return;
    setThemeLoading(false);
    if (!requestUserId || readStoredUser()?.id !== requestUserId) return;
    setThemesLoaded(true);
    if (response.success && response.data) {
      const nextThemes = response.data.themes ?? [];
      const dashboardThemeId = response.data.selected_dashboard_theme_id || "";
      setThemes(nextThemes);
      setSelectedPublicThemeId(response.data.selected_public_theme_id || "");
      setSelectedDashboardThemeId(dashboardThemeId);
      cacheDashboardTheme(
        requestUserId,
        nextThemes.find((theme) => theme.id === dashboardThemeId) ?? null,
        broadcast,
      );
    } else {
      const message = responseError(response);
      setThemeError(message);
      setError(message);
    }
  }

  async function importTheme() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(themeImportText);
    } catch {
      setError(copy.settingsPage.themeJsonInvalid);
      return;
    }
    const candidate = parsed && typeof parsed === "object" && "theme" in parsed
      ? (parsed as { theme: unknown }).theme
      : parsed;
    if (!candidate || typeof candidate !== "object") {
      setError(copy.settingsPage.themeJsonNotObject);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const requestUserId = readStoredUser()?.id;
    if (!requestUserId) return;
    setThemeLoading(true);
    setThemeError(null);
    const response = await apiClient.importTheme(candidate as ImportThemeRequest["theme"], totpCode);
    setThemeLoading(false);
    if (readStoredUser()?.id !== requestUserId) return;
    if (response.success && response.data) {
      setNotice(copy.settingsPage.themeImported.replace("{name}", String(response.data.name)));
      await loadThemes();
    } else {
      const message = responseError(response);
      setThemeError(message);
      setError(message);
    }
  }

  async function loadThemeFile(file?: File | null) {
    if (!file) return;
    try {
      setThemeImportText(await file.text());
    } catch {
      setError(copy.settingsPage.themeFileReadFailed);
    }
  }

  async function selectTheme(theme: ThemeDefinition, target: "public" | "dashboard" | "both") {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const requestUserId = readStoredUser()?.id;
    if (!requestUserId) return;
    setThemeLoading(true);
    setThemeError(null);
    const response = await apiClient.selectTheme(theme.id, target, totpCode);
    setThemeLoading(false);
    if (readStoredUser()?.id !== requestUserId) return;
    if (response.success && response.data) {
      const nextThemes = response.data.themes ?? [];
      const dashboardThemeId = response.data.selected_dashboard_theme_id || "";
      setThemes(nextThemes);
      setSelectedPublicThemeId(response.data.selected_public_theme_id || "");
      setSelectedDashboardThemeId(dashboardThemeId);
      cacheDashboardTheme(
        requestUserId,
        nextThemes.find((item) => item.id === dashboardThemeId) ?? null,
        true,
      );
      setNotice(copy.settingsPage.themeApplied.replace("{name}", String(theme.name)));
    } else {
      const message = responseError(response);
      setThemeError(message);
      setError(message);
    }
  }

  async function deleteTheme(theme: ThemeDefinition) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.deleteThemeConfirm.replace("{name}", String(theme.name)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const requestUserId = readStoredUser()?.id;
    if (!requestUserId) return;
    setThemeLoading(true);
    setThemeError(null);
    const response = await apiClient.deleteTheme(theme.id, totpCode);
    setThemeLoading(false);
    if (readStoredUser()?.id !== requestUserId) return;
    if (response.success) {
      setNotice(copy.settingsPage.themeDeleted.replace("{name}", String(theme.name)));
      await loadThemes(true, true);
    } else {
      const message = responseError(response);
      setThemeError(message);
      setError(message);
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.themeTemplates}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadThemes(true)} disabled={themeLoading}>
            {themeLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        {/* Theme load/apply failures already render below as themeError, so only
            client-side validation errors surface in the card feedback slot. */}
        <SectionFeedback error={error === themeError ? null : error} notice={notice} />
        <InlineError message={themeError} />
        <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]">
          <div className="grid content-start gap-3">
            {(!themesLoaded || themeLoading) && themes.length === 0 ? (
              <div className="grid gap-3" aria-busy="true">
                {Array.from({ length: 3 }).map((_, index) => (
                  <div key={index} className="h-36 animate-pulse border-2 border-black bg-[var(--accent-bg)] shadow-[var(--shadow-brutal-sm)]" />
                ))}
              </div>
            ) : themeError && themes.length === 0 ? null : themes.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noThemes}</p>
            ) : (
              themes.map((theme) => (
                <div key={theme.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <h3 className="break-words text-lg font-black uppercase">{theme.name}</h3>
                        <StatusBadge tone={theme.builtin ? "blue" : "pink"}>{theme.builtin ? copy.settingsPage.builtin : copy.settingsPage.custom}</StatusBadge>
                        <StatusBadge tone="gray">{themeTargetLabel(theme.target, copy)}</StatusBadge>
                        {selectedPublicThemeId === theme.id ? <StatusBadge tone="green">{copy.settingsPage.publicPageBadge}</StatusBadge> : null}
                        {selectedDashboardThemeId === theme.id ? <StatusBadge tone="yellow">{copy.settingsPage.dashboardBadge}</StatusBadge> : null}
                      </div>
                      <p className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{theme.id}</p>
                      {theme.description ? (
                        <p className="mt-1 text-sm font-bold text-[var(--text-muted)]">{theme.description}</p>
                      ) : null}
                    </div>
                    <div className="flex shrink-0 flex-wrap gap-1">
                      {themeSwatches(theme).map(([key, value]) => (
                        <span
                          key={key}
                          title={`${key}: ${value}`}
                          className="h-7 w-7 border-2 border-black shadow-[var(--shadow-brutal-sm)]"
                          style={{ background: value }}
                        />
                      ))}
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      className={buttonClass("secondary")}
                      onClick={() => void selectTheme(theme, "public")}
                      disabled={themeLoading || !themeSupportsTarget(theme, "public") || selectedPublicThemeId === theme.id}
                    >
                      {copy.settingsPage.publicAction}
                    </button>
                    <button
                      type="button"
                      className={buttonClass("secondary")}
                      onClick={() => void selectTheme(theme, "dashboard")}
                      disabled={themeLoading || !themeSupportsTarget(theme, "dashboard") || selectedDashboardThemeId === theme.id}
                    >
                      {copy.settingsPage.dashboardAction}
                    </button>
                    <button
                      type="button"
                      className={buttonClass("primary")}
                      onClick={() => void selectTheme(theme, "both")}
                      disabled={themeLoading || theme.target !== "both" || (selectedPublicThemeId === theme.id && selectedDashboardThemeId === theme.id)}
                    >
                      {copy.settingsPage.bothAction}
                    </button>
                    {!theme.builtin ? (
                      <button type="button" className={buttonClass("danger")} onClick={() => void deleteTheme(theme)} disabled={themeLoading}>
                        {copy.settingsPage.delete}
                      </button>
                    ) : null}
                  </div>
                </div>
              ))
            )}
          </div>
          <div className="grid content-start gap-3">
            <Field label={copy.settingsPage.themeFile}>
              <input
                className={inputClass}
                type="file"
                accept=".json,application/json"
                onChange={(event) => void loadThemeFile(event.target.files?.[0])}
              />
            </Field>
            <Field label={copy.settingsPage.importJson}>
              <textarea
                className={`${textareaClass} min-h-80 font-mono text-xs`}
                value={themeImportText}
                onChange={(event) => setThemeImportText(event.target.value)}
                spellCheck={false}
              />
            </Field>
            <div className="flex flex-wrap justify-end gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => setThemeImportText(defaultThemeImportText())}>
                {copy.settingsPage.example}
              </button>
              <button type="button" className={buttonClass("primary")} onClick={() => void importTheme()} disabled={themeLoading}>
                {copy.settingsPage.importTheme}
              </button>
            </div>
          </div>
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}

function themeTargetLabel(target: string, copy: Translations): string {
  if (target === "public") return copy.settingsPage.publicAction;
  if (target === "dashboard") return copy.settingsPage.dashboardAction;
  if (target === "both") return copy.settingsPage.bothAction;
  return target;
}

function themeSupportsTarget(theme: ThemeDefinition, target: "public" | "dashboard"): boolean {
  return theme.target === "both" || theme.target === target;
}

function themeSwatches(theme: ThemeDefinition): Array<[string, string]> {
  return Object.entries(theme.variables ?? {})
    .filter(([, value]) => /^(#|rgb\(|rgba\(|hsl\(|hsla\()/i.test(value.trim()))
    .slice(0, 6);
}

function defaultThemeImportText(): string {
  return JSON.stringify(
    {
      theme: {
        id: "custom-green",
        name: "Custom Green",
        description: "Imported dashboard and public status theme",
        target: "both",
        variables: {
          "--bg-page": "#f7f7f2",
          "--bg-card": "#ffffff",
          "--text-main": "#111827",
          "--text-muted": "#4b5563",
          "--border-color": "#111827",
          "--accent-color": "#16a34a",
          "--accent-bg": "#dcfce7",
          "--btn-bg": "#111827",
          "--btn-text": "#ffffff",
          "--dot-color": "#d1d5db",
        },
      },
    },
    null,
    2,
  );
}
