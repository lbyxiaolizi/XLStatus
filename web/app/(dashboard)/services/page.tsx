"use client";

import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  ProgressBar,
  StatusBadge,
  TableSkeleton,
  asRecord,
  asString,
  buttonClass,
  formatDate,
  formatMs,
  formatPercent,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useServerData } from "@/app/components/ServerDataProvider";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";
import type { servicesPage } from "@/lib/i18n/pages/services";

type ServiceKind = "http" | "tcp" | "icmp";
type ProtocolFilter = "all" | "http" | "tcp" | "icmp";
type StatusFilter = "all" | "online" | "offline";
type ServiceCoverMode = "local" | "all" | "specific" | "exclude";

interface ServiceHistoryPoint {
  status: string;
  delay_ms?: number | null | undefined;
  server_id?: string | null | undefined;
  created_at: string;
}

interface Service {
  id: string;
  name: string;
  kind?: string | undefined;
  type?: string | undefined;
  service_type?: string | undefined;
  target: string;
  interval_seconds?: number | undefined;
  timeout_seconds?: number | undefined;
  enabled?: boolean | undefined;
  server_id?: string | null | undefined;
  server_ids?: string[] | undefined;
  cover_mode?: string | undefined;
  exclude_server_ids?: string[] | undefined;
  failure_task_ids?: string[] | undefined;
  recovery_task_ids?: string[] | undefined;
  config_warnings?: string[] | undefined;
  last_status?: string | undefined;
  last_check_at?: string | undefined;
  last_latency_ms?: number | null | undefined;
  latency_ms?: number | null | undefined;
  delay_ms?: number | null | undefined;
  cert_fingerprint?: string | undefined;
  cert_not_after?: string | undefined;
  history?: unknown[] | undefined;
}

interface ServiceForm {
  name: string;
  service_type: ServiceKind;
  target: string;
  cover_mode: ServiceCoverMode;
  server_ids: string[];
  exclude_server_ids: string[];
  interval_seconds: string;
  timeout_seconds: string;
  enabled: boolean;
  failure_task_ids: string;
  recovery_task_ids: string;
}

interface ServerOption {
  id: string;
  name: string;
}

const HISTORY_FETCH_LIMIT = 48;

const blankForm: ServiceForm = {
  name: "",
  service_type: "http",
  target: "",
  cover_mode: "local",
  server_ids: [],
  exclude_server_ids: [],
  interval_seconds: "60",
  timeout_seconds: "10",
  enabled: true,
  failure_task_ids: "",
  recovery_task_ids: "",
};

