"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  compactId,
  formatBytes,
  formatDate,
  formatMs,
  formatPercent,
  responseError,
} from "@/app/components/M7Primitives";
import { apiClient } from "@/lib/api";
import { durationLabel, formatRate } from "@/app/lib/format";
import type { Translations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";

interface PageProps {
  params: Promise<{ id: string }> | { id: string };
}

interface PublicServerDetail {
  id: string;
  name: string;
  remark?: string | null;
  public_note?: string | null;
  accent_color?: string | null;
  status: string;
  last_seen_at?: string | null;
  resources?: PublicServerResources | null;
  metrics?: PublicServerMetrics | null;
}

interface PublicServerResources {
  cpu_percent?: number | null;
  memory_used?: number | null;
  memory_total?: number | null;
  memory_percent?: number | null;
  disk_used?: number | null;
  disk_total?: number | null;
  disk_percent?: number | null;
  load_1?: number | null;
  net_rx_bps?: number | null;
  net_tx_bps?: number | null;
  network_in_total?: number | null;
  network_out_total?: number | null;
  uptime_seconds?: number | null;
  tcp_connections?: number | null;
  udp_connections?: number | null;
  process_count?: number | null;
}

interface PublicServerMetrics {
  range: string;
  samples: PublicMetricSample[];
}

interface PublicMetricSample {
  sample_at: string;
  cpu_percent?: number | null;
  memory_percent?: number | null;
  disk_percent?: number | null;
  load_1?: number | null;
  net_rx_bps?: number | null;
  net_tx_bps?: number | null;
  network_in_total?: number | null;
  network_out_total?: number | null;
  tcp_connections?: number | null;
  udp_connections?: number | null;
  process_count?: number | null;
}

interface PublicServiceResult {
  server_id?: string | null;
  status: string;
  delay_ms?: number | null;
  created_at: string;
}

interface PublicService {
  id: string;
  name: string;
  service_type?: string;
  kind?: string;
  type?: string;
  server_id?: string | null;
  server_ids?: string[];
  last_status?: string | null;
  last_check_at?: string | null;
  history?: PublicServiceResult[];
}

interface PublicServiceDay {
  key: string;
  label: string;
  uptime: number;
  avgDelay?: number;
  total: number;
}

interface ChartPoint {
  time: number;
  value: number;
}

interface ChartSeries {
  id: string;
  label: string;
  color: string;
  points: ChartPoint[];
}

export default function PublicServerDetailPage({ params }: PageProps) {
  const [serverId, setServerId] = useState("");
  const { t: copy } = useI18n();
  const [server, setServer] = useState<PublicServerDetail | null>(null);
  const [services, setServices] = useState<PublicService[]>([]);
  const [loading, setLoading] = useState(true);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [servicesError, setServicesError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.resolve(params).then((value) => setServerId(value.id));
  }, [params]);

  const loadServer = useCallback(
    async (signal?: AbortSignal) => {
      if (!serverId) return;
      setLoading(true);
      setError(null);
      const response = await apiClient.getPublicServer(serverId, { signal });
      if (signal?.aborted) return;
      setLoading(false);
      if (response.success && response.data) {
        setServer(response.data as unknown as PublicServerDetail);
      } else {
        setError(responseError(response));
        setServer(null);
      }
    },
    [serverId],
  );

  const loadServices = useCallback(
    async (signal?: AbortSignal) => {
      if (!serverId) return;
      setServicesLoading(true);
      setServicesError(null);
      const response = await apiClient.getPublicStatus({ signal });
      if (signal?.aborted) return;
      setServicesLoading(false);
      if (response.success && response.data) {
        const publicServices = ((response.data.services as PublicService[]) ?? []).filter((service) =>
          serviceBelongsToServer(service, serverId),
        );
        setServices(publicServices);
      } else {
        setServices([]);
        setServicesError(responseError(response));
      }
    },
    [serverId],
  );

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServer(controller.signal);
    return () => controller.abort();
  }, [loadServer]);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServices(controller.signal);
    return () => controller.abort();
  }, [loadServices]);

  const visibleServices = useMemo(
    () =>
      services.map((service) => ({
        ...service,
        history: (service.history ?? []).filter((result) => !result.server_id || result.server_id === serverId),
      })),
    [serverId, services],
  );

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow={copy.statusDetailPage.eyebrow}
          title={server?.name || compactId(serverId)}
          detail={server ? copy.statusDetailPage.lastSeen.replace("{time}", formatDate(server.last_seen_at)) : copy.statusDetailPage.headerDetailFallback}
          actions={
            <>
              <Link href="/status" className={buttonClass("secondary")}>{copy.statusDetailPage.backToStatus}</Link>
              {server ? <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status, copy)}</StatusBadge> : null}
            </>
          }
        />
        <InlineError message={error} />

        {loading && !server ? <EmptyState title={copy.statusDetailPage.loadingServer} /> : null}

        {server ? (
          <div className="mt-5 grid gap-5">
            <ServerSummary server={server} />
            {server.resources ? <PublicResources resources={server.resources} /> : null}
            {server.metrics?.samples?.length ? <PublicMetricsCharts metrics={server.metrics} /> : null}
            <section>
              <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                <h2 className="text-xl font-black uppercase">{copy.statusDetailPage.relatedServicesHeading}</h2>
                {servicesLoading ? (
                  <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
                    {copy.statusDetailPage.loading}
                  </span>
                ) : null}
              </div>
              <InlineError message={servicesError} />
              {visibleServices.length ? (
                <div className="grid gap-4 lg:grid-cols-2">
                  {visibleServices.map((service) => (
                    <PublicServiceCard key={service.id} service={service} />
                  ))}
                </div>
              ) : (
                <EmptyState title={copy.statusDetailPage.relatedServicesEmptyTitle} detail={copy.statusDetailPage.relatedServicesEmptyDetail} />
              )}
            </section>
          </div>
        ) : null}
      </PageShell>
    </div>
  );
}

