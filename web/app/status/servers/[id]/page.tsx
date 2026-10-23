"use client";

import Link from "next/link";
import { use, useCallback, useEffect, useMemo, useState } from "react";
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
import {
  MiniLineChart,
  averageDelay,
  buildPublicServiceDays,
  publicServiceDayClass,
  type ChartPoint,
  type ChartSeries,
  type PublicServiceDay,
} from "@/app/components/charts";
import { apiClient } from "@/lib/api";
import { durationLabel, formatRate } from "@/app/lib/format";
import { sanitizeCssVariableValue } from "@/app/lib/security";
import type { Translations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";

interface PageProps {
  params: Promise<{ id: string }> | { id: string };
}

interface PublicServerDetail {
  id: string;
  name: string;
  remark?: string | null | undefined;
  public_note?: string | null | undefined;
  accent_color?: string | null | undefined;
  status: string;
  last_seen_at?: string | null | undefined;
  resources?: PublicServerResources | null | undefined;
  metrics?: PublicServerMetrics | null | undefined;
}

interface PublicServerResources {
  cpu_percent?: number | null | undefined;
  memory_used?: number | null | undefined;
  memory_total?: number | null | undefined;
  memory_percent?: number | null | undefined;
  disk_used?: number | null | undefined;
  disk_total?: number | null | undefined;
  disk_percent?: number | null | undefined;
  load_1?: number | null | undefined;
  net_rx_bps?: number | null | undefined;
  net_tx_bps?: number | null | undefined;
  network_in_total?: number | null | undefined;
  network_out_total?: number | null | undefined;
  uptime_seconds?: number | null | undefined;
  tcp_connections?: number | null | undefined;
  udp_connections?: number | null | undefined;
  process_count?: number | null | undefined;
}

interface PublicServerMetrics {
  range: string;
  samples: PublicMetricSample[];
}

interface PublicMetricSample {
  sample_at: string;
  cpu_percent?: number | null | undefined;
  memory_percent?: number | null | undefined;
  disk_percent?: number | null | undefined;
  load_1?: number | null | undefined;
  net_rx_bps?: number | null | undefined;
  net_tx_bps?: number | null | undefined;
  network_in_total?: number | null | undefined;
  network_out_total?: number | null | undefined;
  tcp_connections?: number | null | undefined;
  udp_connections?: number | null | undefined;
  process_count?: number | null | undefined;
}

interface PublicServiceResult {
  server_id?: string | null | undefined;
  status: string;
  delay_ms?: number | null | undefined;
  created_at: string;
}

interface PublicService {
  id: string;
  name: string;
  service_type?: string | undefined;
  kind?: string | undefined;
  type?: string | undefined;
  server_id?: string | null | undefined;
  server_ids?: string[] | undefined;
  last_status?: string | null | undefined;
  last_check_at?: string | null | undefined;
  history?: PublicServiceResult[] | undefined;
}

export default function PublicServerDetailPage({ params }: PageProps) {
  const resolved = params instanceof Promise ? use(params) : params;
  const serverId = resolved.id;
  const { t: copy } = useI18n();
  const [server, setServer] = useState<PublicServerDetail | null>(null);
  const [services, setServices] = useState<PublicService[]>([]);
  const [loading, setLoading] = useState(true);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [servicesError, setServicesError] = useState<string | null>(null);

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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount
    void loadServer(controller.signal);
    return () => controller.abort();
  }, [loadServer]);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount
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
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">{copy.statusDetailPage.relatedServicesHeading}</h2>
                {servicesLoading ? (
                  <span className="rounded-full border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 text-xs font-medium text-[var(--text-muted)]">
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
  const safeAccentColor = sanitizeCssVariableValue(server.accent_color);

  return (
    <section
      className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-xs)]"
      style={safeAccentColor ? { borderTopColor: safeAccentColor, borderTopWidth: "3px" } : undefined}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="break-words text-lg font-semibold tracking-tight text-[var(--text-main)]">{server.name}</h2>
          <p className="mt-0.5 break-all font-mono text-xs text-[var(--text-muted)]">{server.id}</p>
          <p className="mt-2 max-w-3xl break-words text-sm text-[var(--text-muted)]">{note}</p>
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
  const charts = useMemo(() => buildMetricCharts(metrics.samples), [metrics.samples]);
  const resourceSeries = useMemo(
    () =>
      [
        { id: "cpu", label: copy.statusDetailPage.seriesCpu, color: "var(--accent-color)", points: charts.cpu },
        { id: "memory", label: copy.statusDetailPage.seriesMemory, color: "#6366f1", points: charts.memory },
        { id: "disk", label: copy.statusDetailPage.seriesDisk, color: "#f59e0b", points: charts.disk },
      ].filter((series) => series.points.length > 0),
    [charts, copy],
  );
  const networkSeries = useMemo(
    () =>
      [
        { id: "rx", label: copy.statusDetailPage.seriesDownload, color: "var(--accent-color)", points: charts.rx },
        { id: "tx", label: copy.statusDetailPage.seriesUpload, color: "#6366f1", points: charts.tx },
      ].filter((series) => series.points.length > 0),
    [charts, copy],
  );

  if (!resourceSeries.length && !networkSeries.length) return null;

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">{copy.statusDetailPage.chartsHeading}</h2>
        <span className="rounded-full border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 font-mono text-xs font-medium text-[var(--text-muted)]">
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
  maxValue?: number | undefined;
  formatValue: (value: number) => string;
}) {
  const { t: copy } = useI18n();
  return (
    <section className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-xs)]">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold tracking-tight text-[var(--text-main)]">{title}</h3>
          <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">{value}</p>
        </div>
        <span className="rounded-full border border-emerald-500/25 bg-[var(--status-ok-bg)] px-2 py-0.5 font-mono text-[10px] font-medium text-[var(--status-ok-text)]">
          LIVE
        </span>
      </div>
      <MiniLineChart
        series={series}
        maxValue={maxValue}
        formatValue={formatValue}
        emptyLabel={copy.statusDetailPage.noHistory}
        ariaLabel={copy.statusDetailPage.chartAriaLabel}
        interactive={false}
        variant="detail"
      />
      <div className="mt-3 flex flex-wrap gap-1.5">
        {series.map((item) => (
          <span key={item.id ?? item.label} className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-xs font-medium text-[var(--text-muted)]">
            <span className="h-2 w-2 rounded-full" style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </section>
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
          <h3 className="break-words text-base font-semibold text-[var(--text-main)]">{service.name}</h3>
          <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">{serviceKind(service, copy)}</p>
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
    <div className="mt-4 border-t border-[var(--border-color)] pt-3">
      <div className="grid grid-cols-[repeat(30,minmax(0,1fr))] gap-1">
        {days.map((day) => (
          <span
            key={day.key}
            title={`${day.label} ${formatPercent(day.uptime)} ${day.avgDelay === undefined ? "" : formatMs(Math.round(day.avgDelay))}`}
            className={`h-6 rounded-xs ${publicServiceDayClass(day)}`}
          />
        ))}
      </div>
      <div className="mt-2 flex justify-between text-[11px] text-[var(--text-muted)]">
        <span>{copy.statusDetailPage.daysAgo30}</span>
        <span>{copy.statusDetailPage.today}</span>
      </div>
    </div>
  );
}

function InfoTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
      <div className="text-[11px] font-medium text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words font-mono text-sm font-semibold text-[var(--text-main)]">{value}</div>
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
