"use client";

import { FormEvent, useEffect, useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  responseError,
  textareaClass,
} from "@/app/components/M7Primitives";
import {
  apiClient,
  getApiBaseUrl,
  type CloudflaredStatusResponse,
  type GeoIpLookupResponse,
  type GeoIpMmdbStatus,
  type ImportThemeRequest,
  type MaintenanceRestoreResponse,
  type MaintenanceStatusResponse,
  type OAuthAccount,
  type OAuthProvider,
  type PatInfo,
  type SystemSettingsResponse,
  type ThemeDefinition,
  type TotpSetupResponse,
  type TotpStatusResponse,
  type TsdbCompactResponse,
  type TsdbRetentionResponse,
} from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";

const DEFAULT_AGENT_VERSION = "v0.1";
const GITHUB_RELEASES_API = "https://api.github.com/repos/lbyxiaolizi/XLStatus/releases?per_page=20";

interface UserAccount {
  id: string;
  username: string;
  role: string;
  created_at?: string;
  updated_at?: string;
}

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

interface WafBan {
  id: string;
  ip: string;
  reason: string;
  failed_count: number;
  banned_until?: string;
  created_at?: string;
  updated_at?: string;
}

interface NotificationGroup {
  id: string;
  name: string;
}

export default function SettingsPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState("server:read service:read task:* nat:* ddns:*");
  const [patExpiresAt, setPatExpiresAt] = useState(() => defaultPatExpiresAt());
  const [patServerIds, setPatServerIds] = useState("");
  const [tokens, setTokens] = useState<PatInfo[]>([]);
  const [createdToken, setCreatedToken] = useState("");
  const [users, setUsers] = useState<UserAccount[]>([]);
  const [usersLoading, setUsersLoading] = useState(false);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(false);
  const [wafBans, setWafBans] = useState<WafBan[]>([]);
  const [wafLoading, setWafLoading] = useState(false);
  const [wafBanDraft, setWafBanDraft] = useState({ ips: "", reason: "manual WAF ban", minutes: "30" });
  const [maintenanceStatus, setMaintenanceStatus] = useState<MaintenanceStatusResponse | null>(null);
  const [maintenanceLoading, setMaintenanceLoading] = useState(false);
  const [restoreFile, setRestoreFile] = useState<File | null>(null);
  const [restoreResult, setRestoreResult] = useState<MaintenanceRestoreResponse | null>(null);
  const [tsdbCompactResult, setTsdbCompactResult] = useState<TsdbCompactResponse | null>(null);
  const [tsdbRetentionResult, setTsdbRetentionResult] = useState<TsdbRetentionResponse | null>(null);
  const [tsdbRetentionDraft, setTsdbRetentionDraft] = useState("30");
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [totpSetup, setTotpSetup] = useState<TotpSetupResponse | null>(null);
  const [totpCode, setTotpCode] = useState("");
  const [totpLoading, setTotpLoading] = useState(false);
  const [oauthProviders, setOauthProviders] = useState<OAuthProvider[]>([]);
  const [oauthBindings, setOauthBindings] = useState<OAuthAccount[]>([]);
  const [geoIp, setGeoIp] = useState({ ip: "1.1.1.1", provider: "mmdb", token: "" });
  const [geoIpResult, setGeoIpResult] = useState<GeoIpLookupResponse | null>(null);
  const [geoIpMmdbStatus, setGeoIpMmdbStatus] = useState<GeoIpMmdbStatus | null>(null);
  const [geoIpMmdbUrl, setGeoIpMmdbUrl] = useState("");
  const [geoIpMmdbPath, setGeoIpMmdbPath] = useState("");
  const [geoIpMmdbFile, setGeoIpMmdbFile] = useState<File | null>(null);
  const [geoIpIpChange, setGeoIpIpChange] = useState({ enabled: true, notification_group_id: "", server_ids: "", severity: "info" });
  const [ddnsResolverUrl, setDdnsResolverUrl] = useState("");
  const [cloudflaredStatus, setCloudflaredStatus] = useState<CloudflaredStatusResponse | null>(null);
  const [cloudflaredToken, setCloudflaredToken] = useState("");
  const [cloudflaredLoading, setCloudflaredLoading] = useState(false);
  const [geoIpLoading, setGeoIpLoading] = useState(false);
  const [systemSettings, setSystemSettings] = useState<SystemSettingsResponse | null>(null);
  const [publicBranding, setPublicBranding] = useState({
    siteName: "XLStatus",
    logoUrl: "",
    faviconUrl: "",
    themeColor: "",
    backgroundUrl: "",
    customHead: "",
    customBody: "",
  });
  const [themes, setThemes] = useState<ThemeDefinition[]>([]);
  const [selectedPublicThemeId, setSelectedPublicThemeId] = useState("");
  const [selectedDashboardThemeId, setSelectedDashboardThemeId] = useState("");
  const [themeImportText, setThemeImportText] = useState(() => defaultThemeImportText());
  const [themeLoading, setThemeLoading] = useState(false);
  const [notificationGroups, setNotificationGroups] = useState<NotificationGroup[]>([]);
  const [settingsLoading, setSettingsLoading] = useState(false);
  const [newUser, setNewUser] = useState({ username: "", password: "", role: "member" });
  const [passwordEdits, setPasswordEdits] = useState<Record<string, string>>({});
  const [agentServerUrl, setAgentServerUrl] = useState(() => getApiBaseUrl());
  const [agentGrpcUrl, setAgentGrpcUrl] = useState(() => defaultGrpcUrl(getApiBaseUrl()));
  const [agentName, setAgentName] = useState("$(hostname)");
  const [agentVersion, setAgentVersion] = useState(DEFAULT_AGENT_VERSION);
  const [agentUseLatestVersion, setAgentUseLatestVersion] = useState(true);
  const [agentLatestVersionStatus, setAgentLatestVersionStatus] = useState(copy.settingsPage.agentVersionWaiting);
  const [agentLatestVersionLoading, setAgentLatestVersionLoading] = useState(false);
  const [enrollmentHours, setEnrollmentHours] = useState("1");
  const [enrollmentToken, setEnrollmentToken] = useState("");
  const [enrollmentExpiresAt, setEnrollmentExpiresAt] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const installScriptUrl = apiClient.getAgentInstallScriptUrl({
    server_url: agentServerUrl,
    grpc_server: agentGrpcUrl,
    enrollment_token: enrollmentToken || "xle_...",
    agent_name: agentName,
    version: agentVersion,
  });
  const githubScriptUrl = `https://github.com/lbyxiaolizi/XLStatus/releases/download/${encodeURIComponent(agentVersion)}/install-agent.sh`;
  const agentInstallCommand = buildAgentInstallCommand({
    installScriptUrl,
  });

  useEffect(() => {
    void loadTokens();
    void loadUsers();
    void loadSessions();
    void loadWafBans();
    void loadMaintenanceStatus();
    void loadTotpStatus();
    void loadOAuthProviders();
    void loadOAuthBindings();
    void loadSystemSettings();
    void loadGeoIpStatus();
    void loadNotificationGroups();
    void loadCloudflaredStatus();
    void loadThemes();
  }, []);

  useEffect(() => {
    if (agentUseLatestVersion) {
      void refreshLatestAgentVersion();
    }
    // Refresh only when the auto-latest switch changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentUseLatestVersion]);

  async function loadTokens() {
    const response = await apiClient.listPats();
    if (response.success && response.data) {
      setTokens(response.data);
    } else {
      setError(responseError(response));
    }
  }

  async function loadUsers() {
    setUsersLoading(true);
    const response = await apiClient.listUsers(200, 0);
    setUsersLoading(false);
    if (response.success && response.data) {
      setUsers(((response.data.users as UserAccount[]) ?? []).filter((user) => user.id));
    } else {
      setError(responseError(response));
    }
  }

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

  async function loadMaintenanceStatus() {
    setMaintenanceLoading(true);
    const response = await apiClient.getMaintenanceStatus();
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setMaintenanceStatus(response.data);
      if (response.data.tsdb_retention_days) {
        setTsdbRetentionDraft(String(response.data.tsdb_retention_days));
      }
    } else {
      setError(responseError(response));
    }
  }

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

  async function loadTotpStatus() {
    setTotpLoading(true);
    const response = await apiClient.getTotpStatus();
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

  async function loadSystemSettings() {
    setSettingsLoading(true);
    const response = await apiClient.getSettings();
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

  async function loadThemes() {
    setThemeLoading(true);
    const response = await apiClient.listThemes();
    setThemeLoading(false);
    if (response.success && response.data) {
      setThemes(response.data.themes ?? []);
      setSelectedPublicThemeId(response.data.selected_public_theme_id || "");
      setSelectedDashboardThemeId(response.data.selected_dashboard_theme_id || "");
    } else {
      setError(responseError(response));
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
    setThemeLoading(true);
    const response = await apiClient.importTheme(candidate as ImportThemeRequest["theme"], totpCode);
    setThemeLoading(false);
    if (response.success && response.data) {
      setNotice(copy.settingsPage.themeImported.replace("{name}", String(response.data.name)));
      await loadThemes();
    } else {
      setError(responseError(response));
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
    setThemeLoading(true);
    const response = await apiClient.selectTheme(theme.id, target, totpCode);
    setThemeLoading(false);
    if (response.success && response.data) {
      setThemes(response.data.themes ?? []);
      setSelectedPublicThemeId(response.data.selected_public_theme_id || "");
      setSelectedDashboardThemeId(response.data.selected_dashboard_theme_id || "");
      setNotice(copy.settingsPage.themeApplied.replace("{name}", String(theme.name)));
    } else {
      setError(responseError(response));
    }
  }

  async function deleteTheme(theme: ThemeDefinition) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.deleteThemeConfirm.replace("{name}", String(theme.name)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setThemeLoading(true);
    const response = await apiClient.deleteTheme(theme.id, totpCode);
    setThemeLoading(false);
    if (response.success) {
      setNotice(copy.settingsPage.themeDeleted.replace("{name}", String(theme.name)));
      await loadThemes();
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
    const response = await apiClient.getGeoIpStatus();
    if (response.success && response.data) {
      setGeoIpMmdbStatus(response.data);
    }
  }

  async function sensitiveTotpCode(): Promise<string | undefined | null> {
    let enabled = totpStatus?.enabled;
    if (totpStatus === null) {
      const response = await apiClient.getTotpStatus();
      if (!response.success || !response.data) {
        setError(responseError(response));
        return null;
      }
      setTotpStatus(response.data);
      enabled = response.data.enabled;
    }
    if (!enabled) return undefined;
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.settingsPage.totpEnterSixDigits);
      return null;
    }
    return trimmed;
  }

  async function createPanelUser(event: FormEvent) {
    event.preventDefault();
    if (newUser.password.length < 8) {
      setError(copy.settingsPage.userPasswordTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createUser({
      username: newUser.username.trim(),
      password: newUser.password,
      role: newUser.role,
    }, totpCode);
    if (response.success) {
      setNotice(copy.settingsPage.userCreated);
      setNewUser({ username: "", password: "", role: "member" });
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function updateUserRole(user: UserAccount, role: string) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(user.id, { role }, totpCode);
    if (response.success) {
      setNotice(copy.settingsPage.userRoleUpdated.replace("{username}", String(user.username)));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function resetUserPassword(user: UserAccount) {
    const password = passwordEdits[user.id] ?? "";
    if (password.length < 8) {
      setError(copy.settingsPage.userPasswordResetTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(user.id, { password }, totpCode);
    if (response.success) {
      setNotice(copy.settingsPage.userPasswordReset.replace("{username}", String(user.username)));
      setPasswordEdits((current) => ({ ...current, [user.id]: "" }));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelUser(user: UserAccount) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.deleteUserConfirm.replace("{username}", String(user.username)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteUser(user.id, totpCode);
    if (response.success) {
      setNotice(copy.settingsPage.userDeleted.replace("{username}", String(user.username)));
      await loadUsers();
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
      setNotice(copy.settingsPage.sessionRevoked);
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

  async function createPanelWafBan(ipsOverride?: string[]) {
    const ips = ipsOverride ?? [wafBanDraft.ips];
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
      setNotice(copy.settingsPage.wafBansCreated.replace("{count}", String(count)));
      if (!ipsOverride) {
        setWafBanDraft((current) => ({ ...current, ips: "" }));
      }
      await loadWafBans();
    } else {
      setError(responseError(response));
    }
  }

  async function banSessionIp(session: SessionInfo) {
    const ip = session.ip?.trim();
    if (!ip) {
      setError(copy.settingsPage.sessionNoBannableIp);
      return;
    }
    if (!(await dialogs.confirm({ message: copy.settingsPage.banIpConfirm.replace("{ip}", String(ip)), danger: true }))) return;
    await createPanelWafBan([ip]);
  }

  async function deletePanelWafBan(ban: WafBan) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.unbanConfirm.replace("{ip}", String(ban.ip)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteWafBan(ban.id, totpCode);
    if (response.success) {
      setNotice(copy.settingsPage.wafBanRemoved.replace("{ip}", String(ban.ip)));
      await loadWafBans();
    } else {
      setError(responseError(response));
    }
  }

  async function runSqliteVacuum() {
    if (!(await dialogs.confirm({ message: copy.settingsPage.vacuumConfirm, danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.vacuumSqlite(totpCode);
    setMaintenanceLoading(false);
    if (response.success) {
      setNotice(copy.settingsPage.vacuumDone);
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function runTsdbCompact() {
    if (!(await dialogs.confirm({ message: copy.settingsPage.tsdbCompactConfirm, danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.compactTsdb(totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbCompactResult(response.data);
      setNotice(copy.settingsPage.tsdbCompactDone.replace("{count}", String(response.data.removed_samples)));
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function updateTsdbRetention() {
    const parsed = Number.parseInt(tsdbRetentionDraft.trim(), 10);
    if (!Number.isFinite(parsed) || parsed < 1 || parsed > 3650) {
      setError(copy.settingsPage.tsdbRetentionRange);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.updateTsdbRetention(parsed, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbRetentionResult(response.data);
      setNotice(copy.settingsPage.tsdbRetentionUpdated.replace("{days}", String(response.data.retention_days)));
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function downloadMaintenanceExport(kind: "backup" | "archive") {
    const supported = kind === "backup" ? maintenanceStatus?.backup_supported : maintenanceStatus?.archive_supported;
    if (!supported) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = kind === "backup"
      ? await apiClient.downloadMaintenanceBackup(totpCode)
      : await apiClient.downloadMaintenanceArchive(totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      saveBlob(response.data.blob, response.data.filename);
      setNotice(kind === "backup" ? copy.settingsPage.backupDownloadStarted : copy.settingsPage.archiveDownloadStarted);
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

  async function restoreSqliteBackup(dryRun: boolean) {
    if (!restoreFile) {
      setError(copy.settingsPage.selectSqliteBackup);
      return;
    }
    if (!maintenanceStatus?.restore_supported) {
      setError(copy.settingsPage.restoreUnsupported);
      return;
    }
    if (!dryRun && !(await dialogs.confirm({ message: copy.settingsPage.restoreConfirm, danger: true }))) return;
    const totpCode = dryRun ? undefined : await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.restoreBackup(restoreFile, dryRun, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setRestoreResult(response.data);
      setNotice(dryRun ? copy.settingsPage.backupVerified : copy.settingsPage.backupRestored);
      await loadMaintenanceStatus();
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
      setNotice(copy.settingsPage.totpSecretGenerated);
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
      setNotice(copy.settingsPage.totpEnabled);
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
      setNotice(copy.settingsPage.totpDisabled);
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
      setNotice(copy.settingsPage.oauthUnbound.replace("{name}", String(displayName)));
      await loadOAuthBindings();
    } else {
      setError(responseError(response));
    }
  }

  async function bindOAuthProvider(providerId: string) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.startOAuthBind(providerId, "/settings", totpCode);
    if (response.success && response.data?.authorization_url) {
      window.location.href = response.data.authorization_url;
    } else {
      setError(responseError(response));
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

  async function createToken(event: FormEvent) {
    event.preventDefault();
    const serverIds = splitSettingList(patServerIds);
    const expiresAt = new Date(patExpiresAt);
    if (Number.isNaN(expiresAt.getTime())) {
      setError(copy.settingsPage.enterValidPatExpiry);
      return;
    }
    if (serverIds.length === 0 && !(await dialogs.confirm({ message: copy.settingsPage.globalPatConfirm }))) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createPat({
      name,
      scopes: scopes.split(/\s+/).filter(Boolean),
      expires_at: expiresAt.toISOString(),
      ...(serverIds.length > 0 ? { server_ids: serverIds } : {}),
    }, totpCode);
    if (response.success && response.data) {
      setCreatedToken(response.data.token);
      setNotice(copy.settingsPage.patCreated.replace("{expiresAt}", String(formatDate(response.data.expires_at))));
      setName("");
      setPatExpiresAt(defaultPatExpiresAt());
      setPatServerIds("");
      await loadTokens();
    } else {
      setError(responseError(response));
    }
  }

  async function createEnrollmentToken() {
    const expiresInHours = Number.parseInt(enrollmentHours, 10);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createEnrollmentToken(
      Number.isFinite(expiresInHours) && expiresInHours > 0 ? expiresInHours : 1,
      totpCode,
    );
    if (response.success && response.data) {
      setEnrollmentToken(response.data.token);
      setEnrollmentExpiresAt(response.data.expires_at);
      setNotice(copy.settingsPage.enrollmentTokenCreated);
    } else {
      setError(responseError(response));
    }
  }

  async function refreshLatestAgentVersion() {
    setAgentLatestVersionLoading(true);
    try {
      const response = await fetch(GITHUB_RELEASES_API, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!response.ok) {
        throw new Error(copy.settingsPage.githubHttpError.replace("{status}", String(response.status)));
      }
      const data = (await response.json()) as Array<{ tag_name?: string; draft?: boolean }>;
      const release = data.find((item) => !item.draft && item.tag_name?.trim());
      const tagName = release?.tag_name?.trim();
      if (!tagName) {
        throw new Error(copy.settingsPage.githubNoTagName);
      }
      setAgentVersion(tagName);
      setAgentLatestVersionStatus(copy.settingsPage.agentLatestFetched.replace("{tag}", String(tagName)));
    } catch (err) {
      const message = err instanceof Error ? err.message : copy.settingsPage.unknownError;
      setAgentLatestVersionStatus(
        copy.settingsPage.agentLatestFetchFailed
          .replace("{version}", String(agentVersion || DEFAULT_AGENT_VERSION))
          .replace("{message}", String(message)),
      );
      if (!agentVersion.trim()) {
        setAgentVersion(DEFAULT_AGENT_VERSION);
      }
    } finally {
      setAgentLatestVersionLoading(false);
    }
  }

  async function copyAgentCommand() {
    try {
      await navigator.clipboard.writeText(agentInstallCommand);
      setNotice(copy.settingsPage.agentCommandCopied);
    } catch {
      setError(copy.settingsPage.clipboardCommandFailed);
    }
  }

  async function copyTotpUri() {
    if (!totpSetup?.otpauth_uri) return;
    try {
      await navigator.clipboard.writeText(totpSetup.otpauth_uri);
      setNotice(copy.settingsPage.totpUriCopied);
    } catch {
      setError(copy.settingsPage.clipboardUriFailed);
    }
  }

  const oauthBindingMap = new Map(oauthBindings.map((binding) => [binding.provider, binding]));
  const oauthProviderIds = new Set(oauthProviders.map((provider) => provider.id));
  const staleOAuthBindings = oauthBindings.filter((binding) => !oauthProviderIds.has(binding.provider));

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.settingsPage.eyebrow}
          title={copy.settingsPage.title}
          detail={copy.settingsPage.detail}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <BrutalCard accent className="mb-6">
          <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
            <div>
              <h2 className="mb-4 text-xl font-black uppercase">{copy.settingsPage.agentInstall}</h2>
              <div className="grid gap-4 md:grid-cols-2">
                <Field label={copy.settingsPage.serverUrl}>
                  <input className={inputClass} value={agentServerUrl} onChange={(e) => setAgentServerUrl(e.target.value)} />
                </Field>
                <Field label={copy.settingsPage.grpcUrl}>
                  <input className={inputClass} value={agentGrpcUrl} onChange={(e) => setAgentGrpcUrl(e.target.value)} />
                </Field>
                <Field label={copy.settingsPage.releaseVersion}>
                  <input
                    className={inputClass}
                    value={agentVersion}
                    onChange={(e) => setAgentVersion(e.target.value)}
                    disabled={agentUseLatestVersion}
                  />
                </Field>
                <Field label={copy.settingsPage.agentName}>
                  <input className={inputClass} value={agentName} onChange={(e) => setAgentName(e.target.value)} />
                </Field>
                <Field label={copy.settingsPage.enrollmentHours}>
                  <input className={inputClass} type="number" min="1" max="24" value={enrollmentHours} onChange={(e) => setEnrollmentHours(e.target.value)} />
                </Field>
              </div>
              <div className="mt-4 flex flex-wrap items-center gap-2">
                <label className="inline-flex cursor-pointer items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
                  <input
                    type="checkbox"
                    checked={agentUseLatestVersion}
                    onChange={(event) => setAgentUseLatestVersion(event.target.checked)}
                  />
                  {copy.settingsPage.fetchLatestFromGithub}
                </label>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void refreshLatestAgentVersion()}
                  disabled={agentLatestVersionLoading}
                >
                  {agentLatestVersionLoading ? copy.settingsPage.fetching : copy.settingsPage.refreshVersion}
                </button>
              </div>
              <p className="mt-2 text-xs font-bold text-[var(--text-muted)]">
                {agentLatestVersionStatus}
              </p>
              <div className="mt-4 flex flex-wrap gap-2">
                <button type="button" className={buttonClass("primary")} onClick={createEnrollmentToken}>
                  {copy.settingsPage.generateEnrollmentToken}
                </button>
                <button type="button" className={buttonClass("secondary")} onClick={copyAgentCommand} disabled={!enrollmentToken}>
                  {copy.settingsPage.copyInstallCommand}
                </button>
                <a className={buttonClass("secondary")} href={installScriptUrl} target="_blank" rel="noreferrer">
                  {copy.settingsPage.openParameterizedLink}
                </a>
              </div>
              {enrollmentExpiresAt ? (
                <p className="mt-3 text-xs font-black uppercase text-[var(--text-muted)]">
                  {copy.settingsPage.tokenExpiresAt.replace("{value}", String(enrollmentExpiresAt))}
                </p>
              ) : null}
              <p className="mt-3 break-all text-xs font-bold text-[var(--text-muted)]">
                {copy.settingsPage.githubScriptSource.replace("{url}", String(githubScriptUrl))}
              </p>
            </div>
            <div>
              <p className="mb-2 text-xs font-black uppercase text-[var(--text-muted)]">{copy.settingsPage.oneLineInstallCommand}</p>
              <pre className="min-h-40 overflow-auto whitespace-pre-wrap break-all border-2 border-black bg-black p-3 font-mono text-xs text-green-300 shadow-[var(--shadow-brutal-sm)]">
                {agentInstallCommand}
              </pre>
            </div>
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.themeTemplates}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadThemes()} disabled={themeLoading}>
              {themeLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]">
            <div className="grid content-start gap-3">
              {themes.length === 0 ? (
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

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.cloudflareTunnel}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadCloudflaredStatus()}>
              {cloudflaredLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
            <div className="grid gap-3 md:grid-cols-3">
              <MaintenanceCapability label={copy.settingsPage.runningStatus} enabled={Boolean(cloudflaredStatus?.running)} copy={copy} />
              <MaintenanceCapability label={copy.settingsPage.tokenLabel} enabled={Boolean(cloudflaredStatus?.token_configured)} copy={copy} />
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

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.accountSecurity}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadTotpStatus()}>
              {totpLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,0.45fr)]">
            <div className="min-w-0">
              <div className="mb-3 flex flex-wrap items-center gap-2">
                <StatusBadge tone={totpStatus?.enabled ? "green" : "gray"}>
                  {totpStatus?.enabled ? copy.settingsPage.totpEnabledBadge : copy.settingsPage.totpDisabledBadge}
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
                <p className="text-sm font-bold text-[var(--text-muted)]">
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
                <button type="button" className={buttonClass("primary")} onClick={() => void setupTotp()} disabled={totpLoading}>
                  {copy.settingsPage.generateSecret}
                </button>
                <button type="button" className={buttonClass("secondary")} onClick={() => void enableTotp()} disabled={totpLoading || !totpStatus?.setup_pending}>
                  {copy.settingsPage.enable}
                </button>
                <button type="button" className={buttonClass("danger")} onClick={() => void disableTotp()} disabled={totpLoading || !totpStatus?.enabled}>
                  {copy.settingsPage.disable}
                </button>
              </div>
              {oauthProviders.length > 0 || staleOAuthBindings.length > 0 ? (
                <div className="grid gap-3 border-t-2 border-black pt-3">
                  {oauthProviders.map((provider) => {
                    const binding = oauthBindingMap.get(provider.id);
                    return (
                      <div key={provider.id} className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3">
                        <div className="flex flex-wrap items-start justify-between gap-2">
                          <div className="min-w-0">
                            <p className="truncate text-sm font-black uppercase">{provider.display_name}</p>
                            <p className="break-all text-xs font-bold text-[var(--text-muted)]">
                              {binding?.email || binding?.display_name || binding?.subject || provider.scopes.join(" ")}
                            </p>
                            {binding ? (
                              <p className="text-xs font-bold text-[var(--text-muted)]">
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
                              onClick={() => void unbindOAuthProvider(provider.id, provider.display_name)}
                            >
                              {copy.settingsPage.unbind}
                            </button>
                          ) : (
                            <button
                              type="button"
                              className={buttonClass("secondary")}
                              onClick={() => void bindOAuthProvider(provider.id)}
                            >
                              {copy.settingsPage.bind}
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                  {staleOAuthBindings.map((binding) => (
                    <div key={binding.provider} className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3">
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div className="min-w-0">
                          <p className="truncate text-sm font-black uppercase">{binding.provider_display_name}</p>
                          <p className="break-all text-xs font-bold text-[var(--text-muted)]">
                            {binding.email || binding.display_name || binding.subject}
                          </p>
                          <p className="text-xs font-bold text-[var(--text-muted)]">
                            {copy.settingsPage.configRemovedBoundAt.replace("{date}", String(formatDate(binding.created_at)))}
                          </p>
                        </div>
                        <StatusBadge tone="yellow">{copy.settingsPage.staleBinding}</StatusBadge>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        <button
                          type="button"
                          className={buttonClass("danger")}
                          onClick={() => void unbindOAuthProvider(binding.provider, binding.provider_display_name)}
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

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.publicStatusPage}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadSystemSettings()}>
              {settingsLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge tone={systemSettings?.public_site_enabled ? "green" : "red"}>
                {systemSettings?.public_site_enabled ? copy.settingsPage.public : copy.settingsPage.private}
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
                {systemSettings?.public_server_details_enabled ? copy.settingsPage.showDetails : copy.settingsPage.hideDetails}
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

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.userManagement}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadUsers()}>
              {usersLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <form onSubmit={createPanelUser} className="mb-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_10rem_auto] md:items-end">
            <Field label={copy.settingsPage.username}>
              <input className={inputClass} value={newUser.username} onChange={(e) => setNewUser({ ...newUser, username: e.target.value })} required />
            </Field>
            <Field label={copy.settingsPage.password}>
              <input className={inputClass} type="password" value={newUser.password} onChange={(e) => setNewUser({ ...newUser, password: e.target.value })} required />
            </Field>
            <Field label={copy.settingsPage.role}>
              <select className={inputClass} value={newUser.role} onChange={(e) => setNewUser({ ...newUser, role: e.target.value })}>
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
            </Field>
            <button className={buttonClass("primary")}>{copy.settingsPage.createUser}</button>
          </form>
          <div className="grid gap-3">
            {users.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noUsers}</p>
            ) : (
              users.map((user) => (
                <div key={user.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_10rem_minmax(15rem,0.8fr)_auto] xl:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-words text-sm font-black">{user.username}</span>
                      <StatusBadge tone={user.role === "admin" ? "yellow" : "gray"}>{user.role}</StatusBadge>
                    </div>
                    <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{user.id}</div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">{copy.settingsPage.createdAt.replace("{date}", String(formatDate(user.created_at)))}</div>
                  </div>
                  <select className={inputClass} value={user.role} onChange={(e) => void updateUserRole(user, e.target.value)}>
                    <option value="member">member</option>
                    <option value="admin">admin</option>
                  </select>
                  <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                    <input
                      className={inputClass}
                      type="password"
                      placeholder={copy.settingsPage.newPassword}
                      value={passwordEdits[user.id] ?? ""}
                      onChange={(e) => setPasswordEdits((current) => ({ ...current, [user.id]: e.target.value }))}
                    />
                    <button type="button" className={buttonClass("secondary")} onClick={() => void resetUserPassword(user)}>
                      {copy.settingsPage.reset}
                    </button>
                  </div>
                  <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelUser(user)}>
                    {copy.settingsPage.delete}
                  </button>
                </div>
              ))
            )}
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.activeSessions}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadSessions()}>
              {sessionsLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
          <div className="grid gap-3">
            {sessions.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noActiveSessions}</p>
            ) : (
              sessions.map((session) => (
                <div key={session.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_14rem_auto] xl:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-words text-sm font-black">{session.username}</span>
                      <StatusBadge tone={session.role === "admin" ? "yellow" : "gray"}>{session.role}</StatusBadge>
                      {session.is_current ? <StatusBadge tone="green">{copy.settingsPage.current}</StatusBadge> : null}
                    </div>
                    <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{session.id}</div>
                    <div className="mt-1 break-all text-xs font-bold text-[var(--text-muted)]">
                      {session.ip || "IP N/A"} / {session.user_agent || "UA N/A"}
                    </div>
                  </div>
                  <div className="text-xs font-bold text-[var(--text-muted)]">
                    <div>{copy.settingsPage.createdLabel.replace("{date}", String(formatDate(session.created_at)))}</div>
                    <div>{copy.settingsPage.expiresLabel.replace("{date}", String(formatDate(session.expires_at)))}</div>
                  </div>
                  <div className="flex flex-wrap gap-2 xl:justify-end">
                    <button type="button" className={buttonClass("secondary")} onClick={() => void banSessionIp(session)} disabled={!session.ip}>
                      {copy.settingsPage.banIp}
                    </button>
                    <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelSession(session)}>
                      {copy.settingsPage.revoke}
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.wafBans}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadWafBans()}>
              {wafLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
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
            <button type="button" className={buttonClass("danger")} onClick={() => void createPanelWafBan()} disabled={wafLoading}>
              {copy.settingsPage.bulkBan}
            </button>
          </div>
          <div className="grid gap-3">
            {wafBans.length === 0 ? (
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
                  <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelWafBan(ban)}>
                    {copy.settingsPage.remove}
                  </button>
                </div>
              ))
            )}
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.backupAndMaintenance}</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadMaintenanceStatus()}>
              {maintenanceLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
            </button>
          </div>
              <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-8">
                  <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                    <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{copy.settingsPage.database}</div>
                    <div className="mt-2 text-lg font-black">{maintenanceStatus?.database_backend ?? "N/A"}</div>
                  </div>
                  <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                    <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{copy.settingsPage.tsdb}</div>
                    <div className="mt-2 text-sm font-black">{maintenanceStatus?.tsdb_backend ?? "N/A"}</div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                      {maintenanceStatus?.tsdb_status ?? "unknown"} / {maintenanceStatus?.tsdb_samples ?? "N/A"} samples
                    </div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                      {copy.settingsPage.retentionDays.replace("{days}", String(maintenanceStatus?.tsdb_retention_days ?? "N/A"))}
                    </div>
                  </div>
                  <MaintenanceCapability label={copy.settingsPage.backupDownload} enabled={Boolean(maintenanceStatus?.backup_supported)} copy={copy} />
                  <MaintenanceCapability label={copy.settingsPage.fullArchive} enabled={Boolean(maintenanceStatus?.archive_supported)} copy={copy} />
                  <MaintenanceCapability label={copy.settingsPage.backupRestore} enabled={Boolean(maintenanceStatus?.restore_supported)} copy={copy} />
                  <MaintenanceCapability label={copy.settingsPage.sqliteVacuum} enabled={Boolean(maintenanceStatus?.vacuum_supported)} copy={copy} />
                  <MaintenanceCapability label={copy.settingsPage.tsdbCompact} enabled={Boolean(maintenanceStatus?.tsdb_compact_supported)} copy={copy} />
                  <MaintenanceCapability label={copy.settingsPage.tsdbRetention} enabled={Boolean(maintenanceStatus?.tsdb_retention_configurable)} copy={copy} />
                </div>
            <div className="flex flex-wrap gap-2 xl:justify-end">
              <input
                className={`${inputClass} w-32`}
                value={tsdbRetentionDraft}
                onChange={(event) => setTsdbRetentionDraft(event.target.value)}
                inputMode="numeric"
                aria-label={copy.settingsPage.tsdbRetentionDaysAria}
                disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
              />
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void updateTsdbRetention()}
                disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
              >
                {copy.settingsPage.saveRetention}
              </button>
              <button
                type="button"
                className={buttonClass(maintenanceStatus?.backup_supported ? "primary" : "secondary")}
                disabled={!maintenanceStatus?.backup_supported || maintenanceLoading}
                onClick={() => void downloadMaintenanceExport("backup")}
              >
                {copy.settingsPage.downloadBackup}
              </button>
              <button
                type="button"
                className={buttonClass(maintenanceStatus?.archive_supported ? "primary" : "secondary")}
                disabled={!maintenanceStatus?.archive_supported || maintenanceLoading}
                onClick={() => void downloadMaintenanceExport("archive")}
              >
                {copy.settingsPage.downloadArchive}
              </button>
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void runSqliteVacuum()}
                disabled={!maintenanceStatus?.vacuum_supported || maintenanceLoading}
                >
                  {copy.settingsPage.runVacuum}
                </button>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void runTsdbCompact()}
                  disabled={!maintenanceStatus?.tsdb_compact_supported || maintenanceLoading}
                >
                  {copy.settingsPage.runTsdbCompact}
                </button>
              </div>
            </div>
          <div className="mt-4 grid gap-3 border-t-2 border-black pt-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
            <Field label={copy.settingsPage.sqliteBackupFile}>
              <input
                className={inputClass}
                type="file"
                accept=".sqlite,.sqlite3,.db,application/vnd.sqlite3,application/x-sqlite3"
                onChange={(event) => {
                  setRestoreFile(event.target.files?.[0] ?? null);
                  setRestoreResult(null);
                }}
              />
            </Field>
            <div className="flex flex-wrap gap-2 xl:justify-end">
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void restoreSqliteBackup(true)}
                disabled={!maintenanceStatus?.restore_supported || !restoreFile || maintenanceLoading}
              >
                {copy.settingsPage.verifyBackup}
              </button>
              <button
                type="button"
                className={buttonClass("danger")}
                onClick={() => void restoreSqliteBackup(false)}
                disabled={!maintenanceStatus?.restore_supported || !restoreFile || maintenanceLoading}
              >
                {copy.settingsPage.restoreBackupAction}
              </button>
            </div>
          </div>
          {restoreResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone={restoreResult.restored ? "green" : "blue"}>{restoreResult.restored ? copy.settingsPage.restored : copy.settingsPage.verified}</StatusBadge>
              <span>{copy.settingsPage.tableCount.replace("{count}", String(restoreResult.table_count))}</span>
              <span>{copy.settingsPage.rowCount.replace("{count}", String(restoreResult.row_count))}</span>
              <span>{copy.settingsPage.userVersion.replace("{version}", String(restoreResult.user_version))}</span>
            </div>
          ) : null}
          {tsdbCompactResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone="green">{copy.settingsPage.tsdbCompact}</StatusBadge>
              <span>{copy.settingsPage.removedSamples.replace("{count}", String(tsdbCompactResult.removed_samples))}</span>
              <span>{copy.settingsPage.samplesBefore.replace("{count}", String(tsdbCompactResult.samples_before ?? "N/A"))}</span>
              <span>{copy.settingsPage.samplesAfter.replace("{count}", String(tsdbCompactResult.samples_after ?? "N/A"))}</span>
            </div>
          ) : null}
          {tsdbRetentionResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone="green">{copy.settingsPage.tsdbRetention}</StatusBadge>
              <span>{copy.settingsPage.daysCount.replace("{days}", String(tsdbRetentionResult.retention_days))}</span>
              <span>{copy.settingsPage.samplesBefore.replace("{count}", String(tsdbRetentionResult.samples_before ?? "N/A"))}</span>
              <span>{copy.settingsPage.samplesAfter.replace("{count}", String(tsdbRetentionResult.samples_after ?? "N/A"))}</span>
            </div>
          ) : null}
          <p className="mt-3 text-xs font-bold text-[var(--text-muted)]">
            {copy.settingsPage.maintenanceHint}
          </p>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.geoIp}</h2>
              <div className="mt-2 flex flex-wrap gap-2">
                <StatusBadge tone="blue">{copy.settingsPage.defaultProvider.replace("{provider}", String(systemSettings?.geoip_provider ?? "empty"))}</StatusBadge>
                <StatusBadge tone={systemSettings?.geoip_ipinfo_token_configured ? "green" : "gray"}>
                  {systemSettings?.geoip_ipinfo_token_configured ? copy.settingsPage.ipinfoTokenConfigured : copy.settingsPage.ipinfoTokenNotConfigured}
                </StatusBadge>
                <StatusBadge tone={geoIpMmdbStatus?.configured ? "green" : "gray"}>
                  {geoIpMmdbStatus?.configured ? copy.settingsPage.mmdbConfigured.replace("{type}", String(geoIpMmdbStatus.database_type || "ready")) : copy.settingsPage.mmdbNotConfigured}
                </StatusBadge>
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
                {settingsLoading ? copy.settingsPage.saving : copy.settingsPage.saveDefault}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void updateGeoIpDatabase()} disabled={geoIpLoading}>
                {copy.settingsPage.mmdbUpdate}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void loadGeoIpStatus()} disabled={geoIpLoading}>
                {copy.settingsPage.mmdbStatus}
              </button>
            </div>
          </div>
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

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">{copy.settingsPage.ddns}</h2>
              <div className="mt-2 flex flex-wrap gap-2">
                <StatusBadge tone={ddnsResolverUrl.trim() ? "green" : "gray"}>
                  {ddnsResolverUrl.trim() ? copy.settingsPage.resolverConfigured : copy.settingsPage.resolverSystem}
                </StatusBadge>
              </div>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
              {settingsLoading ? copy.settingsPage.saving : copy.settingsPage.saveDdns}
            </button>
          </div>
          <Field label={copy.settingsPage.dohResolverUrl}>
            <input className={inputClass} value={ddnsResolverUrl} onChange={(event) => setDdnsResolverUrl(event.target.value)} placeholder="https://dns.google/resolve" />
          </Field>
        </BrutalCard>

        <div className="grid gap-6 lg:grid-cols-2">
          <BrutalCard accent>
            <h2 className="mb-4 text-xl font-black uppercase">{copy.settingsPage.createPat}</h2>
            <form onSubmit={createToken} className="space-y-4">
              <Field label={copy.settingsPage.nameLabel}><input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} required /></Field>
              <Field label={copy.settingsPage.scopeLabel}><textarea className={`${textareaClass} min-h-24`} value={scopes} onChange={(e) => setScopes(e.target.value)} /></Field>
              <Field label={copy.settingsPage.expiryLabel}><input className={inputClass} type="datetime-local" value={patExpiresAt} onChange={(e) => setPatExpiresAt(e.target.value)} required /></Field>
              <Field label={copy.settingsPage.serverAllowlist}><textarea className={`${textareaClass} min-h-20`} value={patServerIds} onChange={(e) => setPatServerIds(e.target.value)} placeholder="server id, server id" /></Field>
              <button className={buttonClass("primary")}>{copy.settingsPage.createToken}</button>
            </form>
            {createdToken ? (
              <div className="mt-5 border-2 border-black bg-black p-3 font-mono text-xs text-green-300">
                {createdToken}
              </div>
            ) : null}
          </BrutalCard>

          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">{copy.settingsPage.existingTokens}</h2>
            <div className="grid gap-3">
              {tokens.length === 0 ? (
                <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noTokens}</p>
              ) : (
                tokens.map((token) => {
                  const serverIds = token.server_ids ?? [];
                  return (
                    <div key={token.id} className="border-2 border-black bg-[var(--accent-bg)] p-3 text-sm font-bold">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="break-words text-base font-black">{token.name || copy.settingsPage.unnamedToken}</div>
                          <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-muted)]">{token.id}</div>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          {serverIds.length === 0 ? (
                            <StatusBadge tone="red">{copy.settingsPage.globalPat}</StatusBadge>
                          ) : (
                            <StatusBadge tone="green">{copy.settingsPage.serversCount.replace("{count}", String(serverIds.length))}</StatusBadge>
                          )}
                        </div>
                      </div>
                      <div className="mt-3 grid gap-2 text-xs text-[var(--text-muted)]">
                        <div>{copy.settingsPage.scopeValue.replace("{value}", String(token.scopes?.join(" ") || "N/A"))}</div>
                        <div>{copy.settingsPage.expiryValue.replace("{value}", String(formatDate(token.expires_at)))}</div>
                        <div>{copy.settingsPage.lastUsedValue.replace("{value}", String(formatDate(token.last_used_at)))}</div>
                        {serverIds.length > 0 ? <div className="break-all">{copy.settingsPage.serverAllowlistValue.replace("{value}", String(serverIds.join(", ")))}</div> : null}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </BrutalCard>
        </div>
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function MaintenanceCapability({
  label,
  enabled,
  copy,
}: {
  label: string;
  enabled: boolean;
  copy: ReturnType<typeof useI18n>["t"];
}) {
  return (
    <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2">
        <StatusBadge tone={enabled ? "green" : "gray"}>{enabled ? copy.settingsPage.supported : copy.settingsPage.unsupported}</StatusBadge>
      </div>
    </div>
  );
}

function defaultGrpcUrl(apiBaseUrl: string): string {
  try {
    const url = new URL(apiBaseUrl);
    url.port = "50051";
    return url.toString().replace(/\/$/, "");
  } catch {
    return "http://localhost:50051";
  }
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\"'\"'")}'`;
}

function splitSettingList(value: string): string[] {
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

function defaultPatExpiresAt(): string {
  const date = new Date(Date.now() + 90 * 24 * 60 * 60 * 1000);
  const timezoneOffsetMs = date.getTimezoneOffset() * 60 * 1000;
  return new Date(date.getTime() - timezoneOffsetMs).toISOString().slice(0, 16);
}

function nullableText(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function themeTargetLabel(target: string, copy: ReturnType<typeof useI18n>["t"]): string {
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

function buildAgentInstallCommand({
  installScriptUrl,
}: {
  installScriptUrl: string;
}): string {
  return `curl -fsSL ${shellQuote(installScriptUrl)} | sudo bash`;
}

function saveBlob(blob: Blob, filename: string) {
  const url = window.URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename || "download";
  document.body.appendChild(link);
  link.click();
  link.remove();
  window.URL.revokeObjectURL(url);
}
