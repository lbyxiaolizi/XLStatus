"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  PageHeader,
  PageShell,
  ProgressBar,
  Skeleton,
  StatusBadge,
  buttonClass,
  formatBytes,
  formatDate,
  formatPercent,
  responseError,
} from "@/app/components/M7Primitives";
import { useServerData } from "@/app/components/ServerDataProvider";
import { apiClient } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

interface Service {
  id: string;
  name: string;
  target: string;
  service_type?: string | undefined;
  last_status?: string | undefined;
  last_latency_ms?: number | null | undefined;
}

interface AlertEvent {
  id?: string | undefined;
  rule_name?: string | undefined;
  message?: string | undefined;
  status?: string | undefined;
  severity?: string | undefined;
  created_at?: string | undefined;
}

const DASHBOARD_REFRESH_INTERVAL_MS = 30_000;

export default function DashboardPage() {
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const {
    servers,
    observedAt,
    loading: serversLoading,
    error: serversError,
    refresh: refreshServers,
    revalidate: revalidateServers,
  } = useServerData();
  const [services, setServices] = useState<Service[]>([]);
  const [alerts, setAlerts] = useState<AlertEvent[]>([]);
  const [errors, setErrors] = useState({ services: "", alerts: "" });
  const [loading, setLoading] = useState({ services: true, alerts: true });
  const [fleetFilter, setFleetFilter] = useState<"all" | "online" | "offline">("all");

  useEffect(() => {
    let stopped = false;
    let controller: AbortController | null = null;
    let timer: number | null = null;

    function stopPolling() {
      if (timer !== null) {
        window.clearInterval(timer);
        timer = null;
      }
    }

    function startPolling() {
      stopPolling();
      if (document.visibilityState !== "visible") return;
      timer = window.setInterval(() => refreshAll(), DASHBOARD_REFRESH_INTERVAL_MS);
    }

    async function loadSupplementalData(force = false) {
      controller?.abort();
      controller = new AbortController();
      const { signal } = controller;
      const [servicesResponse, alertsResponse] = await Promise.all([
        apiClient.listServices(100, 0, false, { signal, bypassClientCache: force }),
        apiClient.listAlertEvents(10, { signal, bypassClientCache: force }),
      ]);
      if (stopped || signal.aborted) return;

      if (servicesResponse.success && servicesResponse.data) {
        setServices((servicesResponse.data["services"] as Service[]) ?? []);
        setErrors((current) => ({ ...current, services: "" }));
      } else {
        setErrors((current) => ({ ...current, services: responseError(servicesResponse) }));
      }
      if (alertsResponse.success && alertsResponse.data) {
        setAlerts((alertsResponse.data["events"] as AlertEvent[]) ?? []);
        setErrors((current) => ({ ...current, alerts: "" }));
      } else {
        setErrors((current) => ({ ...current, alerts: responseError(alertsResponse) }));
      }
      setLoading({ services: false, alerts: false });
    }

    function refreshAll(force = false) {
      void (force ? refreshServers(true) : revalidateServers());
      void loadSupplementalData(force);
    }

    function handleVisibilityChange() {
      if (document.visibilityState !== "visible") {
        stopPolling();
        controller?.abort();
        controller = null;
        return;
      }
      refreshAll(true);
      startPolling();
    }

    document.addEventListener("visibilitychange", handleVisibilityChange);
    if (document.visibilityState === "visible") {
      refreshAll();
      startPolling();
    }

    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      stopPolling();
      controller?.abort();
    };
  }, [refreshServers, revalidateServers]);

  const summary = useMemo(() => {
    const onlineList = servers.filter((server) => server.status === "online");
    const onlineServers = onlineList.length;
    const activeAlerts = alerts.filter((alert) => alert.status !== "resolved").length;
    const onlineServices = services.filter((service) =>
      ["success", "up"].includes(service.last_status || ""),
    ).length;

    let totalRxBps = 0;
    let totalTxBps = 0;
    let totalRxBytes = 0;
    let totalTxBytes = 0;
    let cpuSum = 0;
    let cpuCount = 0;
    let memUsedSum = 0;
    let memTotalSum = 0;

    for (const s of servers) {
      totalRxBps += Number(s.net_rx_bps ?? 0) || 0;
      totalTxBps += Number(s.net_tx_bps ?? 0) || 0;
      totalRxBytes += Number(s.network_in_total ?? 0) || 0;
      totalTxBytes += Number(s.network_out_total ?? 0) || 0;
      if (s.status === "online" && typeof s.cpu_percent === "number") {
        cpuSum += s.cpu_percent;
        cpuCount += 1;
      }
      if (
        s.status === "online" &&
        typeof s.memory_used === "number" &&
        typeof s.memory_total === "number" &&
        s.memory_total > 0
      ) {
        memUsedSum += s.memory_used;
        memTotalSum += s.memory_total;
      }
    }

    return {
      onlineServers,
      activeAlerts,
      onlineServices,
      totalRxBps,
      totalTxBps,
      totalRxBytes,
      totalTxBytes,
      avgCpu: cpuCount > 0 ? cpuSum / cpuCount : 0,
      avgMem: memTotalSum > 0 ? (memUsedSum / memTotalSum) * 100 : 0,
      onlineRatio: servers.length > 0 ? (onlineServers / servers.length) * 100 : 0,
      serviceRatio: services.length > 0 ? (onlineServices / services.length) * 100 : 100,
    };
  }, [alerts, servers, services]);

  const filteredServers = useMemo(() => {
    if (fleetFilter === "online") return servers.filter((s) => s.status === "online");
    if (fleetFilter === "offline") return servers.filter((s) => s.status !== "online");
    return servers;
  }, [fleetFilter, servers]);

  return (
    <PageShell wide>
      <PageHeader
        eyebrow={copy.dashboardPage.eyebrow}
        title={copy.dashboardPage.title}
        detail={copy.dashboardPage.detail}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge tone={observedAt !== null ? "green" : "yellow"}>
              {observedAt !== null
                ? isZh
                  ? "实时遥测已同步"
                  : "Telemetry Synced"
                : isZh
                  ? "正在同步..."
                  : "Syncing..."}
            </StatusBadge>
            <Link href="/servers" className={buttonClass("secondary")}>
              {isZh ? "管理节点" : "Manage Nodes"}
            </Link>
            <Link href="/terminal" className={buttonClass("primary")}>
              {isZh ? "打开终端" : "Web Terminal"}
            </Link>
          </div>
        }
      />

      <div className="space-y-2.5">
        <InlineError
          message={serversError ? `${copy.dashboardPage.kpiServers}: ${serversError}` : null}
        />
        <InlineError
          message={errors.services ? `${copy.dashboardPage.kpiServices}: ${errors.services}` : null}
        />
        <InlineError
          message={errors.alerts ? `${copy.dashboardPage.kpiAlerts}: ${errors.alerts}` : null}
        />
      </div>

      {/* Top High-Density Telemetry KPI Strip */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <BrutalCard className="flex flex-col justify-between p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-[var(--text-muted)]">
              {copy.dashboardPage.kpiServers}
            </span>
            <StatusBadge tone={summary.onlineServers === servers.length && servers.length > 0 ? "green" : "yellow"}>
              {summary.onlineServers} / {servers.length}
            </StatusBadge>
          </div>
          {serversLoading ? (
            <Skeleton className="mt-3 h-8 w-24" />
          ) : (
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {summary.onlineServers}
                <span className="ml-1 text-sm font-normal text-[var(--text-muted)]">
                  / {servers.length} {isZh ? "在线" : "online"}
                </span>
              </span>
              <span className="font-mono text-xs text-[var(--text-muted)]">
                CPU {formatPercent(summary.avgCpu)} · RAM {formatPercent(summary.avgMem)}
              </span>
            </div>
          )}
          <div className="mt-3">
            <ProgressBar value={summary.onlineRatio} tone="emerald" />
          </div>
        </BrutalCard>

        <BrutalCard className="flex flex-col justify-between p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-[var(--text-muted)]">
              {isZh ? "全网实时吞吐" : "Live Network Throughput"}
            </span>
            <span className="font-mono text-[11px] text-emerald-500">LIVE</span>
          </div>
          {serversLoading ? (
            <Skeleton className="mt-3 h-8 w-36" />
          ) : (
            <div className="mt-2 grid grid-cols-2 gap-2 font-mono">
              <div>
                <div className="text-[11px] text-[var(--text-muted)]">↓ RX/s</div>
                <div className="text-lg font-semibold text-[var(--text-main)] tabular-nums">
                  {formatBytes(summary.totalRxBps)}/s
                </div>
              </div>
              <div>
                <div className="text-[11px] text-[var(--text-muted)]">↑ TX/s</div>
                <div className="text-lg font-semibold text-[var(--text-main)] tabular-nums">
                  {formatBytes(summary.totalTxBps)}/s
                </div>
              </div>
            </div>
          )}
          <div className="mt-2.5 flex items-center justify-between border-t border-[var(--border-color)] pt-2 font-mono text-[11px] text-[var(--text-muted)]">
            <span>↓ {formatBytes(summary.totalRxBytes)}</span>
            <span>↑ {formatBytes(summary.totalTxBytes)}</span>
          </div>
        </BrutalCard>

        <BrutalCard className="flex flex-col justify-between p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-[var(--text-muted)]">
              {copy.dashboardPage.kpiServices}
            </span>
            <StatusBadge tone={summary.onlineServices === services.length ? "green" : "yellow"}>
              {formatPercent(summary.serviceRatio)}
            </StatusBadge>
          </div>
          {loading.services ? (
            <Skeleton className="mt-3 h-8 w-20" />
          ) : (
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {summary.onlineServices}
                <span className="ml-1 text-sm font-normal text-[var(--text-muted)]">
                  / {services.length} {isZh ? "正常" : "healthy"}
                </span>
              </span>
              <Link
                href="/services"
                className="text-xs font-medium text-[var(--accent-color)] hover:underline"
              >
                {isZh ? "查看探针 →" : "Probes →"}
              </Link>
            </div>
          )}
          <div className="mt-3">
            <ProgressBar value={summary.serviceRatio} tone="indigo" />
          </div>
        </BrutalCard>

        <BrutalCard className="flex flex-col justify-between p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-medium text-[var(--text-muted)]">
              {copy.dashboardPage.kpiAlerts}
            </span>
            <StatusBadge tone={summary.activeAlerts > 0 ? "red" : "green"}>
              {summary.activeAlerts > 0
                ? `${summary.activeAlerts} ${isZh ? "未恢复" : "Active"}`
                : isZh
                  ? "无活跃告警"
                  : "All Clear"}
            </StatusBadge>
          </div>
          {loading.alerts ? (
            <Skeleton className="mt-3 h-8 w-20" />
          ) : (
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {summary.activeAlerts}
              </span>
              <Link
                href="/alerts"
                className="text-xs font-medium text-[var(--accent-color)] hover:underline"
              >
                {isZh ? "告警规则 →" : "Alert Rules →"}
              </Link>
            </div>
          )}
          <div className="mt-2.5 border-t border-[var(--border-color)] pt-2 text-xs text-[var(--text-muted)]">
            {copy.dashboardPage.kpiAlertsDetail}
          </div>
        </BrutalCard>
      </div>

      {/* Main Two-Column Ops Workspace */}
      <div className="mt-6 grid gap-6 xl:grid-cols-[minmax(0,1.65fr)_minmax(0,1fr)]">
        {/* Left: Real-Time Fleet Telemetry Matrix */}
        <BrutalCard className="flex flex-col">
          <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2.5">
              <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                {copy.dashboardPage.serversHeading}
              </h2>
              <StatusBadge tone="green">
                {summary.onlineServers} / {servers.length}
              </StatusBadge>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
                {(["all", "online", "offline"] as const).map((mode) => {
                  const active = fleetFilter === mode;
                  const label =
                    mode === "all"
                      ? isZh
                        ? "全部"
                        : "All"
                      : mode === "online"
                        ? isZh
                          ? "在线"
                          : "Online"
                        : isZh
                          ? "离线"
                          : "Offline";
                  return (
                    <button
                      key={mode}
                      type="button"
                      onClick={() => setFleetFilter(mode)}
                      className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                        active
                          ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
              <Link href="/servers" className={buttonClass("secondary")}>
                {isZh ? "全部节点" : "All Servers"}
              </Link>
            </div>
          </div>

          {serversLoading ? (
            <div className="grid gap-2.5">
              {Array.from({ length: 4 }).map((_, index) => (
                <div
                  key={index}
                  className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5"
                >
                  <Skeleton className="h-4 w-48" />
                  <Skeleton className="mt-2.5 h-3 w-full" />
                </div>
              ))}
            </div>
          ) : filteredServers.length === 0 ? (
            <EmptyState
              title={copy.dashboardPage.serversEmptyTitle}
              detail={copy.dashboardPage.serversEmptyDetail}
            />
          ) : (
            <div className="grid gap-2.5">
              {filteredServers.slice(0, 12).map((server) => {
                const isOnline = server.status === "online";
                const cpu = typeof server.cpu_percent === "number" ? server.cpu_percent : 0;
                const memPercent =
                  typeof server.memory_used === "number" &&
                  typeof server.memory_total === "number" &&
                  server.memory_total > 0
                    ? (server.memory_used / server.memory_total) * 100
                    : 0;
                const rxBps = Number(server.net_rx_bps ?? 0) || 0;
                const txBps = Number(server.net_tx_bps ?? 0) || 0;
                const regionText = [server.provider, server.region || server.country]
                  .filter(Boolean)
                  .join(" · ");

                return (
                  <div
                    key={server.id}
                    className="group grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]/60 p-3.5 transition-all hover:border-[var(--border-strong)] hover:bg-[var(--bg-card)] hover:shadow-[var(--shadow-xs)] md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_9.5rem_auto] md:items-center"
                  >
                    {/* Server Identity */}
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span
                          className={`h-2 w-2 shrink-0 rounded-full ${
                            isOnline ? "bg-emerald-500" : "bg-rose-500"
                          }`}
                        />
                        <Link
                          href={`/servers/${encodeURIComponent(server.id)}`}
                          className="truncate text-sm font-semibold text-[var(--text-main)] hover:text-[var(--accent-color)]"
                        >
                          {server.name}
                        </Link>
                        <StatusBadge tone={isOnline ? "green" : "red"}>
                          {serverStatusLabel(server.status, copy)}
                        </StatusBadge>
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-2 pl-4 text-[11px] text-[var(--text-muted)]">
                        {regionText ? <span>{regionText}</span> : null}
                        {typeof server.load_1 === "number" ? (
                          <span className="font-mono">Load {server.load_1.toFixed(2)}</span>
                        ) : null}
                        <span className="font-mono opacity-75">
                          {formatDate(server.last_seen_at)}
                        </span>
                      </div>
                    </div>

                    {/* CPU & Memory Mini Bars */}
                    <div className="grid grid-cols-2 gap-3">
                      <div>
                        <div className="mb-1 flex items-center justify-between text-[11px]">
                          <span className="text-[var(--text-muted)]">CPU</span>
                          <span className="font-mono font-medium text-[var(--text-main)] tabular-nums">
                            {isOnline ? formatPercent(cpu) : "-"}
                          </span>
                        </div>
                        <ProgressBar value={isOnline ? cpu : 0} tone="emerald" />
                      </div>
                      <div>
                        <div className="mb-1 flex items-center justify-between text-[11px]">
                          <span className="text-[var(--text-muted)]">RAM</span>
                          <span className="font-mono font-medium text-[var(--text-main)] tabular-nums">
                            {isOnline ? formatPercent(memPercent) : "-"}
                          </span>
                        </div>
                        <ProgressBar value={isOnline ? memPercent : 0} tone="indigo" />
                      </div>
                    </div>

                    {/* Live Network Speed */}
                    <div className="flex items-center justify-between gap-2 font-mono text-xs md:flex-col md:items-end md:justify-center md:gap-0.5">
                      <span className="text-emerald-600 dark:text-emerald-400 tabular-nums">
                        ↓ {formatBytes(rxBps)}/s
                      </span>
                      <span className="text-indigo-600 dark:text-indigo-400 tabular-nums">
                        ↑ {formatBytes(txBps)}/s
                      </span>
                    </div>

                    {/* Quick Action Buttons */}
                    <div className="flex items-center justify-end gap-1.5">
                      <Link
                        href={`/terminal?server=${encodeURIComponent(server.id)}`}
                        title={isZh ? "打开 Web 终端" : "Open Web Terminal"}
                        className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1.5 font-mono text-[11px] font-medium text-[var(--text-muted)] transition hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
                      >
                        &gt;_
                      </Link>
                      <Link
                        href={`/servers/${encodeURIComponent(server.id)}`}
                        className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-main)] transition hover:border-[var(--accent-color)] hover:text-[var(--accent-color)]"
                      >
                        {isZh ? "详情" : "Detail"}
                      </Link>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </BrutalCard>

        {/* Right Column: Service Probes & Recent Alerts */}
        <div className="grid content-start gap-6">
          {/* Active Service Probes */}
          <BrutalCard>
            <div className="mb-4 flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {copy.dashboardPage.kpiServices}
                </h2>
                <StatusBadge tone={summary.onlineServices === services.length ? "green" : "yellow"}>
                  {summary.onlineServices} / {services.length}
                </StatusBadge>
              </div>
              <Link
                href="/services"
                className="text-xs font-medium text-[var(--text-muted)] hover:text-[var(--text-main)]"
              >
                {isZh ? "管理 →" : "Manage →"}
              </Link>
            </div>

            {loading.services ? (
              <div className="grid gap-2">
                {Array.from({ length: 3 }).map((_, index) => (
                  <Skeleton key={index} className="h-12 w-full" />
                ))}
              </div>
            ) : services.length === 0 ? (
              <div className="rounded-xl border border-dashed border-[var(--border-color)] bg-[var(--bg-subtle)] px-4 py-6 text-center text-xs text-[var(--text-muted)]">
                {isZh ? "尚未配置服务拨测监控" : "No service probes configured"}
              </div>
            ) : (
              <div className="grid gap-2">
                {services.slice(0, 6).map((service) => {
                  const up = ["success", "up"].includes(service.last_status || "");
                  return (
                    <div
                      key={service.id}
                      className="flex items-center justify-between gap-3 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3.5 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span
                            className={`h-2 w-2 shrink-0 rounded-full ${
                              up ? "bg-emerald-500" : "bg-rose-500"
                            }`}
                          />
                          <span className="truncate text-xs font-semibold text-[var(--text-main)]">
                            {service.name}
                          </span>
                        </div>
                        <div className="mt-0.5 truncate pl-4 font-mono text-[11px] text-[var(--text-muted)]">
                          {service.target}
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        {typeof service.last_latency_ms === "number" ? (
                          <span className="font-mono text-xs text-[var(--text-muted)] tabular-nums">
                            {service.last_latency_ms} ms
                          </span>
                        ) : null}
                        <StatusBadge tone={up ? "green" : "red"}>
                          {up ? "UP" : service.last_status || "DOWN"}
                        </StatusBadge>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </BrutalCard>

          {/* Recent Alerts Timeline */}
          <BrutalCard>
            <div className="mb-4 flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {copy.dashboardPage.recentAlertsHeading}
                </h2>
                <StatusBadge tone={summary.activeAlerts > 0 ? "yellow" : "gray"}>
                  {alerts.length}
                </StatusBadge>
              </div>
              <Link
                href="/alerts"
                className="text-xs font-medium text-[var(--text-muted)] hover:text-[var(--text-main)]"
              >
                {isZh ? "全部告警 →" : "All Alerts →"}
              </Link>
            </div>
            {loading.alerts ? (
              <div className="grid gap-2.5">
                {Array.from({ length: 3 }).map((_, index) => (
                  <div
                    key={index}
                    className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5"
                  >
                    <Skeleton className="h-4 w-40" />
                    <Skeleton className="mt-2 h-3.5 w-3/4" />
                  </div>
                ))}
              </div>
            ) : alerts.length === 0 ? (
              <EmptyState
                title={copy.dashboardPage.alertsEmptyTitle}
                detail={copy.dashboardPage.alertsEmptyDetail}
              />
            ) : (
              <div className="grid gap-2.5">
                {alerts.slice(0, 6).map((alert, index) => (
                  <div
                    key={alert.id || index}
                    className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5 transition-colors hover:border-[var(--border-strong)]"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-semibold text-[var(--text-main)]">
                        {alert.rule_name || copy.dashboardPage.alertFallbackName}
                      </span>
                      <StatusBadge tone={alert.status === "resolved" ? "green" : "yellow"}>
                        {alert.status || "firing"}
                      </StatusBadge>
                    </div>
                    <div className="mt-1 text-xs text-[var(--text-muted)]">
                      {alert.message || alert.status || copy.dashboardPage.alertFallbackEvent}
                    </div>
                    <div className="mt-2 font-mono text-[11px] text-[var(--text-muted)]">
                      {formatDate(alert.created_at)}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </BrutalCard>
        </div>
      </div>
    </PageShell>
  );
}

function serverStatusLabel(status: string, copy: Translations): string {
  if (status === "online") return copy.serversPage.statusOnline;
  if (status === "offline") return copy.serversPage.statusOffline;
  if (status === "revoked") return copy.serversPage.statusRevoked;
  return status;
}
