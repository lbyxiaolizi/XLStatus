"use client";

import Link from "next/link";
import { geoEquirectangular, geoPath } from "d3-geo";
import { useMemo, useState } from "react";
import { EmptyState, StatusBadge } from "@/app/components/M7Primitives";
import worldCountries from "@/app/components/world-countries-110m.json";

interface ServerLocationLike {
  source?: string | null;
  provider?: string | null;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  timezone?: string | null;
}

export interface MapServerLike {
  id: string;
  name: string;
  status: string;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  location?: ServerLocationLike | null;
}

interface WorldServerMapProps<TServer extends MapServerLike> {
  servers: TServer[];
  title: string;
  detailLabel?: string;
  ariaLabel: string;
  serverHref?: (server: TServer) => string;
}

interface CountryFeature {
  type: "Feature";
  properties: {
    iso2: string;
    iso3?: string;
    name: string;
  };
  geometry: GeoJSON.Geometry;
}

interface RegionPoint {
  key: string;
  label: string;
  countryCode?: string;
  longitude: number;
  latitude: number;
  source: "manual" | "geoip" | "country";
}

interface RegionBucket<TServer extends MapServerLike> {
  point: RegionPoint;
  servers: TServer[];
}

interface TooltipState<TServer extends MapServerLike> {
  x: number;
  y: number;
  label: string;
  source: string;
  servers: TServer[];
}

const width = 900;
const height = 500;
const projection = geoEquirectangular()
  .scale(140)
  .translate([width / 2, height / 2])
  .rotate([-12, 0, 0]);
const pathGenerator = geoPath(projection);
const countryFeatures = (worldCountries.features as CountryFeature[]).filter((feature) => feature.properties.iso2);
const countryFeatureByIso2 = new Map(countryFeatures.map((feature) => [feature.properties.iso2.toUpperCase(), feature]));

