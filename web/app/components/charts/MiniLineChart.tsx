"use client";

import { memo, useCallback, useMemo, useState, type MouseEvent } from "react";
import { maxOf, minOf } from "@/app/lib/format";
import { ChartHoverOverlay } from "./ChartHoverOverlay";
import {
  finitePoints,
  formatChartTime,
  nearestPoint,
  nearestValue,
  svgPointerX,
  type ChartPoint,
  type ChartSeries,
} from "./helpers";

export type MiniLineChartVariant = "detail" | "compact";

interface ChartPreset {
  width: number;
  height: number;
  padding: { top: number; right: number; bottom: number; left: number };
  svgClass: string;
  emptyClass: string;
}

// Geometry presets: "detail" is the dashboard server detail size, "compact" the
// denser public status card. Everything else (grid, typography, stroke widths)
// is shared so both surfaces stay visually identical.
const PRESETS: Record<MiniLineChartVariant, ChartPreset> = {
  detail: {
    width: 640,
    height: 190,
    padding: { top: 14, right: 16, bottom: 30, left: 46 },
    svgClass: "h-48 w-full border-2 border-black bg-[var(--bg-page)]",
    emptyClass: "flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black",
  },
  compact: {
    width: 640,
    height: 160,
    padding: { top: 12, right: 14, bottom: 28, left: 44 },
    svgClass: "h-40 w-full border-2 border-black bg-[var(--bg-page)]",
    emptyClass: "flex h-36 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-xs font-black",
  },
};

const GRID_RATIOS = [0, 0.25, 0.5, 0.75, 1];
const TOOLTIP_WIDTH = 210;

interface LineLayout {
  preset: ChartPreset;
  plotWidth: number;
  plotHeight: number;
  minTime: number;
  maxTime: number;
  timeSpan: number;
  timeValues: number[];
  yMax: number;
  x: (time: number) => number;
  y: (value: number) => number;
}

interface HoverState {
  x: number;
  time: number;
  values: Array<{ label: string; color: string; value: number }>;
}

function buildLineLayout(
  series: ChartSeries[],
  maxValue: number | undefined,
  preset: ChartPreset,
  withTimeValues: boolean,
): LineLayout | null {
  const points: ChartPoint[] = [];
  for (const item of series) {
    for (const point of item.points) {
      if (Number.isFinite(point.time) && Number.isFinite(point.value)) points.push(point);
    }
  }
  if (!points.length) return null;

  const times = points.map((point) => point.time);
  const minTime = minOf(times);
  const maxTime = maxOf(times);
  const maxSeriesValue = maxOf(points.map((point) => point.value), 1);
  const yMax = maxValue ?? Math.max(1, Math.ceil(maxSeriesValue * 1.2));
  const timeSpan = Math.max(maxTime - minTime, 1);
  const plotWidth = preset.width - preset.padding.left - preset.padding.right;
  const plotHeight = preset.height - preset.padding.top - preset.padding.bottom;
  const single = points.length === 1;

  return {
    preset,
    plotWidth,
    plotHeight,
    minTime,
    maxTime,
    timeSpan,
    // Hover snaps to a real sample, so the unique sorted timeline is only worth
    // building for interactive charts.
    timeValues: withTimeValues ? Array.from(new Set(times)).sort((a, b) => a - b) : [],
    yMax,
    x: (time: number) => (single ? preset.padding.left + plotWidth / 2 : preset.padding.left + ((time - minTime) / timeSpan) * plotWidth),
    y: (value: number) => {
      const clamped = Math.max(0, Math.min(yMax, value));
      return preset.padding.top + plotHeight - (clamped / yMax) * plotHeight;
    },
  };
}

