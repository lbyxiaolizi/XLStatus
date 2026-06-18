'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { apiClient } from '@/lib/api';
import Navigation from '@/app/components/Navigation';

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

type ConnectionState = 'connecting' | 'open' | 'closed' | 'error';

function getCookie(name: string): string | null {
  if (typeof document === 'undefined') return null;
  const row = document.cookie
    .split('; ')
    .find((r) => r.startsWith(`${name}=`));
  return row?.split('=')[1] ?? null;
}

function buildWsUrl(): string {
  const apiBase =
    process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';
  const u = new URL(apiBase);
  const protocol = u.protocol === 'https:' ? 'wss' : 'ws';
  return `${protocol}://${u.host}/ws/servers`;
}

function memPercent(state: LiveState | undefined): number | null {
  if (!state?.memory_used || !state?.memory_total) return null;
  return (state.memory_used / state.memory_total) * 100;
}

export default function ServersPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [live, setLive] = useState<Record<string, LiveState>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [conn, setConn] = useState<ConnectionState>('connecting');
  const wsRef = useRef<WebSocket | null>(null);

  const loadServers = useCallback(async () => {
    try {
      setLoading(true);
      const response = await apiClient.listServers();
      if (response.success && response.data) {
        setServers(response.data.servers as Server[]);
      } else {
        setError(response.error || 'Failed to load servers');
      }
    } catch {
      setError('Network error');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const id = window.setTimeout(() => {
      void loadServers();
    }, 0);
    return () => window.clearTimeout(id);
  }, [loadServers]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const session = getCookie('xlstatus_session');
    if (!session) {
      return;
    }
    let cancelled = false;
    let backoff = 1000;

    function connect() {
      if (cancelled) return;
      setConn('connecting');
      const ws = new WebSocket(buildWsUrl());
      wsRef.current = ws;
      ws.onopen = () => {
        if (cancelled) return;
        setConn('open');
        backoff = 1000;
      };
      ws.onmessage = (ev) => {
        if (cancelled) return;
        try {
          const msg = JSON.parse(ev.data as string);
          if (msg.type === 'snapshot' && Array.isArray(msg.events)) {
            setLive((prev) => {
              const next = { ...prev };
              for (const e of msg.events) {
                if (e?.kind === 'host_state') {
                  next[e.agent_id] = { ...e.payload, received_at: e.received_at };
                }
              }
              return next;
            });
          } else if (msg.type === 'event' && msg.event) {
            const e = msg.event;
            if (e.kind === 'host_state') {
              setLive((prev) => ({
                ...prev,
                [e.agent_id]: { ...e.payload, received_at: e.received_at },
              }));
            }
          }
        } catch {
          // ignore malformed frames
        }
      };
      ws.onerror = () => {
        if (cancelled) return;
        setConn('error');
      };
      ws.onclose = () => {
        if (cancelled) return;
        setConn('closed');
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

  const merged = useMemo(
    () =>
      servers.map((s) => {
        const live_state = live[s.id];
        return {
          ...s,
          cpu_percent: live_state?.cpu_percent ?? s.cpu_percent,
          memory_used: live_state?.memory_used ?? s.memory_used,
          memory_total: live_state?.memory_total ?? s.memory_total,
          load_1: live_state?.load_1 ?? s.load_1,
          last_event_at: live_state?.received_at,
        } as Server;
      }),
    [servers, live],
  );

  function connLabel(): { text: string; color: string } {
    switch (conn) {
      case 'open':
        return { text: 'live', color: 'text-green-600' };
      case 'connecting':
        return { text: 'connecting…', color: 'text-yellow-600' };
      case 'closed':
        return { text: 'offline', color: 'text-gray-500' };
      case 'error':
        return { text: 'error', color: 'text-red-600' };
    }
  }

  if (loading) {
    return (
      <div>
        <Navigation />
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
          <p>Loading...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <Navigation />
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
          <div className="bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded">
            {error}
          </div>
        </div>
      </div>
    );
  }

  const connInfo = connLabel();

  return (
    <div>
      <Navigation />
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="mb-6 flex justify-between items-center">
          <h1 className="text-2xl font-bold text-gray-900">Servers</h1>
          <div className={`text-sm font-medium ${connInfo.color}`}>
            ws: {connInfo.text}
          </div>
        </div>

        {merged.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-gray-500">No servers found</p>
            <p className="text-sm text-gray-400 mt-2">
              Servers will appear here once agents connect
            </p>
          </div>
        ) : (
          <div className="bg-white shadow overflow-hidden sm:rounded-lg">
            <table className="min-w-full divide-y divide-gray-200">
              <thead className="bg-gray-50">
                <tr>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Name
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Status
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    CPU
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Memory
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Load (1m)
                  </th>
                  <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                    Last seen
                  </th>
                </tr>
              </thead>
              <tbody className="bg-white divide-y divide-gray-200">
                {merged.map((server) => {
                  const memPct = memPercent(live[server.id]);
                  return (
                    <tr
                      key={server.id}
                      className="hover:bg-gray-50"
                      data-testid={`server-row-${server.id}`}
                    >
                      <td className="px-6 py-4 whitespace-nowrap">
                        <a
                          href={`/servers/${server.id}`}
                          className="text-sm font-medium text-blue-700 hover:underline"
                        >
                          {server.name}
                        </a>
                        <div className="text-sm text-gray-500">
                          {server.id}
                        </div>
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap">
                        <span
                          className={`text-sm font-medium ${
                            server.status === 'online'
                              ? 'text-green-600'
                              : server.status === 'offline'
                                ? 'text-red-600'
                                : 'text-gray-600'
                          }`}
                        >
                          {server.status}
                        </span>
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                        {server.cpu_percent !== undefined
                          ? `${server.cpu_percent.toFixed(1)}%`
                          : 'N/A'}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                        {memPct !== null
                          ? `${memPct.toFixed(1)}%`
                          : server.memory_used !== undefined &&
                              server.memory_total !== undefined
                            ? `${(server.memory_used / 1e9).toFixed(1)} / ${(server.memory_total / 1e9).toFixed(1)} GB`
                            : 'N/A'}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                        {server.load_1 !== undefined
                          ? server.load_1.toFixed(2)
                          : 'N/A'}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-500">
                        {server.last_seen_at || 'Never'}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