export default function ServicesPage() {
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const {
    servers,
    error: serverError,
    revalidate: revalidateServers,
  } = useServerData();

  const [services, setServices] = useState<Service[]>([]);
  const [historyMap, setHistoryMap] = useState<Record<string, ServiceHistoryPoint[]>>({});
  const [historyLoadingId, setHistoryLoadingId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Collapsible drawer & inspector states
  const [drawerMode, setDrawerMode] = useState<"create" | "edit" | null>(null);
  const [editing, setEditing] = useState<Service | null>(null);
  const [inspectingId, setInspectingId] = useState<string | null>(null);
  const [form, setForm] = useState<ServiceForm>(blankForm);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [probeResult, setProbeResult] = useState<string | null>(null);

  // Unified control bar filters
  const [query, setQuery] = useState("");
  const [protocolFilter, setProtocolFilter] = useState<ProtocolFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");

  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();
  const [probeBusy, runProbe] = useBusy();
  const initialLoadRef = useRef(true);
  const servicesLoadRequestIdRef = useRef(0);

  const loadServices = useCallback(async (signal?: AbortSignal, bypassClientCache = false) => {
    const requestId = servicesLoadRequestIdRef.current + 1;
    servicesLoadRequestIdRef.current = requestId;
    setError(null);
    if (initialLoadRef.current) setLoading(true);
    const response = await apiClient.listServices(200, 0, false, {
      includeHistory: true,
      historyLimit: HISTORY_FETCH_LIMIT,
      bypassClientCache,
      ...(signal ? { signal } : {}),
    });
    if (signal?.aborted || requestId !== servicesLoadRequestIdRef.current) return;
    initialLoadRef.current = false;
    setLoading(false);
    if (response.success && response.data) {
      const nextServices = (response.data["services"] as Service[]) ?? [];
      setServices(nextServices);
      setHistoryMap((prev) => {
        const next: Record<string, ServiceHistoryPoint[]> = { ...prev };
        for (const item of nextServices) {
          const embedded = extractEmbeddedHistory(item);
          if (embedded.length > 0) {
            next[item.id] = embedded;
          }
        }
        return next;
      });
    } else {
      setError(responseError(response));
    }
  }, []);

  const loadServiceHistory = useCallback(async (serviceId: string, bypassClientCache = false) => {
    setHistoryLoadingId(serviceId);
    const response = await apiClient.getServiceHistory(serviceId, HISTORY_FETCH_LIMIT, {
      bypassClientCache,
    });
    setHistoryLoadingId((current) => (current === serviceId ? null : current));
    if (response.success && response.data) {
      const rawList = Array.isArray(response.data["results"])
        ? response.data["results"]
        : Array.isArray(response.data["history"])
          ? response.data["history"]
          : [];
      const points = rawList
        .map((entry) => normalizeHistoryPoint(entry))
        .filter((entry): entry is ServiceHistoryPoint => entry !== null);
      setHistoryMap((prev) => ({ ...prev, [serviceId]: points }));
    }
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServices(controller.signal);
    return () => {
      controller.abort();
      servicesLoadRequestIdRef.current += 1;
    };
  }, [loadServices]);

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  // Compute latest latency for a service from direct fields or history
  const getServiceLatencyMs = useCallback(
    (service: Service): number | null => {
      if (typeof service.last_latency_ms === "number" && Number.isFinite(service.last_latency_ms)) {
        return service.last_latency_ms;
      }
      if (typeof service.latency_ms === "number" && Number.isFinite(service.latency_ms)) {
        return service.latency_ms;
      }
      if (typeof service.delay_ms === "number" && Number.isFinite(service.delay_ms)) {
        return service.delay_ms;
      }
      const points = historyMap[service.id] ?? extractEmbeddedHistory(service);
      for (let i = points.length - 1; i >= 0; i -= 1) {
        const pt = points[i];
        if (pt && typeof pt.delay_ms === "number" && Number.isFinite(pt.delay_ms)) {
          return pt.delay_ms;
        }
      }
      return null;
    },
    [historyMap],
  );

  // Top KPI Strip metrics
  const kpi = useMemo(() => {
    const total = services.length;
    const enabledCount = services.filter((s) => s.enabled !== false).length;
    const healthyCount = services.filter((s) => isServiceHealthy(s.last_status)).length;
    const degradedOrOfflineCount = services.filter((s) => isServiceDegradedOrOffline(s.last_status)).length;
    const healthyRatio = total > 0 ? (healthyCount / total) * 100 : 100;

    let latencySum = 0;
    let latencyCount = 0;
    for (const service of services) {
      const ms = getServiceLatencyMs(service);
      if (ms !== null && ms >= 0) {
        latencySum += ms;
        latencyCount += 1;
      }
    }
    const avgLatencyMs = latencyCount > 0 ? Math.round(latencySum / latencyCount) : null;

    return {
      total,
      enabledCount,
      healthyCount,
      degradedOrOfflineCount,
      healthyRatio,
      avgLatencyMs,
      latencyCount,
    };
  }, [services, getServiceLatencyMs]);

  const deferredQuery = useDeferredValue(query);
  const filtered = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return services.filter((service) => {
      const kind = normalizeKind(serviceKind(copy, service));
      if (protocolFilter !== "all" && kind !== protocolFilter) {
        return false;
      }
      if (statusFilter === "online" && !isServiceHealthy(service.last_status)) {
        return false;
      }
      if (statusFilter === "offline" && isServiceHealthy(service.last_status)) {
        return false;
      }
      if (!needle) return true;
      return [
        service.name,
        service.target,
        serviceKind(copy, service),
        service.last_status,
        ...serviceConfigWarnings(service),
        ...serviceServerIds(service),
        serviceServerLabel(copy, servers, service),
      ]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle));
    });
  }, [deferredQuery, protocolFilter, statusFilter, services, servers, copy]);

  const inspectedService = useMemo(
    () => (inspectingId ? services.find((item) => item.id === inspectingId) ?? null : null),
    [inspectingId, services],
  );

  function toggleCreateDrawer() {
    if (drawerMode === "create") {
      setDrawerMode(null);
      return;
    }
    setEditing(null);
    setForm(blankForm);
    setProbeResult(null);
    setDrawerMode("create");
  }

  function openEdit(service: Service) {
    setEditing(service);
    setForm({
      name: service.name || "",
      service_type: normalizeKind(serviceKind(copy, service)),
      target: service.target || "",
      cover_mode: normalizeCoverMode(service.cover_mode, serviceServerIds(service)),
      server_ids: serviceServerIds(service),
      exclude_server_ids: service.exclude_server_ids ?? [],
      interval_seconds: String(service.interval_seconds ?? 60),
      timeout_seconds: String(service.timeout_seconds ?? 10),
      enabled: service.enabled ?? true,
      failure_task_ids: (service.failure_task_ids ?? []).join(", "),
      recovery_task_ids: (service.recovery_task_ids ?? []).join(", "),
    });
    setProbeResult(null);
    setDrawerMode("edit");
  }

  function openHistoryInspector(service: Service) {
    if (inspectingId === service.id) {
      setInspectingId(null);
      return;
    }
    setInspectingId(service.id);
    void loadServiceHistory(service.id);
  }

  async function submitForm() {
    setSaving(true);
    const payload = formToPayload(form);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) {
      setSaving(false);
      return;
    }
    const response =
      drawerMode === "edit" && editing
        ? await apiClient.updateService(editing.id, payload, totpCode)
        : await apiClient.createService(payload, totpCode);
    setSaving(false);

    if (response.success) {
      const wasEdit = drawerMode === "edit";
      setDrawerMode(null);
      setEditing(null);
      showToast(wasEdit ? copy.servicesPage.noticeUpdated : copy.servicesPage.noticeCreated);
      await loadServices(undefined, true);
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
      setError(copy.servicesPage.totpInvalid);
      return null;
    }
    return trimmed;
  }

  async function testProbe() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setTesting(true);
    const response = await apiClient.testProbe(
      {
        service_type: form.service_type,
        target: form.target.trim(),
        timeout_seconds: Number(form.timeout_seconds),
      },
      totpCode,
    );
    setTesting(false);
    if (response.success && response.data) {
      const status = response.data["success"] ? copy.servicesPage.probeSuccess : copy.servicesPage.probeFailure;
      const latency = response.data["latency_ms"] ? `, ${String(response.data["latency_ms"])} ms` : "";
      const code = response.data["status_code"] ? `, HTTP ${String(response.data["status_code"])}` : "";
      const reason = response.data["error"] ? `, ${String(response.data["error"])}` : "";
      const cert = response.data["cert_not_after"]
        ? copy.servicesPage.certExpiry.replace("{date}", String(formatDate(String(response.data["cert_not_after"]))))
        : "";
      setProbeResult(
        copy.servicesPage.probeResult
          .replace("{status}", status)
          .replace("{latency}", latency)
          .replace("{code}", code)
          .replace("{cert}", cert)
          .replace("{reason}", reason),
      );
    } else {
      setProbeResult(responseError(response));
    }
  }

  async function deleteService(service: Service) {
    if (
      !(await dialogs.confirm({
        message: copy.servicesPage.deleteConfirm.replace("{name}", String(service.name)),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteService(service.id, totpCode);
    if (response.success) {
      if (inspectingId === service.id) setInspectingId(null);
      if (editing?.id === service.id) {
        setDrawerMode(null);
        setEditing(null);
      }
      showToast(copy.servicesPage.noticeDeleted);
      await loadServices(undefined, true);
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell wide>
        <PageHeader
          eyebrow={copy.servicesPage.eyebrow}
          title={copy.servicesPage.title}
          detail={copy.servicesPage.detail}
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => void loadServices(undefined, true)}
                className={buttonClass("secondary")}
              >
                {isZh ? "刷新状态" : "Refresh"}
              </button>
              <button
                type="button"
                onClick={toggleCreateDrawer}
                className={buttonClass(drawerMode === "create" ? "secondary" : "primary")}
              >
                {drawerMode === "create"
                  ? copy.common.close
                  : isZh
                    ? `+ 新建服务监控`
                    : `+ New Monitor`}
              </button>
            </div>
          }
        />

        <div className="mb-4 space-y-2.5">
          <InlineError message={error} />
          <InlineError message={serverError} />
        </div>

        {/* 1. Top KPI Strip */}
        <div className="mb-6 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <BrutalCard className="flex flex-col justify-between p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "监控探针总数" : "Total Probes"}
              </span>
              <StatusBadge tone="blue">
                {kpi.enabledCount} {isZh ? "已启用" : "Active"}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {kpi.total}
              </span>
              <span className="font-mono text-xs text-[var(--text-muted)]">
                HTTP {services.filter((s) => normalizeKind(serviceKind(copy, s)) === "http").length} · TCP{" "}
                {services.filter((s) => normalizeKind(serviceKind(copy, s)) === "tcp").length} · PING{" "}
                {services.filter((s) => normalizeKind(serviceKind(copy, s)) === "icmp").length}
              </span>
            </div>
            <div className="mt-2.5 border-t border-[var(--border-color)] pt-2 text-xs text-[var(--text-muted)]">
              {isZh
                ? `${kpi.enabledCount} 个运行中 / ${kpi.total - kpi.enabledCount} 个已暂停`
                : `${kpi.enabledCount} enabled / ${kpi.total - kpi.enabledCount} paused`}
            </div>
          </BrutalCard>

          <BrutalCard className="flex flex-col justify-between p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "服务可用率 (Healthy Ratio)" : "Availability Ratio"}
              </span>
              <StatusBadge tone={kpi.healthyCount === kpi.total && kpi.total > 0 ? "green" : kpi.degradedOrOfflineCount > 0 ? "yellow" : "gray"}>
                {kpi.healthyCount} / {kpi.total}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {kpi.total > 0 ? formatPercent(kpi.healthyRatio) : "N/A"}
              </span>
              <span className="text-xs text-[var(--text-muted)]">
                {isZh ? `${kpi.healthyCount} 正常` : `${kpi.healthyCount} online`}
              </span>
            </div>
            <div className="mt-3">
              <ProgressBar value={kpi.total > 0 ? kpi.healthyRatio : 0} tone="emerald" />
            </div>
          </BrutalCard>

          <BrutalCard className="flex flex-col justify-between p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "平均响应延迟" : "Average Latency"}
              </span>
              <StatusBadge
                tone={
                  kpi.avgLatencyMs === null
                    ? "gray"
                    : kpi.avgLatencyMs > 500
                      ? "yellow"
                      : "green"
                }
              >
                {kpi.avgLatencyMs !== null && kpi.avgLatencyMs <= 200
                  ? isZh
                    ? "极速"
                    : "Fast"
                  : kpi.avgLatencyMs !== null
                    ? isZh
                      ? "实时"
                      : "Live"
                    : "N/A"}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {kpi.avgLatencyMs !== null ? `${kpi.avgLatencyMs} ms` : "N/A"}
              </span>
              <span className="font-mono text-xs text-[var(--text-muted)]">
                {kpi.latencyCount} {isZh ? "个样本源" : "probes"}
              </span>
            </div>
            <div className="mt-3">
              <ProgressBar
                value={kpi.avgLatencyMs !== null ? Math.min(100, (kpi.avgLatencyMs / 1000) * 100) : 0}
                tone="indigo"
              />
            </div>
          </BrutalCard>

          <BrutalCard
            className={`flex flex-col justify-between p-4 ${
              kpi.degradedOrOfflineCount > 0 ? "border-rose-500/40" : ""
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "降级 / 异常探针" : "Degraded / Offline"}
              </span>
              <StatusBadge tone={kpi.degradedOrOfflineCount > 0 ? "red" : "green"}>
                {kpi.degradedOrOfflineCount > 0
                  ? isZh
                    ? "需关注"
                    : "Attention"
                  : isZh
                    ? "全部正常"
                    : "All Healthy"}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span
                className={`font-mono text-2xl font-semibold tracking-tight tabular-nums ${
                  kpi.degradedOrOfflineCount > 0
                    ? "text-rose-600 dark:text-rose-400"
                    : "text-[var(--text-main)]"
                }`}
              >
                {kpi.degradedOrOfflineCount}
              </span>
              {kpi.degradedOrOfflineCount > 0 ? (
                <button
                  type="button"
                  onClick={() => setStatusFilter(statusFilter === "offline" ? "all" : "offline")}
                  className="text-xs font-medium text-rose-600 underline-offset-2 hover:underline dark:text-rose-400"
                >
                  {statusFilter === "offline"
                    ? isZh
                      ? "显示全部"
                      : "Show all"
                    : isZh
                      ? "仅看异常 →"
                      : "Filter offline →"}
                </button>
              ) : null}
            </div>
            <div className="mt-2.5 border-t border-[var(--border-color)] pt-2 text-xs text-[var(--text-muted)]">
              {kpi.degradedOrOfflineCount > 0
                ? isZh
                  ? "检测到连接失败、超时或状态降级"
                  : "Probe failures, timeouts, or degraded checks detected"
                : isZh
                  ? "所有活动拨测目标响应正常"
                  : "All active targets responding normally"}
            </div>
          </BrutalCard>
        </div>

        {/* 2. Collapsible Creation / Edit Drawer */}
        {drawerMode !== null ? (
          <BrutalCard className="mb-6 border-[var(--accent-color)]/40 bg-[var(--bg-card)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3.5">
              <div className="flex items-center gap-2.5">
                <span className="h-2.5 w-2.5 rounded-full bg-[var(--accent-color)]" />
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {drawerMode === "edit"
                    ? `${copy.servicesPage.modalTitleEdit}${editing ? ` — ${editing.name}` : ""}`
                    : copy.servicesPage.modalTitleCreate}
                </h2>
                <StatusBadge tone={drawerMode === "edit" ? "yellow" : "blue"}>
                  {drawerMode === "edit" ? (isZh ? "编辑模式" : "Edit Mode") : isZh ? "新建配置" : "Create Mode"}
                </StatusBadge>
              </div>
              <button
                type="button"
                onClick={() => {
                  setDrawerMode(null);
                  setEditing(null);
                }}
                className={buttonClass("secondary")}
              >
                {copy.common.close}
              </button>
            </div>

            <form
              onSubmit={(event) => {
                event.preventDefault();
                void runForm(() => submitForm());
              }}
              className="space-y-4"
            >
              {editing && serviceConfigWarnings(editing).length ? (
                <InlineNotice tone="yellow">{serviceWarningText(copy, editing)}</InlineNotice>
              ) : null}

              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <Field label={copy.servicesPage.fieldName}>
                  <input
                    className={inputClass}
                    value={form.name}
                    onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                    placeholder={isZh ? "如：核心 API 网关" : "e.g. Core API Gateway"}
                    required
                  />
                </Field>
                <Field label={copy.servicesPage.fieldType}>
                  <select
                    className={selectClass}
                    value={form.service_type}
                    onChange={(e) => setForm((f) => ({ ...f, service_type: e.target.value as ServiceKind }))}
                  >
                    <option value="http">HTTP / HTTPS</option>
                    <option value="tcp">TCP</option>
                    <option value="icmp">ICMP (PING)</option>
                  </select>
                </Field>
                <Field label={copy.servicesPage.fieldInterval}>
                  <input
                    className={inputClass}
                    value={form.interval_seconds}
                    onChange={(e) => setForm((f) => ({ ...f, interval_seconds: e.target.value }))}
                  />
                </Field>
                <Field label={copy.servicesPage.fieldTimeout}>
                  <input
                    className={inputClass}
                    value={form.timeout_seconds}
                    onChange={(e) => setForm((f) => ({ ...f, timeout_seconds: e.target.value }))}
                  />
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
                <Field label={copy.servicesPage.fieldTarget}>
                  <input
                    className={`${inputClass} font-mono`}
                    value={form.target}
                    onChange={(e) => setForm((f) => ({ ...f, target: e.target.value }))}
                    placeholder={
                      form.service_type === "http"
                        ? "https://example.com/health"
                        : form.service_type === "tcp"
                          ? "1.1.1.1:443"
                          : "1.1.1.1"
                    }
                    required
                  />
                </Field>
                <Field label={copy.servicesPage.fieldCoverMode}>
                  <select
                    className={selectClass}
                    value={form.cover_mode}
                    onChange={(e) => setForm((f) => ({ ...f, cover_mode: e.target.value as ServiceCoverMode }))}
                  >
                    <option value="local">{copy.servicesPage.coverLocal}</option>
                    <option value="all">{copy.servicesPage.coverAll}</option>
                    <option value="specific">{copy.servicesPage.coverSpecific}</option>
                    <option value="exclude">{copy.servicesPage.coverExclude}</option>
                  </select>
                </Field>
              </div>

              {form.cover_mode === "specific" ? (
                <Field label={copy.servicesPage.fieldSpecificServers}>
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, server_ids: serverIds }))}
                  />
                </Field>
              ) : null}
              {form.cover_mode === "exclude" ? (
                <Field label={copy.servicesPage.fieldExcludeServers}>
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.exclude_server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, exclude_server_ids: serverIds }))}
                  />
                </Field>
              ) : null}

              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.servicesPage.fieldFailureTaskIds}>
                  <input
                    className={inputClass}
                    value={form.failure_task_ids}
                    onChange={(e) => setForm((f) => ({ ...f, failure_task_ids: e.target.value }))}
                  />
                </Field>
                <Field label={copy.servicesPage.fieldRecoveryTaskIds}>
                  <input
                    className={inputClass}
                    value={form.recovery_task_ids}
                    onChange={(e) => setForm((f) => ({ ...f, recovery_task_ids: e.target.value }))}
                  />
                </Field>
              </div>

              {probeResult ? <InlineNotice tone="pink">{probeResult}</InlineNotice> : null}

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-color)] pt-3.5">
                <label className="flex items-center gap-2.5 text-sm font-medium text-[var(--text-main)]">
                  <input
                    type="checkbox"
                    checked={form.enabled}
                    onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
                  />
                  {copy.servicesPage.fieldEnabled}
                </label>

                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    disabled={testing || probeBusy || !form.target.trim()}
                    onClick={() => void runProbe(() => testProbe())}
                    className={buttonClass("secondary")}
                  >
                    {testing ? copy.servicesPage.testing : copy.servicesPage.testProbe}
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setDrawerMode(null);
                      setEditing(null);
                    }}
                    className={buttonClass("ghost")}
                  >
                    {copy.common.cancel}
                  </button>
                  <button type="submit" disabled={saving || formBusy} className={buttonClass("primary")}>
                    {saving ? copy.servicesPage.saving : copy.servicesPage.save}
                  </button>
                </div>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* 3. Unified Control Bar */}
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3 shadow-[var(--shadow-xs)]">
          <div className="min-w-[14rem] flex-1">
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className="block w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-1.5 text-sm text-[var(--text-main)] outline-none transition placeholder:text-[var(--text-faint)] focus:border-[var(--accent-color)] focus:bg-[var(--bg-card)] focus:ring-2 focus:ring-[var(--ring-color)]"
              placeholder={copy.servicesPage.searchPlaceholder}
            />
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {/* Protocol Filter Pills */}
            <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
              {(
                [
                  { id: "all", label: "ALL" },
                  { id: "http", label: "HTTP" },
                  { id: "tcp", label: "TCP" },
                  { id: "icmp", label: "PING" },
                ] as const
              ).map((pill) => {
                const active = protocolFilter === pill.id;
                return (
                  <button
                    key={pill.id}
                    type="button"
                    onClick={() => setProtocolFilter(pill.id)}
                    className={`rounded-md px-2.5 py-1 font-mono text-xs font-medium transition ${
                      active
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {pill.label}
                  </button>
                );
              })}
            </div>

            {/* Status Filter Pills */}
            <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
              {(
                [
                  { id: "all", label: isZh ? "全部" : "all" },
                  { id: "online", label: isZh ? "正常" : "online" },
                  { id: "offline", label: isZh ? "异常" : "offline" },
                ] as const
              ).map((pill) => {
                const active = statusFilter === pill.id;
                return (
                  <button
                    key={pill.id}
                    type="button"
                    onClick={() => setStatusFilter(pill.id)}
                    className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                      active
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {pill.label}
                  </button>
                );
              })}
            </div>

            <button
              type="button"
              onClick={toggleCreateDrawer}
              className={buttonClass(drawerMode === "create" ? "secondary" : "primary")}
            >
              {isZh ? "+ 新建服务监控" : "+ New Monitor"}
            </button>
          </div>
        </div>

        {/* 4. Interactive Service Matrix & Inline History Inspector */}
        {loading ? (
          <TableSkeleton columns={7} rows={5} />
        ) : filtered.length === 0 ? (
          <EmptyState
            title={copy.servicesPage.emptyTitle}
            detail={copy.servicesPage.emptyDetail}
            action={
              <button type="button" onClick={toggleCreateDrawer} className={buttonClass("primary")}>
                {isZh ? "+ 新建服务监控" : "+ New Monitor"}
              </button>
            }
          />
        ) : (
          <div className="space-y-5">
            {inspectedService ? (
              <ServiceHistoryInspector
                service={inspectedService}
                points={historyMap[inspectedService.id] ?? extractEmbeddedHistory(inspectedService)}
                loading={historyLoadingId === inspectedService.id}
                servers={servers}
                isZh={isZh}
                copy={copy}
                onRefresh={() => void loadServiceHistory(inspectedService.id, true)}
                onEdit={() => openEdit(inspectedService)}
                onClose={() => setInspectingId(null)}
              />
            ) : null}

            <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
              <table className="w-full min-w-[68rem]">
                <thead>
                  <tr>
                    <th className={thClass}>{copy.servicesPage.colName}</th>
                    <th className={thClass}>{copy.servicesPage.colType}</th>
                    <th className={thClass}>{copy.servicesPage.colTarget}</th>
                    <th className={thClass}>{copy.servicesPage.colStatus}</th>
                    <th className={thClass}>{isZh ? "响应延迟 / 趋势" : "Latency & Trend"}</th>
                    <th className={thClass}>{isZh ? "周期 / 探测节点" : "Schedule & Nodes"}</th>
                    <th className={thClass}>{copy.servicesPage.colActions}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--border-color)]">
                  {filtered.map((service) => {
                    const kind = normalizeKind(serviceKind(copy, service));
                    const latencyMs = getServiceLatencyMs(service);
                    const historyPoints = historyMap[service.id] ?? extractEmbeddedHistory(service);
                    const isInspecting = inspectingId === service.id;
                    const intervalSec = service.interval_seconds ?? 60;
                    const timeoutSec = service.timeout_seconds ?? 10;
                    const latencyPct =
                      latencyMs !== null ? Math.min(100, Math.max(4, (latencyMs / 600) * 100)) : 0;

                    return (
                      <tr
                        key={service.id}
                        className={`transition-colors ${
                          isInspecting
                            ? "bg-[var(--accent-soft)]/35"
                            : "hover:bg-[var(--bg-subtle)]"
                        }`}
                      >
                        <td className={tdClass}>
                          <div className="space-y-1">
                            <div className="flex items-center gap-2">
                              <span
                                className={`h-2 w-2 shrink-0 rounded-full ${
                                  service.enabled === false
                                    ? "bg-zinc-400"
                                    : isServiceHealthy(service.last_status)
                                      ? "bg-emerald-500"
                                      : isServiceDegradedOrOffline(service.last_status)
                                        ? "bg-rose-500"
                                        : "bg-zinc-400"
                                }`}
                              />
                              <span className="font-semibold text-[var(--text-main)]">{service.name}</span>
                              {service.enabled === false ? (
                                <StatusBadge tone="gray">{copy.common.disabled}</StatusBadge>
                              ) : null}
                            </div>
                            {serviceConfigWarnings(service).length ? (
                              <div className="text-xs text-amber-600 dark:text-amber-400">
                                {serviceWarningText(copy, service)}
                              </div>
                            ) : null}
                          </div>
                        </td>

                        <td className={tdClass}>
                          <StatusBadge tone={protocolTone(kind)}>
                            {kind === "icmp" ? "PING / ICMP" : kind.toUpperCase()}
                          </StatusBadge>
                        </td>

                        <td className={tdClass}>
                          <div className="space-y-1">
                            <span className="inline-block max-w-[20rem] truncate rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 font-mono text-xs text-[var(--text-main)]">
                              {service.target}
                            </span>
                            {service.cert_not_after ? (
                              <div className="font-mono text-[11px] text-[var(--text-muted)]">
                                TLS: {formatDate(service.cert_not_after)}
                              </div>
                            ) : null}
                          </div>
                        </td>

                        <td className={tdClass}>
                          <div className="space-y-1">
                            <StatusBadge tone={serviceTone(service.last_status)}>
                              {statusLabel(copy.servicesPage, service.last_status)}
                            </StatusBadge>
                            {service.last_check_at ? (
                              <div className="font-mono text-[11px] text-[var(--text-muted)]">
                                {formatDate(service.last_check_at)}
                              </div>
                            ) : null}
                          </div>
                        </td>

                        <td className={tdClass}>
                          <div className="min-w-[9.5rem] space-y-1.5">
                            <div className="flex items-center justify-between gap-2 font-mono text-xs">
                              <span className="font-semibold text-[var(--text-main)] tabular-nums">
                                {latencyMs !== null ? `${Math.round(latencyMs)} ms` : "N/A"}
                              </span>
                              {historyPoints.length > 0 ? (
                                <span className="text-[11px] text-[var(--text-muted)]">
                                  {historyPoints.length} {isZh ? "次采样" : "checks"}
                                </span>
                              ) : null}
                            </div>
                            <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-muted)]">
                              <div
                                className={`h-full rounded-full transition-all duration-300 ${
                                  latencyMs === null
                                    ? "bg-zinc-400"
                                    : latencyMs > 400
                                      ? "bg-amber-500"
                                      : latencyMs > 800
                                        ? "bg-rose-500"
                                        : "bg-emerald-500"
                                }`}
                                style={{ width: `${latencyPct}%` }}
                              />
                            </div>
                          </div>
                        </td>

                        <td className={tdClass}>
                          <div className="space-y-1">
                            <div className="font-mono text-xs text-[var(--text-main)]">
                              {intervalSec}s {isZh ? "间隔" : "int"} · {timeoutSec}s {isZh ? "超时" : "timeout"}
                            </div>
                            <div className="max-w-[14rem] truncate text-xs text-[var(--text-muted)]">
                              {serviceServerLabel(copy, servers, service)}
                            </div>
                          </div>
                        </td>

                        <td className={tdClass}>
                          <div className="flex flex-wrap gap-1.5">
                            <button
                              type="button"
                              className={buttonClass(isInspecting ? "primary" : "secondary")}
                              onClick={() => openHistoryInspector(service)}
                            >
                              {isZh ? "历史趋势" : "History"}
                            </button>
                            <button
                              type="button"
                              className={buttonClass("secondary")}
                              onClick={() => openEdit(service)}
                            >
                              {copy.servicesPage.edit}
                            </button>
                            <button
                              type="button"
                              className={buttonClass("danger")}
                              disabled={rowBusy}
                              onClick={() => void runRow(() => deleteService(service))}
                            >
                              {copy.servicesPage.delete}
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function ServiceHistoryInspector({
  service,
  points,
  loading,
  servers,
  isZh,
  copy,
  onRefresh,
  onEdit,
  onClose,
}: {
  service: Service;
  points: ServiceHistoryPoint[];
  loading: boolean;
  servers: ServerOption[];
  isZh: boolean;
  copy: Translations;
  onRefresh: () => void;
  onEdit: () => void;
  onClose: () => void;
}) {
  const stats = useMemo(() => {
    const total = points.length;
    const upCount = points.filter((p) => isServiceHealthy(p.status)).length;
    const upRatio = total > 0 ? (upCount / total) * 100 : null;
    const latencies = points
      .map((p) => p.delay_ms)
      .filter((v): v is number => typeof v === "number" && Number.isFinite(v) && v >= 0);
    const minMs = latencies.length > 0 ? Math.min(...latencies) : null;
    const maxMs = latencies.length > 0 ? Math.max(...latencies) : null;
    const avgMs =
      latencies.length > 0
        ? Math.round(latencies.reduce((sum, v) => sum + v, 0) / latencies.length)
        : null;
    return { total, upCount, upRatio, minMs, maxMs, avgMs };
  }, [points]);

  const recentPoints = useMemo(() => points.slice(-36), [points]);
  const maxBarScale = useMemo(() => {
    const maxVal = stats.maxMs ?? 100;
    return Math.max(50, maxVal);
  }, [stats.maxMs]);

  return (
    <BrutalCard className="border-[var(--accent-color)]/40 bg-[var(--bg-card)]">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-[var(--border-color)] pb-4">
        <div className="space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
              {service.name}
            </h3>
            <StatusBadge tone={protocolTone(normalizeKind(serviceKind(copy, service)))}>
              {serviceKind(copy, service).toUpperCase()}
            </StatusBadge>
            <StatusBadge tone={serviceTone(service.last_status)}>
              {statusLabel(copy.servicesPage, service.last_status)}
            </StatusBadge>
          </div>
          <div className="flex flex-wrap items-center gap-3 text-xs text-[var(--text-muted)]">
            <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 font-mono">
              {service.target}
            </span>
            <span>{serviceServerLabel(copy, servers, service)}</span>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button type="button" onClick={onRefresh} disabled={loading} className={buttonClass("secondary")}>
            {loading ? copy.common.loading : isZh ? "刷新记录" : "Refresh"}
          </button>
          <button type="button" onClick={onEdit} className={buttonClass("secondary")}>
            {copy.servicesPage.edit}
          </button>
          <button type="button" onClick={onClose} className={buttonClass("ghost")}>
            {copy.common.close}
          </button>
        </div>
      </div>

      {/* Latency & Uptime Summary Stats */}
      <div className="mt-4 grid gap-3 sm:grid-cols-4">
        <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
          <div className="text-[11px] font-medium text-[var(--text-muted)]">
            {isZh ? "近期可用率 (Up Ratio)" : "Up Check Ratio"}
          </div>
          <div className="mt-1 font-mono text-lg font-semibold text-[var(--text-main)] tabular-nums">
            {stats.upRatio !== null ? formatPercent(stats.upRatio) : "N/A"}
          </div>
          <div className="mt-0.5 font-mono text-[11px] text-[var(--text-muted)]">
            {stats.upCount} / {stats.total} {isZh ? "成功" : "up"}
          </div>
        </div>

        <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
          <div className="text-[11px] font-medium text-[var(--text-muted)]">
            {isZh ? "最低延迟 (Min)" : "Min Latency"}
          </div>
          <div className="mt-1 font-mono text-lg font-semibold text-emerald-600 dark:text-emerald-400 tabular-nums">
            {stats.minMs !== null ? formatMs(Math.round(stats.minMs)) : "N/A"}
          </div>
          <div className="mt-0.5 text-[11px] text-[var(--text-muted)]">
            {isZh ? "最快响应" : "Best sample"}
          </div>
        </div>

        <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
          <div className="text-[11px] font-medium text-[var(--text-muted)]">
            {isZh ? "平均延迟 (Avg)" : "Avg Latency"}
          </div>
          <div className="mt-1 font-mono text-lg font-semibold text-[var(--text-main)] tabular-nums">
            {stats.avgMs !== null ? formatMs(stats.avgMs) : "N/A"}
          </div>
          <div className="mt-0.5 text-[11px] text-[var(--text-muted)]">
            {isZh ? `最近 ${points.length} 次拨测` : `Across ${points.length} checks`}
          </div>
        </div>

        <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
          <div className="text-[11px] font-medium text-[var(--text-muted)]">
            {isZh ? "峰值延迟 (Max)" : "Max Latency"}
          </div>
          <div className="mt-1 font-mono text-lg font-semibold text-amber-600 dark:text-amber-400 tabular-nums">
            {stats.maxMs !== null ? formatMs(Math.round(stats.maxMs)) : "N/A"}
          </div>
          <div className="mt-0.5 text-[11px] text-[var(--text-muted)]">
            {isZh ? "最高耗时样本" : "Peak sample"}
          </div>
        </div>
      </div>

      {/* Visual Latency Bars for Recent Checks */}
      <div className="mt-4 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--text-muted)]">
          <span className="font-medium text-[var(--text-main)]">
            {isZh ? "近期探测延迟分布图" : "Recent Probe Latency Bars"}
          </span>
          <span className="font-mono">
            {recentPoints.length > 0
              ? `${formatDate(recentPoints[0]?.created_at)} → ${formatDate(recentPoints[recentPoints.length - 1]?.created_at)}`
              : isZh
                ? "暂无历史记录"
                : "No history records"}
          </span>
        </div>

        {recentPoints.length === 0 ? (
          <div className="py-6 text-center text-xs text-[var(--text-muted)]">
            {loading
              ? copy.common.loading
              : isZh
                ? "当前服务暂无历史拨测样本数据"
                : "No probe history samples recorded yet"}
          </div>
        ) : (
          <div className="flex h-24 items-end gap-1 pt-2">
            {recentPoints.map((pt, idx) => {
              const ok = isServiceHealthy(pt.status);
              const ms = typeof pt.delay_ms === "number" && Number.isFinite(pt.delay_ms) ? pt.delay_ms : 0;
              const heightPct = ok
                ? Math.min(100, Math.max(18, Math.round((ms / maxBarScale) * 100)))
                : 100;
              const barColor = !ok
                ? "bg-rose-500"
                : ms > 400
                  ? "bg-amber-500"
                  : "bg-emerald-500";
              return (
                <div
                  key={`${pt.created_at}-${idx}`}
                  title={`${formatDate(pt.created_at)} · ${statusLabel(copy.servicesPage, pt.status)}${
                    typeof pt.delay_ms === "number" ? ` · ${Math.round(pt.delay_ms)} ms` : ""
                  }`}
                  className="group relative flex h-full flex-1 items-end"
                >
                  <div
                    className={`w-full rounded-t-xs transition-all group-hover:opacity-80 ${barColor}`}
                    style={{ height: `${heightPct}%` }}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>
    </BrutalCard>
  );
}

function extractEmbeddedHistory(service: Service): ServiceHistoryPoint[] {
  if (!Array.isArray(service.history)) return [];
  return service.history
    .map((entry) => normalizeHistoryPoint(entry))
    .filter((entry): entry is ServiceHistoryPoint => entry !== null);
}

function normalizeHistoryPoint(entry: unknown): ServiceHistoryPoint | null {
  const rec = asRecord(entry);
  const createdAt = asString(rec["created_at"]) || asString(rec["checked_at"]);
  const status = asString(rec["status"]);
  if (!createdAt && !status) return null;
  const rawDelay = rec["delay_ms"] ?? rec["latency_ms"];
  const delayMs =
    typeof rawDelay === "number" && Number.isFinite(rawDelay) ? rawDelay : null;
  return {
    status: status || "unknown",
    delay_ms: delayMs,
    server_id: asString(rec["server_id"]) || null,
    created_at: createdAt || new Date().toISOString(),
  };
}

function isServiceHealthy(status?: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

function isServiceDegradedOrOffline(status?: string): boolean {
  return (
    status === "failure" ||
    status === "down" ||
    status === "timeout" ||
    status === "degraded"
  );
}

function protocolTone(kind: ServiceKind): "blue" | "pink" | "green" {
  if (kind === "tcp") return "pink";
  if (kind === "icmp") return "green";
  return "blue";
}

function serviceKind(copy: Translations, service: Service): string {
  return service.service_type || service.kind || service.type || copy.servicesPage.defaultKind;
}

function statusLabel(copy: typeof servicesPage, status?: string): string {
  if (!status) return copy.statusUnknown;
  const labels: Record<string, string> = {
    success: copy.statusSuccess,
    up: copy.statusUp,
    failure: copy.statusFailure,
    down: copy.statusDown,
    timeout: copy.statusTimeout,
    degraded: copy.statusDegraded,
  };
  return labels[status] ?? status;
}

function normalizeKind(value: string): ServiceKind {
  const lower = value.toLowerCase();
  if (lower === "tcp") return "tcp";
  if (lower === "icmp" || lower === "ping") return "icmp";
  return "http";
}

function normalizeCoverMode(value: string | undefined, serverIds: string[]): ServiceCoverMode {
  if (value === "all" || value === "specific" || value === "exclude" || value === "local") return value;
  return serverIds.length ? "specific" : "local";
}

function formToPayload(form: ServiceForm): JsonObject {
  const serverIds = form.cover_mode === "specific" ? form.server_ids : [];
  return {
    name: form.name.trim(),
    service_type: form.service_type,
    target: form.target.trim(),
    cover_mode: form.cover_mode,
    server_id: serverIds[0] || null,
    server_ids: serverIds,
    exclude_server_ids: form.cover_mode === "exclude" ? form.exclude_server_ids : [],
    interval_seconds: Number(form.interval_seconds),
    timeout_seconds: Number(form.timeout_seconds),
    enabled: form.enabled,
    failure_task_ids: splitIds(form.failure_task_ids),
    recovery_task_ids: splitIds(form.recovery_task_ids),
  };
}

function splitIds(value: string): string[] {
  return value
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function ServerMultiPicker({
  servers,
  selected,
  onChange,
}: {
  servers: ServerOption[];
  selected: string[];
  onChange: (serverIds: string[]) => void;
}) {
  const { t: copy } = useI18n();
  return (
    <div className="space-y-2.5">
      <button
        type="button"
        onClick={() => onChange([])}
        className={`rounded-lg border px-3 py-2 text-left text-xs font-medium transition-colors ${
          selected.length === 0
            ? "border-[var(--accent-color)] bg-[var(--accent-soft)] text-[var(--accent-color)]"
            : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-main)] hover:bg-[var(--bg-elevated)]"
        }`}
      >
        {copy.servicesPage.pickerMasterUnassigned}
      </button>
      <div className="grid max-h-56 gap-2 overflow-auto rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-2.5 sm:grid-cols-2 lg:grid-cols-3">
        {servers.length === 0 ? (
          <div className="text-xs text-[var(--text-muted)]">{copy.servicesPage.pickerNoServers}</div>
        ) : (
          servers.map((server) => {
            const checked = selected.includes(server.id);
            return (
              <label
                key={server.id}
                className={`flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 text-sm font-medium transition-colors ${
                  checked
                    ? "border-[var(--accent-color)] bg-[var(--accent-soft)] text-[var(--text-main)]"
                    : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-main)] hover:bg-[var(--bg-elevated)]"
                }`}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => onChange(toggleServerId(selected, server.id))}
                  className="h-4 w-4 accent-[var(--accent-color)]"
                />
                <span className="min-w-0 truncate">{server.name || server.id}</span>
              </label>
            );
          })
        )}
      </div>
    </div>
  );
}

function toggleServerId(selected: string[], serverId: string): string[] {
  return selected.includes(serverId)
    ? selected.filter((id) => id !== serverId)
    : [...selected, serverId];
}

function serviceServerIds(service: Service): string[] {
  const ids = Array.isArray(service.server_ids) ? service.server_ids : [];
  const all = [...ids, ...(service.server_id ? [service.server_id] : [])];
  return all.filter((id, index) => id && all.indexOf(id) === index);
}

function serviceConfigWarnings(service: Service): string[] {
  return Array.isArray(service.config_warnings)
    ? service.config_warnings.filter((warning) => typeof warning === "string" && warning.trim())
    : [];
}

function serviceWarningText(copy: Translations, service: Service): string {
  const warnings = serviceConfigWarnings(service);
  if (!warnings.length) return "";
  return `${copy.servicesPage.warningPrefix}${warnings.join("；")}`;
}

function serviceServerLabel(copy: Translations, servers: ServerOption[], service: Service): string {
  if (service.cover_mode === "all") return copy.servicesPage.serverLabelAllOnline;
  if (service.cover_mode === "exclude") {
    const excluded = service.exclude_server_ids ?? [];
    if (!excluded.length) return copy.servicesPage.serverLabelAllOnline;
    return `${copy.servicesPage.serverLabelExcludePrefix}${excluded.map((id) => serverName(servers, id)).join("、")}`;
  }
  const ids = serviceServerIds(service);
  if (!ids.length) return copy.servicesPage.serverLabelLocal;
  return ids.map((id) => serverName(servers, id)).join("、");
}

function serverName(servers: ServerOption[], id?: string | null): string {
  if (!id) return "";
  return servers.find((server) => server.id === id)?.name || id;
}

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}
