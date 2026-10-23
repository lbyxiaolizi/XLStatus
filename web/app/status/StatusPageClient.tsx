"use client";

import Link from "next/link";
import type { CSSProperties } from "react";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  ProgressBar,
  SkeletonCard,
  StatusBadge,
  buttonClass,
  compactId,
  formatBytes,
  formatDate,
  formatMs,
  formatPercent,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import { WorldServerMap } from "@/app/components/WorldServerMap";
import {
  MiniLineChart,
  averageDelay,
  buildPublicServiceDays,
  publicServiceDayClass,
  serviceOk,
  type ChartPoint,
  type ChartSeries,
  type PublicServiceResult,
} from "@/app/components/charts";
import { apiClient, type PublicSiteBranding, type PublicStatusResponse, type ThemeDefinition } from "@/lib/api";
import { formatRate } from "@/app/lib/format";
import {
  sanitizeAssetUrl,
  sanitizeCssVariableValue,
  sanitizeThemeVariables,
} from "@/app/lib/security";
import type { Translations } from "@/lib/i18n";
import { useI18n } from "@/lib/use-i18n";
import { useStoredPreference } from "@/app/lib/client-store";

interface ServerLocationInfo {
  source?: string | null | undefined;
  provider?: string | null | undefined;
  country?: string | null | undefined;
  region?: string | null | undefined;
  city?: string | null | undefined;
  latitude?: number | null | undefined;
  longitude?: number | null | undefined;
  timezone?: string | null | undefined;
}

