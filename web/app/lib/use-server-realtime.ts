"use client";

import { useEffect, useRef, useState } from "react";
import type { LiveState, ServerConnectionState } from "@/app/components/ServerDataProvider";
import { optionalNumber } from "@/app/lib/format";
import { buildWebSocketUrl } from "@/lib/api";

interface ServerRealtimeState {
  live: Record<string, LiveState>;
  connection: ServerConnectionState;
  now: number;
}

interface PendingLiveEvent {
  event: Record<string, unknown>;
  snapshot: boolean;
}

interface ScopedLiveState {
  scope: string;
  values: Record<string, LiveState>;
}

interface ScopedConnectionState {
  scope: string;
  value: ServerConnectionState;
}

const EMPTY_LIVE_STATE: Record<string, LiveState> = {};

export function useServerRealtime(scope: string | null, revision: number): ServerRealtimeState {
  const [scopedLive, setScopedLive] = useState<ScopedLiveState | null>(null);
  const live = scopedLive?.scope === scope ? scopedLive.values : EMPTY_LIVE_STATE;
  const [scopedConnection, setScopedConnection] = useState<ScopedConnectionState | null>(null);
  const connection = scopedConnection?.scope === scope ? scopedConnection.value : "closed";
  const [now, setNow] = useState(() => Date.now());
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimerRef = useRef<number | null>(null);
  const pendingLiveRef = useRef<PendingLiveEvent[]>([]);
  const liveRafRef = useRef<number | null>(null);

  useEffect(() => {
    let timer: number | null = null;

    function startTicking() {
      if (timer !== null) return;
      timer = window.setInterval(() => setNow(Date.now()), 5_000);
    }

    function stopTicking() {
      if (timer === null) return;
      window.clearInterval(timer);
      timer = null;
    }

    function handleVisibility() {
      // A hidden tab renders nothing, so the clock only burns wakeups there.
      if (document.hidden) {
        stopTicking();
        return;
      }
      setNow(Date.now());
      startTicking();
    }

    if (!document.hidden) startTicking();
    document.addEventListener("visibilitychange", handleVisibility);
    return () => {
      stopTicking();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  useEffect(() => {
    if (!scope) return;
    const activeScope = scope;
    let cancelled = false;
    let backoff = 1000;

    function flushLive() {
      liveRafRef.current = null;
      if (cancelled) return;
      const events = pendingLiveRef.current;
      if (events.length === 0) return;
      pendingLiveRef.current = [];
      setScopedLive((currentScope) => {
        const previous = currentScope?.scope === activeScope ? currentScope.values : EMPTY_LIVE_STATE;
        let next = previous;
        for (const item of events) {
          const event = item.event;
          if (event.kind !== "host_state" || typeof event.agent_id !== "string") continue;
          const current = next[event.agent_id];
          const normalized = normalizeLiveState(
            event.payload,
            event.received_at,
            current,
            item.snapshot,
          );
          if (normalized === current) continue;
          if (next === previous) next = { ...previous };
          next[event.agent_id] = normalized;
        }
        if (next === previous && currentScope?.scope === activeScope) return currentScope;
        return { scope: activeScope, values: next };
      });
    }

    function scheduleFlush() {
      if (liveRafRef.current !== null || cancelled) return;
      liveRafRef.current = window.requestAnimationFrame(flushLive);
    }

    function teardown() {
      if (reconnectTimerRef.current !== null) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
      if (liveRafRef.current !== null) {
        window.cancelAnimationFrame(liveRafRef.current);
        liveRafRef.current = null;
      }
      pendingLiveRef.current = [];
      const socket = wsRef.current;
      wsRef.current = null;
      if (!socket) return;
      // Detach handlers before closing so this close cannot schedule a reconnect.
      socket.onopen = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.close();
    }

    function connect() {
      if (cancelled || document.hidden) return;
      setScopedConnection({ scope: activeScope, value: "connecting" });
      const ws = new WebSocket(buildWebSocketUrl("/ws/servers"));
      wsRef.current = ws;
      ws.onopen = () => {
        if (cancelled) return;
        setScopedConnection({ scope: activeScope, value: "open" });
        backoff = 1000;
      };
      ws.onmessage = (event) => {
        if (cancelled) return;
        try {
          const message = JSON.parse(event.data as string) as Record<string, unknown>;
          const snapshot = message.type === "snapshot";
          const events = snapshot && Array.isArray(message.events)
            ? message.events
            : message.type === "event"
              ? [message.event]
              : [];
          for (const raw of events) {
            if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
            pendingLiveRef.current.push({
              event: raw as Record<string, unknown>,
              snapshot,
            });
          }
          if (pendingLiveRef.current.length > 0) scheduleFlush();
        } catch {
          // Ignore malformed live frames.
        }
      };
      ws.onerror = () => {
        if (!cancelled) setScopedConnection({ scope: activeScope, value: "error" });
      };
      ws.onclose = () => {
        if (cancelled) return;
        setScopedConnection({ scope: activeScope, value: "closed" });
        if (document.hidden) return;
        backoff = Math.min(backoff * 2, 15_000);
        reconnectTimerRef.current = window.setTimeout(connect, backoff);
      };
    }

    function handleVisibility() {
      if (cancelled) return;
      if (document.hidden) {
        teardown();
        setScopedConnection({ scope: activeScope, value: "closed" });
        return;
      }
      // A socket or a pending reconnect already covers the visible tab.
      if (wsRef.current !== null || reconnectTimerRef.current !== null) return;
      backoff = 1000;
      connect();
    }

    document.addEventListener("visibilitychange", handleVisibility);
    if (document.hidden) {
      setScopedConnection({ scope: activeScope, value: "closed" });
    } else {
      connect();
    }
    return () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", handleVisibility);
      teardown();
    };
  }, [revision, scope]);

  return { live, connection, now };
}

function normalizeLiveState(
  payload: unknown,
  receivedAtValue: unknown,
  previous: LiveState | undefined,
  snapshot: boolean,
): LiveState {
  const parsedReceivedAt = typeof receivedAtValue === "string"
    ? new Date(receivedAtValue).getTime()
    : Number.NaN;
  const receivedAt = Number.isFinite(parsedReceivedAt)
    ? new Date(parsedReceivedAt).toISOString()
    : new Date().toISOString();
  const previousAt = previous ? new Date(previous.received_at).getTime() : Number.NaN;
  if (previous && Number.isFinite(previousAt) && parsedReceivedAt <= previousAt) return previous;

  const state = asRecord(payload);
  const netInTotal = optionalNumber(state.network_in_total) ?? netIoTotal(state, "bytes_recv");
  const netOutTotal = optionalNumber(state.network_out_total) ?? netIoTotal(state, "bytes_sent");
  return {
    cpu_percent: optionalNumber(state.cpu_percent),
    memory_used: optionalNumber(state.memory_used),
    memory_total: optionalNumber(state.memory_total),
    load_1: optionalNumber(state.load_1),
    net_rx_bps: optionalNumber(state.net_rx_bps)
      ?? rateFromTotalDelta(netInTotal, previous?.network_in_total, receivedAt, previous?.received_at),
    net_tx_bps: optionalNumber(state.net_tx_bps)
      ?? rateFromTotalDelta(netOutTotal, previous?.network_out_total, receivedAt, previous?.received_at),
    network_in_total: netInTotal,
    network_out_total: netOutTotal,
    uptime_seconds: optionalNumber(state.uptime_seconds),
    received_at: receivedAt,
    confirmed_at: snapshot ? previous?.confirmed_at : Date.now(),
  };
}

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function netIoTotal(state: Record<string, unknown>, field: "bytes_recv" | "bytes_sent"): number | undefined {
  const netIo = Array.isArray(state.net_io)
    ? state.net_io
    : Array.isArray(state.network_interfaces)
      ? state.network_interfaces
      : [];
  if (netIo.length === 0) return undefined;
  return netIo.reduce(
    (sum, item) => sum + (optionalNumber(asRecord(item)[field]) ?? 0),
    0,
  );
}

function rateFromTotalDelta(
  nextTotal: number | undefined,
  previousTotal: number | undefined,
  nextReceivedAt: string,
  previousReceivedAt?: string,
): number | undefined {
  if (nextTotal === undefined || previousTotal === undefined || !previousReceivedAt) return undefined;
  const elapsed = (new Date(nextReceivedAt).getTime() - new Date(previousReceivedAt).getTime()) / 1000;
  if (!Number.isFinite(elapsed) || elapsed <= 0 || nextTotal < previousTotal) return undefined;
  return (nextTotal - previousTotal) / elapsed;
}
