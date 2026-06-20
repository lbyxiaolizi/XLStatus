"use client";

import Link from "next/link";
import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import { WorldServerMap } from "@/app/components/WorldServerMap";
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
  provider?: string | null;
  region?: string | null;
  country?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  location?: ServerLocation | null;
  plan?: string | null;
  tags?: string[];
  accent_color?: string | null;
  status: string;
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  net_rx_bps?: number | null;
  net_tx_bps?: number | null;
  network_in_total?: number | null;
  network_out_total?: number | null;
  uptime_seconds?: number | null;
  last_seen_at?: string;
}

interface ServerLocation {
  source?: string | null;
  provider?: string | null;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  timezone?: string | null;
}

interface Service {
  id: string;
  name: string;
  target: string;
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

interface PublicServiceDay {
  key: string;
  label: string;
  uptime: number;
  avgDelay?: number;
  total: number;
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
  const [showMap, setShowMap] = useState(() => initialPublicShowMap());
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

  function toggleMap() {
    setShowMap((current) => {
      const next = !current;
      window.localStorage.setItem("xlstatus_public_show_map", next ? "1" : "0");
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
            <button type="button" onClick={toggleMap} className={buttonClass(showMap ? "primary" : "secondary")}>地图</button>
          </div>
          <button type="button" onClick={toggleServices} className={buttonClass(showServices ? "primary" : "secondary")}>
            {showServices ? "隐藏服务" : "显示服务"}
          </button>
        </div>

        {showMap ? (
          <WorldServerMap
            servers={servers}
            title="地理分布"
            ariaLabel="公开服务器国家和地区分布地图"
            serverHref={(server) => `/status/servers/${encodeURIComponent(server.id)}`}
          />
        ) : null}

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
                        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{service.target}</p>
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
        {site.custom_body ? (
          <div className="mt-6" dangerouslySetInnerHTML={{ __html: site.custom_body }} />
        ) : null}
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
  const memoryPercent = memoryPercentValue(server);
  const tags = Array.isArray(server.tags) ? server.tags.filter(Boolean) : [];
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
            {[server.provider, server.region, server.plan].filter(Boolean).join(" / ") || server.remark || "公开服务器"}
          </p>
        </div>
        <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status)}</StatusBadge>
      </div>
      {tags.length ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {tags.map((tag) => (
            <span key={tag} className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-[11px] font-black shadow-[var(--shadow-brutal-sm)]">
              {tag}
            </span>
          ))}
        </div>
      ) : null}
      <div className="mt-4 grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
        <Metric label="CPU" value={formatPercent(server.cpu_percent)} />
        <Metric label="内存" value={memoryPercent === null ? t.common.notAvailable : `${memoryPercent.toFixed(1)}%`} />
        <Metric label="负载" value={server.load_1 !== undefined ? server.load_1.toFixed(2) : t.common.notAvailable} />
        <Metric label="运行" value={durationLabel(server.uptime_seconds)} />
        <Metric label="上传" value={formatRate(server.net_tx_bps)} />
        <Metric label="下载" value={formatRate(server.net_rx_bps)} />
        <Metric label="累计上传" value={formatBytes(server.network_out_total)} />
        <Metric label="累计下载" value={formatBytes(server.network_in_total)} />
      </div>
    </Link>
  );
}

function PublicCompactServerRow({ server }: { server: Server }) {
  return (
    <Link
      href={`/status/servers/${encodeURIComponent(server.id)}`}
      className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3 text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[var(--shadow-brutal)] md:grid-cols-[minmax(10rem,1.3fr)_repeat(5,minmax(5rem,1fr))]"
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
      <Metric label="CPU" value={formatPercent(server.cpu_percent)} />
      <Metric label="内存" value={memoryLabel(server)} />
      <Metric label="上传" value={formatRate(server.net_tx_bps)} />
      <Metric label="下载" value={formatRate(server.net_rx_bps)} />
    </Link>
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

function memoryLabel(server: Server): string {
  const percent = memoryPercentValue(server);
  return percent === null ? t.common.notAvailable : `${percent.toFixed(1)}%`;
}

function memoryPercentValue(server: Server): number | null {
  if (server.memory_used === undefined || server.memory_used === null || !server.memory_total) return null;
  return (server.memory_used / server.memory_total) * 100;
}

function formatRate(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return t.common.notAvailable;
  return `${formatBytes(value)}/s`;
}

function durationLabel(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return t.common.notAvailable;
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
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
  if (site.custom_head) {
    const template = document.createElement("template");
    template.innerHTML = site.custom_head;
    template.content.childNodes.forEach((node) => {
      const next = node.cloneNode(true);
      if (next instanceof Element) {
        next.setAttribute("data-xlstatus-custom-head", "true");
        document.head.appendChild(next);
        created.push(next);
      }
    });
  }
  if (theme) {
    const style = document.createElement("style");
    style.dataset.xlstatusCustomHead = "true";
    style.textContent = [
      theme.custom_css,
      theme.light_custom_css,
      themeModeCss("[data-public-theme-root=\"true\"]", themeVariablesForMode(theme, true), ".dark-mode"),
      theme.dark_custom_css,
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

function initialPublicShowMap(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_public_show_map") === "1";
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
