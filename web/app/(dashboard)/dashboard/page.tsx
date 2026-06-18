"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { apiClient } from "@/lib/api";

interface User {
  id: string;
  username: string;
  role: string;
}

interface ServerSummary {
  id: string;
  name: string;
  status: string;
  cpu_percent?: number;
  load_1?: number;
  last_seen_at?: string;
}

function getCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  const row = document.cookie
    .split("; ")
    .find((r) => r.startsWith(`${name}=`));
  return row?.split("=")[1] ?? null;
}

function buildWsUrl(): string {
  const apiBase =
    process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";
  const u = new URL(apiBase);
  const protocol = u.protocol === "https:" ? "wss" : "ws";
  return `${protocol}://${u.host}/ws/servers`;
}

export default function DashboardPage() {
  const router = useRouter();
  const [user] = useState<User | null>(() => {
    if (typeof window === "undefined") {
      return null;
    }

    const userStr = window.localStorage.getItem("user");
    if (!userStr) {
      return null;
    }

    try {
      return JSON.parse(userStr) as User;
    } catch {
      return null;
    }
  });

  const [servers, setServers] = useState<ServerSummary[]>([]);
  const [live, setLive] = useState<Record<string, { cpu_percent?: number; load_1?: number; received_at: string }>>({});
  const [conn, setConn] = useState<"connecting" | "open" | "closed" | "error">("connecting");
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    const sessionToken = localStorage.getItem("session_token");
    if (!sessionToken || !user) {
      router.push("/login");
    }
  }, [router, user]);

  const loadServers = useCallback(async () => {
    try {
      const res = await apiClient.listServers();
      if (res.success && res.data) {
        setServers((res.data.servers as ServerSummary[]) ?? []);
      }
    } catch {
      // ignore
    }
  }, []);

  useEffect(() => {
    const id = window.setTimeout(() => {
      void loadServers();
    }, 0);
    return () => window.clearTimeout(id);
  }, [loadServers]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const session = getCookie("xlstatus_session");
    if (!session) {
      return;
    }
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
      ws.onmessage = (ev) => {
        if (cancelled) return;
        try {
          const msg = JSON.parse(ev.data as string);
          if (msg.type === "snapshot" && Array.isArray(msg.events)) {
            setLive((prev) => {
              const next = { ...prev };
              for (const e of msg.events) {
                if (e?.kind === "host_state") {
                  next[e.agent_id] = {
                    cpu_percent: e.payload?.cpu_percent,
                    load_1: e.payload?.load_1,
                    received_at: e.received_at,
                  };
                }
              }
              return next;
            });
          } else if (msg.type === "event" && msg.event?.kind === "host_state") {
            const e = msg.event;
            setLive((prev) => ({
              ...prev,
              [e.agent_id]: {
                cpu_percent: e.payload?.cpu_percent,
                load_1: e.payload?.load_1,
                received_at: e.received_at,
              },
            }));
          }
        } catch {
          // ignore
        }
      };
      ws.onerror = () => {
        if (cancelled) return;
        setConn("error");
      };
      ws.onclose = () => {
        if (cancelled) return;
        setConn("closed");
        const next = Math.min(backoff * 2, 15000);
        backoff = next;
        window.setTimeout(connect, backoff);
      };
    }
    connect();
    return () => {
      cancelled = true;
      wsRef.current?.close();
      wsRef.current = null;
    };
  }, []);

  const handleLogout = () => {
    localStorage.removeItem("session_token");
    localStorage.removeItem("user");
    router.push("/login");
  };

  if (!user) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-600">Loading...</div>
      </div>
    );
  }

  const onlineCount = servers.filter((s) => s.status === "online").length;

  return (
    <div className="min-h-screen bg-gray-100">
      <header className="bg-white shadow">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4 flex justify-between items-center">
          <h1 className="text-2xl font-bold text-gray-900">XLStatus Dashboard</h1>
          <div className="flex items-center gap-4">
            <span className="text-sm text-gray-600">
              {user?.username} ({user?.role})
            </span>
            <span
              className={`text-xs font-medium ${
                conn === "open"
                  ? "text-green-600"
                  : conn === "connecting"
                    ? "text-yellow-600"
                    : "text-red-600"
              }`}
              data-testid="ws-status"
            >
              ws: {conn}
            </span>
            <button
              onClick={handleLogout}
              className="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
            >
              Logout
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Servers</h3>
            <p className="text-3xl font-bold text-blue-600" data-testid="server-count">
              {servers.length}
            </p>
            <p className="text-sm text-gray-500 mt-1">
              {onlineCount} online / {servers.length - onlineCount} offline
            </p>
          </div>
          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Live samples</h3>
            <p className="text-3xl font-bold text-green-600" data-testid="live-count">
              {Object.keys(live).length}
            </p>
            <p className="text-sm text-gray-500 mt-1">agents reporting via WS</p>
          </div>
          <div className="bg-white rounded-lg shadow p-6">
            <h3 className="text-lg font-semibold text-gray-700 mb-2">Connection</h3>
            <p className="text-3xl font-bold text-gray-700">{conn}</p>
            <p className="text-sm text-gray-500 mt-1">WebSocket to /ws/servers</p>
          </div>
        </div>

        <div className="bg-white rounded-lg shadow p-6 mb-8">
          <h2 className="text-xl font-bold text-gray-900 mb-4">Quick Actions</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            <Link
              href="/servers"
              className="p-4 border border-gray-200 rounded hover:bg-gray-50"
            >
              <h3 className="font-semibold text-gray-900">Servers</h3>
              <p className="text-sm text-gray-600 mt-1">
                Live status, CPU, memory, load
              </p>
            </Link>
            <Link
              href="/services"
              className="p-4 border border-gray-200 rounded hover:bg-gray-50"
            >
              <h3 className="font-semibold text-gray-900">Services</h3>
              <p className="text-sm text-gray-600 mt-1">Monitor services</p>
            </Link>
            <Link
              href="/alerts"
              className="p-4 border border-gray-200 rounded hover:bg-gray-50"
            >
              <h3 className="font-semibold text-gray-900">Alerts</h3>
              <p className="text-sm text-gray-600 mt-1">Configure alerts</p>
            </Link>
          </div>
        </div>

        {servers.length > 0 ? (
          <div className="bg-white rounded-lg shadow p-6">
            <h2 className="text-xl font-bold text-gray-900 mb-4">
              Live server overview
            </h2>
            <table className="min-w-full divide-y divide-gray-200" data-testid="live-overview">
              <thead className="bg-gray-50">
                <tr>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 uppercase">
                    Name
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 uppercase">
                    Status
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 uppercase">
                    CPU
                  </th>
                  <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 uppercase">
                    Load
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200">
                {servers.map((s) => {
                  const lv = live[s.id];
                  return (
                    <tr key={s.id}>
                      <td className="px-4 py-2 text-sm text-gray-900">
                        {s.name}
                      </td>
                      <td className="px-4 py-2 text-sm">
                        <span
                          className={
                            s.status === "online"
                              ? "text-green-600"
                              : "text-red-600"
                          }
                        >
                          {s.status}
                        </span>
                      </td>
                      <td className="px-4 py-2 text-sm text-gray-700">
                        {lv?.cpu_percent !== undefined
                          ? `${lv.cpu_percent.toFixed(1)}%`
                          : "—"}
                      </td>
                      <td className="px-4 py-2 text-sm text-gray-700">
                        {lv?.load_1 !== undefined ? lv.load_1.toFixed(2) : "—"}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="mt-8 bg-blue-50 border border-blue-200 rounded-lg p-6">
            <h3 className="text-lg font-semibold text-blue-900 mb-2">
              No servers yet
            </h3>
            <p className="text-blue-800">
              Enroll an agent with the enrollment token from the API and the live
              overview will appear here.
            </p>
          </div>
        )}
      </main>
    </div>
  );
}