function ServerSummary({ server }: { server: PublicServerDetail }) {
  const { t: copy } = useI18n();
  const note = server.public_note || server.remark || copy.statusDetailPage.publicServerNote;

  return (
    <section
      className="border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]"
      style={{ borderTopColor: server.accent_color || "var(--border-color)", borderTopWidth: "8px" }}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="break-words text-2xl font-black uppercase">{server.name}</h2>
          <p className="mt-1 break-all font-mono text-xs font-bold text-[var(--text-muted)]">{server.id}</p>
          <p className="mt-3 max-w-3xl break-words text-sm font-bold text-[var(--text-muted)]">{note}</p>
        </div>
        <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status, copy)}</StatusBadge>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <InfoTile label={copy.statusDetailPage.labelStatus} value={statusLabel(server.status, copy)} />
        <InfoTile label={copy.statusDetailPage.labelLastSeen} value={formatDate(server.last_seen_at)} />
      </div>
    </section>
  );
}

function PublicResources({ resources }: { resources: PublicServerResources }) {
  const { t: copy } = useI18n();
  return (
    <section className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <InfoTile label={copy.statusDetailPage.labelCpu} value={formatPercent(resources.cpu_percent)} />
      <InfoTile label={copy.statusDetailPage.labelMemory} value={resourceBytes(resources.memory_used, resources.memory_total, resources.memory_percent)} />
      <InfoTile label={copy.statusDetailPage.labelDisk} value={resourceBytes(resources.disk_used, resources.disk_total, resources.disk_percent)} />
      <InfoTile label={copy.statusDetailPage.labelLoad} value={resources.load_1 === undefined || resources.load_1 === null ? "N/A" : resources.load_1.toFixed(2)} />
      <InfoTile label={copy.statusDetailPage.labelDownload} value={formatRate(resources.net_rx_bps)} />
      <InfoTile label={copy.statusDetailPage.labelUpload} value={formatRate(resources.net_tx_bps)} />
      <InfoTile label={copy.statusDetailPage.labelTotalTraffic} value={`↓${formatBytes(resources.network_in_total)} ↑${formatBytes(resources.network_out_total)}`} />
      <InfoTile label={copy.statusDetailPage.labelUptime} value={durationLabel(resources.uptime_seconds)} />
      <InfoTile label={copy.statusDetailPage.labelTcpUdp} value={`${numberLabel(resources.tcp_connections)} / ${numberLabel(resources.udp_connections)}`} />
      <InfoTile label={copy.statusDetailPage.labelProcessCount} value={numberLabel(resources.process_count)} />
    </section>
  );
}

