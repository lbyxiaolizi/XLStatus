"use client";

import { FormEvent, useEffect, useState } from "react";
import Navigation from "@/app/components/Navigation";
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

const DEFAULT_AGENT_VERSION = "v0.1.0-alpha.3";
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
  const [agentLatestVersionStatus, setAgentLatestVersionStatus] = useState("等待获取 GitHub 最新版");
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
      setError("主题 JSON 格式无效。");
      return;
    }
    const candidate = parsed && typeof parsed === "object" && "theme" in parsed
      ? (parsed as { theme: unknown }).theme
      : parsed;
    if (!candidate || typeof candidate !== "object") {
      setError("主题 JSON 需要是对象。");
      return;
    }
    setThemeLoading(true);
    const response = await apiClient.importTheme(candidate as ImportThemeRequest["theme"]);
    setThemeLoading(false);
    if (response.success && response.data) {
      setNotice(`主题 ${response.data.name} 已导入。`);
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
      setError("无法读取主题文件。");
    }
  }

  async function selectTheme(theme: ThemeDefinition, target: "public" | "dashboard" | "both") {
    setThemeLoading(true);
    const response = await apiClient.selectTheme(theme.id, target);
    setThemeLoading(false);
    if (response.success && response.data) {
      setThemes(response.data.themes ?? []);
      setSelectedPublicThemeId(response.data.selected_public_theme_id || "");
      setSelectedDashboardThemeId(response.data.selected_dashboard_theme_id || "");
      setNotice(`主题 ${theme.name} 已应用。`);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteTheme(theme: ThemeDefinition) {
    if (!confirm(`确定删除主题 ${theme.name}？`)) return;
    setThemeLoading(true);
    const response = await apiClient.deleteTheme(theme.id);
    setThemeLoading(false);
    if (response.success) {
      setNotice(`主题 ${theme.name} 已删除。`);
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
    const code = window.prompt("请输入 6 位 TOTP 验证码");
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError("请输入 6 位 TOTP 验证码。");
      return null;
    }
    return trimmed;
  }

  async function createPanelUser(event: FormEvent) {
    event.preventDefault();
    if (newUser.password.length < 8) {
      setError("密码至少需要 8 个字符。");
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
      setNotice("用户已创建。");
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
      setNotice(`已更新 ${user.username} 的角色。`);
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function resetUserPassword(user: UserAccount) {
    const password = passwordEdits[user.id] ?? "";
    if (password.length < 8) {
      setError("新密码至少需要 8 个字符。");
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(user.id, { password }, totpCode);
    if (response.success) {
      setNotice(`已重置 ${user.username} 的密码。`);
      setPasswordEdits((current) => ({ ...current, [user.id]: "" }));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelUser(user: UserAccount) {
    if (!confirm(`确定删除用户 ${user.username}？该用户拥有的资源可能会按数据库约束一起删除。`)) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteUser(user.id, totpCode);
    if (response.success) {
      setNotice(`已删除 ${user.username}。`);
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelSession(session: SessionInfo) {
    const label = session.is_current ? "当前会话" : `${session.username} 的会话`;
    if (!confirm(`确定撤销${label}？`)) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteSession(session.id, totpCode);
    if (response.success) {
      setNotice("会话已撤销。");
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
      setError("请输入至少一个 IP。");
      return;
    }
    const minutes = Number.parseInt(wafBanDraft.minutes, 10);
    if (!Number.isFinite(minutes) || minutes <= 0) {
      setError("封禁分钟数必须大于 0。");
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
      setNotice(`已创建 ${count} 条 WAF 封禁。`);
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
      setError("该会话没有可封禁的 IP。");
      return;
    }
    if (!confirm(`确定封禁 ${ip} 30 分钟？`)) return;
    await createPanelWafBan([ip]);
  }

  async function deletePanelWafBan(ban: WafBan) {
    if (!confirm(`确定解除 ${ban.ip} 的 WAF 封禁？`)) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteWafBan(ban.id, totpCode);
    if (response.success) {
      setNotice(`已解除 ${ban.ip} 的封禁。`);
      await loadWafBans();
    } else {
      setError(responseError(response));
    }
  }

  async function runSqliteVacuum() {
    if (!confirm("确定立即执行 SQLite VACUUM？执行期间数据库可能短暂变慢。")) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.vacuumSqlite(totpCode);
    setMaintenanceLoading(false);
    if (response.success) {
      setNotice("SQLite VACUUM 已完成。");
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function runTsdbCompact() {
    if (!confirm("确定立即执行 TSDB compact？这会清理超出 retention 窗口的历史样本。")) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.compactTsdb(totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbCompactResult(response.data);
      setNotice(`TSDB compact 已完成，移除 ${response.data.removed_samples} 条样本。`);
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function updateTsdbRetention() {
    const parsed = Number.parseInt(tsdbRetentionDraft.trim(), 10);
    if (!Number.isFinite(parsed) || parsed < 1 || parsed > 3650) {
      setError("TSDB retention 必须是 1 到 3650 天。");
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.updateTsdbRetention(parsed, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbRetentionResult(response.data);
      setNotice(`TSDB retention 已更新为 ${response.data.retention_days} 天。`);
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
      setNotice(kind === "backup" ? "备份下载已开始。" : "归档下载已开始。");
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
      setNotice("cloudflared token 已保存。");
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
      setNotice(action === "start" ? "cloudflared 已启动。" : "cloudflared 已停止。");
    } else {
      setError(responseError(response));
    }
  }

  async function restoreSqliteBackup(dryRun: boolean) {
    if (!restoreFile) {
      setError("请选择 SQLite 备份文件。");
      return;
    }
    if (!maintenanceStatus?.restore_supported) {
      setError("当前数据库后端不支持在线恢复。");
      return;
    }
    if (!dryRun && !confirm("确定用该备份恢复当前 SQLite 数据库？当前数据会被备份内容覆盖。")) return;
    const totpCode = dryRun ? undefined : await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.restoreBackup(restoreFile, dryRun, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setRestoreResult(response.data);
      setNotice(dryRun ? "备份校验通过。" : "备份恢复已完成。");
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
      setNotice("TOTP 密钥已生成，请加入认证器后输入验证码启用。");
    } else {
      setError(responseError(response));
    }
  }

  async function enableTotp() {
    if (totpCode.trim().length !== 6) {
      setError("请输入 6 位 TOTP 验证码。");
      return;
    }
    setTotpLoading(true);
    const response = await apiClient.enableTotp(totpCode.trim());
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpStatus(response.data);
      setTotpSetup(null);
      setTotpCode("");
      setNotice("TOTP 两步验证已启用。");
    } else {
      setError(responseError(response));
    }
  }

  async function disableTotp() {
    if (totpStatus?.enabled && totpCode.trim().length !== 6) {
      setError("请输入当前 6 位 TOTP 验证码。");
      return;
    }
    if (!confirm("确定停用当前账号的 TOTP 两步验证？")) return;
    setTotpLoading(true);
    const response = await apiClient.disableTotp(totpCode.trim());
    setTotpLoading(false);
    if (response.success && response.data) {
      setTotpStatus(response.data);
      setTotpSetup(null);
      setTotpCode("");
      setNotice("TOTP 两步验证已停用。");
    } else {
      setError(responseError(response));
    }
  }

  async function unbindOAuthProvider(providerId: string, displayName: string) {
    if (!confirm(`确定解绑 ${displayName}？解绑后将不能继续用该 OAuth 账号登录。`)) return;
    const response = await apiClient.unbindOAuthProvider(providerId);
    if (response.success) {
      setNotice(`${displayName} 已解绑。`);
      await loadOAuthBindings();
    } else {
      setError(responseError(response));
    }
  }

  async function bindOAuthProvider(providerId: string) {
    const response = await apiClient.startOAuthBind(providerId, "/settings");
    if (response.success && response.data?.authorization_url) {
      window.location.href = response.data.authorization_url;
    } else {
      setError(responseError(response));
    }
  }

  async function testGeoIp() {
    setGeoIpLoading(true);
    const response = await apiClient.testGeoIp(geoIp.ip.trim(), geoIp.provider, geoIp.token);
    setGeoIpLoading(false);
    if (response.success && response.data) {
      setGeoIpResult(response.data);
      setNotice("GeoIP 查询完成。");
    } else {
      setError(responseError(response));
    }
  }

  async function updateGeoIpDatabase() {
    setGeoIpLoading(true);
    const response = await apiClient.updateGeoIpDatabase({
      source_url: geoIpMmdbUrl.trim() || undefined,
      source_path: geoIpMmdbPath.trim() || undefined,
    });
    setGeoIpLoading(false);
    if (response.success && response.data) {
      if (response.data.status) setGeoIpMmdbStatus(response.data.status);
      const message = response.data.message || "GeoIP 更新请求已完成。";
      setNotice(message);
    } else {
      setError(responseError(response));
    }
  }

  async function uploadGeoIpDatabase() {
    if (!geoIpMmdbFile) {
      setError("请选择 MMDB 文件。");
      return;
    }
    setGeoIpLoading(true);
    const response = await apiClient.uploadGeoIpDatabase(geoIpMmdbFile);
    setGeoIpLoading(false);
    if (response.success && response.data) {
      if (response.data.status) setGeoIpMmdbStatus(response.data.status);
      setGeoIpMmdbFile(null);
      setNotice(response.data.message || "MMDB 上传完成。");
    } else {
      setError(responseError(response));
    }
  }

  async function saveGeoIpSettings() {
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({
      geoip_provider: geoIp.provider,
      geoip_ipinfo_token: geoIp.token,
      geoip_ip_change_enabled: geoIpIpChange.enabled,
      geoip_ip_change_notification_group_id: geoIpIpChange.notification_group_id || null,
      geoip_ip_change_server_ids: splitSettingList(geoIpIpChange.server_ids),
      geoip_ip_change_severity: geoIpIpChange.severity,
      ddns_resolver_url: ddnsResolverUrl.trim(),
    });
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
      setNotice("GeoIP 默认 Provider 已保存。");
    } else {
      setError(responseError(response));
    }
  }

  async function updatePublicSiteEnabled(enabled: boolean) {
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({ public_site_enabled: enabled });
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      setNotice(enabled ? "公开状态页已开启。" : "公开状态页已设为私有。");
    } else {
      setError(responseError(response));
    }
  }

  async function updatePublicServerDetailsEnabled(enabled: boolean) {
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({ public_server_details_enabled: enabled });
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      setNotice(enabled ? "公开状态页已显示服务器详细信息。" : "公开状态页已隐藏服务器详细信息。");
    } else {
      setError(responseError(response));
    }
  }

  async function savePublicBranding() {
    const siteName = publicBranding.siteName.trim();
    if (!siteName) {
      setError("请填写公开状态页名称。");
      return;
    }
    setSettingsLoading(true);
    const response = await apiClient.updateSettings({
      public_site_name: siteName,
      public_logo_url: nullableText(publicBranding.logoUrl),
      public_favicon_url: nullableText(publicBranding.faviconUrl),
      public_theme_color: nullableText(publicBranding.themeColor),
      public_background_url: nullableText(publicBranding.backgroundUrl),
      public_custom_head: null,
      public_custom_body: null,
    });
    setSettingsLoading(false);
    if (response.success && response.data) {
      setSystemSettings(response.data);
      setPublicBranding((current) => ({ ...current, customHead: "", customBody: "" }));
      setNotice("公开状态页品牌设置已保存。");
    } else {
      setError(responseError(response));
    }
  }

  async function createToken(event: FormEvent) {
    event.preventDefault();
    const serverIds = splitSettingList(patServerIds);
    const expiresAt = new Date(patExpiresAt);
    if (Number.isNaN(expiresAt.getTime())) {
      setError("请填写有效的 PAT 过期时间。");
      return;
    }
    if (serverIds.length === 0 && !window.confirm("未填写 server allowlist 会创建全局 PAT。确认继续？")) {
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
      setNotice(`个人访问令牌已创建，过期时间：${formatDate(response.data.expires_at)}。`);
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
    const response = await apiClient.createEnrollmentToken(
      Number.isFinite(expiresInHours) && expiresInHours > 0 ? expiresInHours : 1,
    );
    if (response.success && response.data) {
      setEnrollmentToken(response.data.token);
      setEnrollmentExpiresAt(response.data.expires_at);
      setNotice("Agent 安装令牌已创建。");
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
        throw new Error(`GitHub 返回 HTTP ${response.status}`);
      }
      const data = (await response.json()) as Array<{ tag_name?: string; draft?: boolean }>;
      const release = data.find((item) => !item.draft && item.tag_name?.trim());
      const tagName = release?.tag_name?.trim();
      if (!tagName) {
        throw new Error("GitHub release 响应没有 tag_name");
      }
      setAgentVersion(tagName);
      setAgentLatestVersionStatus(`已获取 GitHub 最新版：${tagName}`);
    } catch (err) {
      const message = err instanceof Error ? err.message : "未知错误";
      setAgentLatestVersionStatus(`获取失败，继续使用 ${agentVersion || DEFAULT_AGENT_VERSION}：${message}`);
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
      setNotice("Agent 安装命令已复制。");
    } catch {
      setError("无法写入剪贴板，请手动复制命令。");
    }
  }

  async function copyTotpUri() {
    if (!totpSetup?.otpauth_uri) return;
    try {
      await navigator.clipboard.writeText(totpSetup.otpauth_uri);
      setNotice("TOTP URI 已复制。");
    } catch {
      setError("无法写入剪贴板，请手动复制 URI。");
    }
  }

  const oauthBindingMap = new Map(oauthBindings.map((binding) => [binding.provider, binding]));
  const oauthProviderIds = new Set(oauthProviders.map((provider) => provider.id));
  const staleOAuthBindings = oauthBindings.filter((binding) => !oauthProviderIds.has(binding.provider));

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="控制面"
          title="设置"
          detail="个人访问令牌和本地管理员辅助工具。"
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <BrutalCard accent className="mb-6">
          <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
            <div>
              <h2 className="mb-4 text-xl font-black uppercase">Agent 安装</h2>
              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Server URL">
                  <input className={inputClass} value={agentServerUrl} onChange={(e) => setAgentServerUrl(e.target.value)} />
                </Field>
                <Field label="gRPC URL">
                  <input className={inputClass} value={agentGrpcUrl} onChange={(e) => setAgentGrpcUrl(e.target.value)} />
                </Field>
                <Field label="Release 版本">
                  <input
                    className={inputClass}
                    value={agentVersion}
                    onChange={(e) => setAgentVersion(e.target.value)}
                    disabled={agentUseLatestVersion}
                  />
                </Field>
                <Field label="Agent 名称">
                  <input className={inputClass} value={agentName} onChange={(e) => setAgentName(e.target.value)} />
                </Field>
                <Field label="令牌有效期（小时）">
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
                  从 GitHub 获取最新版
                </label>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void refreshLatestAgentVersion()}
                  disabled={agentLatestVersionLoading}
                >
                  {agentLatestVersionLoading ? "获取中" : "刷新版本"}
                </button>
              </div>
              <p className="mt-2 text-xs font-bold text-[var(--text-muted)]">
                {agentLatestVersionStatus}
              </p>
              <div className="mt-4 flex flex-wrap gap-2">
                <button type="button" className={buttonClass("primary")} onClick={createEnrollmentToken}>
                  生成安装令牌
                </button>
                <button type="button" className={buttonClass("secondary")} onClick={copyAgentCommand} disabled={!enrollmentToken}>
                  复制安装命令
                </button>
                <a className={buttonClass("secondary")} href={installScriptUrl} target="_blank" rel="noreferrer">
                  打开带参链接
                </a>
              </div>
              {enrollmentExpiresAt ? (
                <p className="mt-3 text-xs font-black uppercase text-[var(--text-muted)]">
                  令牌过期时间：{enrollmentExpiresAt}
                </p>
              ) : null}
              <p className="mt-3 break-all text-xs font-bold text-[var(--text-muted)]">
                GitHub 脚本源：{githubScriptUrl}
              </p>
            </div>
            <div>
              <p className="mb-2 text-xs font-black uppercase text-[var(--text-muted)]">带参数一键安装命令</p>
              <pre className="min-h-40 overflow-auto whitespace-pre-wrap break-all border-2 border-black bg-black p-3 font-mono text-xs text-green-300 shadow-[var(--shadow-brutal-sm)]">
                {agentInstallCommand}
              </pre>
            </div>
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">主题模板</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadThemes()} disabled={themeLoading}>
              {themeLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]">
            <div className="grid content-start gap-3">
              {themes.length === 0 ? (
                <p className="text-sm font-bold text-[var(--text-muted)]">暂无主题。</p>
              ) : (
                themes.map((theme) => (
                  <div key={theme.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <h3 className="break-words text-lg font-black uppercase">{theme.name}</h3>
                          <StatusBadge tone={theme.builtin ? "blue" : "pink"}>{theme.builtin ? "内置" : "自定义"}</StatusBadge>
                          <StatusBadge tone="gray">{themeTargetLabel(theme.target)}</StatusBadge>
                          {selectedPublicThemeId === theme.id ? <StatusBadge tone="green">公开页</StatusBadge> : null}
                          {selectedDashboardThemeId === theme.id ? <StatusBadge tone="yellow">控制面</StatusBadge> : null}
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
                        公开
                      </button>
                      <button
                        type="button"
                        className={buttonClass("secondary")}
                        onClick={() => void selectTheme(theme, "dashboard")}
                        disabled={themeLoading || !themeSupportsTarget(theme, "dashboard") || selectedDashboardThemeId === theme.id}
                      >
                        控制面
                      </button>
                      <button
                        type="button"
                        className={buttonClass("primary")}
                        onClick={() => void selectTheme(theme, "both")}
                        disabled={themeLoading || theme.target !== "both" || (selectedPublicThemeId === theme.id && selectedDashboardThemeId === theme.id)}
                      >
                        两端
                      </button>
                      {!theme.builtin ? (
                        <button type="button" className={buttonClass("danger")} onClick={() => void deleteTheme(theme)} disabled={themeLoading}>
                          删除
                        </button>
                      ) : null}
                    </div>
                  </div>
                ))
              )}
            </div>
            <div className="grid content-start gap-3">
              <Field label="主题文件">
                <input
                  className={inputClass}
                  type="file"
                  accept=".json,application/json"
                  onChange={(event) => void loadThemeFile(event.target.files?.[0])}
                />
              </Field>
              <Field label="导入 JSON">
                <textarea
                  className={`${textareaClass} min-h-80 font-mono text-xs`}
                  value={themeImportText}
                  onChange={(event) => setThemeImportText(event.target.value)}
                  spellCheck={false}
                />
              </Field>
              <div className="flex flex-wrap justify-end gap-2">
                <button type="button" className={buttonClass("secondary")} onClick={() => setThemeImportText(defaultThemeImportText())}>
                  示例
                </button>
                <button type="button" className={buttonClass("primary")} onClick={() => void importTheme()} disabled={themeLoading}>
                  导入主题
                </button>
              </div>
            </div>
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">Cloudflare Tunnel</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadCloudflaredStatus()}>
              {cloudflaredLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
            <div className="grid gap-3 md:grid-cols-3">
              <MaintenanceCapability label="运行状态" enabled={Boolean(cloudflaredStatus?.running)} />
              <MaintenanceCapability label="Token" enabled={Boolean(cloudflaredStatus?.token_configured)} />
              <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">PID</div>
                <div className="mt-2 text-sm font-black">{cloudflaredStatus?.pid ?? "N/A"}</div>
              </div>
            </div>
            <div className="flex flex-wrap gap-2 lg:justify-end">
              <button type="button" className={buttonClass("primary")} onClick={() => void runCloudflared("start")} disabled={cloudflaredLoading}>
                启动
              </button>
              <button type="button" className={buttonClass("danger")} onClick={() => void runCloudflared("stop")} disabled={cloudflaredLoading}>
                停止
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
              保存 token
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
              <h2 className="text-xl font-black uppercase">账号安全</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadTotpStatus()}>
              {totpLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,0.45fr)]">
            <div className="min-w-0">
              <div className="mb-3 flex flex-wrap items-center gap-2">
                <StatusBadge tone={totpStatus?.enabled ? "green" : "gray"}>
                  {totpStatus?.enabled ? "TOTP 已启用" : "TOTP 未启用"}
                </StatusBadge>
                {totpStatus?.setup_pending ? <StatusBadge tone="yellow">待验证</StatusBadge> : null}
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
                      复制 URI
                    </button>
                  </div>
                </div>
              ) : (
                <p className="text-sm font-bold text-[var(--text-muted)]">
                  启用后，登录除了密码还需要认证器生成的 6 位验证码。
                </p>
              )}
            </div>
            <div className="grid content-end gap-3">
              <Field label={totpStatus?.enabled ? "当前验证码" : "启用验证码"}>
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
                  生成密钥
                </button>
                <button type="button" className={buttonClass("secondary")} onClick={() => void enableTotp()} disabled={totpLoading || !totpStatus?.setup_pending}>
                  启用
                </button>
                <button type="button" className={buttonClass("danger")} onClick={() => void disableTotp()} disabled={totpLoading || !totpStatus?.enabled}>
                  停用
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
                                绑定于 {formatDate(binding.created_at)}
                              </p>
                            ) : null}
                          </div>
                          <StatusBadge tone={binding ? "green" : "gray"}>{binding ? "已绑定" : "未绑定"}</StatusBadge>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          {binding ? (
                            <button
                              type="button"
                              className={buttonClass("danger")}
                              onClick={() => void unbindOAuthProvider(provider.id, provider.display_name)}
                            >
                              解绑
                            </button>
                          ) : (
                            <button
                              type="button"
                              className={buttonClass("secondary")}
                              onClick={() => void bindOAuthProvider(provider.id)}
                            >
                              绑定
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
                            配置已移除，绑定于 {formatDate(binding.created_at)}
                          </p>
                        </div>
                        <StatusBadge tone="yellow">旧绑定</StatusBadge>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        <button
                          type="button"
                          className={buttonClass("danger")}
                          onClick={() => void unbindOAuthProvider(binding.provider, binding.provider_display_name)}
                        >
                          解绑
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
              <h2 className="text-xl font-black uppercase">公开状态页</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadSystemSettings()}>
              {settingsLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge tone={systemSettings?.public_site_enabled ? "green" : "red"}>
                {systemSettings?.public_site_enabled ? "公开" : "私有"}
              </StatusBadge>
              <span className="text-sm font-bold text-[var(--text-muted)]">
                关闭后匿名状态页、公开服务器详情和公开指标接口会返回拒绝访问。
              </span>
            </div>
            <label className="inline-flex cursor-pointer items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
              <input
                type="checkbox"
                checked={systemSettings?.public_site_enabled === true}
                onChange={(event) => void updatePublicSiteEnabled(event.target.checked)}
                disabled={settingsLoading || !systemSettings}
              />
              匿名访问
            </label>
          </div>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t-2 border-black pt-3">
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge tone={systemSettings?.public_server_details_enabled ? "green" : "gray"}>
                {systemSettings?.public_server_details_enabled ? "显示详情" : "隐藏详情"}
              </StatusBadge>
              <span className="text-sm font-bold text-[var(--text-muted)]">
                开启后匿名状态页会显示公开服务器的 CPU、内存、磁盘、网络和监控图表。
              </span>
            </div>
            <label className="inline-flex cursor-pointer items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
              <input
                type="checkbox"
                checked={systemSettings?.public_server_details_enabled === true}
                onChange={(event) => void updatePublicServerDetailsEnabled(event.target.checked)}
                disabled={settingsLoading || !systemSettings}
              />
              服务器详细信息
            </label>
          </div>
          <div className="mt-4 grid gap-3 border-t-2 border-black pt-4 lg:grid-cols-2">
            <Field label="站点名称">
              <input
                className={inputClass}
                value={publicBranding.siteName}
                onChange={(event) => setPublicBranding((current) => ({ ...current, siteName: event.target.value }))}
              />
            </Field>
            <Field label="主题色">
              <input
                className={inputClass}
                value={publicBranding.themeColor}
                onChange={(event) => setPublicBranding((current) => ({ ...current, themeColor: event.target.value }))}
                placeholder="#16a34a"
              />
            </Field>
            <Field label="Logo URL">
              <input
                className={inputClass}
                value={publicBranding.logoUrl}
                onChange={(event) => setPublicBranding((current) => ({ ...current, logoUrl: event.target.value }))}
              />
            </Field>
            <Field label="favicon URL">
              <input
                className={inputClass}
                value={publicBranding.faviconUrl}
                onChange={(event) => setPublicBranding((current) => ({ ...current, faviconUrl: event.target.value }))}
              />
            </Field>
            <Field label="背景 URL">
              <input
                className={inputClass}
                value={publicBranding.backgroundUrl}
                onChange={(event) => setPublicBranding((current) => ({ ...current, backgroundUrl: event.target.value }))}
              />
            </Field>
            <div className="flex items-end justify-end">
              <button type="button" className={buttonClass("primary")} onClick={() => void savePublicBranding()} disabled={settingsLoading}>
                保存品牌
              </button>
            </div>
            <Field label="自定义 head（已停用）">
              <textarea
                className={`${textareaClass} min-h-28`}
                value={publicBranding.customHead}
                readOnly
              />
            </Field>
            <Field label="自定义 body（已停用）">
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
              <h2 className="text-xl font-black uppercase">用户管理</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadUsers()}>
              {usersLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <form onSubmit={createPanelUser} className="mb-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_10rem_auto] md:items-end">
            <Field label="用户名">
              <input className={inputClass} value={newUser.username} onChange={(e) => setNewUser({ ...newUser, username: e.target.value })} required />
            </Field>
            <Field label="密码">
              <input className={inputClass} type="password" value={newUser.password} onChange={(e) => setNewUser({ ...newUser, password: e.target.value })} required />
            </Field>
            <Field label="角色">
              <select className={inputClass} value={newUser.role} onChange={(e) => setNewUser({ ...newUser, role: e.target.value })}>
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
            </Field>
            <button className={buttonClass("primary")}>创建用户</button>
          </form>
          <div className="grid gap-3">
            {users.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">暂无用户或当前账号没有权限查看。</p>
            ) : (
              users.map((user) => (
                <div key={user.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_10rem_minmax(15rem,0.8fr)_auto] xl:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-words text-sm font-black">{user.username}</span>
                      <StatusBadge tone={user.role === "admin" ? "yellow" : "gray"}>{user.role}</StatusBadge>
                    </div>
                    <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{user.id}</div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">创建于 {formatDate(user.created_at)}</div>
                  </div>
                  <select className={inputClass} value={user.role} onChange={(e) => void updateUserRole(user, e.target.value)}>
                    <option value="member">member</option>
                    <option value="admin">admin</option>
                  </select>
                  <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                    <input
                      className={inputClass}
                      type="password"
                      placeholder="新密码"
                      value={passwordEdits[user.id] ?? ""}
                      onChange={(e) => setPasswordEdits((current) => ({ ...current, [user.id]: e.target.value }))}
                    />
                    <button type="button" className={buttonClass("secondary")} onClick={() => void resetUserPassword(user)}>
                      重置
                    </button>
                  </div>
                  <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelUser(user)}>
                    删除
                  </button>
                </div>
              ))
            )}
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">活跃会话</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadSessions()}>
              {sessionsLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="grid gap-3">
            {sessions.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">暂无活跃会话。</p>
            ) : (
              sessions.map((session) => (
                <div key={session.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_14rem_auto] xl:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-words text-sm font-black">{session.username}</span>
                      <StatusBadge tone={session.role === "admin" ? "yellow" : "gray"}>{session.role}</StatusBadge>
                      {session.is_current ? <StatusBadge tone="green">当前</StatusBadge> : null}
                    </div>
                    <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{session.id}</div>
                    <div className="mt-1 break-all text-xs font-bold text-[var(--text-muted)]">
                      {session.ip || "IP N/A"} / {session.user_agent || "UA N/A"}
                    </div>
                  </div>
                  <div className="text-xs font-bold text-[var(--text-muted)]">
                    <div>创建：{formatDate(session.created_at)}</div>
                    <div>过期：{formatDate(session.expires_at)}</div>
                  </div>
                  <div className="flex flex-wrap gap-2 xl:justify-end">
                    <button type="button" className={buttonClass("secondary")} onClick={() => void banSessionIp(session)} disabled={!session.ip}>
                      封禁 IP
                    </button>
                    <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelSession(session)}>
                      撤销
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
              <h2 className="text-xl font-black uppercase">WAF 封禁</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadWafBans()}>
              {wafLoading ? "加载中" : "刷新"}
            </button>
          </div>
          <div className="mb-4 grid gap-3 border-b-2 border-black pb-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,0.8fr)_9rem_auto] xl:items-end">
            <Field label="IP 列表">
              <input
                className={inputClass}
                value={wafBanDraft.ips}
                onChange={(event) => setWafBanDraft((current) => ({ ...current, ips: event.target.value }))}
                placeholder="1.2.3.4, 2001:db8::1"
              />
            </Field>
            <Field label="原因">
              <input
                className={inputClass}
                value={wafBanDraft.reason}
                onChange={(event) => setWafBanDraft((current) => ({ ...current, reason: event.target.value }))}
              />
            </Field>
            <Field label="分钟">
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
              批量封禁
            </button>
          </div>
          <div className="grid gap-3">
            {wafBans.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">暂无有效封禁。</p>
            ) : (
              wafBans.map((ban) => (
                <div key={ban.id} className="grid gap-3 border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)] xl:grid-cols-[minmax(0,1fr)_12rem_auto] xl:items-center">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="break-all text-sm font-black">{ban.ip}</span>
                      <StatusBadge tone="red">{ban.failed_count} 次</StatusBadge>
                    </div>
                    <div className="mt-1 break-words text-xs font-bold text-[var(--text-muted)]">{ban.reason}</div>
                    <div className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{ban.id}</div>
                  </div>
                  <div className="text-xs font-bold text-[var(--text-muted)]">
                    <div>解封：{formatDate(ban.banned_until)}</div>
                    <div>更新：{formatDate(ban.updated_at || ban.created_at)}</div>
                  </div>
                  <button type="button" className={buttonClass("danger")} onClick={() => void deletePanelWafBan(ban)}>
                    解除
                  </button>
                </div>
              ))
            )}
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">备份和维护</h2>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void loadMaintenanceStatus()}>
              {maintenanceLoading ? "加载中" : "刷新"}
            </button>
          </div>
              <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-8">
                  <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                    <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">数据库</div>
                    <div className="mt-2 text-lg font-black">{maintenanceStatus?.database_backend ?? "N/A"}</div>
                  </div>
                  <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
                    <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">TSDB</div>
                    <div className="mt-2 text-sm font-black">{maintenanceStatus?.tsdb_backend ?? "N/A"}</div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                      {maintenanceStatus?.tsdb_status ?? "unknown"} / {maintenanceStatus?.tsdb_samples ?? "N/A"} samples
                    </div>
                    <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                      retention {maintenanceStatus?.tsdb_retention_days ?? "N/A"}d
                    </div>
                  </div>
                  <MaintenanceCapability label="备份下载" enabled={Boolean(maintenanceStatus?.backup_supported)} />
                  <MaintenanceCapability label="完整归档" enabled={Boolean(maintenanceStatus?.archive_supported)} />
                  <MaintenanceCapability label="备份恢复" enabled={Boolean(maintenanceStatus?.restore_supported)} />
                  <MaintenanceCapability label="SQLite VACUUM" enabled={Boolean(maintenanceStatus?.vacuum_supported)} />
                  <MaintenanceCapability label="TSDB compact" enabled={Boolean(maintenanceStatus?.tsdb_compact_supported)} />
                  <MaintenanceCapability label="TSDB retention" enabled={Boolean(maintenanceStatus?.tsdb_retention_configurable)} />
                </div>
            <div className="flex flex-wrap gap-2 xl:justify-end">
              <input
                className={`${inputClass} w-32`}
                value={tsdbRetentionDraft}
                onChange={(event) => setTsdbRetentionDraft(event.target.value)}
                inputMode="numeric"
                aria-label="TSDB retention days"
                disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
              />
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void updateTsdbRetention()}
                disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
              >
                保存 retention
              </button>
              <button
                type="button"
                className={buttonClass(maintenanceStatus?.backup_supported ? "primary" : "secondary")}
                disabled={!maintenanceStatus?.backup_supported || maintenanceLoading}
                onClick={() => void downloadMaintenanceExport("backup")}
              >
                下载备份
              </button>
              <button
                type="button"
                className={buttonClass(maintenanceStatus?.archive_supported ? "primary" : "secondary")}
                disabled={!maintenanceStatus?.archive_supported || maintenanceLoading}
                onClick={() => void downloadMaintenanceExport("archive")}
              >
                下载归档
              </button>
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void runSqliteVacuum()}
                disabled={!maintenanceStatus?.vacuum_supported || maintenanceLoading}
                >
                  执行 VACUUM
                </button>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void runTsdbCompact()}
                  disabled={!maintenanceStatus?.tsdb_compact_supported || maintenanceLoading}
                >
                  执行 TSDB compact
                </button>
              </div>
            </div>
          <div className="mt-4 grid gap-3 border-t-2 border-black pt-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
            <Field label="SQLite 备份文件">
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
                校验备份
              </button>
              <button
                type="button"
                className={buttonClass("danger")}
                onClick={() => void restoreSqliteBackup(false)}
                disabled={!maintenanceStatus?.restore_supported || !restoreFile || maintenanceLoading}
              >
                恢复备份
              </button>
            </div>
          </div>
          {restoreResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone={restoreResult.restored ? "green" : "blue"}>{restoreResult.restored ? "已恢复" : "已校验"}</StatusBadge>
              <span>表：{restoreResult.table_count}</span>
              <span>行：{restoreResult.row_count}</span>
              <span>user_version：{restoreResult.user_version}</span>
            </div>
          ) : null}
          {tsdbCompactResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone="green">TSDB compact</StatusBadge>
              <span>移除：{tsdbCompactResult.removed_samples}</span>
              <span>之前：{tsdbCompactResult.samples_before ?? "N/A"}</span>
              <span>之后：{tsdbCompactResult.samples_after ?? "N/A"}</span>
            </div>
          ) : null}
          {tsdbRetentionResult ? (
            <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
              <StatusBadge tone="green">TSDB retention</StatusBadge>
              <span>天数：{tsdbRetentionResult.retention_days}</span>
              <span>之前：{tsdbRetentionResult.samples_before ?? "N/A"}</span>
              <span>之后：{tsdbRetentionResult.samples_after ?? "N/A"}</span>
            </div>
          ) : null}
          <p className="mt-3 text-xs font-bold text-[var(--text-muted)]">
            恢复会先执行 SQLite 完整性、核心表和列兼容检查；TSDB compact 会清理超出 retention 窗口的历史样本。
          </p>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">GeoIP</h2>
              <div className="mt-2 flex flex-wrap gap-2">
                <StatusBadge tone="blue">默认 {systemSettings?.geoip_provider ?? "empty"}</StatusBadge>
                <StatusBadge tone={systemSettings?.geoip_ipinfo_token_configured ? "green" : "gray"}>
                  ipinfo token {systemSettings?.geoip_ipinfo_token_configured ? "已配置" : "未配置"}
                </StatusBadge>
                <StatusBadge tone={geoIpMmdbStatus?.configured ? "green" : "gray"}>
                  MMDB {geoIpMmdbStatus?.configured ? geoIpMmdbStatus.database_type || "ready" : "未配置"}
                </StatusBadge>
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
                {settingsLoading ? "保存中" : "保存默认"}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void updateGeoIpDatabase()} disabled={geoIpLoading}>
                MMDB 更新
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={() => void loadGeoIpStatus()} disabled={geoIpLoading}>
                MMDB 状态
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
                  IP 变化通知
                </label>
                <Field label="通知级别">
                  <select className={inputClass} value={geoIpIpChange.severity} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, severity: event.target.value }))}>
                    <option value="info">info</option>
                    <option value="warning">warning</option>
                    <option value="error">error</option>
                    <option value="critical">critical</option>
                  </select>
                </Field>
                <Field label="通知组">
                  <select className={inputClass} value={geoIpIpChange.notification_group_id} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, notification_group_id: event.target.value }))}>
                    <option value="">全部通知渠道</option>
                    {notificationGroups.map((group) => (
                      <option key={group.id} value={group.id}>{group.name}</option>
                    ))}
                  </select>
                </Field>
                <Field label="服务器范围">
                  <input className={inputClass} value={geoIpIpChange.server_ids} onChange={(event) => setGeoIpIpChange((current) => ({ ...current, server_ids: event.target.value }))} placeholder="server id, server id" />
                </Field>
              </div>
              <button type="button" className={buttonClass("primary")} onClick={() => void testGeoIp()} disabled={geoIpLoading}>
                {geoIpLoading ? "查询中" : "测试 GeoIP"}
              </button>
              <div className="border-t-2 border-black pt-3">
                <div className="grid gap-3">
                  <Field label="MMDB URL">
                    <input className={inputClass} value={geoIpMmdbUrl} onChange={(event) => setGeoIpMmdbUrl(event.target.value)} placeholder="https://example.com/GeoLite2-City.mmdb" />
                  </Field>
                  <Field label="MMDB 路径">
                    <input className={inputClass} value={geoIpMmdbPath} onChange={(event) => setGeoIpMmdbPath(event.target.value)} placeholder="/srv/geoip/GeoLite2-City.mmdb" />
                  </Field>
                  <Field label="MMDB 文件">
                    <input className={inputClass} type="file" accept=".mmdb" onChange={(event) => setGeoIpMmdbFile(event.target.files?.[0] ?? null)} />
                  </Field>
                  {geoIpMmdbFile ? <div className="break-all text-xs font-black text-[var(--text-muted)]">{geoIpMmdbFile.name}</div> : null}
                  <button type="button" className={buttonClass("secondary")} onClick={() => void uploadGeoIpDatabase()} disabled={geoIpLoading || !geoIpMmdbFile}>
                    上传 MMDB
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
                    <div>国家：{geoIpResult.country || "N/A"}</div>
                    <div>地区：{geoIpResult.region || "N/A"}</div>
                    <div>城市：{geoIpResult.city || "N/A"}</div>
                    <div>时区：{geoIpResult.timezone || "N/A"}</div>
                    <div>纬度：{geoIpResult.latitude ?? "N/A"}</div>
                    <div>经度：{geoIpResult.longitude ?? "N/A"}</div>
                    <div className="break-words md:col-span-2">ISP：{geoIpResult.isp || "N/A"}</div>
                    <div className="break-words md:col-span-2">组织：{geoIpResult.organization || "N/A"}</div>
                  </div>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all bg-black p-3 font-mono text-xs text-green-300">
                    {JSON.stringify(geoIpResult.raw ?? geoIpResult, null, 2)}
                  </pre>
                </div>
              ) : (
                <p className="text-sm font-bold text-[var(--text-muted)]">尚未执行 GeoIP 查询。</p>
              )}
              {geoIpMmdbStatus ? (
                <div className="mt-4 border-t-2 border-black pt-3 text-sm font-bold">
                  <div className="grid gap-2 md:grid-cols-2">
                    <div>MMDB：{geoIpMmdbStatus.configured ? "已配置" : "未配置"}</div>
                    <div>类型：{geoIpMmdbStatus.database_type || "N/A"}</div>
                    <div>构建：{formatDate(geoIpMmdbStatus.build_at)}</div>
                    <div>更新：{formatDate(geoIpMmdbStatus.modified_at)}</div>
                    <div>大小：{geoIpMmdbStatus.size_bytes ?? "N/A"} bytes</div>
                    <div>IP：{geoIpMmdbStatus.ip_version || "N/A"}</div>
                    <div className="break-all md:col-span-2">路径：{geoIpMmdbStatus.path}</div>
                    {geoIpMmdbStatus.error ? <div className="break-all text-[var(--btn-bg)] md:col-span-2">错误：{geoIpMmdbStatus.error}</div> : null}
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </BrutalCard>

        <BrutalCard className="mb-6">
          <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-xl font-black uppercase">DDNS</h2>
              <div className="mt-2 flex flex-wrap gap-2">
                <StatusBadge tone={ddnsResolverUrl.trim() ? "green" : "gray"}>
                  resolver {ddnsResolverUrl.trim() ? "已配置" : "system"}
                </StatusBadge>
              </div>
            </div>
            <button type="button" className={buttonClass("secondary")} onClick={() => void saveGeoIpSettings()} disabled={settingsLoading}>
              {settingsLoading ? "保存中" : "保存 DDNS"}
            </button>
          </div>
          <Field label="DoH Resolver URL">
            <input className={inputClass} value={ddnsResolverUrl} onChange={(event) => setDdnsResolverUrl(event.target.value)} placeholder="https://dns.google/resolve" />
          </Field>
        </BrutalCard>

        <div className="grid gap-6 lg:grid-cols-2">
          <BrutalCard accent>
            <h2 className="mb-4 text-xl font-black uppercase">创建 PAT</h2>
            <form onSubmit={createToken} className="space-y-4">
              <Field label="名称"><input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} required /></Field>
              <Field label="Scope"><textarea className={`${textareaClass} min-h-24`} value={scopes} onChange={(e) => setScopes(e.target.value)} /></Field>
              <Field label="过期时间"><input className={inputClass} type="datetime-local" value={patExpiresAt} onChange={(e) => setPatExpiresAt(e.target.value)} required /></Field>
              <Field label="Server Allowlist"><textarea className={`${textareaClass} min-h-20`} value={patServerIds} onChange={(e) => setPatServerIds(e.target.value)} placeholder="server id, server id" /></Field>
              <button className={buttonClass("primary")}>创建令牌</button>
            </form>
            {createdToken ? (
              <div className="mt-5 border-2 border-black bg-black p-3 font-mono text-xs text-green-300">
                {createdToken}
              </div>
            ) : null}
          </BrutalCard>

          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">已有令牌</h2>
            <div className="grid gap-3">
              {tokens.length === 0 ? (
                <p className="text-sm font-bold text-[var(--text-muted)]">暂无令牌。</p>
              ) : (
                tokens.map((token) => {
                  const serverIds = token.server_ids ?? [];
                  return (
                    <div key={token.id} className="border-2 border-black bg-[var(--accent-bg)] p-3 text-sm font-bold">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="break-words text-base font-black">{token.name || "未命名令牌"}</div>
                          <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-muted)]">{token.id}</div>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          {serverIds.length === 0 ? (
                            <StatusBadge tone="red">全局 PAT</StatusBadge>
                          ) : (
                            <StatusBadge tone="green">{serverIds.length} 台服务器</StatusBadge>
                          )}
                        </div>
                      </div>
                      <div className="mt-3 grid gap-2 text-xs text-[var(--text-muted)]">
                        <div>Scope：{token.scopes?.join(" ") || "N/A"}</div>
                        <div>过期：{formatDate(token.expires_at)}</div>
                        <div>最近使用：{formatDate(token.last_used_at)}</div>
                        {serverIds.length > 0 ? <div className="break-all">Server allowlist：{serverIds.join(", ")}</div> : null}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </BrutalCard>
        </div>
      </PageShell>
    </div>
  );
}

function MaintenanceCapability({
  label,
  enabled,
}: {
  label: string;
  enabled: boolean;
}) {
  return (
    <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2">
        <StatusBadge tone={enabled ? "green" : "gray"}>{enabled ? "支持" : "未支持"}</StatusBadge>
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

function themeTargetLabel(target: string): string {
  if (target === "public") return "公开";
  if (target === "dashboard") return "控制面";
  if (target === "both") return "两端";
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