export function WorldServerMap<TServer extends MapServerLike>({
  servers,
  title,
  detailLabel = "服务器",
  ariaLabel,
  serverHref,
}: WorldServerMapProps<TServer>) {
  const buckets = useMemo(() => buildRegionBuckets(servers), [servers]);
  const highlighted = useMemo(() => new Map(buckets.regions.filter((bucket) => bucket.point.countryCode).map((bucket) => [bucket.point.countryCode as string, bucket])), [buckets.regions]);
  const pointBuckets = useMemo(() => buckets.regions.filter((bucket) => !bucket.point.countryCode || !countryFeatureByIso2.has(bucket.point.countryCode)), [buckets.regions]);
  const maxCount = Math.max(1, ...buckets.regions.map((bucket) => bucket.servers.length));
  const [tooltip, setTooltip] = useState<TooltipState<TServer> | null>(null);

  return (
    <section className="mb-5 border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex flex-wrap items-end justify-between gap-3 border-b-4 border-black pb-3">
        <div>
          <h2 className="text-xl font-black uppercase">{title}</h2>
          <p className="mt-1 text-sm font-bold text-[var(--text-muted)]">
            {buckets.regions.length} 个已识别地区 / {servers.length} 台{detailLabel}
          </p>
        </div>
        <span className="border-2 border-black bg-[var(--bg-card)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {buckets.unmatched.length ? `${buckets.unmatched.length} 未识别` : "全部识别"}
        </span>
      </div>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
        <div
          className="relative overflow-hidden border-2 border-black bg-[var(--bg-page)] shadow-[var(--shadow-brutal-sm)]"
          onMouseLeave={() => setTooltip(null)}
        >
          <svg className="block aspect-[9/5] min-h-72 w-full" viewBox={`0 0 ${width} ${height}`} role="img" aria-label={ariaLabel}>
            <defs>
              <pattern id="world-map-grid" width="18" height="18" patternUnits="userSpaceOnUse">
                <path d="M 18 0 L 0 0 0 18" fill="none" stroke="var(--border-color)" strokeOpacity="0.08" strokeWidth="1" />
              </pattern>
              <filter id="world-map-shadow" x="-20%" y="-20%" width="140%" height="140%">
                <feDropShadow dx="2" dy="2" stdDeviation="0" floodColor="var(--border-color)" floodOpacity="0.45" />
              </filter>
            </defs>
            <rect width={width} height={height} fill="var(--bg-page)" />
            <rect width={width} height={height} fill="url(#world-map-grid)" />
            <g>
              {countryFeatures.map((feature) => {
                const countryCode = feature.properties.iso2.toUpperCase();
                const bucket = highlighted.get(countryCode);
                const isHighlighted = Boolean(bucket);
                return (
                  <path
                    key={countryCode}
                    d={pathGenerator(feature) ?? ""}
                    className={`transition-colors duration-150 ${
                      isHighlighted
                        ? "cursor-pointer fill-[var(--accent-color)] stroke-[var(--border-color)]"
                        : "fill-[color-mix(in_srgb,var(--bg-card)_76%,var(--text-muted)_24%)] stroke-[var(--border-color)]/30"
                    }`}
                    strokeWidth={isHighlighted ? 1.1 : 0.45}
                    fillOpacity={isHighlighted ? 0.88 : 0.42}
                    filter={isHighlighted ? "url(#world-map-shadow)" : undefined}
                    onMouseEnter={() => {
                      if (!bucket) {
                        setTooltip(null);
                        return;
                      }
                      const [x, y] = pathGenerator.centroid(feature);
                      setTooltip({
                        x,
                        y,
                        label: bucket.point.label || feature.properties.name,
                        source: sourceLabel(bucket.point.source),
                        servers: bucket.servers,
                      });
                    }}
                  />
                );
              })}
            </g>
            <g>
              {pointBuckets.map((bucket) => {
                const [x, y] = projectPoint(bucket.point.longitude, bucket.point.latitude);
                const blockSize = 7 + Math.min(7, (bucket.servers.length / maxCount) * 5);
                return (
                  <g
                    key={bucket.point.key}
                    className="cursor-pointer"
                    onMouseEnter={() =>
                      setTooltip({
                        x,
                        y,
                        label: bucket.point.label,
                        source: sourceLabel(bucket.point.source),
                        servers: bucket.servers,
                      })
                    }
                  >
                    <path
                      d={regionBlockPath(x, y, blockSize)}
                      fill="var(--accent-color)"
                      fillOpacity="0.9"
                      stroke="var(--border-color)"
                      strokeWidth="2.5"
                      filter="url(#world-map-shadow)"
                    />
                    <path
                      d={regionBlockPath(x, y, Math.max(3.5, blockSize - 4))}
                      fill="var(--btn-text)"
                      fillOpacity="0.18"
                      pointerEvents="none"
                    />
                  </g>
                );
              })}
            </g>
          </svg>
          {tooltip ? <MapTooltip tooltip={tooltip} serverHref={serverHref} /> : null}
        </div>
        <div className="grid content-start gap-3">
          {buckets.regions.length ? (
            buckets.regions.slice(0, 8).map((bucket) => (
              <div key={bucket.point.key} className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
                <div className="flex items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-sm font-black">{bucket.point.label}</span>
                  <StatusBadge tone={bucket.servers.some((server) => server.status !== "online") ? "yellow" : "green"}>
                    {bucket.servers.length} 台
                  </StatusBadge>
                </div>
                <div className="mt-2 grid gap-1">
                  {bucket.servers.slice(0, 4).map((server) =>
                    serverHref ? (
                      <Link key={server.id} href={serverHref(server)} className="truncate text-xs font-bold text-[var(--text-muted)] underline decoration-2 underline-offset-2">
                        {server.name}
                      </Link>
                    ) : (
                      <span key={server.id} className="truncate text-xs font-bold text-[var(--text-muted)]">
                        {server.name}
                      </span>
                    ),
                  )}
                  {bucket.servers.length > 4 ? <span className="text-xs font-black text-[var(--text-muted)]">+{bucket.servers.length - 4} 台</span> : null}
                </div>
              </div>
            ))
          ) : (
            <EmptyState title="暂无可识别地区" detail="GeoIP 或手动位置字段可用后会点亮地图。" />
          )}
          {buckets.regions.length > 8 ? (
            <div className="border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black text-[var(--text-muted)] shadow-[var(--shadow-brutal-sm)]">
              另有 {buckets.regions.length - 8} 个地区
            </div>
          ) : null}
          {buckets.unmatched.length ? (
            <div className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
              <div className="text-sm font-black">未识别</div>
              <div className="mt-2 flex flex-wrap gap-2">
                {buckets.unmatched.slice(0, 8).map((server) => (
                  <span key={server.id} className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-[11px] font-black">
                    {server.name}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}

function MapTooltip<TServer extends MapServerLike>({
  tooltip,
  serverHref,
}: {
  tooltip: TooltipState<TServer>;
  serverHref?: (server: TServer) => string;
}) {
  return (
    <div
      className="pointer-events-auto absolute z-20 hidden max-w-64 border-2 border-black bg-[var(--bg-card)] p-3 text-sm shadow-[var(--shadow-brutal-sm)] lg:block"
      style={{
        left: `${(tooltip.x / width) * 100}%`,
        top: `${(tooltip.y / height) * 100}%`,
        transform: tooltip.x > width * 0.72 ? "translate(-108%, -50%)" : "translate(18%, -50%)",
      }}
    >
      <div className="min-w-44">
        <p className="break-words text-sm font-black">{tooltip.label}</p>
        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">
          {tooltip.servers.length} 台服务器 · {tooltip.source}
        </p>
      </div>
      <div className="mt-2 grid max-h-48 gap-1 overflow-y-auto border-t-2 border-black pt-2">
        {tooltip.servers.map((server) =>
          serverHref ? (
            <Link key={server.id} href={serverHref(server)} className="flex min-w-0 items-center gap-2 text-xs font-bold text-[var(--text-muted)] hover:text-[var(--text-main)]">
              <ServerDot status={server.status} />
              <span className="truncate">{server.name}</span>
            </Link>
          ) : (
            <span key={server.id} className="flex min-w-0 items-center gap-2 text-xs font-bold text-[var(--text-muted)]">
              <ServerDot status={server.status} />
              <span className="truncate">{server.name}</span>
            </span>
          ),
        )}
      </div>
    </div>
  );
}

function ServerDot({ status }: { status: string }) {
  return <span className={`h-2 w-2 shrink-0 rounded-full border border-black ${status === "online" ? "bg-[var(--accent-color)]" : "bg-[var(--btn-bg)]"}`} />;
}

function buildRegionBuckets<TServer extends MapServerLike>(servers: TServer[]): {
  regions: Array<RegionBucket<TServer>>;
  unmatched: TServer[];
} {
  const buckets = new Map<string, RegionBucket<TServer>>();
  const unmatched: TServer[] = [];

  for (const server of servers) {
    const point = serverLocationPoint(server);
    if (!point) {
      unmatched.push(server);
      continue;
    }
    const current = buckets.get(point.key);
    if (current) {
      current.servers.push(server);
    } else {
      buckets.set(point.key, { point, servers: [server] });
    }
  }

  return {
    regions: Array.from(buckets.values()).sort((a, b) => b.servers.length - a.servers.length || a.point.label.localeCompare(b.point.label, "zh-CN")),
    unmatched,
  };
}

function serverLocationPoint(server: MapServerLike): RegionPoint | null {
  const location = server.location ?? null;
  const latitude = numberOrNull(location?.latitude ?? server.latitude);
  const longitude = numberOrNull(location?.longitude ?? server.longitude);
  const countryCode = countryCodeForLocation(location, server);
  const label = locationLabel(location, server, countryCode ? countryName(countryCode) : undefined);

  if (countryCode && countryFeatureByIso2.has(countryCode)) {
    const fallback = countryCoordinates[countryCode];
    const coord = latitude !== null && longitude !== null ? { lat: latitude, lng: longitude } : fallback;
    return {
      key: countryCode,
      label,
      countryCode,
      latitude: coord?.lat ?? 0,
      longitude: coord?.lng ?? 0,
      source: latitude !== null && longitude !== null ? (location?.source === "manual" ? "manual" : "geoip") : "country",
    };
  }

  if (latitude !== null && longitude !== null) {
    return {
      key: `${roundCoordinate(latitude)},${roundCoordinate(longitude)}`,
      label,
      latitude,
      longitude,
      source: location?.source === "manual" ? "manual" : "geoip",
    };
  }

  if (countryCode && countryCoordinates[countryCode]) {
    const coord = countryCoordinates[countryCode];
    return {
      key: countryCode,
      label,
      countryCode,
      latitude: coord.lat,
      longitude: coord.lng,
      source: "country",
    };
  }

  const centroid = centroidForLocation(location, server);
  if (!centroid) return null;
  return {
    key: centroid.key,
    label: locationLabel(location, server, centroid.label),
    latitude: centroid.lat,
    longitude: centroid.lng,
    source: "country",
  };
}

function projectPoint(longitude: number, latitude: number): [number, number] {
  return projection([longitude, latitude]) ?? [width / 2, height / 2];
}

function regionBlockPath(x: number, y: number, size: number): string {
  return `M ${x} ${y - size} L ${x + size} ${y} L ${x} ${y + size} L ${x - size} ${y} Z`;
}

function numberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function roundCoordinate(value: number): string {
  return value.toFixed(3);
}

function locationLabel(location: ServerLocationLike | null, server: MapServerLike, fallback?: string): string {
  return [location?.country ?? server.country, location?.region ?? server.region, location?.city ?? server.city].filter(Boolean).join(" / ") || fallback || server.region || server.name;
}

function countryName(countryCode: string): string | undefined {
  return countryFeatureByIso2.get(countryCode)?.properties.name ?? countryCoordinates[countryCode]?.name;
}

function countryCodeForLocation(location: ServerLocationLike | null, server: MapServerLike): string | undefined {
  const values = [location?.country, server.country, location?.region, server.region, location?.city, server.city];
  for (const value of values) {
    const code = normalizeCountryCode(value);
    if (code) return code;
  }
  return undefined;
}

function normalizeCountryCode(value?: string | null): string | undefined {
  const text = String(value ?? "").trim();
  if (!text) return undefined;
  const upper = text.toUpperCase();
  if (/^[A-Z]{2}$/.test(upper) && (countryFeatureByIso2.has(upper) || countryCoordinates[upper])) return upper;
  return countryAliases[normalizeLocationKey(text)];
}

function centroidForLocation(location: ServerLocationLike | null, server: MapServerLike): LocationCentroid | null {
  const keys = [location?.country, server.country, location?.region, server.region, location?.city, server.city];
  for (const value of keys) {
    const key = normalizeLocationKey(value);
    if (key && locationCentroids[key]) return locationCentroids[key];
  }
  return null;
}

function normalizeLocationKey(value?: string | null): string {
  return String(value ?? "")
    .trim()
    .toLowerCase()
    .replace(/[\s_.]+/g, "-");
}

function sourceLabel(source: RegionPoint["source"]): string {
  if (source === "manual") return "手动坐标";
  if (source === "geoip") return "GeoIP";
  return "国家/地区";
}

interface LocationCentroid {
  key: string;
  label: string;
  lat: number;
  lng: number;
}

const countryCoordinates: Record<string, { lat: number; lng: number; name: string }> = {
  AE: { lat: 24, lng: 54, name: "United Arab Emirates" },
  AU: { lat: -27, lng: 133, name: "Australia" },
  BR: { lat: -10, lng: -55, name: "Brazil" },
  CA: { lat: 60, lng: -95, name: "Canada" },
  CN: { lat: 35, lng: 105, name: "China" },
  DE: { lat: 51, lng: 9, name: "Germany" },
  FR: { lat: 46, lng: 2, name: "France" },
  GB: { lat: 54, lng: -2, name: "United Kingdom" },
  HK: { lat: 22, lng: 114, name: "Hong Kong" },
  IN: { lat: 20, lng: 77, name: "India" },
  JP: { lat: 36, lng: 138, name: "Japan" },
  KR: { lat: 37, lng: 127.5, name: "South Korea" },
  NL: { lat: 52.5, lng: 5.75, name: "Netherlands" },
  RU: { lat: 60, lng: 100, name: "Russia" },
  SG: { lat: 1.3667, lng: 103.8, name: "Singapore" },
  TR: { lat: 39, lng: 35, name: "Turkey" },
  TW: { lat: 23.5, lng: 121, name: "Taiwan" },
  US: { lat: 38, lng: -97, name: "United States" },
  ZA: { lat: -29, lng: 24, name: "South Africa" },
};

const manualCountryAliases: Record<string, string> = {
  america: "US",
  canada: "CA",
  china: "CN",
  cn: "CN",
  de: "DE",
  france: "FR",
  germany: "DE",
  hk: "HK",
  "hong-kong": "HK",
  japan: "JP",
  korea: "KR",
  netherlands: "NL",
  russia: "RU",
  singapore: "SG",
  "south-korea": "KR",
  taiwan: "TW",
  uk: "GB",
  "united-kingdom": "GB",
  "united-states": "US",
  us: "US",
  usa: "US",
  "中国": "CN",
  "中国大陆": "CN",
  "美国": "US",
  "加拿大": "CA",
  "英国": "GB",
  "德国": "DE",
  "法国": "FR",
  "荷兰": "NL",
  "俄罗斯": "RU",
  "新加坡": "SG",
  "香港": "HK",
  "台湾": "TW",
  "日本": "JP",
  "韩国": "KR",
  "南非": "ZA",
  "澳大利亚": "AU",
};

const countryAliases: Record<string, string> = {
  ...Object.fromEntries(
    countryFeatures.flatMap((feature) => [
      [normalizeLocationKey(feature.properties.iso2), feature.properties.iso2.toUpperCase()],
      [normalizeLocationKey(feature.properties.name), feature.properties.iso2.toUpperCase()],
    ]),
  ),
  ...manualCountryAliases,
};

const locationCentroids: Record<string, LocationCentroid> = Object.fromEntries(
  Object.entries(countryCoordinates).flatMap(([code, value]) => [
    [normalizeLocationKey(code), { key: code, label: value.name, lat: value.lat, lng: value.lng }],
    [normalizeLocationKey(value.name), { key: code, label: value.name, lat: value.lat, lng: value.lng }],
  ]),
);
