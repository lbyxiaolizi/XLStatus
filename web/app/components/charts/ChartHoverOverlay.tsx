"use client";

import { memo } from "react";

export interface ChartHoverEntry {
  label: string;
  color: string;
  text: string;
}

// Crosshair + tooltip layer. Kept separate from the chart bodies so a pointer
// move only re-renders this subtree while the memoized series paths stay put.
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
      <line x1={x} x2={x} y1={top} y2={top + plotHeight} stroke="var(--border-color)" strokeDasharray="6 5" strokeWidth="2" />
      <rect x={tooltipX} y={tooltipY} width={tooltipWidth} height={tooltipHeight} fill="var(--bg-card)" stroke="var(--border-color)" strokeWidth="2" />
      <text x={tooltipX + 10} y={tooltipY + 18} fill="var(--text-main)" fontSize="12" fontWeight="900">
        {timeLabel}
      </text>
      {entries.map((entry, index) => (
        <g key={`${entry.label}-${index}`} transform={`translate(${tooltipX + 10}, ${tooltipY + 36 + index * 20})`}>
          <rect width="9" height="9" y="-8" fill={entry.color} stroke="var(--border-color)" strokeWidth="1.5" />
          <text x="16" y="0" fill="var(--text-main)" fontSize="12" fontWeight="900">
            {entry.text}
          </text>
        </g>
      ))}
    </g>
  );
});
