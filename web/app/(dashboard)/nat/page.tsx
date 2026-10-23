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
  inputClass,
  responseError,
  selectClass,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type NatMapping, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useServerData } from "@/app/components/ServerDataProvider";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";

type ProtocolFilter = "ALL" | "tcp" | "udp";

const PROTOCOL_FILTERS: readonly ProtocolFilter[] = ["ALL", "tcp", "udp"];

interface NatFormState {
  agent_id: string;
  description: string;
  protocol: "tcp" | "udp";
  local_host: string;
  local_port: string;
  public_port: string;
  enabled: boolean;
  allowed_sources: string;
  max_active_tunnels: string;
  idle_timeout_seconds: string;
  max_bytes_per_tunnel: string;
  max_bandwidth_bytes_per_second: string;
  rate_limit_window_seconds: string;
  max_connections_per_window: string;
  max_bytes_per_window: string;
}

const initialNatForm: NatFormState = {
  agent_id: "",
  description: "",
  protocol: "tcp",
  local_host: "127.0.0.1",
  local_port: "80",
  public_port: "10080",
  enabled: true,
  allowed_sources: "",
  max_active_tunnels: "",
  idle_timeout_seconds: "",
  max_bytes_per_tunnel: "",
  max_bandwidth_bytes_per_second: "",
  rate_limit_window_seconds: "",
  max_connections_per_window: "",
  max_bytes_per_window: "",
};

function normalizeProtocol(value?: string | null): "tcp" | "udp" {
  return value?.toLowerCase() === "udp" ? "udp" : "tcp";
}

