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
import {
  apiClient,
  type GeoIpLookupResponse,
  type GeoIpMmdbStatus,
  type SystemSettingsResponse,
} from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { SectionFeedback, splitSettingList } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

interface NotificationGroup {
  id: string;
  name: string;
}

// The DDNS resolver lives in the same settings payload as the GeoIP provider,
// so both cards share one save call and one settings snapshot. Only the GeoIP
// card defers its load; the DDNS card below renders from the same state.
export function GeoIpCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [systemSettings, setSystemSettings] = useState<SystemSettingsResponse | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [geoIp, setGeoIp] = useState({ ip: "1.1.1.1", provider: "mmdb", token: "" });
  const [geoIpResult, setGeoIpResult] = useState<GeoIpLookupResponse | null>(null);
  const [geoIpMmdbStatus, setGeoIpMmdbStatus] = useState<GeoIpMmdbStatus | null>(null);
  const [geoIpMmdbUrl, setGeoIpMmdbUrl] = useState("");
  const [geoIpMmdbPath, setGeoIpMmdbPath] = useState("");
  const [geoIpMmdbFile, setGeoIpMmdbFile] = useState<File | null>(null);
  const [geoIpIpChange, setGeoIpIpChange] = useState({ enabled: true, notification_group_id: "", server_ids: "", severity: "info" });
  const [ddnsResolverUrl, setDdnsResolverUrl] = useState("");
  const [notificationGroups, setNotificationGroups] = useState<NotificationGroup[]>([]);
  const [geoIpLoading, setGeoIpLoading] = useState(false);
  const [geoIpStatusLoading, setGeoIpStatusLoading] = useState(true);
  const cardRef = useLoadOnVisible(() => {
    void Promise.all([loadSystemSettings(), loadGeoIpStatus(), loadNotificationGroups()]);
  });

  async function loadSystemSettings() {
    setSettingsLoading(true);
    const response = await apiClient.getSettings();
    setSettingsLoading(false);
    if (response.success && response.data) {
      const settings = response.data;
      setSystemSettings(settings);
      setGeoIp((current) => ({ ...current, provider: settings.geoip_provider || current.provider }));
      setGeoIpIpChange({
        enabled: settings.geoip_ip_change_enabled,
        notification_group_id: settings.geoip_ip_change_notification_group_id || "",
        server_ids: settings.geoip_ip_change_server_ids.join(", "),
        severity: settings.geoip_ip_change_severity || "info",
      });
      setDdnsResolverUrl(settings.ddns_resolver_url || "");
    } else {
      setError(responseError(response));
    }
  }

  async function loadNotificationGroups() {
    const response = await apiClient.listNotificationGroups(200, 0);
    if (response.success && response.data) {
      setNotificationGroups((response.data.groups as NotificationGroup[]) ?? []);
    }
  }

  async function loadGeoIpStatus() {
    setGeoIpStatusLoading(true);
    const response = await apiClient.getGeoIpStatus();
    setGeoIpStatusLoading(false);
    if (response.success && response.data) {
      setGeoIpMmdbStatus(response.data);
    }
  }

  async function testGeoIp() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setGeoIpLoading(true);
    const response = await apiClient.testGeoIp(geoIp.ip.trim(), geoIp.provider, geoIp.token, totpCode);
    setGeoIpLoading(false);
    if (response.success && response.data) {
      setGeoIpResult(response.data);
      setNotice(copy.settingsPage.geoIpQueryDone);
    } else {
      setError(responseError(response));
    }
  }

  async function updateGeoIpDatabase() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setGeoIpLoading(true);
    const response = await apiClient.updateGeoIpDatabase({
      source_url: geoIpMmdbUrl.trim() || undefined,
      source_path: geoIpMmdbPath.trim() || undefined,
    }, totpCode);
    setGeoIpLoading(false);
    if (response.success && response.data) {
      if (response.data.status) setGeoIpMmdbStatus(response.data.status);
      const message = response.data.message || copy.settingsPage.geoIpUpdateDone;
      setNotice(message);
    } else {
      setError(responseError(response));
    }
  }

  async function uploadGeoIpDatabase() {
    if (!geoIpMmdbFile) {
      setError(copy.settingsPage.selectMmdbFile);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setGeoIpLoading(true);
    const response = await apiClient.uploadGeoIpDatabase(geoIpMmdbFile, totpCode);
    setGeoIpLoading(false);
    if (response.success && response.data) {
      if (response.data.status) setGeoIpMmdbStatus(response.data.status);
      setGeoIpMmdbFile(null);
      setNotice(response.data.message || copy.settingsPage.mmdbUploadDone);
    } else {
      setError(responseError(response));
    }
  }

  async function saveGeoIpSettings() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({
      geoip_provider: geoIp.provider,
      geoip_ipinfo_token: geoIp.token,
      geoip_ip_change_enabled: geoIpIpChange.enabled,
      geoip_ip_change_notification_group_id: geoIpIpChange.notification_group_id || null,
      geoip_ip_change_server_ids: splitSettingList(geoIpIpChange.server_ids),
      geoip_ip_change_severity: geoIpIpChange.severity,
      ddns_resolver_url: ddnsResolverUrl.trim(),
    }, totpCode);
    setSettingsLoading(false);
    if (response.success && response.data) {
      const settings = response.data;
      setSystemSettings(settings);
      setGeoIp((current) => ({ ...current, provider: settings.geoip_provider || current.provider, token: "" }));
      setGeoIpIpChange({
        enabled: settings.geoip_ip_change_enabled,
        notification_group_id: settings.geoip_ip_change_notification_group_id || "",
        server_ids: settings.geoip_ip_change_server_ids.join(", "),
        severity: settings.geoip_ip_change_severity || "info",
      });
      setDdnsResolverUrl(settings.ddns_resolver_url || "");
      setNotice(copy.settingsPage.geoIpProviderSaved);
    } else {
      setError(responseError(response));
    }
  }

  return (
    <>
      <div ref={cardRef} className="mb-6">
        <BrutalCard>
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.geoIp}</h2>
              <div className="mt-2 flex flex-wrap gap-2">
                <StatusBadge tone="blue">{systemSettings ? copy.settingsPage.defaultProvider.replace("{provider}", String(systemSettings.geoip_provider || "empty")) : copy.common.notAvailable}</StatusBadge>
                <StatusBadge tone={systemSettings?.geoip_ipinfo_token_configured ? "green" : "gray"}>
                  {systemSettings ? (systemSettings.geoip_ipinfo_token_configured ? copy.settingsPage.ipinfoTokenConfigured : copy.settingsPage.ipinfoTokenNotConfigured) : copy.common.notAvailable}
                </StatusBadge>
                <StatusBadge tone={geoIpMmdbStatus?.configured ? "green" : "gray"}>
                  {geoIpMmdbStatus
                    ? (geoIpMmdbStatus.configured ? copy.settingsPage.mmdbConfigured.replace("{type}", String(geoIpMmdbStatus.database_type || "ready")) : copy.settingsPage.mmdbNotConfigured)
                    : geoIpStatusLoading ? copy.settingsPage.loading : copy.common.notAvailable}
                </StatusBadge>
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
                {settingsLoading ? (systemSettings ? copy.settingsPage.saving : copy.settingsPage.loading) : copy.settingsPage.saveDefault}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void updateGeoIpDatabase()} disabled={geoIpLoading}>
                {copy.settingsPage.mmdbUpdate}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void loadGeoIpStatus()} disabled={geoIpLoading}>
                {copy.settingsPage.mmdbStatus}
              </button>
            </div>
          </div>
          <SectionFeedback error={error} notice={notice} />
          <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
            <div className="grid content-start gap-3">
              <Field label="IP">
                <input className={inputClass} value={geoIp.ip} onChange={(event) => setGeoIp({ ...geoIp, ip: event.target.value })} />
              </Field>
              <Field label="Provider">
                <select className={inputClass} value={geoIp.provider} onChange={(event) => setGeoIp({ ...geoIp, provider: event.target.value })}>
                  <option value="empty">empty</option>
                  <option value="geojs">geojs</option>
                  <option value="ip-api">ip-api</option>
                  <option value="ipinfo">ipinfo</option>
                  <option value="mmdb">mmdb</option>
                </select>
              </Field>
              <Field label="ipinfo token">
                <input className={inputClass} value={geoIp.token} onChange={(event) => setGeoIp({ ...geoIp, token: event.target.value })} placeholder="optional" />
              </Field>
              <div className="grid gap-3 border-t-2 border-black pt-3 sm:grid-cols-2">
                <label className="flex items-center gap-2 text-sm font-black">
                  <input type="checkbox" checked={geoIpIpChange.enabled} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, enabled: event.target.checked }))} />
                  {copy.settingsPage.ipChangeNotification}
                </label>
                <Field label={copy.settingsPage.notificationSeverity}>
                  <select className={inputClass} value={geoIpIpChange.severity} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, severity: event.target.value }))}>
                    <option value="info">info</option>
                    <option value="warning">warning</option>
                    <option value="error">error</option>
                    <option value="critical">critical</option>
                  </select>
                </Field>
                <Field label={copy.settingsPage.notificationGroup}>
                  <select className={inputClass} value={geoIpIpChange.notification_group_id} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, notification_group_id: event.target.value }))}>
                    <option value="">{copy.settingsPage.allNotificationChannels}</option>
                    {notificationGroups.map((group) => (
                      <option key={group.id} value={group.id}>{group.name}</option>
                    ))}
                  </select>
                </Field>
                <Field label={copy.settingsPage.serverScope}>
                  <input className={inputClass} value={geoIpIpChange.server_ids} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, server_ids: event.target.value }))} placeholder="server id, server id" />
                </Field>
              </div>
              <button type="button" className={buttonClass("primary")} onClick={() => void testGeoIp()} disabled={geoIpLoading}>
                {geoIpLoading ? copy.settingsPage.querying : copy.settingsPage.testGeoIp}
              </button>
              <div className="border-t-2 border-black pt-3">
                <div className="grid gap-3">
                  <Field label={copy.settingsPage.mmdbUrl}>
                    <input className={inputClass} value={geoIpMmdbUrl} onChange={(event) => setGeoIpMmdbUrl(event.target.value)} placeholder="https://example.com/GeoLite2-City.mmdb" />
                  </Field>
                  <Field label={copy.settingsPage.mmdbPath}>
                    <input className={inputClass} value={geoIpMmdbPath} onChange={(event) => setGeoIpMmdbPath(event.target.value)} placeholder="/srv/geoip/GeoLite2-City.mmdb" />
                  </Field>
                  <Field label={copy.settingsPage.mmdbFile}>
                    <input className={inputClass} type="file" accept=".mmdb" onChange={(event) => setGeoIpMmdbFile(event.target.files?.[0] ?? null)} />
                  </Field>
                  {geoIpMmdbFile ? <div className="break-all text-xs font-black text-[var(--text-muted)]">{geoIpMmdbFile.name}</div> : null}
                  <button type="button" className={buttonClass("secondary")} onClick={() => void uploadGeoIpDatabase()} disabled={geoIpLoading || !geoIpMmdbFile}>
                    {copy.settingsPage.uploadMmdb}
                  </button>
                </div>
              </div>
            </div>
            <div className="min-w-0 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
              {geoIpResult ? (
                <div className="grid gap-3">
                  <div className="flex flex-wrap gap-2">
                    <StatusBadge tone="blue">{geoIpResult.provider}</StatusBadge>
                    <StatusBadge tone="gray">{geoIpResult.ip}</StatusBadge>
                  </div>
                  <div className="grid gap-2 text-sm font-bold md:grid-cols-2">
                    <div>{copy.settingsPage.countryLabel.replace("{value}", String(geoIpResult.country || "N/A"))}</div>
                    <div>{copy.settingsPage.regionLabel.replace("{value}", String(geoIpResult.region || "N/A"))}</div>
                    <div>{copy.settingsPage.cityLabel.replace("{value}", String(geoIpResult.city || "N/A"))}</div>
                    <div>{copy.settingsPage.timezoneLabel.replace("{value}", String(geoIpResult.timezone || "N/A"))}</div>
                    <div>{copy.settingsPage.latitudeLabel.replace("{value}", String(geoIpResult.latitude ?? "N/A"))}</div>
                    <div>{copy.settingsPage.longitudeLabel.replace("{value}", String(geoIpResult.longitude ?? "N/A"))}</div>
                    <div className="break-words md:col-span-2">{copy.settingsPage.ispLabel.replace("{value}", String(geoIpResult.isp || "N/A"))}</div>
                    <div className="break-words md:col-span-2">{copy.settingsPage.organizationLabel.replace("{value}", String(geoIpResult.organization || "N/A"))}</div>
                  </div>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all bg-black p-3 font-mono text-xs text-green-300">
                    {JSON.stringify(geoIpResult.raw ?? geoIpResult, null, 2)}
                  </pre>
                </div>
              ) : (
                <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noGeoIpQuery}</p>
              )}
              {geoIpMmdbStatus ? (
                <div className="mt-4 border-t-2 border-black pt-3 text-sm font-bold">
                  <div className="grid gap-2 md:grid-cols-2">
                    <div>{copy.settingsPage.mmdbStatusLabel.replace("{value}", geoIpMmdbStatus.configured ? copy.settingsPage.configured : copy.settingsPage.notConfigured)}</div>
                    <div>{copy.settingsPage.typeLabel.replace("{value}", String(geoIpMmdbStatus.database_type || "N/A"))}</div>
                    <div>{copy.settingsPage.buildLabel.replace("{date}", String(formatDate(geoIpMmdbStatus.build_at)))}</div>
                    <div>{copy.settingsPage.modifiedLabel.replace("{date}", String(formatDate(geoIpMmdbStatus.modified_at)))}</div>
                    <div>{copy.settingsPage.sizeLabel.replace("{value}", String(geoIpMmdbStatus.size_bytes ?? "N/A"))}</div>
                    <div>{copy.settingsPage.ipVersionLabel.replace("{value}", String(geoIpMmdbStatus.ip_version || "N/A"))}</div>
                    <div className="break-all md:col-span-2">{copy.settingsPage.pathLabel.replace("{value}", String(geoIpMmdbStatus.path))}</div>
                    {geoIpMmdbStatus.error ? <div className="break-all text-[var(--btn-bg)] md:col-span-2">{copy.settingsPage.errorLabel.replace("{value}", String(geoIpMmdbStatus.error))}</div> : null}
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </BrutalCard>
      </div>

      <BrutalCard className="mb-6">
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.ddns}</h2>
            <div className="mt-2 flex flex-wrap gap-2">
              <StatusBadge tone={ddnsResolverUrl.trim() ? "green" : "gray"}>
                {systemSettings ? (ddnsResolverUrl.trim() ? copy.settingsPage.resolverConfigured : copy.settingsPage.resolverSystem) : copy.common.notAvailable}
              </StatusBadge>
            </div>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
            {settingsLoading ? (systemSettings ? copy.settingsPage.saving : copy.settingsPage.loading) : copy.settingsPage.saveDdns}
          </button>
        </div>
        <Field label={copy.settingsPage.dohResolverUrl}>
          <input className={inputClass} value={ddnsResolverUrl} onChange={(event) => setDdnsResolverUrl(event.target.value)} placeholder="https://dns.google/resolve" />
        </Field>
      </BrutalCard>
      {dialogs.element}
    </>
  );
}
