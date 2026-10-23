"use client";

import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  KpiCard,
  PageHeader,
  PageShell,
  StatusBadge,
  TableSkeleton,
  buttonClass,
  compactId,
  formatDate,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type DdnsConfig, type JsonObject } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useServerData } from "@/app/components/ServerDataProvider";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";

type IpMode = "ipv4" | "ipv6" | "dual";
type IpModeFilter = "ALL" | IpMode;

interface DdnsHistory {
  id?: string | undefined;
  success?: boolean | undefined;
  message?: string | undefined;
  created_at?: string | undefined;
}

interface DdnsFormState {
  agent_id: string;
  name: string;
  provider: string;
  ip_mode: IpMode;
  domain: string;
  record_id: string;
  zone_id: string;
  api_token: string;
  api_key: string;
  api_secret: string;
  webhook_url: string;
  enabled: boolean;
}

const initialFormState: DdnsFormState = {
  agent_id: "",
  name: "",
  provider: "webhook",
  ip_mode: "ipv4",
  domain: "",
  record_id: "",
  zone_id: "",
  api_token: "",
  api_key: "",
  api_secret: "",
  webhook_url: "",
  enabled: true,
};

const IP_MODE_FILTERS: readonly IpModeFilter[] = ["ALL", "ipv4", "ipv6", "dual"];

function resolveConfigIpMode(config: DdnsConfig, localModes: Record<string, IpMode>): IpMode {
  const raw = (config as unknown as Record<string, unknown>)["ip_mode"];
  if (raw === "ipv4" || raw === "ipv6" || raw === "dual") {
    return raw;
  }
  const override = localModes[config.id];
  if (override) return override;
  const ip = (config.last_applied_ip || config.current_ip || "").trim();
  if (ip.includes(",") || (ip.includes(".") && ip.includes(":"))) {
    return "dual";
  }
  if (ip.includes(":")) {
    return "ipv6";
  }
  return "ipv4";
}

function ipModeTone(mode: IpMode): "blue" | "pink" | "green" {
  if (mode === "ipv6") return "pink";
  if (mode === "dual") return "green";
  return "blue";
}

function providerTone(provider: string): "blue" | "yellow" | "pink" | "gray" {
  const normalized = provider.toLowerCase();
  if (normalized === "cloudflare") return "yellow";
  if (normalized === "aliyun") return "blue";
  if (normalized === "webhook") return "pink";
  return "gray";
}