export default function NatPage() {
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isEn = locale === "en-US";
  const {
    servers,
    error: serverLoadError,
    revalidate: revalidateServers,
  } = useServerData();

  const [mappings, setMappings] = useState<NatMapping[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);

  const [drawerOpen, setDrawerOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [protocolFilter, setProtocolFilter] = useState<ProtocolFilter>("ALL");
  const [serverFilter, setServerFilter] = useState<string>("all");

  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();
  const [form, setForm] = useState<NatFormState>(initialNatForm);

  const load = useCallback(async () => {
    setLoading(true);
    const mappingResponse = await apiClient.listNatMappings();
    setLoading(false);
    if (mappingResponse.success && mappingResponse.data) {
      setMappings(mappingResponse.data.mappings ?? []);
    } else {
      setError(responseError(mappingResponse));
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
  }, [load]);

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- picker default derived from the shared server list, kept out of render
    setForm((current) => {
      if (current.agent_id) return current;
      const fallback = servers[0]?.id ?? "";
      return fallback ? { ...current, agent_id: fallback } : current;
    });
  }, [servers]);

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
    const total = mappings.length;
    const activeCount = mappings.filter((m) => m.enabled !== false).length;
    const tcpCount = mappings.filter((m) => normalizeProtocol(m.protocol) === "tcp").length;
    const udpCount = mappings.filter((m) => normalizeProtocol(m.protocol) === "udp").length;
    const boundNodes = new Set(mappings.map((m) => m.agent_id).filter(Boolean)).size;
    return { total, activeCount, tcpCount, udpCount, boundNodes };
  }, [mappings]);

  const deferredQuery = useDeferredValue(searchQuery);

  const filteredMappings = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return mappings.filter((mapping) => {
      const proto = normalizeProtocol(mapping.protocol);
      if (protocolFilter !== "ALL" && proto !== protocolFilter) {
        return false;
      }
      if (serverFilter !== "all" && mapping.agent_id !== serverFilter) {
        return false;
      }
      if (!needle) return true;
      const serverName = serverNameById.get(mapping.agent_id) ?? "";
      return [
        mapping.description ?? "",
        mapping.id,
        mapping.agent_id,
        serverName,
        proto,
        String(mapping.public_port ?? ""),
        mapping.local_host ?? "",
        String(mapping.local_port ?? ""),
        mapping.allowed_sources ?? "",
      ].some((field) => field.toLowerCase().includes(needle));
    });
  }, [deferredQuery, mappings, protocolFilter, serverFilter, serverNameById]);

  function openCreateDrawer() {
    setEditingId(null);
    setForm({
      ...initialNatForm,
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

  function openEditDrawer(mapping: NatMapping) {
    setEditingId(mapping.id);
    setForm({
      agent_id: mapping.agent_id,
      description: mapping.description ?? "",
      protocol: normalizeProtocol(mapping.protocol),
      local_host: mapping.local_host ?? "127.0.0.1",
      local_port: String(mapping.local_port ?? 80),
      public_port: String(mapping.public_port ?? 10080),
      enabled: mapping.enabled !== false,
      allowed_sources: mapping.allowed_sources ?? "",
      max_active_tunnels:
        mapping.max_active_tunnels !== undefined && mapping.max_active_tunnels !== null
          ? String(mapping.max_active_tunnels)
          : "",
      idle_timeout_seconds:
        mapping.idle_timeout_seconds !== undefined && mapping.idle_timeout_seconds !== null
          ? String(mapping.idle_timeout_seconds)
          : "",
      max_bytes_per_tunnel:
        mapping.max_bytes_per_tunnel !== undefined && mapping.max_bytes_per_tunnel !== null
          ? String(mapping.max_bytes_per_tunnel)
          : "",
      max_bandwidth_bytes_per_second:
        mapping.max_bandwidth_bytes_per_second !== undefined &&
        mapping.max_bandwidth_bytes_per_second !== null
          ? String(mapping.max_bandwidth_bytes_per_second)
          : "",
      rate_limit_window_seconds:
        mapping.rate_limit_window_seconds !== undefined &&
        mapping.rate_limit_window_seconds !== null
          ? String(mapping.rate_limit_window_seconds)
          : "",
      max_connections_per_window:
        mapping.max_connections_per_window !== undefined &&
        mapping.max_connections_per_window !== null
          ? String(mapping.max_connections_per_window)
          : "",
      max_bytes_per_window:
        mapping.max_bytes_per_window !== undefined && mapping.max_bytes_per_window !== null
          ? String(mapping.max_bytes_per_window)
          : "",
    });
    setDrawerOpen(true);
  }

  async function submit() {
    setError(null);
    const payload: JsonObject = {
      agent_id: form.agent_id,
      protocol: form.protocol,
      local_host: form.local_host.trim() || "127.0.0.1",
      local_port: Number(form.local_port),
      public_port: Number(form.public_port),
      enabled: form.enabled,
      description: form.description.trim() || null,
      allowed_sources: form.allowed_sources.trim() || null,
      max_active_tunnels: form.max_active_tunnels.trim() ? Number(form.max_active_tunnels) : null,
      idle_timeout_seconds: form.idle_timeout_seconds.trim() ? Number(form.idle_timeout_seconds) : null,
      max_bytes_per_tunnel: form.max_bytes_per_tunnel.trim() ? Number(form.max_bytes_per_tunnel) : null,
      max_bandwidth_bytes_per_second: form.max_bandwidth_bytes_per_second.trim()
        ? Number(form.max_bandwidth_bytes_per_second)
        : null,
      rate_limit_window_seconds: form.rate_limit_window_seconds.trim()
        ? Number(form.rate_limit_window_seconds)
        : null,
      max_connections_per_window: form.max_connections_per_window.trim()
        ? Number(form.max_connections_per_window)
        : null,
      max_bytes_per_window: form.max_bytes_per_window.trim() ? Number(form.max_bytes_per_window) : null,
    };
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = editingId
      ? await apiClient.updateNatMapping(editingId, payload, totpCode)
      : await apiClient.createNatMapping(payload, totpCode);
    if (response.success) {
      showToast(copy.natPage.mappingCreated);
      setDrawerOpen(false);
      setEditingId(null);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteMapping(mapping: NatMapping) {
    if (
      !(await dialogs.confirm({
        message: copy.natPage.deleteMappingConfirm.replace("{name}", String(mapping.description || mapping.id)),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNatMapping(mapping.id, totpCode);
    if (response.success) {
      showToast(copy.natPage.mappingDeleted);
      await load();
    } else {
      setError(responseError(response));
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
      setError(copy.natPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.natPage.eyebrow}
          title={copy.natPage.title}
          detail={copy.natPage.detail}
          actions={
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
                : "+ 新建端口转发"}
            </button>
          }
        />

        {/* Top KPI Strip */}
        <section className="mb-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="NAT KPIs">
          <KpiCard
            label={isEn ? "Total Rules" : "转发规则总数"}
            value={kpiStats.total}
            hint={isEn ? "Configured port forwarding rules" : "已配置的隧道端口映射规则"}
            badge={<StatusBadge tone="blue">{copy.natPage.title}</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Active / Enabled Rules" : "活跃 / 启用规则"}
            value={kpiStats.activeCount}
            hint={
              kpiStats.total > 0
                ? `${Math.round((kpiStats.activeCount / kpiStats.total) * 100)}% ${isEn ? "enabled" : "已启用"}`
                : isEn
                  ? "No active tunnels"
                  : "暂无活跃规则"
            }
            badge={
              <StatusBadge tone={kpiStats.activeCount > 0 ? "green" : "gray"}>
                {copy.natPage.enabled}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "TCP vs UDP" : "TCP / UDP 协议分布"}
            value={`${kpiStats.tcpCount} / ${kpiStats.udpCount}`}
            hint={`TCP: ${kpiStats.tcpCount} · UDP: ${kpiStats.udpCount}`}
            badge={<StatusBadge tone="pink">L4</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Bound Nodes" : "绑定节点数"}
            value={kpiStats.boundNodes}
            hint={
              servers.length > 0
                ? `${kpiStats.boundNodes} / ${servers.length} ${isEn ? "nodes" : "台服务器"}`
                : isEn
                  ? "Target agent nodes"
                  : "关联 Agent 节点"
            }
            badge={<StatusBadge tone="yellow">{copy.natPage.colAgent}</StatusBadge>}
          />
        </section>

        {/* Unified Control Bar */}
        <BrutalCard className="mb-5 p-4">
          <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
            <div className="flex flex-1 flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-center">
              <div className="min-w-[14rem] flex-1">
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className={`${inputClass} mt-0`}
                  placeholder={
                    isEn
                      ? "Search description, port, local host, or node..."
                      : "搜索规则描述、公网端口、内网目标或节点..."
                  }
                  aria-label={copy.common.search}
                />
              </div>

              {/* Protocol Filter Pills */}
              <div
                className="inline-flex items-center gap-1 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                role="group"
                aria-label="Protocol Filter"
              >
                {PROTOCOL_FILTERS.map((proto) => {
                  const active = protocolFilter === proto;
                  return (
                    <button
                      key={proto}
                      type="button"
                      onClick={() => setProtocolFilter(proto)}
                      className={`rounded-lg px-2.5 py-1 font-mono text-xs font-medium transition ${
                        active
                          ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {proto}
                    </button>
                  );
                })}
              </div>

              {/* Server Filter Dropdown */}
              <div className="w-full sm:w-52">
                <select
                  value={serverFilter}
                  onChange={(e) => setServerFilter(e.target.value)}
                  className={`${selectClass} mt-0`}
                  aria-label={copy.natPage.fieldAgent}
                >
                  <option value="all">{isEn ? "All Target Servers" : "全部目标节点"}</option>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name}
                    </option>
                  ))}
                </select>
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
                  : "+ 新建端口转发"}
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
                      ? "Edit Port Forwarding Rule"
                      : "编辑端口转发规则"
                    : copy.natPage.modalTitle}
                </h2>
                <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">
                  External :{form.public_port || "*"} ➔ Internal {form.local_host || "127.0.0.1"}:{form.local_port || "*"} ({form.protocol.toUpperCase()})
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
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                <Field label={copy.natPage.fieldAgent}>
                  <select
                    className={selectClass}
                    value={form.agent_id}
                    onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}
                    disabled={Boolean(editingId)}
                  >
                    {servers.map((server) => (
                      <option key={server.id} value={server.id}>
                        {server.name}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label={copy.natPage.fieldDescription}>
                  <input
                    className={inputClass}
                    value={form.description}
                    onChange={(e) => setForm((f) => ({ ...f, description: e.target.value }))}
                    placeholder="SSH / Web Loopback"
                  />
                </Field>
                <Field label={copy.natPage.fieldProtocol}>
                  <select
                    className={selectClass}
                    value={form.protocol}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, protocol: e.target.value === "udp" ? "udp" : "tcp" }))
                    }
                  >
                    <option value="tcp">tcp</option>
                    <option value="udp">udp</option>
                  </select>
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldPublicPort}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.public_port}
                    onChange={(e) => setForm((f) => ({ ...f, public_port: e.target.value }))}
                  />
                </Field>
                <Field label={copy.natPage.fieldLocalHost}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.local_host}
                    onChange={(e) => setForm((f) => ({ ...f, local_host: e.target.value }))}
                    placeholder="127.0.0.1"
                  />
                </Field>
                <Field label={copy.natPage.fieldLocalPort}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.local_port}
                    onChange={(e) => setForm((f) => ({ ...f, local_port: e.target.value }))}
                  />
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldSourceCidr}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.allowed_sources}
                    onChange={(e) => setForm((f) => ({ ...f, allowed_sources: e.target.value }))}
                    placeholder="203.0.113.0/24"
                  />
                </Field>
                <Field label={copy.natPage.fieldMaxConcurrent}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.max_active_tunnels}
                    onChange={(e) => setForm((f) => ({ ...f, max_active_tunnels: e.target.value }))}
                    placeholder="2"
                  />
                </Field>
                <Field label={copy.natPage.fieldIdleTimeoutSeconds}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.idle_timeout_seconds}
                    onChange={(e) => setForm((f) => ({ ...f, idle_timeout_seconds: e.target.value }))}
                    placeholder="300"
                  />
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
                <Field label={copy.natPage.fieldMaxBytesPerTunnel}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.max_bytes_per_tunnel}
                    onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_tunnel: e.target.value }))}
                    placeholder="104857600"
                  />
                </Field>
                <Field label={copy.natPage.fieldBandwidth}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.max_bandwidth_bytes_per_second}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, max_bandwidth_bytes_per_second: e.target.value }))
                    }
                    placeholder="1048576"
                  />
                </Field>
                <Field label={copy.natPage.fieldWindowSeconds}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.rate_limit_window_seconds}
                    onChange={(e) => setForm((f) => ({ ...f, rate_limit_window_seconds: e.target.value }))}
                    placeholder="60"
                  />
                </Field>
                <Field label={copy.natPage.fieldMaxConnectionsPerWindow}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.max_connections_per_window}
                    onChange={(e) => setForm((f) => ({ ...f, max_connections_per_window: e.target.value }))}
                    placeholder="30"
                  />
                </Field>
                <Field label={copy.natPage.fieldMaxBytesPerWindow}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.max_bytes_per_window}
                    onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_window: e.target.value }))}
                    placeholder="104857600"
                  />
                </Field>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 pt-2">
                <label className="inline-flex cursor-pointer items-center gap-2 text-xs font-medium text-[var(--text-main)]">
                  <input
                    type="checkbox"
                    checked={form.enabled}
                    onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
                  />
                  <span>{copy.natPage.enabled}</span>
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
                    {copy.natPage.saveMapping}
                  </button>
                </div>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* Visual Port Forwarding Topology Table */}
        {loading && mappings.length === 0 ? (
          <TableSkeleton columns={6} rows={4} />
        ) : filteredMappings.length === 0 ? (
          <EmptyState
            title={mappings.length === 0 ? copy.natPage.emptyTitle : copy.common.noMatches}
            detail={
              mappings.length === 0
                ? copy.natPage.emptyDetail
                : isEn
                  ? "No port forwarding rules match the current filters."
                  : "没有符合当前筛选条件的端口转发规则。"
            }
            action={
              mappings.length === 0 ? (
                <button type="button" className={buttonClass("primary")} onClick={openCreateDrawer}>
                  + 新建端口转发
                </button>
              ) : undefined
            }
          />
        ) : (
          <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
            <table className="w-full min-w-[66rem]">
              <thead>
                <tr>
                  <th className={thClass}>{copy.natPage.colDescription}</th>
                  <th className={thClass}>{copy.natPage.colAgent}</th>
                  <th className={thClass}>
                    {isEn ? "Port Forwarding Topology" : "端口转发拓扑 (External ➔ Internal)"}
                  </th>
                  <th className={thClass}>{copy.natPage.colPolicy}</th>
                  <th className={thClass}>{copy.natPage.colStatus}</th>
                  <th className={thClass}>{copy.natPage.colActions}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-color)]">
                {filteredMappings.map((mapping) => {
                  const proto = normalizeProtocol(mapping.protocol);
                  const targetServerName =
                    serverNameById.get(mapping.agent_id) ?? compactId(mapping.agent_id);
                  const localHost = mapping.local_host || "127.0.0.1";
                  const localPort = mapping.local_port ?? 0;
                  const publicPort = mapping.public_port ?? 0;

                  return (
                    <tr key={mapping.id} className="transition-colors hover:bg-[var(--bg-subtle)]">
                      {/* Rule Description */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-0.5">
                          <span className="font-semibold text-[var(--text-main)]">
                            {mapping.description || compactId(mapping.id)}
                          </span>
                          <span className="font-mono text-[11px] text-[var(--text-muted)]">
                            {compactId(mapping.id)}
                          </span>
                        </div>
                      </td>

                      {/* Target Server */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-0.5">
                          <span className="text-xs font-semibold text-[var(--text-main)]">
                            {targetServerName}
                          </span>
                          <span className="font-mono text-[11px] text-[var(--text-muted)]">
                            {compactId(mapping.agent_id)}
                          </span>
                        </div>
                      </td>

                      {/* Visual Port Forwarding Topology Flow Badge */}
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-2">
                          <StatusBadge tone={proto === "udp" ? "pink" : "blue"}>
                            {proto.toUpperCase()}
                          </StatusBadge>
                          <div className="inline-flex flex-wrap items-center gap-2 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-subtle)] px-2.5 py-1 font-mono text-xs">
                            <span className="font-semibold text-[var(--text-main)]">
                              External :{publicPort}
                            </span>
                            <span className="text-[var(--accent-color)]" aria-hidden="true">
                              ➔
                            </span>
                            <span className="font-medium text-[var(--text-main)]">
                              Internal {localHost}:{localPort}
                            </span>
                          </div>
                        </div>
                      </td>

                      {/* Policy Summary */}
                      <td className={tdClass}>
                        <div className="grid gap-1 font-mono text-xs text-[var(--text-muted)] sm:grid-cols-2">
                          <div className="break-all">
                            {copy.natPage.policySource}
                            {mapping.allowed_sources || copy.natPage.policyGlobalPolicy}
                          </div>
                          <div>
                            {copy.natPage.policyConcurrency}
                            {mapping.max_active_tunnels ?? copy.natPage.policyGlobalLimit}
                          </div>
                          <div>
                            {copy.natPage.policyIdle}
                            {mapping.idle_timeout_seconds
                              ? `${mapping.idle_timeout_seconds}s`
                              : copy.natPage.policyUnlimited}
                          </div>
                          <div>
                            {copy.natPage.policyBandwidth}
                            {mapping.max_bandwidth_bytes_per_second
                              ? `${mapping.max_bandwidth_bytes_per_second} B/s`
                              : copy.natPage.policyUnlimited}
                          </div>
                        </div>
                      </td>

                      {/* Status */}
                      <td className={tdClass}>
                        <StatusBadge tone={mapping.enabled === false ? "gray" : "green"}>
                          {mapping.enabled === false ? copy.natPage.disabled : copy.natPage.enabled}
                        </StatusBadge>
                      </td>

                      {/* Actions */}
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <button
                            type="button"
                            className={buttonClass("secondary")}
                            onClick={() => openEditDrawer(mapping)}
                          >
                            {isEn ? "Edit" : "编辑"}
                          </button>
                          <button
                            type="button"
                            className={buttonClass("danger")}
                            disabled={rowBusy}
                            onClick={() => void runRow(() => deleteMapping(mapping))}
                          >
                            {copy.natPage.delete}
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
      </PageShell>
      {dialogs.element}
    </div>
  );
}
