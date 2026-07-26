"use client";

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { responseError } from "@/app/components/M7Primitives";
import { readStoredUser, useStoredUser } from "@/app/lib/client-store";
import {
  clearCachedServerList,
  getServerCacheGeneration,
  useCachedServerList,
  writeCachedServerList,
} from "@/app/lib/server-cache";
import { getCookie } from "@/app/lib/format";
import { apiClient, getApiBaseUrl } from "@/lib/api";

export interface ServerLocation {
  source?: string | null;
  provider?: string | null;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  timezone?: string | null;
}

export interface Server {
  id: string;
  name: string;
  remark?: string | null;
  note?: string | null;
  expires_at?: string | null;
  expired_at?: string | null;
  renewal_price?: string | number | null;
  price?: string | number | null;
  provider?: string | null;
  region?: string | null;
  country?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  location?: ServerLocation | null;
  plan?: string | null;
  tags?: string[];
  accent_color?: string | null;
  dashboard_visible?: boolean | null;
  display_order?: number | null;
  status: string;
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  net_rx_bps?: number | null;
  net_tx_bps?: number | null;
  network_in_total?: number | null;
  network_out_total?: number | null;
  uptime_seconds?: number | null;
  last_seen_at?: string;
  last_event_at?: string;
}

export interface LiveState {
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  net_rx_bps?: number;
  net_tx_bps?: number;
  network_in_total?: number;
  network_out_total?: number;
  uptime_seconds?: number;
  received_at: string;
  confirmed_at?: number;
}

export type ServerConnectionState = "connecting" | "open" | "closed" | "error";

interface ServerDataContextValue {
  servers: Server[];
  observedAt: number | null;
  loading: boolean;
  error: string | null;
  realtimeScope: string | null;
  realtimeRevision: number;
  refresh: (force?: boolean) => Promise<void>;
  revalidate: () => Promise<void>;
}

interface ScopedServers {
  scope: string;
  servers: Server[];
  observedAt: number;
}

interface ScopedError {
  scope: string;
  message: string;
}

interface InFlightRevalidation {
  scope: string;
  request: Promise<void>;
}

const ServerDataContext = createContext<ServerDataContextValue | null>(null);
const EMPTY_SERVERS: Server[] = [];
const SERVER_INITIAL_PAGE_SIZE = 50;
const SERVER_MAX_ROWS = 500;