export default function DdnsPage() {
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isEn = locale === "en-US";
  const {
    servers,
    error: serverLoadError,
    revalidate: revalidateServers,
  } = useServerData();

  const [configs, setConfigs] = useState<DdnsConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [history, setHistory] = useState<DdnsHistory[]>([]);
  const [historyTargetName, setHistoryTargetName] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<{ enabled?: boolean | undefined } | null>(null);

  const [drawerOpen, setDrawerOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [ipModeFilter, setIpModeFilter] = useState<IpModeFilter>("ALL");
  const [localIpModes, setLocalIpModes] = useState<Record<string, IpMode>>({});

  const [actionBusy, runAction] = useBusy();
  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();
  const [form, setForm] = useState<DdnsFormState>(initialFormState);

  const load = useCallback(async () => {
    setLoading(true);
    const response = await apiClient.listDdnsConfigs();
    setLoading(false);
    if (response.success && response.data) {
      setConfigs(response.data.configs ?? []);
    } else {
      setError(responseError(response));
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
  }, [load]);

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  const serverNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const server of servers) {
      if (server.id) {
        map.set(server.id, server.name || server.id);
      }
    }
    return map;
  }, [servers]);

  const kpiStats = useMemo(() => {
    const total = configs.length;
    const enabledCount = configs.filter((c) => c.enabled !== false).length;
    const syncedCount = configs.filter((c) =>
      Boolean((c.last_applied_ip && c.last_applied_ip.trim()) || (c.current_ip && c.current_ip.trim()) || c.last_applied_at),
    ).length;
    const providers = new Set(configs.map((c) => (c.provider || "webhook").toLowerCase()));
    return {
      total,
      enabledCount,
      syncedCount,
      providerCount: providers.size,
      providerList: Array.from(providers),
    };
  }, [configs]);

  const deferredQuery = useDeferredValue(searchQuery);

  const filteredConfigs = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return configs.filter((config) => {
      const mode = resolveConfigIpMode(config, localIpModes);
      if (ipModeFilter !== "ALL" && mode !== ipModeFilter) {
        return false;
      }
      if (!needle) return true;
      const boundServerName = config.agent_id ? (serverNameById.get(config.agent_id) ?? "") : "";
      return [
        config.name,
        config.domain,
        config.provider,
        mode,
        config.agent_id ?? "",
        boundServerName,
        config.last_applied_ip ?? "",
        config.current_ip ?? "",
      ].some((field) => field.toLowerCase().includes(needle));
    });
  }, [configs, deferredQuery, ipModeFilter, localIpModes, serverNameById]);

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
      setError(copy.ddnsPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

  function openCreateDrawer() {
    setEditingId(null);
    setForm({
      ...initialFormState,
      agent_id: servers[0]?.id ?? "",
    });
    setDrawerOpen(true);
  }

  function toggleCreateDrawer() {
    if (drawerOpen) {
      setDrawerOpen(false);
      setEditingId(null);
    } else {
      openCreateDrawer();
    }
  }

  function openEditDrawer(config: DdnsConfig) {
    setEditingId(config.id);
    setForm({
      agent_id: config.agent_id ?? "",
      name: config.name || "",
      provider: config.provider || "webhook",
      ip_mode: resolveConfigIpMode(config, localIpModes),
      domain: config.domain || "",
      record_id: config.record_id ?? "",
      zone_id: config.zone_id ?? "",
      api_token: "",
      api_key: "",
      api_secret: "",
      webhook_url: "",
      enabled: config.enabled !== false,
    });
    setDrawerOpen(true);
  }

  async function submit() {
    setError(null);
    const payload: JsonObject = {
      name: form.name.trim(),
      provider: form.provider,
      domain: form.domain.trim(),
      agent_id: form.agent_id.trim() || null,
      record_id: form.record_id.trim() || null,
      zone_id: form.zone_id.trim() || null,
      api_token: form.api_token.trim() || null,
      api_key: form.api_key.trim() || null,
      api_secret: form.api_secret.trim() || null,
      webhook_url: form.webhook_url.trim() || null,
      enabled: form.enabled,
    };
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;

    const response = editingId
      ? await apiClient.updateDdnsConfig(editingId, payload, totpCode)
      : await apiClient.createDdnsConfig(payload, totpCode);

    if (response.success) {
      const createdOrUpdatedId =
        editingId ??
        ((response.data?.["config"] as { id?: string } | undefined)?.id ?? "");
      if (createdOrUpdatedId) {
        setLocalIpModes((prev) => ({ ...prev, [createdOrUpdatedId]: form.ip_mode }));
      }
      showToast(copy.ddnsPage.configCreated);
      setDrawerOpen(false);
      setEditingId(null);
      setForm(initialFormState);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function reload() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.reloadDdnsProviders(totpCode);
    if (response.success) showToast(copy.ddnsPage.providersReloaded);
    else setError(responseError(response));
  }

  async function checkNow() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.checkDdnsNow(totpCode);
    if (response.success) {
      showToast(copy.ddnsPage.checkTriggered);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function showHistory(config: DdnsConfig) {
    const response = await apiClient.listDdnsHistory(config.id);
    if (response.success && response.data) {
      const label = String(config.name || config.domain || config.id);
      setHistoryTargetName(label);
      setHistory((response.data["history"] as DdnsHistory[]) ?? []);
      showToast(copy.ddnsPage.historyLoaded.replace("{name}", label));
    } else {
      setError(responseError(response));
    }
  }

  async function deleteConfig(config: DdnsConfig) {
    if (
      !(await dialogs.confirm({
        message: copy.ddnsPage.deleteConfigConfirm.replace(
          "{name}",
          String(config.name || config.domain || config.id),
        ),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteDdnsConfig(config.id, totpCode);
    if (response.success) {
      showToast(copy.ddnsPage.configDeleted);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.ddnsPage.eyebrow}
          title={copy.ddnsPage.title}
          detail={copy.ddnsPage.detail}
          actions={
            <>
              <button
                type="button"
                className={buttonClass("secondary")}
                disabled={actionBusy}
                onClick={() => void runAction(() => reload())}
              >
                {copy.ddnsPage.reload}
              </button>
              <button
                type="button"
                className={buttonClass("secondary")}
                disabled={actionBusy}
                onClick={() => void runAction(() => checkNow())}
              >
                {copy.ddnsPage.checkNow}
              </button>
              <button
                type="button"
                className={buttonClass(drawerOpen ? "secondary" : "primary")}
                onClick={toggleCreateDrawer}
                aria-expanded={drawerOpen}
              >
                {drawerOpen
                  ? isEn
                    ? "Close Drawer"
                    : "收起抽屉"
                  : "+ 新建 DDNS"}
              </button>
            </>
          }
        />

        {/* Top KPI Strip */}
        <section className="mb-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="DDNS KPIs">
          <KpiCard
            label={isEn ? "Total DDNS Records" : "DDNS 记录总数"}
            value={kpiStats.total}
            hint={isEn ? "Configured dynamic DNS mappings" : "已配置的动态域名解析记录"}
            badge={<StatusBadge tone="blue">{copy.ddnsPage.eyebrow}</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Enabled Records" : "启用记录数"}
            value={kpiStats.enabledCount}
            hint={
              kpiStats.total > 0
                ? `${Math.round((kpiStats.enabledCount / kpiStats.total) * 100)}% ${isEn ? "active" : "活跃中"}`
                : isEn
                  ? "No active records"
                  : "暂无活跃记录"
            }
            badge={
              <StatusBadge tone={kpiStats.enabledCount > 0 ? "green" : "gray"}>
                {copy.ddnsPage.enabled}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "Last Synced Status" : "已同步状态数"}
            value={`${kpiStats.syncedCount} / ${kpiStats.total}`}
            hint={isEn ? "Records with resolved IP synced" : "已完成至少一次 IP 同步的记录"}
            badge={
              <StatusBadge tone={kpiStats.syncedCount > 0 ? "green" : "yellow"}>
                {isEn ? "Synced" : "同步"}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "Providers Count" : "接入服务商数"}
            value={kpiStats.providerCount}
            hint={
              kpiStats.providerList.length > 0
                ? kpiStats.providerList.join(" · ")
                : "cloudflare · aliyun · webhook"
            }
            badge={<StatusBadge tone="pink">{copy.ddnsPage.colProvider}</StatusBadge>}
          />
        </section>

        {/* Unified Control Bar */}
        <BrutalCard className="mb-5 p-4">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="flex flex-1 flex-col gap-3 sm:flex-row sm:items-center">
              <div className="relative min-w-0 flex-1">
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className={`${inputClass} mt-0`}
                  placeholder={
                    isEn
                      ? "Search domain, provider, server, or synced IP..."
                      : "搜索域名、Provider、绑定节点或同步 IP..."
                  }
                  aria-label={copy.common.search}
                />
              </div>

              {/* IP Mode Filter Pills */}
              <div
                className="inline-flex flex-wrap items-center gap-1 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                role="group"
                aria-label="IP Mode Filter"
              >
                {IP_MODE_FILTERS.map((mode) => {
                  const active = ipModeFilter === mode;
                  return (
                    <button
                      key={mode}
                      type="button"
                      onClick={() => setIpModeFilter(mode)}
                      className={`rounded-lg px-2.5 py-1 font-mono text-xs font-medium transition ${
                        active
                          ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {mode}
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                className={buttonClass(drawerOpen ? "secondary" : "primary")}
                onClick={toggleCreateDrawer}
              >
                {drawerOpen
                  ? isEn
                    ? "Cancel"
                    : "取消新建"
                  : "+ 新建 DDNS"}
              </button>
            </div>
          </div>
        </BrutalCard>

        <div className="mb-5 space-y-3">
          <InlineError message={error ?? serverLoadError} />
        </div>

        {/* Collapsible Creation / Edit Drawer */}
        {drawerOpen ? (
          <BrutalCard accent className="mb-6 border-[var(--border-strong)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3">
              <div>
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {editingId
                    ? isEn
                      ? "Edit DDNS Record"
                      : "编辑 DDNS 记录"
                    : copy.ddnsPage.modalTitle}
                </h2>
                <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                  {isEn
                    ? "Bind a domain record to an agent node and configure provider credentials."
                    : "将动态域名记录绑定到监控节点并配置 DNS 服务商凭据。"}
                </p>
              </div>
              <button
                type="button"
                className={buttonClass("ghost")}
                onClick={() => {
                  setDrawerOpen(false);
                  setEditingId(null);
                }}
              >
                {copy.common.close}
              </button>
            </div>

            <form
              onSubmit={(event) => {
                event.preventDefault();
                void runForm(() => submit());
              }}
              className="space-y-4"
            >
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <Field label={copy.ddnsPage.fieldName}>
                  <input
                    className={inputClass}
                    value={form.name}
                    onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                    placeholder="edge-gateway-ddns"
                    required
                  />
                </Field>
                <Field label={copy.ddnsPage.fieldDomain}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.domain}
                    onChange={(e) => setForm((f) => ({ ...f, domain: e.target.value }))}
                    placeholder="ddns.example.com"
                    required
                  />
                </Field>
                <Field label={copy.ddnsPage.fieldProvider}>
                  <select
                    className={selectClass}
                    value={form.provider}
                    onChange={(e) => setForm((f) => ({ ...f, provider: e.target.value }))}
                  >
                    <option value="webhook">webhook</option>
                    <option value="cloudflare">cloudflare</option>
                    <option value="aliyun">aliyun</option>
                  </select>
                </Field>
                <Field label={isEn ? "IP Mode" : "IP 模式"}>
                  <select
                    className={selectClass}
                    value={form.ip_mode}
                    onChange={(e) => setForm((f) => ({ ...f, ip_mode: e.target.value as IpMode }))}
                  >
                    <option value="ipv4">ipv4 (A)</option>
                    <option value="ipv6">ipv6 (AAAA)</option>
                    <option value="dual">dual (A + AAAA)</option>
                  </select>
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.ddnsPage.fieldAgentId}>
                  {servers.length > 0 ? (
                    <select
                      className={selectClass}
                      value={form.agent_id}
                      onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}
                    >
                      <option value="">{isEn ? "Unbound / Global Resolver" : "未绑定（使用控制面出口）"}</option>
                      {servers.map((server) => (
                        <option key={server.id} value={server.id}>
                          {server.name} ({compactId(server.id)})
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className={`${inputClass} font-mono`}
                      value={form.agent_id}
                      onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}
                      placeholder={isEn ? "Optional Agent UUID" : "可选 Agent UUID"}
                    />
                  )}
                </Field>
                <Field label={copy.ddnsPage.fieldRecordId}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.record_id}
                    onChange={(e) => setForm((f) => ({ ...f, record_id: e.target.value }))}
                    placeholder="record_id"
                  />
                </Field>
                <Field label={copy.ddnsPage.fieldZoneId}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.zone_id}
                    onChange={(e) => setForm((f) => ({ ...f, zone_id: e.target.value }))}
                    placeholder="zone_id"
                  />
                </Field>
              </div>

              {form.provider === "webhook" ? (
                <Field label={copy.ddnsPage.fieldWebhookUrl}>
                  <textarea
                    className={`${textareaClass} min-h-20`}
                    value={form.webhook_url}
                    onChange={(e) => setForm((f) => ({ ...f, webhook_url: e.target.value }))}
                    placeholder="https://example.com/update?ip={{ip}}&hostname={{hostname}}"
                  />
                </Field>
              ) : (
                <div className="grid gap-4 sm:grid-cols-3">
                  <Field label={copy.ddnsPage.fieldApiToken}>
                    <input
                      type="password"
                      className={`${inputClass} font-mono`}
                      value={form.api_token}
                      onChange={(e) => setForm((f) => ({ ...f, api_token: e.target.value }))}
                    />
                  </Field>
                  <Field label={copy.ddnsPage.fieldApiKey}>
                    <input
                      className={`${inputClass} font-mono`}
                      value={form.api_key}
                      onChange={(e) => setForm((f) => ({ ...f, api_key: e.target.value }))}
                    />
                  </Field>
                  <Field label={copy.ddnsPage.fieldApiSecret}>
                    <input
                      type="password"
                      className={`${inputClass} font-mono`}
                      value={form.api_secret}
                      onChange={(e) => setForm((f) => ({ ...f, api_secret: e.target.value }))}
                    />
                  </Field>
                </div>
              )}

              <div className="flex flex-wrap items-center justify-between gap-3 pt-2">
                <label className="inline-flex cursor-pointer items-center gap-2 text-xs font-medium text-[var(--text-main)]">
                  <input
                    type="checkbox"
                    checked={form.enabled}
                    onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
                  />
                  <span>{copy.ddnsPage.enabled}</span>
                </label>

                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => {
                      setDrawerOpen(false);
                      setEditingId(null);
                    }}
                  >
                    {copy.common.cancel}
                  </button>
                  <button type="submit" disabled={formBusy} className={buttonClass("primary")}>
                    {copy.ddnsPage.saveConfig}
                  </button>
                </div>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* Visual DNS Mapping Table */}
        {loading && configs.length === 0 ? (
          <TableSkeleton columns={6} rows={4} />
        ) : filteredConfigs.length === 0 ? (
          <EmptyState
            title={configs.length === 0 ? copy.ddnsPage.emptyTitle : copy.common.noMatches}
            detail={
              configs.length === 0
                ? isEn
                  ? "Click '+ 新建 DDNS' above to create your first dynamic DNS mapping."
                  : "点击右上角「+ 新建 DDNS」创建第一条动态域名解析记录。"
                : isEn
                  ? "Try clearing the search filter or switching IP mode."
                  : "请尝试调整搜索关键词或切换 IP 模式筛选。"
            }
            action={
              configs.length === 0 ? (
                <button type="button" className={buttonClass("primary")} onClick={openCreateDrawer}>
                  + 新建 DDNS
                </button>
              ) : undefined
            }
          />
        ) : (
          <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
            <table className="w-full min-w-[60rem]">
              <thead>
                <tr>
                  <th className={thClass}>{copy.ddnsPage.colDomain}</th>
                  <th className={thClass}>{copy.ddnsPage.colProvider} / IP</th>
                  <th className={thClass}>{isEn ? "Bound Server" : "绑定节点"}</th>
                  <th className={thClass}>{isEn ? "Last Synced IP" : "最近同步 IP"}</th>
                  <th className={thClass}>{copy.ddnsPage.colStatus}</th>
                  <th className={thClass}>{copy.ddnsPage.colActions}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-color)]">
                {filteredConfigs.map((config) => {
                  const ipMode = resolveConfigIpMode(config, localIpModes);
                  const syncedIp = config.last_applied_ip || config.current_ip;
                  const boundServerName = config.agent_id
                    ? serverNameById.get(config.agent_id) ?? compactId(config.agent_id)
                    : null;

                  return (
                    <tr key={config.id} className="transition-colors hover:bg-[var(--bg-subtle)]">
                      {/* Domain Badge & Record Name */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-1">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-subtle)] px-2.5 py-0.5 font-mono text-xs font-semibold text-[var(--text-main)]">
                              <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-color)]" />
                              {config.domain || "-"}
                            </span>
                          </div>
                          <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--text-muted)]">
                            <span className="font-medium text-[var(--text-main)]">
                              {config.name || compactId(config.id)}
                            </span>
                            {config.zone_id ? (
                              <span className="font-mono text-[11px]">Zone: {compactId(config.zone_id)}</span>
                            ) : null}
                          </div>
                        </div>
                      </td>

                      {/* Provider + IP Mode Pill */}
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <StatusBadge tone={providerTone(config.provider || "webhook")}>
                            {config.provider || "webhook"}
                          </StatusBadge>
                          <StatusBadge tone={ipModeTone(ipMode)}>
                            {ipMode.toUpperCase()}
                          </StatusBadge>
                        </div>
                      </td>

                      {/* Bound Server Name */}
                      <td className={tdClass}>
                        {boundServerName ? (
                          <div className="flex flex-col">
                            <span className="text-xs font-semibold text-[var(--text-main)]">
                              {boundServerName}
                            </span>
                            <span className="font-mono text-[11px] text-[var(--text-muted)]">
                              {compactId(config.agent_id)}
                            </span>
                          </div>
                        ) : (
                          <span className="inline-flex items-center rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-xs text-[var(--text-muted)]">
                            {isEn ? "Control Plane" : "控制面默认"}
                          </span>
                        )}
                      </td>

                      {/* Last Synced IP in Monospace */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-0.5">
                          <span className="inline-flex w-fit items-center rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 font-mono text-xs font-medium text-[var(--text-main)]">
                            {syncedIp || "-"}
                          </span>
                          {config.last_applied_at ? (
                            <span className="font-mono text-[11px] text-[var(--text-muted)]">
                              {formatDate(config.last_applied_at)}
                            </span>
                          ) : null}
                        </div>
                      </td>

                      {/* Status */}
                      <td className={tdClass}>
                        <StatusBadge tone={config.enabled === false ? "gray" : "green"}>
                          {config.enabled === false ? copy.ddnsPage.disabled : copy.ddnsPage.enabled}
                        </StatusBadge>
                      </td>

                      {/* Action Buttons */}
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <button
                            type="button"
                            className={buttonClass("secondary")}
                            onClick={() => void showHistory(config)}
                          >
                            {copy.ddnsPage.history}
                          </button>
                          <button
                            type="button"
                            className={buttonClass("secondary")}
                            onClick={() => openEditDrawer(config)}
                          >
                            {isEn ? "Edit" : "编辑"}
                          </button>
                          <button
                            type="button"
                            className={buttonClass("danger")}
                            disabled={rowBusy}
                            onClick={() => void runRow(() => deleteConfig(config))}
                          >
                            {copy.ddnsPage.delete}
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}

        {/* Sync History Section */}
        {history.length > 0 ? (
          <section className="mt-6">
            <div className="mb-3 flex items-center justify-between gap-2">
              <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                {copy.ddnsPage.historyHeading}
                {historyTargetName ? ` · ${historyTargetName}` : ""}
              </h2>
              <button
                type="button"
                className={buttonClass("ghost")}
                onClick={() => {
                  setHistory([]);
                  setHistoryTargetName("");
                }}
              >
                {copy.common.close}
              </button>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              {history.map((item, index) => (
                <div
                  key={item.id || index}
                  className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3.5 shadow-[var(--shadow-xs)]"
                >
                  <div className="flex items-center justify-between gap-3">
                    <StatusBadge tone={item.success ? "green" : "red"}>
                      {item.success ? copy.ddnsPage.success : copy.ddnsPage.failed}
                    </StatusBadge>
                    <span className="font-mono text-xs text-[var(--text-muted)]">
                      {formatDate(item.created_at)}
                    </span>
                  </div>
                  <p className="mt-2 break-all font-mono text-xs text-[var(--text-muted)]">
                    {item.message || "-"}
                  </p>
                </div>
              ))}
            </div>
          </section>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}
