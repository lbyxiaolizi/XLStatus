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
  const [server, setServer] = useState<PublicServerDetail | null>(null);
  const [services, setServices] = useState<PublicService[]>([]);
  const [loading, setLoading] = useState(true);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [servicesError, setServicesError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.resolve(params).then((value) => setServerId(value.id));
  }, [params]);

  const loadServer = useCallback(async () => {
    if (!serverId) return;
    setLoading(true);
    setError(null);
    const response = await apiClient.getPublicServer(serverId);
    setLoading(false);
    if (response.success && response.data) {
      setServer(response.data as unknown as PublicServerDetail);
    } else {
      setError(responseError(response));
      setServer(null);
    }
  }, [serverId]);

  const loadServices = useCallback(async () => {
    if (!serverId) return;
    setServicesLoading(true);
    setServicesError(null);
    const response = await apiClient.getPublicStatus();
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
  }, [serverId]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServer();
  }, [loadServer]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServices();
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
          eyebrow="公开服务器"
          title={server?.name || compactId(serverId)}
          detail={server ? `最后上报 ${formatDate(server.last_seen_at)}` : "公开状态详情"}
          actions={
            <>
              <Link href="/status" className={buttonClass("secondary")}>返回状态页</Link>
              {server ? <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status)}</StatusBadge> : null}
            </>
          }
        />
        <InlineError message={error} />

        {loading && !server ? <EmptyState title="正在加载公开服务器" /> : null}

        {server ? (
          <div className="mt-5 grid gap-5">
            <ServerSummary server={server} />
            {server.resources ? <PublicResources resources={server.resources} /> : null}
            {server.metrics?.samples?.length ? <PublicMetricsCharts metrics={server.metrics} /> : null}
            <section>
              <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
                <h2 className="text-xl font-black uppercase">关联公开服务</h2>
                {servicesLoading ? (
                  <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
                    加载中
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
                <EmptyState title="暂无关联公开服务" detail="仅展示已关联到该服务器的公开服务监控历史。" />
              )}
            </section>
          </div>
        ) : null}
      </PageShell>
    </div>
  );
}

function ServerSummary({ server }: { server: PublicServerDetail }) {
  const note = server.public_note || server.remark || "公开服务器";

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
        <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status)}</StatusBadge>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <InfoTile label="状态" value={statusLabel(server.status)} />
        <InfoTile label="最后上报" value={formatDate(server.last_seen_at)} />
      </div>
    </section>
  );
}

function PublicResources({ resources }: { resources: PublicServerResources }) {
  return (
    <section className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <InfoTile label="CPU" value={formatPercent(resources.cpu_percent)} />
      <InfoTile label="内存" value={resourceBytes(resources.memory_used, resources.memory_total, resources.memory_percent)} />
      <InfoTile label="磁盘" value={resourceBytes(resources.disk_used, resources.disk_total, resources.disk_percent)} />
      <InfoTile label="负载" value={resources.load_1 === undefined || resources.load_1 === null ? "N/A" : resources.load_1.toFixed(2)} />
      <InfoTile label="下载" value={formatRate(resources.net_rx_bps)} />
      <InfoTile label="上传" value={formatRate(resources.net_tx_bps)} />
      <InfoTile label="累计流量" value={`↓${formatBytes(resources.network_in_total)} ↑${formatBytes(resources.network_out_total)}`} />
      <InfoTile label="运行时间" value={durationLabel(resources.uptime_seconds)} />
      <InfoTile label="TCP / UDP" value={`${numberLabel(resources.tcp_connections)} / ${numberLabel(resources.udp_connections)}`} />
      <InfoTile label="进程数" value={numberLabel(resources.process_count)} />
    </section>
  );
}

function PublicMetricsCharts({ metrics }: { metrics: PublicServerMetrics }) {
  const charts = buildMetricCharts(metrics.samples);
  const resourceSeries = [
    { id: "cpu", label: "CPU", color: "var(--accent-color)", points: charts.cpu },
    { id: "memory", label: "内存", color: "var(--btn-bg)", points: charts.memory },
    { id: "disk", label: "磁盘", color: "#f97316", points: charts.disk },
  ].filter((series) => series.points.length > 0);
  const networkSeries = [
    { id: "rx", label: "下载", color: "var(--accent-color)", points: charts.rx },
    { id: "tx", label: "上传", color: "var(--btn-bg)", points: charts.tx },
  ].filter((series) => series.points.length > 0);

  if (!resourceSeries.length && !networkSeries.length) return null;

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-xl font-black uppercase">监控图表</h2>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {metrics.range || "1d"}
        </span>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        {resourceSeries.length ? (
          <MetricChartCard title="资源使用率" value={latestPercentLabel(resourceSeries)} series={resourceSeries} maxValue={100} formatValue={formatPercent} />
        ) : null}
        {networkSeries.length ? (
          <MetricChartCard title="网络速率" value={latestRateLabel(networkSeries)} series={networkSeries} formatValue={formatRate} />
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
  const width = 640;
  const height = 190;
  const padding = { top: 14, right: 16, bottom: 30, left: 46 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const allPoints = series.flatMap((item) => item.points).filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
  if (!allPoints.length) {
    return (
      <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
        暂无历史数据
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
    <svg className="h-48 w-full border-2 border-black bg-[var(--bg-page)]" viewBox={`0 0 ${width} ${height}`} role="img" aria-label="公开服务器监控趋势图">
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
          <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{serviceKind(service)}</p>
        </div>
        <StatusBadge tone={serviceTone(service.last_status ?? undefined)}>{statusLabel(service.last_status ?? undefined)}</StatusBadge>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-3">
        <InfoTile label="可用率" value={uptime === undefined ? "N/A" : `${formatPercent(uptime)} 可用`} />
        <InfoTile label="平均延迟" value={avgDelay === undefined ? "N/A" : formatMs(Math.round(avgDelay))} />
        <InfoTile label="最近检查" value={formatDate(service.last_check_at)} />
      </div>
      <PublicServiceHistory days={days} />
    </BrutalCard>
  );
}

function PublicServiceHistory({ days }: { days: PublicServiceDay[] }) {
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
        <span>30 天前</span>
        <span>今天</span>
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

function formatRate(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return `${formatBytes(value)}/s`;
}

function numberLabel(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return String(value);
}

function durationLabel(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
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

function serviceKind(service: PublicService): string {
  return service.service_type || service.kind || service.type || "服务";
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

function statusLabel(status?: string | null): string {
  if (!status) return "未知";
  const labels: Record<string, string> = {
    online: "在线",
    offline: "离线",
    down: "异常",
    degraded: "降级",
    revoked: "已撤销",
    success: "成功",
    up: "正常",
    failure: "失败",
    timeout: "超时",
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