function PublicMetricsCharts({ metrics }: { metrics: PublicServerMetrics }) {
  const { t: copy } = useI18n();
  const charts = buildMetricCharts(metrics.samples);
  const resourceSeries = [
    { id: "cpu", label: copy.statusDetailPage.seriesCpu, color: "var(--accent-color)", points: charts.cpu },
    { id: "memory", label: copy.statusDetailPage.seriesMemory, color: "var(--btn-bg)", points: charts.memory },
    { id: "disk", label: copy.statusDetailPage.seriesDisk, color: "#f97316", points: charts.disk },
  ].filter((series) => series.points.length > 0);
  const networkSeries = [
    { id: "rx", label: copy.statusDetailPage.seriesDownload, color: "var(--accent-color)", points: charts.rx },
    { id: "tx", label: copy.statusDetailPage.seriesUpload, color: "var(--btn-bg)", points: charts.tx },
  ].filter((series) => series.points.length > 0);

  if (!resourceSeries.length && !networkSeries.length) return null;

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-xl font-black uppercase">{copy.statusDetailPage.chartsHeading}</h2>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {metrics.range || "1d"}
        </span>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        {resourceSeries.length ? (
          <MetricChartCard title={copy.statusDetailPage.chartResourceUsage} value={latestPercentLabel(resourceSeries)} series={resourceSeries} maxValue={100} formatValue={formatPercent} />
        ) : null}
        {networkSeries.length ? (
          <MetricChartCard title={copy.statusDetailPage.chartNetworkRate} value={latestRateLabel(networkSeries)} series={networkSeries} formatValue={formatRate} />
        ) : null}
      </div>
    </section>
  );
}

function MetricChartCard({
  title,
  value,
  series,
  maxValue,
  formatValue,
}: {
  title: string;
  value: string;
  series: ChartSeries[];
  maxValue?: number;
  formatValue: (value?: number | null) => string;
}) {
  return (
    <section className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-xl font-black uppercase">{title}</h3>
          <p className="mt-1 text-sm font-black text-[var(--text-muted)]">{value}</p>
        </div>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          LIVE
        </span>
      </div>
      <MiniLineChart series={series} maxValue={maxValue} formatValue={formatValue} />
      <div className="mt-3 flex flex-wrap gap-2">
        {series.map((item) => (
          <span key={item.id} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
            <span className="h-2.5 w-2.5 border-2 border-black" style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </section>
  );
}

function MiniLineChart({ series, maxValue, formatValue }: { series: ChartSeries[]; maxValue?: number; formatValue: (value?: number | null) => string }) {
  const { t: copy } = useI18n();
  const width = 640;
  const height = 190;
  const padding = { top: 14, right: 16, bottom: 30, left: 46 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const allPoints = series.flatMap((item) => item.points).filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
  if (!allPoints.length) {
    return (
      <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
        {copy.statusDetailPage.noHistory}
      </div>
    );
  }

  const minTime = Math.min(...allPoints.map((point) => point.time));
  const maxTime = Math.max(...allPoints.map((point) => point.time));
  const maxSeriesValue = Math.max(...allPoints.map((point) => point.value), 1);
  const yMax = maxValue ?? Math.max(1, Math.ceil(maxSeriesValue * 1.2));
  const timeSpan = Math.max(maxTime - minTime, 1);

  function x(time: number): number {
    if (allPoints.length === 1) return padding.left + plotWidth / 2;
    return padding.left + ((time - minTime) / timeSpan) * plotWidth;
  }

  function y(value: number): number {
    const clamped = Math.max(0, Math.min(yMax, value));
    return padding.top + plotHeight - (clamped / yMax) * plotHeight;
  }

  return (
    <svg className="h-48 w-full border-2 border-black bg-[var(--bg-page)]" viewBox={`0 0 ${width} ${height}`} role="img" aria-label={copy.statusDetailPage.chartAriaLabel}>
      {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
        const lineY = padding.top + plotHeight * ratio;
        return <line key={ratio} x1={padding.left} x2={width - padding.right} y1={lineY} y2={lineY} stroke="var(--border-color)" strokeOpacity="0.18" strokeWidth="2" />;
      })}
      <text x={8} y={padding.top + 10} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatValue(yMax)}
      </text>
      <text x={8} y={padding.top + plotHeight} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatValue(0)}
      </text>
      <text x={padding.left} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatChartTime(minTime)}
      </text>
      <text x={width - padding.right} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
        {formatChartTime(maxTime)}
      </text>
      {series.map((item) => {
        const points = item.points.filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
        const path = points.map((point, index) => `${index === 0 ? "M" : "L"} ${x(point.time).toFixed(2)} ${y(point.value).toFixed(2)}`).join(" ");
        return (
          <g key={item.id}>
            <path d={path} fill="none" stroke={item.color} strokeLinecap="square" strokeLinejoin="round" strokeWidth="4" />
            {points.length === 1 ? <circle cx={x(points[0].time)} cy={y(points[0].value)} r="5" fill={item.color} stroke="var(--border-color)" strokeWidth="2" /> : null}
          </g>
        );
      })}
    </svg>
  );
}