export const MiniLineChart = memo(function MiniLineChart({
  series,
  maxValue,
  formatValue,
  emptyLabel,
  ariaLabel,
  interactive = true,
  variant = "detail",
}: {
  series: ChartSeries[];
  maxValue?: number;
  formatValue: (value: number) => string;
  emptyLabel: string;
  ariaLabel: string;
  interactive?: boolean;
  variant?: MiniLineChartVariant;
}) {
  const [hover, setHover] = useState<HoverState | null>(null);
  const preset = PRESETS[variant];
  const layout = useMemo(() => buildLineLayout(series, maxValue, preset, interactive), [series, maxValue, preset, interactive]);

  const chrome = useMemo(() => {
    if (!layout) return null;
    const { padding, width, height } = layout.preset;
    return (
      <>
        {GRID_RATIOS.map((ratio) => {
          const lineY = padding.top + layout.plotHeight * ratio;
          return (
            <line
              key={ratio}
              x1={padding.left}
              x2={width - padding.right}
              y1={lineY}
              y2={lineY}
              stroke="var(--border-color)"
              strokeOpacity="0.18"
              strokeWidth="2"
            />
          );
        })}
        <text x={8} y={padding.top + 10} fill="var(--text-muted)" fontSize="12" fontWeight="900">
          {formatValue(layout.yMax)}
        </text>
        <text x={8} y={padding.top + layout.plotHeight} fill="var(--text-muted)" fontSize="12" fontWeight="900">
          {formatValue(0)}
        </text>
        <text x={padding.left} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900">
          {formatChartTime(layout.minTime)}
        </text>
        <text x={width - padding.right} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
          {formatChartTime(layout.maxTime)}
        </text>
      </>
    );
  }, [formatValue, layout]);

  const paths = useMemo(() => {
    if (!layout) return null;
    return series.map((item) => {
      const points = finitePoints(item.points);
      const path = points
        .map((point, index) => `${index === 0 ? "M" : "L"} ${layout.x(point.time).toFixed(2)} ${layout.y(point.value).toFixed(2)}`)
        .join(" ");
      return (
        <g key={item.id ?? item.label}>
          <path d={path} fill="none" stroke={item.color} strokeLinecap="square" strokeLinejoin="round" strokeWidth="4" />
          {points.length === 1 ? (
            <circle cx={layout.x(points[0].time)} cy={layout.y(points[0].value)} r="5" fill={item.color} stroke="var(--border-color)" strokeWidth="2" />
          ) : null}
        </g>
      );
    });
  }, [layout, series]);

  const handleMove = useCallback(
    (event: MouseEvent<SVGSVGElement>) => {
      if (!layout) return;
      const { padding, width } = layout.preset;
      const localX = svgPointerX(event, width);
      if (localX < padding.left || localX > width - padding.right) {
        setHover(null);
        return;
      }
      const targetTime = layout.minTime + ((localX - padding.left) / layout.plotWidth) * layout.timeSpan;
      const hoverTime = nearestValue(layout.timeValues, targetTime);
      const values = series
        .map((item) => {
          const point = nearestPoint(item.points, hoverTime);
          return point ? { label: item.label, color: item.color, value: point.value } : null;
        })
        .filter((item): item is { label: string; color: string; value: number } => Boolean(item));
      if (!values.length) {
        setHover(null);
        return;
      }
      setHover({ x: layout.x(hoverTime), time: hoverTime, values });
    },
    [layout, series],
  );

  const handleLeave = useCallback(() => setHover(null), []);

  if (!layout) {
    return <div className={preset.emptyClass}>{emptyLabel}</div>;
  }

  return (
    <svg
      className={preset.svgClass}
      viewBox={`0 0 ${preset.width} ${preset.height}`}
      role="img"
      aria-label={ariaLabel}
      onMouseMove={interactive ? handleMove : undefined}
      onMouseLeave={interactive ? handleLeave : undefined}
    >
      {chrome}
      {paths}
      {interactive && hover ? (
        <ChartHoverOverlay
          x={hover.x}
          timeLabel={formatChartTime(hover.time)}
          entries={hover.values.map((item) => ({
            label: item.label,
            color: item.color,
            text: `${item.label}: ${formatValue(item.value)}`,
          }))}
          tooltipWidth={TOOLTIP_WIDTH}
          left={preset.padding.left}
          right={preset.width - preset.padding.right}
          top={preset.padding.top}
          plotHeight={layout.plotHeight}
        />
      ) : null}
    </svg>
  );
});