export function ServerDataProvider({ children }: { children: ReactNode }) {
  const user = useStoredUser();
  const csrfToken = getCookie("xlstatus_csrf");
  const dataScope = buildDataScope(user?.id, csrfToken);
  const cacheScope = csrfToken ? dataScope : null;
  const cachedSnapshot = useCachedServerList(cacheScope);
  const cachedServers = useMemo(
    () => cachedSnapshot ? (cachedSnapshot.servers as Server[]).filter(isServer) : EMPTY_SERVERS,
    [cachedSnapshot],
  );
  const [freshSnapshot, setFreshSnapshot] = useState<ScopedServers | null>(null);
  const freshServers = freshSnapshot?.scope === dataScope ? freshSnapshot.servers : null;
  const [settledScope, setSettledScope] = useState<string | null>(null);
  const hasSettled = Boolean(dataScope && settledScope === dataScope);
  const [scopedError, setScopedError] = useState<ScopedError | null>(null);
  const error = scopedError?.scope === dataScope ? scopedError.message : null;
  const [realtimeRevision, setRealtimeRevision] = useState(0);
  const requestIdRef = useRef(0);
  const inFlightRevalidationRef = useRef<InFlightRevalidation | null>(null);
  const requestAbortRef = useRef<AbortController | null>(null);
  const hasVisibleSnapshotRef = useRef(freshServers !== null || cachedSnapshot !== null);

  useEffect(() => {
    hasVisibleSnapshotRef.current = freshServers !== null || cachedSnapshot !== null;
  }, [cachedSnapshot, freshServers]);

  const loadServers = useCallback(async (force = false, rotateRealtime = false) => {
    if (!dataScope) return;
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    requestAbortRef.current?.abort();
    const controller = new AbortController();
    requestAbortRef.current = controller;
    const cacheGeneration = getServerCacheGeneration();
    const publishFirstPage = !hasVisibleSnapshotRef.current;
    setSettledScope(null);
    setScopedError(null);

    const response = await apiClient.listServers(SERVER_INITIAL_PAGE_SIZE, 0, false, {
      signal: controller.signal,
      bypassClientCache: force,
    });
    if (
      controller.signal.aborted
      || requestId !== requestIdRef.current
      || currentDataScope() !== dataScope
    ) {
      return;
    }
    if (!response.success || !response.data) {
      if (requestAbortRef.current === controller) requestAbortRef.current = null;
      setSettledScope(dataScope);
      if (response.status === 401 || response.status === 403) {
        if (cacheScope) clearCachedServerList(cacheScope);
        setFreshSnapshot(null);
      }
      setScopedError({ scope: dataScope, message: responseError(response) });
      return;
    }

    const firstRows = Array.isArray(response.data.servers) ? response.data.servers : [];
    const firstServers = firstRows.filter(isServer);
    if (publishFirstPage) {
      setFreshSnapshot({ scope: dataScope, servers: firstServers, observedAt: Date.now() });
    }

    const reportedTotal = typeof response.data.total === "number" && Number.isFinite(response.data.total)
      ? Math.max(0, Math.floor(response.data.total))
      : firstServers.length;
    const cappedTotal = Math.min(reportedTotal, SERVER_MAX_ROWS);
    let nextServers = firstServers;
    if (cappedTotal > SERVER_INITIAL_PAGE_SIZE) {
      const remainder = await apiClient.listServers(
        cappedTotal - SERVER_INITIAL_PAGE_SIZE,
        SERVER_INITIAL_PAGE_SIZE,
        false,
        {
          signal: controller.signal,
          bypassClientCache: force,
        },
      );
      if (
        controller.signal.aborted
        || requestId !== requestIdRef.current
        || currentDataScope() !== dataScope
      ) {
        return;
      }
      if (!remainder.success || !remainder.data) {
        if (requestAbortRef.current === controller) requestAbortRef.current = null;
        setSettledScope(dataScope);
        setScopedError({ scope: dataScope, message: responseError(remainder) });
        return;
      }
      const remainderRows = Array.isArray(remainder.data.servers) ? remainder.data.servers : [];
      nextServers = [...firstServers, ...remainderRows.filter(isServer)];
    }

    if (requestAbortRef.current === controller) requestAbortRef.current = null;
    setSettledScope(dataScope);
    setFreshSnapshot({ scope: dataScope, servers: nextServers, observedAt: Date.now() });
    if (rotateRealtime) setRealtimeRevision((current) => current + 1);
    if (cacheScope) writeCachedServerList(cacheScope, nextServers, cacheGeneration);
  }, [cacheScope, dataScope]);

  const refresh = useCallback(
    (force = false) => loadServers(force, true),
    [loadServers],
  );

  const revalidate = useCallback(() => {
    if (!dataScope) return Promise.resolve();
    if (inFlightRevalidationRef.current?.scope === dataScope) {
      return inFlightRevalidationRef.current.request;
    }
    const request = loadServers(false, false);
    const inFlight = { scope: dataScope, request };
    inFlightRevalidationRef.current = inFlight;
    void request.finally(() => {
      if (inFlightRevalidationRef.current === inFlight) {
        inFlightRevalidationRef.current = null;
      }
    });
    return request;
  }, [dataScope, loadServers]);

  useEffect(() => {
    if (!dataScope) return;
    void revalidate();
    return () => {
      requestAbortRef.current?.abort();
      requestAbortRef.current = null;
      inFlightRevalidationRef.current = null;
      requestIdRef.current += 1;
    };
  }, [dataScope, revalidate]);

  const servers = freshServers ?? cachedServers;
  const observedAt = freshSnapshot?.scope === dataScope
    ? freshSnapshot.observedAt
    : cachedSnapshot?.cachedAt ?? null;
  const value = useMemo<ServerDataContextValue>(() => ({
    servers,
    observedAt,
    loading: freshServers === null && cachedSnapshot === null && !hasSettled,
    error,
    realtimeScope: cacheScope,
    realtimeRevision,
    refresh,
    revalidate,
  }), [cacheScope, cachedSnapshot, error, freshServers, hasSettled, observedAt, realtimeRevision, refresh, revalidate, servers]);

  return <ServerDataContext.Provider value={value}>{children}</ServerDataContext.Provider>;
}

export function useServerData(): ServerDataContextValue {
  const value = useContext(ServerDataContext);
  if (!value) throw new Error("useServerData must be used inside ServerDataProvider");
  return value;
}

export function useOptionalServerData(): ServerDataContextValue | null {
  return useContext(ServerDataContext);
}

function isServer(value: unknown): value is Server {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const row = value as Record<string, unknown>;
  const tagsValid = row.tags === undefined
    || row.tags === null
    || (Array.isArray(row.tags) && row.tags.every((tag) => typeof tag === "string"));
  return tagsValid
    && typeof row.id === "string"
    && typeof row.name === "string"
    && typeof row.status === "string";
}

function sessionFingerprint(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(36);
}

function buildDataScope(userId: string | undefined, csrfToken: string | null): string | null {
  if (!userId) return null;
  const sessionScope = csrfToken ? sessionFingerprint(csrfToken) : "no-session";
  return `${getApiBaseUrl()}|${userId}|${sessionScope}`;
}

function currentDataScope(): string | null {
  return buildDataScope(readStoredUser()?.id, getCookie("xlstatus_csrf"));
}