function PublicServiceCard({ service }: { service: PublicService }) {
  const { t: copy } = useI18n();
  const days = buildPublicServiceDays(service.history ?? []);
  const checks = days.reduce((sum, day) => sum + day.total, 0);
  const avgDelay = averageDelay(service.history ?? []);
  const uptime = checks
    ? days.reduce((sum, day) => sum + (day.uptime * day.total), 0) / checks
    : undefined;

  return (
    <BrutalCard>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="break-words text-xl font-black">{service.name}</h3>
          <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{serviceKind(service, copy)}</p>
        </div>
        <StatusBadge tone={serviceTone(service.last_status ?? undefined)}>{statusLabel(service.last_status ?? undefined, copy)}</StatusBadge>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-3">
        <InfoTile label={copy.statusDetailPage.labelUptimeRate} value={uptime === undefined ? "N/A" : copy.statusDetailPage.uptimeSuffix.replace("{percent}", formatPercent(uptime))} />
        <InfoTile label={copy.statusDetailPage.labelAvgDelay} value={avgDelay === undefined ? "N/A" : formatMs(Math.round(avgDelay))} />
        <InfoTile label={copy.statusDetailPage.labelLastCheck} value={formatDate(service.last_check_at)} />
      </div>
      <PublicServiceHistory days={days} />
    </BrutalCard>
  );
}

function PublicServiceHistory({ days }: { days: PublicServiceDay[] }) {
  const { t: copy } = useI18n();
  return (
    <div className="mt-4 border-t-2 border-black pt-3">
      <div className="grid grid-cols-[repeat(30,minmax(0,1fr))] gap-1">
        {days.map((day) => (
          <span
            key={day.key}
            title={`${day.label} ${formatPercent(day.uptime)} ${day.avgDelay === undefined ? "" : formatMs(Math.round(day.avgDelay))}`}
            className={`h-7 border-2 border-black ${publicServiceDayClass(day)}`}
          />
        ))}
      </div>
      <div className="mt-2 flex justify-between text-xs font-black text-[var(--text-muted)]">
        <span>{copy.statusDetailPage.daysAgo30}</span>
        <span>{copy.statusDetailPage.today}</span>
      </div>
    </div>
  );
}

function InfoTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words text-sm font-black">{value}</div>
    </div>
  );
}

function buildMetricCharts(samples: PublicMetricSample[]) {
  const cpu: ChartPoint[] = [];
  const memory: ChartPoint[] = [];
  const disk: ChartPoint[] = [];
  const rx: ChartPoint[] = [];
  const tx: ChartPoint[] = [];

  for (const sample of samples) {
    const time = new Date(sample.sample_at).getTime();
    if (!Number.isFinite(time)) continue;
    pushPoint(cpu, time, sample.cpu_percent);
    pushPoint(memory, time, sample.memory_percent);
    pushPoint(disk, time, sample.disk_percent);
    pushPoint(rx, time, sample.net_rx_bps);
    pushPoint(tx, time, sample.net_tx_bps);
  }

  return { cpu, memory, disk, rx, tx };
}

