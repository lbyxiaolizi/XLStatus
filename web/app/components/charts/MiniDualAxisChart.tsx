"use client";

import { memo, useCallback, useMemo, useState, type MouseEvent } from "react";
import { formatMs } from "@/app/components/M7Primitives";
import { maxOf, minOf } from "@/app/lib/format";
import { ChartHoverOverlay, type ChartHoverEntry } from "./ChartHoverOverlay";
import {
  finitePoints,
  formatChartTime,
  nearestPoint,
  nearestValue,
  svgPointerX,
  type ChartPoint,
  type ChartSeries,
} from "./helpers";

const WIDTH = 760;
const HEIGHT = 240;
const PADDING = { top: 16, right: 52, bottom: 34, left: 52 };
const PLOT_WIDTH = WIDTH - PADDING.left - PADDING.right;
const PLOT_HEIGHT = HEIGHT - PADDING.top - PADDING.bottom;
const GRID_RATIOS = [0, 0.25, 0.5, 0.75, 1];
const TOOLTIP_WIDTH = 240;
const LOSS_MAX = 100;
const LOSS_FILL = "#f59e0b";
const LOSS_STROKE = "#d97706";

interface DualLayout {
  minTime: number;
  maxTime: number;
  timeSpan: number;
  timeValues: number[];
  latencyMax: number;
  x: (time: number) => number;
  yLatency: (value: number) => number;
  yLoss: (value: number) => number;
}

interface HoverState {
  x: number;
  time: number;
  values: ChartHoverEntry[];
}

function formatLatency(value: number): string {
  return formatMs(Math.round(value));
}

function formatLoss(value: number): string {
  return `${value.toFixed(1)}%`;
}

function buildDualLayout(latencySeries: ChartSeries[], lossSeries: ChartSeries[]): DualLayout | null {
  const points: ChartPoint[] = [];
  const latencyValues: number[] = [];
  for (const item of latencySeries) {
    for (const point of item.points) {
      if (Number.isFinite(point.value)) latencyValues.push(point.value);
      if (Number.isFinite(point.time) && Number.isFinite(point.value)) points.push(point);
    }
  }
  for (const item of lossSeries) {
    for (const point of item.points) {
      if (Number.isFinite(point.time) && Number.isFinite(point.value)) points.push(point);
    }
  }
  if (!points.length) return null;

  const times = points.map((point) => point.time);
  const minTime = minOf(times);
  const maxTime = maxOf(times);
  const timeSpan = Math.max(maxTime - minTime, 1);
  const latencyMax = Math.max(1, Math.ceil(maxOf(latencyValues, 1) * 1.2));
  const single = points.length === 1;

  return {
    minTime,
    maxTime,
    timeSpan,
    timeValues: Array.from(new Set(times)).sort((a, b) => a - b),
    latencyMax,
    x: (time: number) => (single ? PADDING.left + PLOT_WIDTH / 2 : PADDING.left + ((time - minTime) / timeSpan) * PLOT_WIDTH),
    yLatency: (value: number) => {
      const clamped = Math.max(0, Math.min(latencyMax, value));
      return PADDING.top + PLOT_HEIGHT - (clamped / latencyMax) * PLOT_HEIGHT;
    },
    yLoss: (value: number) => {
      const clamped = Math.max(0, Math.min(LOSS_MAX, value));
      return PADDING.top + PLOT_HEIGHT - (clamped / LOSS_MAX) * PLOT_HEIGHT;
    },
  };
}

