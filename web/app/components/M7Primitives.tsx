"use client";

import { ReactNode, useEffect, useState } from "react";
import type { ApiResponse } from "@/lib/api";

export interface StoredUser {
  id: string;
  username: string;
  role: string;
}

export function useStoredUser(): StoredUser | null {
  const [user, setUser] = useState<StoredUser | null>(null);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setUser(readStoredUser());
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, []);

  return user;
}

export function readStoredUser(): StoredUser | null {
  if (typeof window === "undefined") {
    return null;
  }

  const userStr = window.localStorage.getItem("user");
  if (!userStr) {
    return null;
  }

  try {
    return JSON.parse(userStr) as StoredUser;
  } catch {
    return null;
  }
}

export function isAdmin(user: StoredUser | null): boolean {
  return user?.role?.toLowerCase() === "admin";
}

export function responseError(response: ApiResponse<unknown>): string {
  const suffix = response.request_id ? ` (${response.request_id})` : "";
  if (response.status === 401) {
    return `Authentication required${suffix}`;
  }
  if (response.status === 403) {
    return `Permission denied${suffix}`;
  }
  if (response.status === 404) {
    return `Backend route or resource not found${suffix}`;
  }
  return `${response.error || "Request failed"}${suffix}`;
}

export function formatDate(value?: string | null): string {
  if (!value) {
    return "Never";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

export function formatPercent(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) {
    return "N/A";
  }
  return `${value.toFixed(1)}%`;
}

export function formatMs(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) {
    return "N/A";
  }
  return `${value} ms`;
}

export function compactId(value?: string | null): string {
  if (!value) {
    return "-";
  }
  return value.length > 13 ? `${value.slice(0, 8)}...${value.slice(-4)}` : value;
}

export function StatusBadge({
  tone,
  children,
}: {
  tone: "green" | "red" | "yellow" | "gray" | "blue";
  children: ReactNode;
}) {
  const classes = {
    green: "bg-green-100 text-green-800 border-green-200",
    red: "bg-red-100 text-red-800 border-red-200",
    yellow: "bg-yellow-100 text-yellow-800 border-yellow-200",
    gray: "bg-gray-100 text-gray-800 border-gray-200",
    blue: "bg-blue-100 text-blue-800 border-blue-200",
  };

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-semibold ${classes[tone]}`}
    >
      {children}
    </span>
  );
}

export function InlineError({ message }: { message: string | null }) {
  if (!message) {
    return null;
  }

  return (
    <div className="rounded border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
      {message}
    </div>
  );
}

export function InlineNotice({
  tone = "blue",
  children,
}: {
  tone?: "blue" | "yellow" | "green";
  children: ReactNode;
}) {
  const classes = {
    blue: "border-blue-200 bg-blue-50 text-blue-800",
    yellow: "border-yellow-200 bg-yellow-50 text-yellow-800",
    green: "border-green-200 bg-green-50 text-green-800",
  };

  return (
    <div className={`rounded border px-4 py-3 text-sm ${classes[tone]}`}>
      {children}
    </div>
  );
}

export function EmptyState({
  title,
  detail,
}: {
  title: string;
  detail?: string;
}) {
  return (
    <div className="rounded-lg bg-white px-4 py-10 text-center shadow">
      <p className="font-medium text-gray-700">{title}</p>
      {detail ? <p className="mt-2 text-sm text-gray-500">{detail}</p> : null}
    </div>
  );
}

export function Field({
  label,
  error,
  children,
}: {
  label: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium text-gray-700">{label}</span>
      {children}
      {error ? <span className="mt-1 block text-xs text-red-600">{error}</span> : null}
    </label>
  );
}

export function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-gray-900/40 px-4 py-8">
      <div className="w-full max-w-2xl rounded-lg bg-white shadow-xl">
        <div className="flex items-center justify-between border-b border-gray-200 px-5 py-4">
          <h2 className="text-lg font-semibold text-gray-900">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded px-2 py-1 text-sm text-gray-500 hover:bg-gray-100 hover:text-gray-900"
          >
            Close
          </button>
        </div>
        <div className="px-5 py-5">{children}</div>
      </div>
    </div>
  );
}

export function PageShell({ children }: { children: ReactNode }) {
  return (
    <div className="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8 lg:py-8">
      {children}
    </div>
  );
}

export function buttonClass(variant: "primary" | "secondary" | "danger" = "secondary") {
  if (variant === "primary") {
    return "rounded bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50";
  }
  if (variant === "danger") {
    return "rounded bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-50";
  }
  return "rounded border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:cursor-not-allowed disabled:opacity-50";
}

export const inputClass =
  "mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm text-gray-900 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500";

export const selectClass =
  "mt-1 block w-full rounded border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500";

export const textareaClass =
  "mt-1 block w-full rounded border border-gray-300 px-3 py-2 text-sm text-gray-900 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500";