function pushPoint(points: ChartPoint[], time: number, value?: number | null) {
  if (typeof value === "number" && Number.isFinite(value)) {
    points.push({ time, value });
  }
}

function latestPercentLabel(series: ChartSeries[]): string {
  return series
    .map((item) => `${item.label} ${formatPercent(latestPointValue(item.points))}`)
    .join(" / ");
}

function latestRateLabel(series: ChartSeries[]): string {
  return series
    .map((item) => `${item.label} ${formatRate(latestPointValue(item.points))}`)
    .join(" / ");
}

function latestPointValue(points: ChartPoint[]): number | undefined {
  return points.length ? points[points.length - 1]?.value : undefined;
}

function resourceBytes(used?: number | null, total?: number | null, percent?: number | null): string {
  if (used !== undefined && used !== null && total !== undefined && total !== null && total > 0) {
    return `${formatBytes(used)} / ${formatBytes(total)}`;
  }
  if (percent !== undefined && percent !== null) return formatPercent(percent);
  return "N/A";
}

function numberLabel(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return String(value);
}

function formatChartTime(value: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function serviceBelongsToServer(service: PublicService, serverId: string): boolean {
  const serverIds = Array.isArray(service.server_ids) ? service.server_ids : [];
  return service.server_id === serverId || serverIds.includes(serverId);
}

function serviceKind(service: PublicService, copy: Translations): string {
  return service.service_type || service.kind || service.type || copy.statusDetailPage.serviceKindFallback;
}

function serverTone(status: string): "green" | "red" | "yellow" | "gray" {
  if (status === "online") return "green";
  if (status === "offline" || status === "down") return "red";
  if (status === "degraded" || status === "revoked") return "yellow";
  return "gray";
}

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}

function statusLabel(status: string | null | undefined, copy: Translations): string {
  if (!status) return copy.common.unknown;
  const labels: Record<string, string> = {
    online: copy.statusDetailPage.statusOnline,
    offline: copy.statusDetailPage.statusOffline,
    down: copy.statusDetailPage.statusDown,
    degraded: copy.statusDetailPage.statusDegraded,
    revoked: copy.statusDetailPage.statusRevoked,
    success: copy.common.success,
    up: copy.statusDetailPage.statusUp,
    failure: copy.common.failure,
    timeout: copy.statusDetailPage.statusTimeout,
  };
  return labels[status] || status;
}

function buildPublicServiceDays(results: PublicServiceResult[]): PublicServiceDay[] {
  const today = startOfDay(new Date());
  const buckets = new Map<string, PublicServiceResult[]>();
  for (const result of results) {
    const date = new Date(result.created_at);
    if (Number.isNaN(date.getTime())) continue;
    const key = dayKey(date);
    buckets.set(key, [...(buckets.get(key) ?? []), result]);
  }

  return Array.from({ length: 30 }, (_, index) => {
    const date = new Date(today);
    date.setDate(today.getDate() - (29 - index));
    const key = dayKey(date);
    const rows = buckets.get(key) ?? [];
    const success = rows.filter((result) => serviceOk(result.status)).length;
    const delays = rows
      .map((result) => (typeof result.delay_ms === "number" ? result.delay_ms : undefined))
      .filter((value): value is number => value !== undefined);
    return {
      key,
      label: date.toLocaleDateString("zh-CN"),
      uptime: rows.length ? (success / rows.length) * 100 : 0,
      avgDelay: delays.length ? delays.reduce((sum, value) => sum + value, 0) / delays.length : undefined,
      total: rows.length,
    };
  });
}

function publicServiceDayClass(day: PublicServiceDay): string {
  if (day.total === 0) return "bg-[var(--accent-bg)]";
  if (day.uptime >= 99) return "bg-[var(--accent-color)]";
  if (day.uptime >= 95) return "bg-yellow-300";
  return "bg-[var(--btn-bg)]";
}

function averageDelay(results: PublicServiceResult[]): number | undefined {
  const values = results
    .map((result) => (typeof result.delay_ms === "number" ? result.delay_ms : undefined))
    .filter((value): value is number => value !== undefined);
  return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : undefined;
}

function serviceOk(status: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