export const MiniDualAxisChart = memo(function MiniDualAxisChart({
  latencySeries,
  lossSeries,
  emptyLabel,
  ariaLabel,
  latencyLabelTemplate,
  lossLabelTemplate,
}: {
  latencySeries: ChartSeries[];
  lossSeries: ChartSeries[];
  emptyLabel: string;
  ariaLabel: string;
  latencyLabelTemplate: string;
  lossLabelTemplate: string;
}) {
  const [hover, setHover] = useState<HoverState | null>(null);
  const layout = useMemo(() => buildDualLayout(latencySeries, lossSeries), [latencySeries, lossSeries]);

  const chrome = useMemo(() => {
    if (!layout) return null;
    return (
      <>
        {GRID_RATIOS.map((ratio) => {
          const lineY = PADDING.top + PLOT_HEIGHT * ratio;
          return (
            <line
              key={ratio}
              x1={PADDING.left}
              x2={WIDTH - PADDING.right}
              y1={lineY}
              y2={lineY}
              stroke="var(--border-color)"
              strokeOpacity="0.6"
              strokeWidth="1"
            />
          );
        })}
        <text x={8} y={PADDING.top + 10} fill="var(--text-muted)" fontSize="11" fontWeight="500">
          {formatMs(layout.latencyMax)}
        </text>
        <text x={8} y={PADDING.top + PLOT_HEIGHT} fill="var(--text-muted)" fontSize="11" fontWeight="500">
          0 ms
        </text>
        <text x={WIDTH - 8} y={PADDING.top + 10} fill="var(--text-muted)" fontSize="11" fontWeight="500" textAnchor="end">
          100%
        </text>
        <text x={WIDTH - 8} y={PADDING.top + PLOT_HEIGHT} fill="var(--text-muted)" fontSize="11" fontWeight="500" textAnchor="end">
          0%
        </text>
        <text x={PADDING.left} y={HEIGHT - 10} fill="var(--text-muted)" fontSize="11" fontWeight="500">
          {formatChartTime(layout.minTime)}
        </text>
        <text x={WIDTH - PADDING.right} y={HEIGHT - 10} fill="var(--text-muted)" fontSize="11" fontWeight="500" textAnchor="end">
          {formatChartTime(layout.maxTime)}
        </text>
      </>
    );
  }, [layout]);

  const lossPaths = useMemo(() => {
    if (!layout) return null;
    const baseline = PADDING.top + PLOT_HEIGHT;
    return lossSeries.map((item) => {
      const points = finitePoints(item.points);
      const linePath = points
        .map((point, index) => `${index === 0 ? "M" : "L"} ${layout.x(point.time).toFixed(2)} ${layout.yLoss(point.value).toFixed(2)}`)
        .join(" ");
      const first = points[0];
      const last = points[points.length - 1];
      const areaPath = first && last
        ? `${linePath} L ${layout.x(last.time).toFixed(2)} ${baseline.toFixed(2)} L ${layout.x(first.time).toFixed(2)} ${baseline.toFixed(2)} Z`
        : "";
      return (
        <g key={`${item.id ?? item.label}-loss`}>
          {areaPath ? <path d={areaPath} fill={LOSS_FILL} fillOpacity="0.14" stroke="none" /> : null}
          <path d={linePath} fill="none" stroke={LOSS_STROKE} strokeDasharray="5 4" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.75" />
        </g>
      );
    });
  }, [layout, lossSeries]);

  const latencyPaths = useMemo(() => {
    if (!layout) return null;
    return latencySeries.map((item) => {
      const points = finitePoints(item.points);
      const path = points
        .map((point, index) => `${index === 0 ? "M" : "L"} ${layout.x(point.time).toFixed(2)} ${layout.yLatency(point.value).toFixed(2)}`)
        .join(" ");
      const firstPoint = points[0];
      return (
        <g key={item.id ?? item.label}>
          <path d={path} fill="none" stroke={item.color} strokeLinecap="round" strokeLinejoin="round" strokeWidth="2.25" />
          {points.length === 1 && firstPoint ? (
            <circle cx={layout.x(firstPoint.time)} cy={layout.yLatency(firstPoint.value)} r="4" fill={item.color} stroke="var(--bg-card)" strokeWidth="1.5" />
          ) : null}
        </g>
      );
    });
  }, [latencySeries, layout]);

  const handleMove = useCallback(
    (event: MouseEvent<SVGSVGElement>) => {
      if (!layout) return;
      const localX = svgPointerX(event, WIDTH);
      if (localX < PADDING.left || localX > WIDTH - PADDING.right) {
        setHover(null);
        return;
      }
      const targetTime = layout.minTime + ((localX - PADDING.left) / PLOT_WIDTH) * layout.timeSpan;
      const hoverTime = nearestValue(layout.timeValues, targetTime);
      const values = [
        ...latencySeries.map((item) => {
          const point = nearestPoint(item.points, hoverTime);
          if (!point) return null;
          const label = latencyLabelTemplate.replace("{label}", item.label);
          return { label, color: item.color, text: `${label}: ${formatLatency(point.value)}` };
        }),
        ...lossSeries.map((item) => {
          const point = nearestPoint(item.points, hoverTime);
          if (!point) return null;
          const label = lossLabelTemplate.replace("{label}", item.label);
          return { label, color: LOSS_FILL, text: `${label}: ${formatLoss(point.value)}` };
        }),
      ].filter((item): item is ChartHoverEntry => Boolean(item));
      if (!values.length) {
        setHover(null);
        return;
      }
      setHover({ x: layout.x(hoverTime), time: hoverTime, values });
    },
    [latencyLabelTemplate, latencySeries, layout, lossLabelTemplate, lossSeries],
  );

  const handleLeave = useCallback(() => setHover(null), []);

  if (!layout) {
    return (
      <div className="flex h-56 items-center justify-center rounded-xl border border-dashed border-[var(--border-color)] bg-[var(--bg-subtle)] text-xs font-medium text-[var(--text-muted)]">
        {emptyLabel}
      </div>
    );
  }

  return (
    <svg
      className="h-60 w-full rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]"
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      role="img"
      aria-label={ariaLabel}
      onMouseMove={handleMove}
      onMouseLeave={handleLeave}
    >
      {chrome}
      {lossPaths}
      {latencyPaths}
      {hover ? (
        <ChartHoverOverlay
          x={hover.x}
          timeLabel={formatChartTime(hover.time)}
          entries={hover.values}
          tooltipWidth={TOOLTIP_WIDTH}
          left={PADDING.left}
          right={WIDTH - PADDING.right}
          top={PADDING.top}
          plotHeight={PLOT_HEIGHT}
        />
      ) : null}
    </svg>
  );
});
