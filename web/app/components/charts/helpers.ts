// Shared chart data helpers for the dashboard server detail page and the public
// status pages. The three pages previously carried their own copies and the
// copies had drifted; keep the single source of truth here.

import type { MouseEvent } from "react";

export interface ChartPoint {
  time: number;
  value: number;
}

export type ChartAxis = "left" | "right";
export type ChartKind = "line" | "area";

export interface ChartSeries {
  id?: string | undefined;
  label: string;
  color: string;
  points: ChartPoint[];
  axis?: ChartAxis | undefined;
  kind?: ChartKind | undefined;
  opacity?: number | undefined;
  strokeDasharray?: string | undefined;
  strokeWidth?: number | undefined;
}

export interface PublicServiceResult {
  status: string;
  delay_ms?: number | null | undefined;
  created_at: string;
}

export interface PublicServiceDay {
  key: string;
  label: string;
  uptime: number;
  avgDelay?: number | undefined;
  total: number;
}

// Drop samples the SVG cannot place. Charts call this per series, so it stays a
// single pass without intermediate allocations.
export function finitePoints(points: ChartPoint[]): ChartPoint[] {
  return points.filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
}

// Downsample to at most `limit` evenly spaced points, always keeping the last
// sample so the chart head matches the live value.
export function thinPoints(points: ChartPoint[], limit = 240): ChartPoint[] {
  if (points.length <= limit) return points;
  const lastIndex = points.length - 1;
  const step = lastIndex / (limit - 1);
  const seen = new Set<number>();
  const out: ChartPoint[] = [];
  for (let index = 0; index < limit; index += 1) {
    const point = points[Math.round(index * step)];
    if (point && !seen.has(point.time)) {
      seen.add(point.time);
      out.push(point);
    }
  }
  const last = points[lastIndex];
  if (last && !seen.has(last.time)) out.push(last);
  return out;
}

export function chartHasData(series: ChartSeries[]): boolean {
  return series.some((item) => item.points.some((point) => Number.isFinite(point.time) && Number.isFinite(point.value)));
}

export function formatChartTime(value: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function nearestPoint(points: ChartPoint[], targetTime: number): ChartPoint | null {
  let best: ChartPoint | null = null;
  for (const point of points) {
    if (!best || Math.abs(point.time - targetTime) < Math.abs(best.time - targetTime)) {
      best = point;
    }
  }
  return best;
}

export function nearestValue(values: number[], target: number): number {
  let best = values[0] ?? target;
  for (const value of values) {
    if (Math.abs(value - target) < Math.abs(best - target)) best = value;
  }
  return best;
}

// Pointer x in viewBox units. getScreenCTM is exact under CSS transforms; the
// bounding-rect ratio is the fallback for browsers that return no matrix.
export function svgPointerX(event: MouseEvent<SVGSVGElement>, fallbackWidth: number): number {
  const svg = event.currentTarget;
  const matrix = svg.getScreenCTM();
  if (matrix) {
    const point = svg.createSVGPoint();
    point.x = event.clientX;
    point.y = event.clientY;
    return point.matrixTransform(matrix.inverse()).x;
  }

  const rect = svg.getBoundingClientRect();
  return ((event.clientX - rect.left) / rect.width) * fallbackWidth;
}

export function serviceOk(status: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

export function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

export function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

// 30-day uptime strip. Bucketing pushes into the existing array instead of
// rebuilding it per row, which keeps a month of probe results linear.
export function buildPublicServiceDays(results: PublicServiceResult[]): PublicServiceDay[] {
  const today = startOfDay(new Date());
  const buckets = new Map<string, PublicServiceResult[]>();
  for (const result of results) {
    const date = new Date(result.created_at);
    if (Number.isNaN(date.getTime())) continue;
    const key = dayKey(date);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(result);
    else buckets.set(key, [result]);
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

export function publicServiceDayClass(day: PublicServiceDay): string {
  if (day.total === 0) return "bg-[var(--accent-bg)]";
  if (day.uptime >= 99) return "bg-[var(--accent-color)]";
  if (day.uptime >= 95) return "bg-yellow-300";
  return "bg-[var(--btn-bg)]";
}

export function averageDelay(results: PublicServiceResult[]): number | undefined {
  const values = results
    .map((result) => (typeof result.delay_ms === "number" ? result.delay_ms : undefined))
    .filter((value): value is number => value !== undefined);
  return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : undefined;
}
