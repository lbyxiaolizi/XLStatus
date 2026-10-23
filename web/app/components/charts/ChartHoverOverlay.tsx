"use client";

import { memo } from "react";

export interface ChartHoverEntry {
  label: string;
  color: string;
  text: string;
}

export const ChartHoverOverlay = memo(function ChartHoverOverlay({
  x,
  timeLabel,
  entries,
  tooltipWidth,
  left,
  right,
  top,
  plotHeight,
}: {
  x: number;
  timeLabel: string;
  entries: ChartHoverEntry[];
  tooltipWidth: number;
  left: number;
  right: number;
  top: number;
  plotHeight: number;
}) {
  const tooltipHeight = 28 + entries.length * 20;
  const tooltipX = Math.min(Math.max(x + 12, left), right - tooltipWidth);
  const tooltipY = Math.max(top + 4, top + plotHeight - tooltipHeight - 8);

  return (
    <g>
      <line x1={x} x2={x} y1={top} y2={top + plotHeight} stroke="var(--text-muted)" strokeDasharray="4 4" strokeWidth="1" />
      <rect x={tooltipX} y={tooltipY} width={tooltipWidth} height={tooltipHeight} rx="8" fill="var(--bg-card)" stroke="var(--border-color)" strokeWidth="1" />
      <text x={tooltipX + 10} y={tooltipY + 18} fill="var(--text-muted)" fontSize="11" fontWeight="600">
        {timeLabel}
      </text>
      {entries.map((entry, index) => (
        <g key={`${entry.label}-${index}`} transform={`translate(${tooltipX + 10}, ${tooltipY + 36 + index * 20})`}>
          <circle cx="4" cy="-4" r="3.5" fill={entry.color} />
          <text x="14" y="0" fill="var(--text-main)" fontSize="11" fontWeight="600">
            {entry.text}
          </text>
        </g>
      ))}
    </g>
  );
});
