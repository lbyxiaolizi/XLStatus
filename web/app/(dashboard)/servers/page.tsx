"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  formatDate,
  formatPercent,
  inputClass,
  responseError,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient } from "@/lib/api";

interface Server {
  id: string;
  name: string;
  status: string;
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  last_seen_at?: string;
  last_event_at?: string;
}

interface LiveState {
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  received_at: string;
}

type ConnectionState = "connecting" | "open" | "closed" | "error";

export default function ServersPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [live, setLive] = useState<Record<string, LiveState>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [conn, setConn] = useState<ConnectionState>("closed");
  const wsRef = useRef<WebSocket | null>(null);

  const loadServers = useCallback(async () => {
    setLoading(true);
    setError(null);
    const response = await apiClient.listServers(200, 0);
    setLoading(false);
    if (response.success && response.data) {
      setServers((response.data.servers as Server[]) ?? []);
    } else {
      setError(responseError(response));
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServers();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServers]);

  useEffect(() => {
    if (typeof window === "undefined" || !hasBrowserSessionSignal()) return;
    let cancelled = false;
    let backoff = 1000;

    function connect() {
      if (cancelled) return;
      setConn("connecting");
      const ws = new WebSocket(buildWsUrl());
      wsRef.current = ws;
      ws.onopen = () => {
        if (cancelled) return;
        setConn("open");
        backoff = 1000;
      };
      ws.onmessage = (event) => {
        if (cancelled) return;
        try {
          const msg = JSON.parse(event.data as string) as Record<string, unknown>;
          const events = msg.type === "snapshot" && Array.isArray(msg.events) ? msg.events : msg.type === "event" ? [msg.event] : [];
          setLive((prev) => {
            const next = { ...prev };
            for (const raw of events) {
              const item = raw as Record<string, unknown>;
              if (item?.kind === "host_state" && typeof item.agent_id === "string") {
                next[item.agent_id] = {
                  ...((item.payload as Record<string, unknown>) ?? {}),
                  received_at: String(item.received_at || new Date().toISOString()),
                } as LiveState;
              }
            }
            return next;
          });
        } catch {
          // Ignore malformed live frames.
        }
      };
      ws.onerror = () => setConn("error");
      ws.onclose = () => {
        if (cancelled) return;
        setConn("closed");
        backoff = Math.min(backoff * 2, 15000);
        window.setTimeout(connect, backoff);
      };
    }

    connect();
    return () => {
      cancelled = true;
      wsRef.current?.close();
    };
  }, []);

  const merged = useMemo(
    () =>
      servers.map((server) => {
        const state = live[server.id];
        return {
          ...server,
          cpu_percent: state?.cpu_percent ?? server.cpu_percent,
          memory_used: state?.memory_used ?? server.memory_used,
          memory_total: state?.memory_total ?? server.memory_total,
          load_1: state?.load_1 ?? server.load_1,
          last_event_at: state?.received_at ?? server.last_event_at,
        };
      }),
    [live, servers],
  );

  const filtered = merged.filter((server) => {
    const needle = query.trim().toLowerCase();
    return !needle || [server.name, server.id, server.status].some((value) => value.toLowerCase().includes(needle));
  });

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow={`ws: ${conn}`}
          title="Servers"
          detail="接入的 Agent、实时主机状态和远程运维入口。"
          actions={<button type="button" onClick={() => void loadServers()} className={buttonClass("secondary")}>Refresh</button>}
        />
        <InlineError message={error} />
        <div className="mt-5 mb-5">
          <input value={query} onChange={(event) => setQuery(event.target.value)} className={inputClass} placeholder="Search servers" />
        </div>

        {loading ? (
          <BrutalCard>Loading servers...</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title="No servers found" detail="Servers will appear here once agents connect." />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>Name</th>
                  <th className={thClass}>Status</th>
                  <th className={thClass}>CPU</th>
                  <th className={thClass}>Memory</th>
                  <th className={thClass}>Load</th>
                  <th className={thClass}>Last Event</th>
                  <th className={thClass}>Action</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((server) => (
                  <tr key={server.id}>
                    <td className={tdClass}>
                      <div className="font-black">{server.name}</div>
                      <div className="text-xs text-[var(--text-muted)]">{server.id}</div>
                    </td>
                    <td className={tdClass}><StatusBadge tone={server.status === "online" ? "green" : "red"}>{server.status}</StatusBadge></td>
                    <td className={tdClass}>{formatPercent(server.cpu_percent)}</td>
                    <td className={tdClass}>{memoryLabel(server)}</td>
                    <td className={tdClass}>{server.load_1 === undefined ? "N/A" : server.load_1.toFixed(2)}</td>
                    <td className={tdClass}>{formatDate(server.last_event_at || server.last_seen_at)}</td>
                    <td className={tdClass}>
                      <Link className={buttonClass("primary")} href={`/servers/${encodeURIComponent(server.id)}`}>
                        Open
                      </Link>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </PageShell>
    </div>
  );
}

function memoryLabel(server: Server): string {
  if (!server.memory_used || !server.memory_total) return "N/A";
  return `${((server.memory_used / server.memory_total) * 100).toFixed(1)}%`;
}

function getCookie(name: string): string | null {
  return document.cookie.split("; ").find((row) => row.startsWith(`${name}=`))?.split("=")[1] ?? null;
}

function hasBrowserSessionSignal(): boolean {
  return Boolean(getCookie("xlstatus_csrf") || window.localStorage.getItem("session_token"));
}

function buildWsUrl(): string {
  const apiBase = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";
  const url = new URL(apiBase);
  return `${url.protocol === "https:" ? "wss" : "ws"}://${url.host}/ws/servers`;
}
