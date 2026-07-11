"use client";

import { useCallback, useEffect, useSyncExternalStore } from "react";

export interface CachedServerRow {
  id: string;
  name: string;
  status: string;
}

export interface CachedServerListSnapshot {
  cachedAt: number;
  servers: CachedServerRow[];
}

interface StoredServerListSnapshot extends CachedServerListSnapshot {
  version: 1;
  scope: string;
}

const SERVER_CACHE_EVENT = "xlstatus:server-cache-change";
const SERVER_CACHE_KEY_PREFIX = "xlstatus:servers:v1:";
const SERVER_CACHE_MAX_AGE_MS = 10 * 60_000;
const SERVER_CACHE_MAX_ROWS = 500;
const SERVER_CACHE_MAX_JSON_LENGTH = 2 * 1024 * 1024;
const EMPTY_SERVER_CACHE_SNAPSHOT = null;

const memorySnapshots = new Map<string, CachedServerListSnapshot>();
const hydratedScopes = new Set<string>();
let cacheGeneration = 0;

function storageKey(scope: string): string {
  return `${SERVER_CACHE_KEY_PREFIX}${encodeURIComponent(scope)}`;
}

function isServerRow(value: unknown): value is CachedServerRow {
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

function isFresh(cachedAt: number): boolean {
  const now = Date.now();
  return Number.isFinite(cachedAt)
    && cachedAt > 0
    && cachedAt <= now + 30_000
    && now - cachedAt <= SERVER_CACHE_MAX_AGE_MS;
}

function removeStoredSnapshot(scope: string): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.removeItem(storageKey(scope));
  } catch {
    // Ignore unavailable browser storage.
  }
}

function parseSnapshot(raw: string, scope: string): CachedServerListSnapshot | null {
  if (!raw || raw.length > SERVER_CACHE_MAX_JSON_LENGTH) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<StoredServerListSnapshot>;
    if (
      parsed.version !== 1
      || parsed.scope !== scope
      || typeof parsed.cachedAt !== "number"
      || !isFresh(parsed.cachedAt)
      || !Array.isArray(parsed.servers)
      || parsed.servers.length > SERVER_CACHE_MAX_ROWS
      || !parsed.servers.every(isServerRow)
    ) {
      return null;
    }
    return {
      cachedAt: parsed.cachedAt,
      servers: parsed.servers,
    };
  } catch {
    return null;
  }
}

function notifyServerCacheChange(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(SERVER_CACHE_EVENT));
}

function subscribeServerCache(callback: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  window.addEventListener(SERVER_CACHE_EVENT, callback);
  return () => window.removeEventListener(SERVER_CACHE_EVENT, callback);
}

export function readCachedServerList(scope: string | null): CachedServerListSnapshot | null {
  if (!scope || typeof window === "undefined") return null;

  const memorySnapshot = memorySnapshots.get(scope);
  if (memorySnapshot && isFresh(memorySnapshot.cachedAt)) return memorySnapshot;
  if (memorySnapshot) {
    memorySnapshots.delete(scope);
    removeStoredSnapshot(scope);
  }

  if (hydratedScopes.has(scope)) return null;
  hydratedScopes.add(scope);

  try {
    const key = storageKey(scope);
    const raw = window.sessionStorage.getItem(key);
    if (!raw) return null;
    const snapshot = parseSnapshot(raw, scope);
    if (!snapshot) {
      window.sessionStorage.removeItem(key);
      return null;
    }
    memorySnapshots.set(scope, snapshot);
    return snapshot;
  } catch {
    return null;
  }
}

export function getServerCacheGeneration(): number {
  return cacheGeneration;
}

export function writeCachedServerList(
  scope: string,
  servers: CachedServerRow[],
  expectedGeneration = cacheGeneration,
): void {
  if (typeof window === "undefined") return;
  if (expectedGeneration !== cacheGeneration) return;
  if (servers.length > SERVER_CACHE_MAX_ROWS || !servers.every(isServerRow)) return;

  const cachedAt = Date.now();
  const snapshot: CachedServerListSnapshot = { cachedAt, servers };
  const stored: StoredServerListSnapshot = {
    version: 1,
    scope,
    cachedAt,
    servers,
  };

  memorySnapshots.set(scope, snapshot);
  hydratedScopes.add(scope);
  try {
    const raw = JSON.stringify(stored);
    if (raw.length <= SERVER_CACHE_MAX_JSON_LENGTH) {
      window.sessionStorage.setItem(storageKey(scope), raw);
    }
  } catch {
    // Memory caching still works when storage is unavailable or full.
  }
  notifyServerCacheChange();
}

export function clearCachedServerList(scope: string, invalidatePendingWrites = true): void {
  if (invalidatePendingWrites) cacheGeneration += 1;
  memorySnapshots.delete(scope);
  hydratedScopes.delete(scope);
  removeStoredSnapshot(scope);
  notifyServerCacheChange();
}

export function clearAllCachedServerLists(): void {
  cacheGeneration += 1;
  memorySnapshots.clear();
  hydratedScopes.clear();
  if (typeof window !== "undefined") {
    try {
      for (let index = window.sessionStorage.length - 1; index >= 0; index -= 1) {
        const key = window.sessionStorage.key(index);
        if (key?.startsWith(SERVER_CACHE_KEY_PREFIX)) {
          window.sessionStorage.removeItem(key);
        }
      }
    } catch {
      // Ignore unavailable browser storage.
    }
  }
  notifyServerCacheChange();
}

export function useCachedServerList(scope: string | null): CachedServerListSnapshot | null {
  const getSnapshot = useCallback(() => readCachedServerList(scope), [scope]);
  const snapshot = useSyncExternalStore(
    subscribeServerCache,
    getSnapshot,
    () => EMPTY_SERVER_CACHE_SNAPSHOT,
  );
  useEffect(() => {
    if (!scope || !snapshot) return;
    const remaining = Math.max(0, snapshot.cachedAt + SERVER_CACHE_MAX_AGE_MS - Date.now());
    const timer = window.setTimeout(() => clearCachedServerList(scope, false), remaining + 1);
    return () => window.clearTimeout(timer);
  }, [scope, snapshot]);
  return snapshot;
}