interface Server {
  id: string;
  name: string;
  remark?: string | null | undefined;
  public_note?: string | null | undefined;
  accent_color?: string | null | undefined;
  status: string;
  country?: string | null | undefined;
  region?: string | null | undefined;
  city?: string | null | undefined;
  provider?: string | null | undefined;
  tags?: string[] | null | undefined;
  latitude?: number | null | undefined;
  longitude?: number | null | undefined;
  location?: ServerLocationInfo | null | undefined;
  last_seen_at?: string | undefined;
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

interface Service {
  id: string;
  name: string;
  last_status?: string | undefined;
  last_check_at?: string | undefined;
  kind?: string | undefined;
  type?: string | undefined;
  service_type?: string | undefined;
  server_id?: string | null | undefined;
  server_ids?: string[] | undefined;
  history?: PublicServiceResult[] | undefined;
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

type PublicServerViewMode = "cards" | "compact" | "map";
const PUBLIC_SERVER_VIEW_MODES = ["cards", "compact", "map"] as const;
const BINARY_PREFERENCE_VALUES = ["0", "1"] as const;
type StatusFilter = "all" | "online" | "offline";

const defaultSiteBranding: PublicSiteBranding = {
  site_name: "XLStatus",
};

const INITIAL_DATA_MAX_AGE_MS = 5000;

interface InitialStatus {
  servers: Server[];
  services: Service[];
  updatedAt: string;
  site: PublicSiteBranding;
  theme: ThemeDefinition | null;
}

function readInitialStatus(initialData: unknown): InitialStatus | null {
  if (!initialData || typeof initialData !== "object") return null;
  const payload = initialData as Partial<PublicStatusResponse>;
  if (!Array.isArray(payload.servers)) return null;
  return {
    servers: payload.servers as Server[],
    services: Array.isArray(payload.services) ? (payload.services as Service[]) : [],
    updatedAt: typeof payload.updated_at === "string" ? payload.updated_at : "",
    site: payload.site ?? defaultSiteBranding,
    theme: payload.theme ?? null,
  };
}

function useHydrated(): boolean {
  const [hydrated, setHydrated] = useState(false);
  // eslint-disable-next-line react-hooks/set-state-in-effect -- flip after first client commit
  useEffect(() => setHydrated(true), []);
  return hydrated;
}

function stabilizeServersById(
  cache: Map<string, { raw: string; value: Server }>,
  incoming: Server[],
): Server[] {
  const nextIds = new Set<string>();
  const result = incoming.map((server) => {
    nextIds.add(server.id);
    const raw = JSON.stringify(server);
    const cached = cache.get(server.id);
    if (cached && cached.raw === raw) return cached.value;
    cache.set(server.id, { raw, value: server });
    return server;
  });
  for (const id of cache.keys()) {
    if (!nextIds.has(id)) cache.delete(id);
  }
  return result;
}

function extractServerRegion(server: Server): string | null {
  const candidate =
    server.location?.country ||
    server.country ||
    server.location?.region ||
    server.region ||
    server.location?.city ||
    server.city ||
    null;
  const trimmed = candidate ? String(candidate).trim() : "";
  return trimmed ? trimmed : null;
}

function serverSearchText(server: Server): string {
  return [
    server.name,
    server.id,
    server.public_note,
    server.remark,
    server.provider,
    server.location?.provider,
    server.country,
    server.location?.country,
    server.region,
    server.location?.region,
    server.city,
    server.location?.city,
    ...(server.tags ?? []),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

export default function StatusPageClient({
  initialData,
  initialFetchedAt,
}: {
  initialData?: unknown;
  initialFetchedAt?: number | undefined;
}) {
  const initial = readInitialStatus(initialData);
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const hydrated = useHydrated();
  const [serverCache] = useState(() => new Map<string, { raw: string; value: Server }>());
  const [servers, setServers] = useState<Server[]>(() =>
    initial ? stabilizeServersById(serverCache, initial.servers) : [],
  );
  const [services, setServices] = useState<Service[]>(() => initial?.services ?? []);
  const [loading, setLoading] = useState(!initial);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState(() => initial?.updatedAt ?? "");
  const [site, setSite] = useState<PublicSiteBranding>(() => initial?.site ?? defaultSiteBranding);
  const [theme, setTheme] = useState<ThemeDefinition | null>(() => initial?.theme ?? null);
  const [showServicesPreference, setShowServicesPreference] = useStoredPreference("xlstatus_public_show_services", "0", BINARY_PREFERENCE_VALUES);
  const [serverViewMode, setServerViewMode] = useStoredPreference<PublicServerViewMode>("xlstatus_public_server_view", "cards", PUBLIC_SERVER_VIEW_MODES);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [regionFilter, setRegionFilter] = useState<string>("all");
  const showServices = showServicesPreference === "1";
  const initialLoadRef = useRef(!initial);
  const skipMountLoadRef = useRef(Boolean(initial));

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
      timer = window.setInterval(() => void load(), 30000);
    }

    async function load(bypassClientCache = false) {
      controller?.abort();
      controller = new AbortController();
      const signal = controller.signal;
      if (initialLoadRef.current) setLoading(true);
      setError(null);
      const response = await apiClient.getPublicStatus({ signal, bypassClientCache });

      if (signal.aborted || stopped) return;

      if (response.success && response.data) {
        setServers(stabilizeServersById(serverCache, (response.data.servers as Server[]) ?? []));
        setServices((response.data.services as Service[]) ?? []);
        setUpdatedAt(response.data.updated_at || new Date().toISOString());
        setSite(response.data.site ?? defaultSiteBranding);
        setTheme(response.data.theme ?? null);
      } else {
        setError(responseError(response));
      }

      initialLoadRef.current = false;
      setLoading(false);
    }

    function handleVisibilityChange() {
      if (document.visibilityState !== "visible") {
        stopPolling();
        controller?.abort();
        controller = null;
        return;
      }

      void load(true);
      startPolling();
    }

    document.addEventListener("visibilitychange", handleVisibilityChange);
    if (document.visibilityState === "visible") {
      const skipMountLoad = skipMountLoadRef.current
        && (initialFetchedAt === undefined || Date.now() - initialFetchedAt < INITIAL_DATA_MAX_AGE_MS);
      skipMountLoadRef.current = false;
      if (!skipMountLoad) void load();
      startPolling();
    }

    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      stopPolling();
      controller?.abort();
    };
  }, [initialFetchedAt, serverCache]);

  useEffect(() => applyPublicHead(site, theme), [site, theme]);

  const overall = useMemo(() => {
    if (servers.length === 0 && services.length === 0) {
      return { label: copy.statusPage.overallNoData, tone: "gray" as const };
    }
    if (
      servers.some((server) => server.status !== "online") ||
      services.some((service) => !serviceOk(service.last_status ?? ""))
    ) {
      return { label: copy.statusPage.overallPartial, tone: "yellow" as const };
    }
    return { label: copy.statusPage.overallOk, tone: "green" as const };
  }, [servers, services, copy]);

  const onlineServersCount = useMemo(
    () => servers.filter((server) => server.status === "online").length,
    [servers],
  );
  const offlineServersCount = servers.length - onlineServersCount;

