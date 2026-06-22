"use client";

import Link from "next/link";
import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  InlineNotice,
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
import { apiClient, type PublicSiteBranding, type ThemeDefinition } from "@/lib/api";
import { t } from "@/lib/i18n";

interface Server {
  id: string;
  name: string;
  remark?: string | null;
  public_note?: string | null;
  accent_color?: string | null;
  status: string;
  last_seen_at?: string;
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

interface Service {
  id: string;
  name: string;
  last_status?: string;
  last_check_at?: string;
  kind?: string;
  type?: string;
  service_type?: string;
  server_id?: string | null;
  server_ids?: string[];
  history?: PublicServiceResult[];
}

interface PublicServiceResult {
  status: string;
  delay_ms?: number | null;
  created_at: string;
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

type PublicServerViewMode = "cards" | "compact";

const defaultSiteBranding: PublicSiteBranding = {
  site_name: "XLStatus",
};

export default function StatusPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [services, setServices] = useState<Service[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState("");
  const [site, setSite] = useState<PublicSiteBranding>(defaultSiteBranding);
  const [theme, setTheme] = useState<ThemeDefinition | null>(null);
  const [showServices, setShowServices] = useState(() => initialPublicShowServices());
  const [serverViewMode, setServerViewMode] = useState<PublicServerViewMode>(() => initialPublicServerViewMode());

  useEffect(() => {
    let cancelled = false;

    async function load() {
      setLoading(true);
      setError(null);
      const response = await apiClient.getPublicStatus();

      if (cancelled) return;

      if (response.success && response.data) {
        setServers((response.data.servers as Server[]) ?? []);
        setServices((response.data.services as Service[]) ?? []);
        setUpdatedAt(response.data.updated_at || new Date().toISOString());
        setSite(response.data.site ?? defaultSiteBranding);
        setTheme(response.data.theme ?? null);
      } else {
        setError(responseError(response));
        setUpdatedAt(new Date().toISOString());
        setTheme(null);
      }

      setLoading(false);
    }

    void load();
    const timer = window.setInterval(() => void load(), 30000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => applyPublicHead(site, theme), [site, theme]);

  const overall = useMemo(() => {
    if (servers.length === 0 && services.length === 0) {
      return { label: "暂无公开数据", tone: "gray" as const };
    }
    if (
      servers.some((server) => server.status === "down" || server.status === "offline") ||
      services.some((service) => service.last_status === "failure" || service.last_status === "down")
    ) {
      return { label: "部分异常", tone: "yellow" as const };
    }
    return { label: "运行正常", tone: "green" as const };
  }, [servers, services]);

  function changeServerViewMode(next: PublicServerViewMode) {
    setServerViewMode(next);
    window.localStorage.setItem("xlstatus_public_server_view", next);
  }

  function toggleServices() {
    setShowServices((current) => {
      const next = !current;
      window.localStorage.setItem("xlstatus_public_show_services", next ? "1" : "0");
      return next;
    });
  }

  return (
    <div className="min-h-screen" style={publicPageStyle(site, theme)} data-public-theme-root="true">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="公开状态"
          title={site.site_name || "XLStatus"}
          detail="服务器与服务可用性概览，数据来自后端公开 API。"
          actions={
            <div className="flex flex-wrap items-center justify-end gap-2">
              {site.logo_url ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img src={site.logo_url} alt="" className="h-11 w-11 border-2 border-black bg-[var(--bg-card)] object-contain p-1 shadow-[var(--shadow-brutal-sm)]" />
              ) : null}
              <StatusBadge tone={overall.tone}>{overall.label}</StatusBadge>
            </div>
          }
        />

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          <InlineNotice tone="pink">
            {updatedAt ? `更新于 ${formatDate(updatedAt)}。` : "等待首次刷新。"} 每 30 秒自动刷新。
          </InlineNotice>
        </div>

        <div className="mb-6 grid gap-4 sm:grid-cols-3">
          <Kpi title="服务器" value={String(servers.length)} detail={`${servers.filter((s) => s.status === "online").length} 台在线`} />
          <Kpi title="服务" value={String(services.length)} detail="公开监控项" />
          <Kpi title="刷新" value="30s" detail="实时轮询" />
        </div>

        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal)]">
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={() => changeServerViewMode("cards")} className={buttonClass(serverViewMode === "cards" ? "primary" : "secondary")}>卡片</button>
            <button type="button" onClick={() => changeServerViewMode("compact")} className={buttonClass(serverViewMode === "compact" ? "primary" : "secondary")}>紧凑</button>
          </div>
          <button type="button" onClick={toggleServices} className={buttonClass(showServices ? "primary" : "secondary")}>
            {showServices ? "隐藏服务" : "显示服务"}
          </button>
        </div>

        <div className={`grid gap-6 ${showServices ? "lg:grid-cols-2" : ""}`}>
          <section>
            <h2 className="mb-3 text-xl font-black uppercase">服务器</h2>
            {loading && servers.length === 0 ? (
              <BrutalCard>正在加载公开服务器...</BrutalCard>
            ) : servers.length === 0 ? (
              <EmptyState title="暂无公开服务器" detail="隐藏或未授权的服务器不会出现在公开状态页。" />
            ) : serverViewMode === "compact" ? (
              <div className="grid gap-2">
                {servers.map((server) => (
                  <PublicCompactServerRow key={server.id} server={server} />
                ))}
              </div>
            ) : (
              <div className="grid gap-4">
                {servers.map((server) => (
                  <PublicServerCard key={server.id} server={server} />
                ))}
              </div>
            )}
          </section>

          {showServices ? <section>
            <h2 className="mb-3 text-xl font-black uppercase">服务</h2>
            {services.length === 0 ? (
              <EmptyState title="暂无公开服务" detail="服务监控项公开后会显示在这里。" />
            ) : (
              <div className="grid gap-4">
                {services.map((service) => (
                  <BrutalCard key={service.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-xl font-black">{service.name}</h3>
                      </div>
                      <StatusBadge tone={serviceTone(service.last_status)}>{statusLabel(service.last_status)}</StatusBadge>
                    </div>
                    <div className="mt-4 flex items-center justify-between text-sm font-bold text-[var(--text-muted)]">
                      <span>{serviceKind(service)}</span>
                      <span>检查于 {formatDate(service.last_check_at)}</span>
                    </div>
                    <PublicServiceHistory service={service} />
                  </BrutalCard>
                ))}
              </div>
            )}
          </section> : null}
        </div>
      </PageShell>
    </div>
  );
}

function PublicServiceHistory({ service }: { service: Service }) {
  const days = buildPublicServiceDays(service.history ?? []);
  const checks = days.reduce((sum, day) => sum + day.total, 0);
  const avgDelay = averageDelay(service.history ?? []);
  const uptime = checks
    ? days.reduce((sum, day) => sum + (day.uptime * day.total), 0) / checks
    : undefined;

  return (
    <div className="mt-4 border-t-2 border-black pt-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs font-black text-[var(--text-muted)]">
        <span>{uptime === undefined ? "N/A" : `${formatPercent(uptime)} 可用`}</span>
        <span>{avgDelay === undefined ? "N/A" : formatMs(Math.round(avgDelay))}</span>
      </div>
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

function PublicServerCard({ server }: { server: Server }) {
  const note = server.public_note || server.remark || "公开服务器";
  return (
    <Link
      href={`/status/servers/${encodeURIComponent(server.id)}`}
      className="group block border-2 border-black bg-[var(--bg-card)] p-4 text-[var(--text-main)] shadow-[var(--shadow-brutal)] transition hover:-translate-x-1 hover:-translate-y-1 hover:shadow-[8px_8px_0_0_var(--border-color)] focus:outline-none focus:ring-4 focus:ring-[var(--accent-color)]"
      style={{ borderTopColor: server.accent_color || "var(--border-color)", borderTopWidth: "8px" }}
      aria-label={`查看公开服务器 ${server.name}`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="break-words text-xl font-black uppercase">{server.name}</h3>
          <p className="mt-1 break-all font-mono text-xs font-bold text-[var(--text-muted)]">{server.id}</p>
          <p className="mt-2 text-sm font-bold text-[var(--text-muted)]">
            {note}
          </p>
        </div>
        <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status)}</StatusBadge>
      </div>
      <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
        <Metric label="状态" value={statusLabel(server.status)} />
        <Metric label="最后上报" value={formatDate(server.last_seen_at)} />
      </div>
      {server.resources ? <PublicServerResourceStrip resources={server.resources} /> : null}
      {server.metrics?.samples?.length ? <PublicServerMiniCharts metrics={server.metrics} /> : null}
    </Link>
  );
}

function PublicCompactServerRow({ server }: { server: Server }) {
  return (
    <Link
      href={`/status/servers/${encodeURIComponent(server.id)}`}
      className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3 text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[var(--shadow-brutal)] md:grid-cols-[minmax(10rem,1.3fr)_minmax(5rem,1fr)_minmax(10rem,1fr)]"
      aria-label={`查看公开服务器 ${server.name}`}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className={`h-3 w-3 shrink-0 border-2 border-black ${server.status === "online" ? "bg-[var(--accent-color)]" : "bg-[var(--btn-bg)]"}`} />
          <span className="truncate text-sm font-black">{server.name}</span>
        </div>
        <div className="mt-1 truncate font-mono text-[11px] font-bold text-[var(--text-muted)]">{compactId(server.id)}</div>
      </div>
      <Metric label="状态" value={statusLabel(server.status)} />
      <Metric label="最后上报" value={formatDate(server.last_seen_at)} />
      {server.resources ? (
        <div className="grid grid-cols-2 gap-2 text-xs md:col-span-3 md:grid-cols-4">
          <CompactMetric label="CPU" value={formatPercent(server.resources.cpu_percent)} />
          <CompactMetric label="内存" value={formatPercent(resourceMemoryPercent(server.resources))} />
          <CompactMetric label="下载" value={formatRate(server.resources.net_rx_bps)} />
          <CompactMetric label="上传" value={formatRate(server.resources.net_tx_bps)} />
        </div>
      ) : null}
      {server.metrics?.samples?.length ? (
        <div className="md:col-span-3">
          <PublicServerMiniCharts metrics={server.metrics} compact />
        </div>
      ) : null}
    </Link>
  );
}

function PublicServerResourceStrip({ resources }: { resources: PublicServerResources }) {
  return (
    <div className="mt-4 border-t-2 border-black pt-3">
      <div className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
        <Metric label="CPU" value={formatPercent(resources.cpu_percent)} />
        <Metric label="内存" value={resourceBytes(resources.memory_used, resources.memory_total, resources.memory_percent)} />
        <Metric label="磁盘" value={resourceBytes(resources.disk_used, resources.disk_total, resources.disk_percent)} />
        <Metric label="下载/上传" value={`${formatRate(resources.net_rx_bps)} / ${formatRate(resources.net_tx_bps)}`} />
      </div>
    </div>
  );
}

function PublicServerMiniCharts({ metrics, compact = false }: { metrics: PublicServerMetrics; compact?: boolean }) {
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
    <div className="mt-4 border-t-2 border-black pt-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs font-black text-[var(--text-muted)]">
        <span>监控图表</span>
        <span>{metrics.range || "1d"}</span>
      </div>
      <div className={`grid gap-3 ${compact ? "" : "lg:grid-cols-2"}`}>
        {resourceSeries.length ? (
          <MetricChartCard
            title="资源使用率"
            value={latestPercentLabel(resourceSeries)}
            series={resourceSeries}
            maxValue={100}
            formatValue={formatPercent}
          />
        ) : null}
        {networkSeries.length ? (
          <MetricChartCard
            title="网络速率"
            value={latestRateLabel(networkSeries)}
            series={networkSeries}
            formatValue={formatRate}
          />
        ) : null}
      </div>
    </div>
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
    <div className="min-w-0">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs font-black uppercase text-[var(--text-muted)]">{title}</span>
        <span className="max-w-full truncate text-xs font-black text-[var(--text-muted)]">{value}</span>
      </div>
      <MiniLineChart series={series} maxValue={maxValue} formatValue={formatValue} />
      <div className="mt-2 flex flex-wrap gap-2">
        {series.map((item) => (
          <span key={item.id} className="inline-flex items-center gap-1.5 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-[10px] font-black">
            <span className="h-2.5 w-2.5 border-2 border-black" style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}

function MiniLineChart({ series, maxValue, formatValue }: { series: ChartSeries[]; maxValue?: number; formatValue: (value?: number | null) => string }) {
  const width = 640;
  const height = 160;
  const padding = { top: 12, right: 14, bottom: 28, left: 44 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const allPoints = series.flatMap((item) => item.points).filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
  if (!allPoints.length) {
    return (
      <div className="flex h-36 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-xs font-black">
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
    <svg className="h-40 w-full border-2 border-black bg-[var(--bg-page)]" viewBox={`0 0 ${width} ${height}`} role="img" aria-label="公开服务器监控趋势图">
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

function Kpi({ title, value, detail }: { title: string; value: string; detail: string }) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{title}</div>
      <div className="mt-2 text-4xl font-black">{value}</div>
      <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 font-black">{value}</div>
    </div>
  );
}

function CompactMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[10px] font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="truncate font-black">{value}</div>
    </div>
  );
}

function resourceMemoryPercent(resources: PublicServerResources): number | null | undefined {
  return resources.memory_percent ?? percentFromUsedTotal(resources.memory_used, resources.memory_total);
}

function resourceBytes(used?: number | null, total?: number | null, percent?: number | null): string {
  if (used !== undefined && used !== null && total !== undefined && total !== null && total > 0) {
    return `${formatBytes(used)} / ${formatBytes(total)}`;
  }
  if (percent !== undefined && percent !== null) return formatPercent(percent);
  return "N/A";
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

function percentFromUsedTotal(used?: number | null, total?: number | null): number | undefined {
  if (used === undefined || used === null || !total) return undefined;
  return (used / total) * 100;
}

function formatRate(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return `${formatBytes(value)}/s`;
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

function serviceKind(service: Service): string {
  return service.service_type || service.kind || service.type || "服务";
}

function statusLabel(status?: string): string {
  if (!status) return t.common.unknown;
  const labels: Record<string, string> = {
    online: "在线",
    offline: "离线",
    down: "异常",
    degraded: "降级",
    revoked: "已撤销",
    success: t.common.success,
    up: "正常",
    failure: t.common.failure,
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

function initialPublicShowServices(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_public_show_services") === "1";
}

function publicPageStyle(site: PublicSiteBranding, theme?: ThemeDefinition | null): CSSProperties {
  const style: CSSProperties & Record<string, string> = {};
  if (theme) {
    for (const [key, value] of Object.entries(themeVariablesForMode(theme, false))) {
      if (key.startsWith("--") && value) {
        style[key] = value;
      }
    }
  }
  if (site.theme_color) {
    style["--accent-color"] = site.theme_color;
  }
  if (site.background_url) {
    style.backgroundImage = `linear-gradient(rgba(255,255,255,.88), rgba(255,255,255,.88)), url(${site.background_url})`;
    style.backgroundSize = "cover";
    style.backgroundPosition = "center";
    style.backgroundAttachment = "fixed";
  }
  return style;
}

function applyPublicHead(site: PublicSiteBranding, theme?: ThemeDefinition | null): () => void {
  document.title = site.site_name || "XLStatus";
  const created: Element[] = [];

  if (site.favicon_url) {
    const favicon = document.createElement("link");
    favicon.rel = "icon";
    favicon.href = site.favicon_url;
    favicon.dataset.xlstatusCustomHead = "true";
    document.head.appendChild(favicon);
    created.push(favicon);
  }
  if (site.theme_color) {
    const theme = document.createElement("meta");
    theme.name = "theme-color";
    theme.content = site.theme_color;
    theme.dataset.xlstatusCustomHead = "true";
    document.head.appendChild(theme);
    created.push(theme);
  }
  if (theme) {
    const style = document.createElement("style");
    style.dataset.xlstatusCustomHead = "true";
    style.textContent = [
      themeModeCss("[data-public-theme-root=\"true\"]", themeVariablesForMode(theme, true), ".dark-mode"),
    ]
      .filter(Boolean)
      .join("\n");
    document.head.appendChild(style);
    created.push(style);
  }

  return () => {
    for (const node of created) {
      node.remove();
    }
  };
}

function themeVariablesForMode(theme: ThemeDefinition, isDark: boolean): Record<string, string> {
  const base = theme.variables ?? {};
  const light = Object.keys(theme.light_variables ?? {}).length ? theme.light_variables ?? {} : base;
  const dark = Object.keys(theme.dark_variables ?? {}).length ? theme.dark_variables ?? {} : light;
  return isDark ? { ...base, ...dark } : { ...base, ...light };
}

function themeModeCss(selector: string, variables: Record<string, string>, prefix = ""): string {
  const lines = Object.entries(variables)
    .filter(([key, value]) => key.startsWith("--") && Boolean(value))
    .map(([key, value]) => `${key}: ${value};`);
  if (!lines.length) return "";
  return `${prefix} ${selector} { ${lines.join(" ")} }`;
}

function initialPublicServerViewMode(): PublicServerViewMode {
  if (typeof window === "undefined") return "cards";
  const stored = window.localStorage.getItem("xlstatus_public_server_view");
  return stored === "compact" ? "compact" : "cards";
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