  const regionCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const server of servers) {
      const region = extractServerRegion(server);
      if (!region) continue;
      counts.set(region, (counts.get(region) ?? 0) + 1);
    }
    return Array.from(counts.entries())
      .map(([label, count]) => ({ label, count }))
      .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label, "zh-CN"));
  }, [servers]);

  const activeRegionFilter = useMemo(
    () => (regionFilter !== "all" && regionCounts.some((entry) => entry.label === regionFilter) ? regionFilter : "all"),
    [regionCounts, regionFilter],
  );

  const filteredServers = useMemo(() => {
    const tokens = searchQuery
      .trim()
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean);

    return servers.filter((server) => {
      if (statusFilter === "online" && server.status !== "online") return false;
      if (statusFilter === "offline" && server.status === "online") return false;
      if (activeRegionFilter !== "all" && extractServerRegion(server) !== activeRegionFilter) return false;
      if (tokens.length > 0) {
        const text = serverSearchText(server);
        if (!tokens.every((token) => text.includes(token))) return false;
      }
      return true;
    });
  }, [activeRegionFilter, searchQuery, servers, statusFilter]);

  const hasActiveFilters =
    Boolean(searchQuery.trim()) || statusFilter !== "all" || activeRegionFilter !== "all";
  const safeLogoUrl = sanitizeAssetUrl(site.logo_url);

  function resetFilters() {
    setSearchQuery("");
    setStatusFilter("all");
    setRegionFilter("all");
  }

  function changeServerViewMode(next: PublicServerViewMode) {
    setServerViewMode(next);
  }

  function toggleServices() {
    setShowServicesPreference(showServices ? "0" : "1");
  }

  return (
    <div className="min-h-screen" style={publicPageStyle(site)} data-public-theme-root="true">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow={copy.statusPage.eyebrow}
          title={site.site_name || "XLStatus"}
          detail={copy.statusPage.detail}
          actions={
            <div className="flex flex-wrap items-center justify-end gap-2.5">
              {safeLogoUrl ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img src={safeLogoUrl} alt="" className="h-9 w-9 rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] object-contain p-1 shadow-[var(--shadow-xs)]" />
              ) : null}
              <StatusBadge tone={overall.tone}>{overall.label}</StatusBadge>
            </div>
          }
        />

        <div className="mb-5 space-y-2.5">
          <InlineError message={error} />
          <InlineNotice tone="blue">
            {hydrated && updatedAt ? copy.statusPage.updatedAt.replace("{time}", formatDate(updatedAt)) : copy.statusPage.waitingFirstRefresh} {copy.statusPage.autoRefresh}
          </InlineNotice>
        </div>

        <div className="mb-6 grid gap-4 sm:grid-cols-3">
          <Kpi title={copy.statusPage.kpiServers} value={String(servers.length)} detail={copy.statusPage.kpiServersOnline.replace("{count}", String(onlineServersCount))} />
          <Kpi title={copy.statusPage.kpiServices} value={String(services.length)} detail={copy.statusPage.kpiServicesDetail} />
          <Kpi title={copy.statusPage.kpiRefresh} value="30s" detail={copy.statusPage.kpiRefreshDetail} />
        </div>

        {/* Interactive Quick Filter, Search & View Control Bar */}
        <div className="mb-5 space-y-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3.5 shadow-[var(--shadow-xs)]">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            {/* Search + Status Filter Pills */}
            <div className="flex flex-1 flex-wrap items-center gap-2.5">
              <div className="relative min-w-52 flex-1 sm:max-w-xs">
                <svg
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)]"
                  aria-hidden="true"
                >
                  <circle cx="7" cy="7" r="4.5" />
                  <path d="M10.5 10.5L14 14" strokeLinecap="round" />
                </svg>
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={isZh ? "筛选节点名称、地区、厂商或标签..." : "Filter by name, region, provider or tag..."}
                  aria-label={copy.common.search}
                  className={`${inputClass} mt-0 py-1.5 pr-8 pl-8 text-xs`}
                />
                {searchQuery ? (
                  <button
                    type="button"
                    onClick={() => setSearchQuery("")}
                    aria-label={copy.common.close}
                    className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-xs text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
                  >
                    ×
                  </button>
                ) : null}
              </div>

              <div
                className="inline-flex flex-wrap items-center gap-1 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                role="group"
                aria-label={copy.statusPage.metricLabelStatus}
              >
                <StatusFilterPill
                  active={statusFilter === "all"}
                  onClick={() => setStatusFilter("all")}
                  label={isZh ? "全部" : "All"}
                  count={servers.length}
                />
                <StatusFilterPill
                  active={statusFilter === "online"}
                  onClick={() => setStatusFilter("online")}
                  label={copy.statusPage.statusOnline}
                  count={onlineServersCount}
                  dotClass="bg-emerald-500"
                />
                <StatusFilterPill
                  active={statusFilter === "offline"}
                  onClick={() => setStatusFilter("offline")}
                  label={copy.statusPage.statusOffline}
                  count={offlineServersCount}
                  dotClass="bg-rose-500"
                />
              </div>
            </div>

            {/* View Switcher (Cards / Compact / Global Map) + Services Toggle */}
            <div className="flex flex-wrap items-center gap-2">
              <div className="inline-flex flex-wrap gap-1 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1">
                <button
                  type="button"
                  onClick={() => changeServerViewMode("cards")}
                  className={buttonClass(serverViewMode === "cards" ? "primary" : "ghost")}
                >
                  {copy.statusPage.viewCards}
                </button>
                <button
                  type="button"
                  onClick={() => changeServerViewMode("compact")}
                  className={buttonClass(serverViewMode === "compact" ? "primary" : "ghost")}
                >
                  {copy.statusPage.viewCompact}
                </button>
                <button
                  type="button"
                  onClick={() => changeServerViewMode("map")}
                  className={buttonClass(serverViewMode === "map" ? "primary" : "ghost")}
                >
                  {isZh ? "全球地图" : "Global Map"}
                </button>
              </div>
              <button
                type="button"
                onClick={toggleServices}
                className={buttonClass(showServices ? "primary" : "secondary")}
              >
                {showServices ? copy.statusPage.hideServices : copy.statusPage.showServices}
              </button>
            </div>
          </div>

          {/* Dynamic Region / Country Quick Pills */}
          {regionCounts.length > 1 || hasActiveFilters ? (
            <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-color)] pt-2.5">
              <div className="flex flex-wrap items-center gap-1.5">
                {regionCounts.length > 1 ? (
                  <>
                    <button
                      type="button"
                      onClick={() => setRegionFilter("all")}
                      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium transition ${
                        activeRegionFilter === "all"
                          ? "border-[var(--border-strong)] bg-[var(--bg-muted)] font-semibold text-[var(--text-main)]"
                          : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      <span>{isZh ? "全部地区" : "All Regions"}</span>
                      <span className="font-mono text-[10px] opacity-75">{servers.length}</span>
                    </button>
                    {regionCounts.map((region) => {
                      const selected = activeRegionFilter === region.label;
                      return (
                        <button
                          key={region.label}
                          type="button"
                          onClick={() => setRegionFilter(selected ? "all" : region.label)}
                          className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium transition ${
                            selected
                              ? "border-[var(--accent-color)] bg-[var(--accent-color)]/15 font-semibold text-[var(--text-main)]"
                              : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
                          }`}
                        >
                          <span>{region.label}</span>
                          <span className="font-mono text-[10px] opacity-75">{region.count}</span>
                        </button>
                      );
                    })}
                  </>
                ) : null}
              </div>

              {hasActiveFilters ? (
                <button
                  type="button"
                  onClick={resetFilters}
                  className="text-xs font-medium text-[var(--accent-color)] hover:underline"
                >
                  {isZh ? "重置筛选" : "Reset filters"}
                </button>
              ) : null}
            </div>
          ) : null}
        </div>

        <div className={`grid gap-6 ${showServices ? "lg:grid-cols-2" : ""}`}>
          <section>
            <div className="mb-3 flex items-center justify-between gap-2">
              <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                {copy.statusPage.serversHeading}
              </h2>
              <span className="rounded-full border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 font-mono text-xs font-medium text-[var(--text-muted)]">
                {hasActiveFilters ? `${filteredServers.length} / ${servers.length}` : String(servers.length)}
              </span>
            </div>
            {loading && servers.length === 0 ? (
              <div className="grid gap-4">
                {Array.from({ length: 4 }).map((_, index) => (
                  <SkeletonCard key={index} lines={4} />
                ))}
              </div>
            ) : servers.length === 0 ? (
              <EmptyState title={copy.statusPage.serversEmptyTitle} detail={copy.statusPage.serversEmptyDetail} />
            ) : filteredServers.length === 0 ? (
              <EmptyState
                title={copy.common.noMatches}
                detail={isZh ? "没有匹配的公开服务器，请调整搜索关键词或状态筛选条件。" : "No public servers match your current search or filter criteria."}
                action={
                  <button type="button" onClick={resetFilters} className={buttonClass("secondary")}>
                    {isZh ? "重置筛选" : "Reset filters"}
                  </button>
                }
              />
            ) : serverViewMode === "map" ? (
              <WorldServerMap
                servers={filteredServers}
                title={isZh ? "全球节点分布" : "Global Node Distribution"}
                ariaLabel={isZh ? "公开服务器全球分布图" : "Public servers global distribution map"}
                serverHref={(server) => `/status/servers/${encodeURIComponent(server.id)}`}
              />
            ) : serverViewMode === "compact" ? (
              <div className="grid gap-2.5">
                {filteredServers.map((server) => (
                  <PublicCompactServerRow key={server.id} server={server} hydrated={hydrated} />
                ))}
              </div>
            ) : (
              <div className="grid gap-4">
                {filteredServers.map((server) => (
                  <PublicServerCard key={server.id} server={server} hydrated={hydrated} />
                ))}
              </div>
            )}
          </section>

          {showServices ? (
            <section>
              <h2 className="mb-3 text-base font-semibold tracking-tight text-[var(--text-main)]">{copy.statusPage.servicesHeading}</h2>
              {loading && services.length === 0 ? (
                <div className="grid gap-4">
                  {Array.from({ length: 3 }).map((_, index) => (
                    <SkeletonCard key={index} lines={4} />
                  ))}
                </div>
              ) : services.length === 0 ? (
                <EmptyState title={copy.statusPage.servicesEmptyTitle} detail={copy.statusPage.servicesEmptyDetail} />
              ) : (
                <div className="grid gap-4">
                  {services.map((service) => (
                    <BrutalCard key={service.id}>
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <h3 className="text-base font-semibold text-[var(--text-main)]">{service.name}</h3>
                        </div>
                        <StatusBadge tone={serviceTone(service.last_status)}>{statusLabel(service.last_status, copy)}</StatusBadge>
                      </div>
                      <div className="mt-3 flex items-center justify-between text-xs text-[var(--text-muted)]">
                        <span className="rounded-md bg-[var(--bg-muted)] px-2 py-0.5 font-mono">{serviceKind(service, copy)}</span>
                        <span>{copy.statusPage.checkedAt.replace("{time}", formatDate(service.last_check_at))}</span>
                      </div>
                      <PublicServiceHistory service={service} />
                    </BrutalCard>
                  ))}
                </div>
              )}
            </section>
          ) : null}
        </div>
      </PageShell>
    </div>
  );
}

function StatusFilterPill({
  active,
  onClick,
  label,
  count,
  dotClass,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  count: number;
  dotClass?: string | undefined;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={`inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition ${
        active
          ? "bg-[var(--bg-card)] font-semibold text-[var(--text-main)] shadow-[var(--shadow-xs)]"
          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
      }`}
    >
      {dotClass ? <span className={`h-1.5 w-1.5 rounded-full ${dotClass}`} aria-hidden="true" /> : null}
      <span>{label}</span>
      <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.2 font-mono text-[10px] text-[var(--text-muted)]">
        {count}
      </span>
    </button>
  );
}

function PublicServiceHistory({ service }: { service: Service }) {
  const { t: copy } = useI18n();
  const days = buildPublicServiceDays(service.history ?? []);
  const checks = days.reduce((sum, day) => sum + day.total, 0);
  const avgDelay = averageDelay(service.history ?? []);
  const uptime = checks
    ? days.reduce((sum, day) => sum + (day.uptime * day.total), 0) / checks
    : undefined;

  return (
    <div className="mt-4 border-t border-[var(--border-color)] pt-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs font-medium text-[var(--text-muted)]">
        <span>{uptime === undefined ? "N/A" : copy.statusPage.uptimeSuffix.replace("{percent}", formatPercent(uptime))}</span>
        <span className="font-mono">{avgDelay === undefined ? "N/A" : formatMs(Math.round(avgDelay))}</span>
      </div>
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
        <span>{copy.statusPage.daysAgo30}</span>
        <span>{copy.statusPage.today}</span>
      </div>
    </div>
  );
}

const PublicServerCard = memo(function PublicServerCard({ server, hydrated }: { server: Server; hydrated: boolean }) {
  const { t: copy } = useI18n();
  const note = server.public_note || server.remark || copy.statusPage.publicServerNote;
  const regionLabel = extractServerRegion(server);
  const providerLabel = server.provider || server.location?.provider || null;
  const safeAccentColor = sanitizeCssVariableValue(server.accent_color);
  return (
    <Link
      href={`/status/servers/${encodeURIComponent(server.id)}`}
      className="group block rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 text-[var(--text-main)] shadow-[var(--shadow-xs)] transition hover:border-[var(--border-strong)] hover:shadow-[var(--shadow-md)] focus:outline-none focus:ring-2 focus:ring-[var(--ring-color)]"
      style={safeAccentColor ? { borderTopColor: safeAccentColor, borderTopWidth: "3px" } : undefined}
      aria-label={copy.statusPage.viewPublicServer.replace("{name}", server.name)}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="break-words text-base font-semibold tracking-tight text-[var(--text-main)]">{server.name}</h3>
          <p className="mt-0.5 break-all font-mono text-xs text-[var(--text-muted)]">{server.id}</p>
          {regionLabel || providerLabel || (server.tags && server.tags.length > 0) ? (
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {regionLabel ? (
                <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]">
                  {regionLabel}
                </span>
              ) : null}
              {providerLabel ? (
                <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]">
                  {providerLabel}
                </span>
              ) : null}
              {(server.tags ?? []).slice(0, 3).map((tag) => (
                <span
                  key={tag}
                  className="rounded-md bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]"
                >
                  #{tag}
                </span>
              ))}
            </div>
          ) : null}
          <p className="mt-2 text-xs text-[var(--text-muted)]">
            {note}
          </p>
        </div>
        <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status, copy)}</StatusBadge>
      </div>
      <div className="mt-4 grid grid-cols-2 gap-3 text-xs">
        <Metric label={copy.statusPage.metricLabelStatus} value={statusLabel(server.status, copy)} />
        <Metric label={copy.statusPage.metricLabelLastSeen} value={hydrated ? formatDate(server.last_seen_at) : copy.common.loading} />
      </div>
      {server.resources ? <PublicServerResourceStrip resources={server.resources} /> : null}
      {hydrated && server.metrics?.samples?.length ? <PublicServerMiniCharts metrics={server.metrics} /> : null}
    </Link>
  );
});

const PublicCompactServerRow = memo(function PublicCompactServerRow({ server, hydrated }: { server: Server; hydrated: boolean }) {
  const { t: copy } = useI18n();
  const regionLabel = extractServerRegion(server);
  return (
    <Link
      href={`/status/servers/${encodeURIComponent(server.id)}`}
      className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3.5 text-[var(--text-main)] shadow-[var(--shadow-xs)] transition hover:border-[var(--border-strong)] hover:shadow-[var(--shadow-sm)] md:grid-cols-[minmax(10rem,1.3fr)_minmax(5rem,1fr)_minmax(10rem,1fr)]"
      aria-label={copy.statusPage.viewPublicServer.replace("{name}", server.name)}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className={`h-2 w-2 shrink-0 rounded-full ${server.status === "online" ? "bg-emerald-500" : "bg-rose-500"}`} />
          <span className="truncate text-sm font-semibold">{server.name}</span>
          {regionLabel ? (
            <span className="shrink-0 rounded border border-[var(--border-color)] bg-[var(--bg-subtle)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--text-muted)]">
              {regionLabel}
            </span>
          ) : null}
        </div>
        <div className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">{compactId(server.id)}</div>
      </div>
      <Metric label={copy.statusPage.metricLabelStatus} value={statusLabel(server.status, copy)} />
      <Metric label={copy.statusPage.metricLabelLastSeen} value={hydrated ? formatDate(server.last_seen_at) : copy.common.loading} />
      {server.resources ? (
        <div className="grid grid-cols-2 gap-2 text-xs md:col-span-3 md:grid-cols-4">
          <CompactMetric label={copy.statusPage.metricLabelCpu} value={formatPercent(server.resources.cpu_percent)} />
          <CompactMetric label={copy.statusPage.metricLabelMemory} value={formatPercent(resourceMemoryPercent(server.resources))} />
          <CompactMetric label={copy.statusPage.metricLabelDownload} value={formatRate(server.resources.net_rx_bps)} />
          <CompactMetric label={copy.statusPage.metricLabelUpload} value={formatRate(server.resources.net_tx_bps)} />
        </div>
      ) : null}
      {hydrated && server.metrics?.samples?.length ? (
        <div className="md:col-span-3">
          <PublicServerMiniCharts metrics={server.metrics} compact />
        </div>
      ) : null}
    </Link>
  );
});

function PublicServerResourceStrip({ resources }: { resources: PublicServerResources }) {
  const { t: copy } = useI18n();
  const memPct = resourceMemoryPercent(resources);
  return (
    <div className="mt-4 border-t border-[var(--border-color)] pt-3">
      <div className="grid grid-cols-2 gap-3 text-xs sm:grid-cols-4">
        <div className="space-y-1">
          <Metric label={copy.statusPage.metricLabelCpu} value={formatPercent(resources.cpu_percent)} />
          <ProgressBar value={resources.cpu_percent} tone="emerald" />
        </div>
        <div className="space-y-1">
          <Metric label={copy.statusPage.metricLabelMemory} value={resourceBytes(resources.memory_used, resources.memory_total, resources.memory_percent)} />
          <ProgressBar value={memPct} tone="indigo" />
        </div>
        <div className="space-y-1">
          <Metric label={copy.statusPage.metricLabelDisk} value={resourceBytes(resources.disk_used, resources.disk_total, resources.disk_percent)} />
          <ProgressBar value={resources.disk_percent} tone="amber" />
        </div>
        <Metric label={copy.statusPage.metricLabelDownloadUpload} value={`${formatRate(resources.net_rx_bps)} / ${formatRate(resources.net_tx_bps)}`} />
      </div>
    </div>
  );
}

function PublicServerMiniCharts({ metrics, compact = false }: { metrics: PublicServerMetrics; compact?: boolean | undefined }) {
  const { t: copy } = useI18n();
  const charts = useMemo(() => buildMetricCharts(metrics.samples), [metrics.samples]);
  const resourceSeries = useMemo(
    () =>
      [
        { id: "cpu", label: copy.statusPage.seriesCpu, color: "var(--accent-color)", points: charts.cpu },
        { id: "memory", label: copy.statusPage.seriesMemory, color: "#6366f1", points: charts.memory },
        { id: "disk", label: copy.statusPage.seriesDisk, color: "#f59e0b", points: charts.disk },
      ].filter((series) => series.points.length > 0),
    [charts, copy],
  );
  const networkSeries = useMemo(
    () =>
      [
        { id: "rx", label: copy.statusPage.seriesDownload, color: "var(--accent-color)", points: charts.rx },
        { id: "tx", label: copy.statusPage.seriesUpload, color: "#6366f1", points: charts.tx },
      ].filter((series) => series.points.length > 0),
    [charts, copy],
  );

  if (!resourceSeries.length && !networkSeries.length) return null;

  return (
    <div className="mt-4 border-t border-[var(--border-color)] pt-3">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs font-medium text-[var(--text-muted)]">
        <span>{copy.statusPage.chartsHeading}</span>
        <span className="font-mono">{metrics.range || "1d"}</span>
      </div>
      <div className={`grid gap-3 ${compact ? "" : "lg:grid-cols-2"}`}>
        {resourceSeries.length ? (
          <MetricChartCard
            title={copy.statusPage.chartResourceUsage}
            value={latestPercentLabel(resourceSeries)}
            series={resourceSeries}
            maxValue={100}
            formatValue={formatPercent}
          />
        ) : null}
        {networkSeries.length ? (
          <MetricChartCard
            title={copy.statusPage.chartNetworkRate}
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
  maxValue?: number | undefined;
  formatValue: (value: number) => string;
}) {
  const { t: copy } = useI18n();
  return (
    <div className="min-w-0">
      <div className="mb-1.5 flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs font-medium text-[var(--text-muted)]">{title}</span>
        <span className="max-w-full truncate font-mono text-xs text-[var(--text-muted)]">{value}</span>
      </div>
      <MiniLineChart
        series={series}
        maxValue={maxValue}
        formatValue={formatValue}
        emptyLabel={copy.statusPage.noHistory}
        ariaLabel={copy.statusPage.chartAriaLabel}
        interactive={false}
        variant="compact"
      />
      <div className="mt-2 flex flex-wrap gap-1.5">
        {series.map((item) => (
          <span key={item.id ?? item.label} className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-[10px] font-medium text-[var(--text-muted)]">
            <span className="h-2 w-2 rounded-full" style={{ background: item.color }} />
            {item.label}
          </span>
        ))}
      </div>
    </div>
  );
}

function Kpi({ title, value, detail }: { title: string; value: string; detail: string }) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-medium text-[var(--text-muted)]">{title}</div>
      <div className="mt-1.5 font-mono text-3xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">{value}</div>
      <div className="mt-1 text-xs text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[11px] font-medium text-[var(--text-muted)]">{label}</div>
      <div className="mt-0.5 font-mono font-medium text-[var(--text-main)]">{value}</div>
    </div>
  );
}

function CompactMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[10px] font-medium text-[var(--text-muted)]">{label}</div>
      <div className="truncate font-mono font-medium text-[var(--text-main)]">{value}</div>
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

function serverTone(status: string): "green" | "red" | "yellow" | "gray" {
  if (status === "online") return "green";
  if (status === "offline" || status === "down") return "red";
  if (status === "degraded" || status === "revoked") return "yellow";
  return "gray";
}

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up" || status === "ok") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}

function serviceKind(service: Service, copy: Translations): string {
  return service.service_type || service.kind || service.type || copy.statusPage.serviceKindFallback;
}

function statusLabel(status: string | undefined, copy: Translations): string {
  if (!status) return copy.common.unknown;
  const labels: Record<string, string> = {
    online: copy.statusPage.statusOnline,
    offline: copy.statusPage.statusOffline,
    down: copy.statusPage.statusDown,
    degraded: copy.statusPage.statusDegraded,
    revoked: copy.statusPage.statusRevoked,
    success: copy.common.success,
    up: copy.statusPage.statusUp,
    failure: copy.common.failure,
    timeout: copy.statusPage.statusTimeout,
  };
  return labels[status] || status;
}

function publicPageStyle(site: PublicSiteBranding): CSSProperties {
  const style: Record<string, string> = {
    backgroundColor: "var(--bg-page)",
    color: "var(--text-main)",
  };
  const safeThemeColor = sanitizeCssVariableValue(site.theme_color);
  if (safeThemeColor) {
    style["--accent-color"] = safeThemeColor;
  }
  const safeBackgroundUrl = sanitizeAssetUrl(site.background_url);
  if (safeBackgroundUrl) {
    style["backgroundImage"] = `linear-gradient(rgba(255,255,255,.88), rgba(255,255,255,.88)), url("${safeBackgroundUrl}")`;
    style["backgroundSize"] = "cover";
    style["backgroundPosition"] = "center";
    style["backgroundAttachment"] = "fixed";
  }
  return style as CSSProperties;
}

function applyPublicHead(site: PublicSiteBranding, theme?: ThemeDefinition | null): () => void {
  document.title = site.site_name || "XLStatus";
  const created: Element[] = [];

  const safeFaviconUrl = sanitizeAssetUrl(site.favicon_url);
  if (safeFaviconUrl) {
    const favicon = document.createElement("link");
    favicon.rel = "icon";
    favicon.href = safeFaviconUrl;
    favicon.dataset["xlstatusCustomHead"] = "true";
    document.head.appendChild(favicon);
    created.push(favicon);
  }
  const safeThemeColor = sanitizeCssVariableValue(site.theme_color);
  if (safeThemeColor) {
    const metaTheme = document.createElement("meta");
    metaTheme.name = "theme-color";
    metaTheme.content = safeThemeColor;
    metaTheme.dataset["xlstatusCustomHead"] = "true";
    document.head.appendChild(metaTheme);
    created.push(metaTheme);
  }
  if (theme) {
    const style = document.createElement("style");
    style.dataset["xlstatusCustomHead"] = "true";
    style.textContent = [
      themeModeCss("[data-public-theme-root=\"true\"]", themeVariablesForMode(theme, false)),
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
  const safeVariables = sanitizeThemeVariables(variables);
  const lines = Object.entries(safeVariables).map(([key, value]) => `${key}: ${value};`);
  if (!lines.length) return "";
  return `${prefix} ${selector} { ${lines.join(" ")} }`;
}
