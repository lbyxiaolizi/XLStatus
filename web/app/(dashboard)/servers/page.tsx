"use client";

import Link from "next/link";
import dynamic from "next/dynamic";
import type { ReactNode } from "react";
import { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  type LiveState,
  type Server,
  type ServerConnectionState,
  useServerData,
} from "@/app/components/ServerDataProvider";
import {
  asRecord,
  asString,
  BrutalCard,
  EmptyState,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  Skeleton,
  SkeletonCard,
  StatusBadge,
  buttonClass,
  compactId,
  formatBytes,
  formatDate,
  formatMs,
  formatPercent,
  isAdmin,
  inputClass,
  responseError,
  selectClass,
  useStoredUser,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import { apiClient, type JsonObject, type ServerGroup, type ServerOwnerTransfer } from "@/lib/api";
import { clampPercent, durationLabel, formatRate, optionalNumber } from "@/app/lib/format";
import { sanitizeCssVariableValue } from "@/app/lib/security";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";
import { useStoredPreference } from "@/app/lib/client-store";
import { mapWithConcurrency } from "@/app/lib/concurrency";
import { useBusy } from "@/app/lib/use-busy";
import { useServerRealtime } from "@/app/lib/use-server-realtime";

const WorldServerMap = dynamic(
  () => import("@/app/components/WorldServerMap").then((module) => module.WorldServerMap),
  {
    ssr: false,
    loading: () => <WorldMapSkeleton />,
  },
);

type Copy = Translations;

interface ServiceSummary {
  id: string;
  name: string;
  server_id?: string | null | undefined;
  server_ids?: string[] | undefined;
}

interface ServiceResult {
  service_id: string;
  server_id?: string | null | undefined;
  status: string;
  delay_ms?: number | null | undefined;
  created_at: string;
}

interface ServiceHistoryEntry {
  service: ServiceSummary;
  results: ServiceResult[];
}

interface ServiceTrackerItem {
  id: string;
  name: string;
  uptime: number;
  avgDelay?: number | undefined;
  days: ServiceDay[];
}

interface ServiceDay {
  key: string;
  label: string;
  uptime: number;
  avgDelay?: number | undefined;
  total: number;
}

interface ServerSummary {
  total: number;
  online: number;
  offline: number;
  uploadSpeed: number;
  downloadSpeed: number;
  totalUpload: number;
  totalDownload: number;
}

type ServerStatusFilter = "all" | "online" | "offline" | "other";
type ServerViewMode = "cards" | "compact";
type ServerSortKey =
  | "default"
  | "name"
  | "status"
  | "cpu"
  | "memory"
  | "load"
  | "uptime"
  | "upload"
  | "download"
  | "totalUpload"
  | "totalDownload";
type SortOrder = "asc" | "desc";
const SERVER_VIEW_MODES = ["cards", "compact"] as const;
const BINARY_PREFERENCE_VALUES = ["0", "1"] as const;
const SERVICE_TRACKER_HISTORY_LIMIT = 1200;

export default function ServersPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const storedUser = useStoredUser();
  const canManageTransfers = isAdmin(storedUser);
  const {
    servers,
    observedAt: serversObservedAt,
    loading,
    error: serverLoadError,
    realtimeScope,
    realtimeRevision,
    refresh: loadServers,
    revalidate: revalidateServers,
  } = useServerData();
  const { live, connection: conn, now: realtimeNow } = useServerRealtime(realtimeScope, realtimeRevision);
  const [showWsBanner, setShowWsBanner] = useState(false);
  const wsSawOpenRef = useRef(false);
  const [batchCollapsed, setBatchCollapsed] = useState(false);
  useEffect(() => {
    if (conn === "open") {
      wsSawOpenRef.current = true;
      // eslint-disable-next-line react-hooks/set-state-in-effect -- reset must track the external socket state synchronously or the banner lingers one frame into a live connection
      setShowWsBanner(false);
      return;
    }
    if (!wsSawOpenRef.current || (conn !== "closed" && conn !== "error")) {
      setShowWsBanner(false);
      return;
    }
    const timer = window.setTimeout(() => {
      if (document.visibilityState === "visible") setShowWsBanner(true);
    }, 1500);
    return () => window.clearTimeout(timer);
  }, [conn]);
  const [groupBusy, runGroup] = useBusy();
  const [batchBusy, runBatch] = useBusy();
  const [transferBusy, runTransfer] = useBusy();
  const [services, setServices] = useState<ServiceSummary[]>([]);
  const [serviceTrackers, setServiceTrackers] = useState<ServiceTrackerItem[]>([]);
  const [ownerTransfers, setOwnerTransfers] = useState<ServerOwnerTransfer[]>([]);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [transfersLoading, setTransfersLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const [transferError, setTransferError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<ServerStatusFilter>("all");
  const [groupFilter, setGroupFilter] = useState("all");
  const [viewMode, setViewMode] = useStoredPreference<ServerViewMode>(
    "xlstatus_server_view",
    "cards",
    SERVER_VIEW_MODES,
  );
  const [showServicesPreference, setShowServicesPreference] = useStoredPreference(
    "xlstatus_show_services",
    "0",
    BINARY_PREFERENCE_VALUES,
  );
  const [showMapPreference, setShowMapPreference] = useStoredPreference(
    "xlstatus_show_server_map",
    "0",
    BINARY_PREFERENCE_VALUES,
  );
  const showServices = showServicesPreference === "1";
  const showMap = showMapPreference === "1";
  const [sortKey, setSortKey] = useState<ServerSortKey>("default");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [batchTags, setBatchTags] = useState("");
  const [batchOwnerUserId, setBatchOwnerUserId] = useState("");
  const [serverGroups, setServerGroups] = useState<ServerGroup[]>([]);
  const [newServerGroupName, setNewServerGroupName] = useState("");
  const [selectedServerGroupId, setSelectedServerGroupId] = useState("");
  const [serverGroupEdit, setServerGroupEdit] = useState({ name: "", color: "#2563eb", displayOrder: "" });
  const [groupsDrawerOpen, setGroupsDrawerOpen] = useState(false);
  const [transfersDrawerOpen, setTransfersDrawerOpen] = useState(false);
  const servicesLoadedRef = useRef(false);
  const ownerTransfersLoadedRef = useRef(false);
  const serverGroupsLoadRequestIdRef = useRef(0);
  const servicesPending = showServices && !servicesLoadedRef.current && !serviceError;

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  const loadServerGroups = useCallback(async (force = false) => {
    const requestId = serverGroupsLoadRequestIdRef.current + 1;
    serverGroupsLoadRequestIdRef.current = requestId;
    const response = await apiClient.listServerGroups({ bypassClientCache: force });
    if (requestId !== serverGroupsLoadRequestIdRef.current) return;
    if (response.success && response.data) {
      setServerGroups(response.data.groups ?? []);
    }
  }, []);

  useEffect(() => {
    if (loading) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServerGroups();
    return () => {
      serverGroupsLoadRequestIdRef.current += 1;
    };
  }, [loadServerGroups, loading]);

  const loadOwnerTransfers = useCallback(async () => {
    if (!canManageTransfers) {
      setOwnerTransfers([]);
      ownerTransfersLoadedRef.current = false;
      return;
    }
    setTransfersLoading(true);
    setTransferError(null);
    const response = await apiClient.listServerOwnerTransfers(20, 0);
    setTransfersLoading(false);
    if (response.success && response.data) {
      setOwnerTransfers(response.data.transfers ?? []);
      ownerTransfersLoadedRef.current = true;
    } else {
      setTransferError(responseError(response));
    }
  }, [canManageTransfers]);

  const loadServices = useCallback(async (signal?: AbortSignal) => {
    setServicesLoading(true);
    setServiceError(null);
    const response = await apiClient.listServices(200, 0, false, {
      ...(signal ? { signal } : {}),
      includeHistory: true,
      historyLimit: SERVICE_TRACKER_HISTORY_LIMIT,
    });
    if (signal?.aborted) return;
    if (!response.success || !response.data) {
      setServices([]);
      setServiceTrackers([]);
      setServicesLoading(false);
      setServiceError(responseError(response));
      return;
    }

    const nextServices = ((response.data["services"] as ServiceSummary[]) ?? []).filter((service) => service.id);
    setServices(nextServices);
    const embedded = readEmbeddedHistories(nextServices);
    const histories: (ServiceHistoryEntry | null)[] =
      embedded ??
      (await mapWithConcurrency(nextServices, 6, async (service) => {
        if (signal?.aborted) return null;
        const history = await apiClient.getServiceHistory(
          service.id,
          SERVICE_TRACKER_HISTORY_LIMIT,
          signal ? { signal } : {},
        );
        if (!history.success || !history.data) return null;
        const results = Array.isArray(history.data["results"]) ? history.data["results"] : [];
        return {
          service,
          results: results
            .map((result) => normalizeServiceResult(result))
            .filter((result): result is ServiceResult => Boolean(result)),
        };
      }));
    if (signal?.aborted) return;

    setServiceTrackers(
      histories
        .filter((item): item is { service: ServiceSummary; results: ServiceResult[] } => Boolean(item))
        .map(({ service, results }) => buildServiceTracker(service, results)),
    );
    servicesLoadedRef.current = true;
    setServicesLoading(false);
  }, []);

  useEffect(() => {
    if (!showServices || servicesLoadedRef.current) return;
    const controller = new AbortController();
    void loadServices(controller.signal);
    return () => controller.abort();
  }, [loadServices, showServices]);

  const mergedCacheRef = useRef<
    Map<string, { server: Server; state: LiveState | undefined; status: string; merged: Server }>
  >(new Map());
  const merged = useMemo(() => {
    const cache = mergedCacheRef.current;
    const nextIds = new Set<string>();
    const result = servers.map((server) => {
      const state = live[server.id];
      const status = currentServerStatus(server, state, realtimeNow, serversObservedAt);
      nextIds.add(server.id);
      const cached = cache.get(server.id);
      if (cached && cached.server === server && cached.state === state && cached.status === status) {
        return cached.merged;
      }
      const mergedServer: Server = {
        ...server,
        status,
        cpu_percent: state?.cpu_percent ?? server.cpu_percent,
        memory_used: state?.memory_used ?? server.memory_used,
        memory_total: state?.memory_total ?? server.memory_total,
        load_1: state?.load_1 ?? server.load_1,
        net_rx_bps: state?.net_rx_bps ?? server.net_rx_bps,
        net_tx_bps: state?.net_tx_bps ?? server.net_tx_bps,
        network_in_total: state?.network_in_total ?? server.network_in_total,
        network_out_total: state?.network_out_total ?? server.network_out_total,
        uptime_seconds: state?.uptime_seconds ?? server.uptime_seconds,
        last_event_at: state?.received_at ?? server.last_event_at,
      };
      cache.set(server.id, { server, state, status, merged: mergedServer });
      return mergedServer;
    });
    for (const id of cache.keys()) {
      if (!nextIds.has(id)) cache.delete(id);
    }
    return result;
  }, [live, realtimeNow, servers, serversObservedAt]);

  const tagGroups = useMemo(() => buildServerGroups(merged), [merged]);
  const groupMembership = useMemo(
    () =>
      new Map(
        serverGroups.map((group) => [
          group.id,
          new Set((group.server_ids ?? []).filter(Boolean)),
        ]),
      ),
    [serverGroups],
  );
  const summary = useMemo(() => buildServerSummary(merged), [merged]);
  const deferredQuery = useDeferredValue(query);
  const filtered = useMemo(
    () =>
      merged
        .filter((server) => serverMatchesQuery(server, deferredQuery))
        .filter((server) => statusFilter === "all" || serverStatusGroup(server.status) === statusFilter)
        .filter((server) => serverMatchesGroupFilter(server, groupFilter, groupMembership))
        .sort((a, b) => compareServers(a, b, sortKey, sortOrder)),
    [groupFilter, groupMembership, merged, deferredQuery, sortKey, sortOrder, statusFilter],
  );
  const selectedSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const filteredIds = useMemo(() => filtered.map((server) => server.id), [filtered]);
  const allFilteredSelected = filteredIds.length > 0 && filteredIds.every((id) => selectedSet.has(id));
  const visibleServiceTrackers = useMemo(() => {
    const visibleIds = new Set(filtered.map((server) => server.id));
    return serviceTrackers.filter((tracker) => {
      const service = services.find((item) => item.id === tracker.id);
      return !service || serviceBelongsToVisibleServer(service, visibleIds);
    });
  }, [filtered, serviceTrackers, services]);

  function changeViewMode(next: ServerViewMode) {
    setViewMode(next);
  }

  function toggleServices() {
    if (!showServices && !servicesLoadedRef.current) {
      setServiceError(null);
      setServicesLoading(true);
    }
    setShowServicesPreference(showServices ? "0" : "1");
  }

  function toggleMap() {
    setShowMapPreference(showMap ? "0" : "1");
  }

  const toggleServerSelection = useCallback((id: string) => {
    setSelectedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    );
  }, []);

  function serverGroupDraft(group?: ServerGroup) {
    return {
      name: group?.name ?? "",
      color: group?.color || "#2563eb",
      displayOrder: group?.display_order == null ? "" : String(group.display_order),
    };
  }

  function selectServerGroup(id: string) {
    setSelectedServerGroupId(id);
    setServerGroupEdit(serverGroupDraft(serverGroups.find((group) => group.id === id)));
  }

  function toggleFilteredSelection() {
    setSelectedIds((current) => {
      if (allFilteredSelected) {
        return current.filter((id) => !filteredIds.includes(id));
      }
      return Array.from(new Set([...current, ...filteredIds]));
    });
  }

  async function runBatchAction(action: string, extra: JsonObject = {}) {
    if (selectedIds.length === 0) return false;
    setError(null);
    const totpCode = await sensitiveTotpCodeForAction(
      action === "delete" ||
        action === "transfer_owner" ||
        action === "set_dashboard_visible" ||
        action === "move_group" ||
        action === "set_tags" ||
        action === "add_tags" ||
        action === "remove_tags",
    );
    if (totpCode === null) return false;
    const payload: JsonObject = {
      server_ids: selectedIds,
      action,
      ...extra,
    };
    const response = await apiClient.batchUpdateServers(payload, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return false;
    }
    const data = response.data as { updated?: number | undefined; failed?: number | undefined } | undefined;
    showToast(
      copy.serversPage.batchActionDone
        .replace("{updated}", String(data?.updated ?? selectedIds.length))
        .replace("{failed}", String(data?.failed ?? 0)),
    );
    await loadServers();
    if (action === "transfer_owner") {
      await loadOwnerTransfers();
    }
    return true;
  }

  async function sensitiveTotpCodeForAction(required: boolean): Promise<string | undefined | null> {
    if (!required) return undefined;
    const status = await apiClient.getTotpStatus();
    if (!status.success) {
      setError(responseError(status));
      return null;
    }
    if (!status.data?.enabled) return undefined;
    const code = await dialogs.totp();
    if (code === null) return null;
    return code;
  }

  async function createServerGroup() {
    const name = newServerGroupName.trim();
    if (!name) {
      setError(copy.serversPage.groupNameRequired);
      return;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.createServerGroup({ name }, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    setNewServerGroupName("");
    if (response.data) {
      setSelectedServerGroupId(response.data.id);
      setServerGroupEdit(serverGroupDraft(response.data));
    }
    showToast(copy.serversPage.groupCreated.replace("{name}", name));
    await loadServerGroups();
  }

  async function updateSelectedServerGroup() {
    if (!selectedServerGroupId) {
      setError(copy.serversPage.selectGroupToEdit);
      return;
    }
    const name = serverGroupEdit.name.trim();
    if (!name) {
      setError(copy.serversPage.groupNameRequired);
      return;
    }
    const rawColor = serverGroupEdit.color.trim();
    const safeColor = rawColor ? sanitizeCssVariableValue(rawColor) : null;
    if (rawColor && !safeColor) {
      setError("Invalid group color value");
      return;
    }
    const displayOrder = serverGroupEdit.displayOrder.trim();
    const payload: JsonObject = {
      name,
      color: safeColor ?? "",
    };
    if (displayOrder) {
      const parsed = Number.parseInt(displayOrder, 10);
      if (!Number.isFinite(parsed)) {
        setError(copy.serversPage.orderMustBeNumber);
        return;
      }
      payload["display_order"] = parsed;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.updateServerGroup(selectedServerGroupId, payload, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    showToast(copy.serversPage.groupUpdated.replace("{name}", name));
    await loadServerGroups();
  }

  async function addSelectedToServerGroup() {
    if (!selectedServerGroupId) {
      setError(copy.serversPage.selectTargetGroup);
      return;
    }
    if (!selectedIds.length) {
      setError(copy.serversPage.selectServersFirst);
      return;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.addServerGroupMembers(selectedServerGroupId, selectedIds, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    showToast(copy.serversPage.addedToGroup);
    await loadServerGroups();
  }

  async function moveSelectedToServerGroup() {
    if (!selectedServerGroupId) {
      setError(copy.serversPage.selectTargetGroup);
      return;
    }
    if (!selectedIds.length) {
      setError(copy.serversPage.selectServersFirst);
      return;
    }
    await runBatchAction("move_group", { group_id: selectedServerGroupId });
    await loadServerGroups();
  }

  async function deleteSelectedServerGroup() {
    if (!selectedServerGroupId) {
      setError(copy.serversPage.selectGroupToDelete);
      return;
    }
    const group = serverGroups.find((item) => item.id === selectedServerGroupId);
    if (
      !(await dialogs.confirm({
        message: copy.serversPage.deleteGroupConfirm.replace(
          "{name}",
          String(group?.name ?? selectedServerGroupId),
        ),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.deleteServerGroup(selectedServerGroupId, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    setSelectedServerGroupId("");
    setServerGroupEdit(serverGroupDraft());
    setGroupFilter("all");
    showToast(copy.serversPage.groupDeleted);
    await loadServerGroups();
  }

  async function deleteSelectedServers() {
    if (!selectedIds.length) {
      setError(copy.serversPage.selectServersFirst);
      return;
    }
    if (
      !(await dialogs.confirm({
        message: copy.serversPage.deleteServersConfirm.replace("{count}", String(selectedIds.length)),
        danger: true,
      }))
    ) {
      return;
    }
    const ok = await runBatchAction("delete");
    if (ok) {
      setSelectedIds([]);
      await loadServerGroups();
    }
  }

  function batchTagList(): string[] {
    return batchTags
      .split(/[,\n;，、]+/)
      .map((tag) => tag.trim())
      .filter(Boolean);
  }

  async function runTagBatch(action: "set_tags" | "add_tags" | "remove_tags") {
    const tags = batchTagList();
    if (!tags.length) {
      setError(copy.serversPage.tagRequired);
      return;
    }
    await runBatchAction(action, { tags });
  }

  async function setSelectedDashboardVisible(visible: boolean) {
    await runBatchAction("set_dashboard_visible", { dashboard_visible: visible });
  }

  async function transferSelectedOwner() {
    const ownerUserId = batchOwnerUserId.trim();
    if (!ownerUserId) {
      setError(copy.serversPage.ownerUserIdRequired);
      return;
    }
    await runBatchAction("transfer_owner", { owner_user_id: ownerUserId });
  }

  async function retryOwnerTransfer(transfer: ServerOwnerTransfer) {
    setTransferError(null);
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.retryServerOwnerTransfer(transfer.id, totpCode);
    if (!response.success) {
      setTransferError(responseError(response));
      return;
    }
    showToast(copy.serversPage.transferRetried.replace("{id}", compactId(transfer.id)));
    await loadOwnerTransfers();
    await loadServers();
    await loadServerGroups();
  }

  async function cancelOwnerTransfer(transfer: ServerOwnerTransfer) {
    if (
      !(await dialogs.confirm({
        message: copy.serversPage.cancelTransferConfirm.replace("{id}", compactId(transfer.id)),
        danger: true,
      }))
    ) {
      return;
    }
    setTransferError(null);
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.cancelServerOwnerTransfer(transfer.id, totpCode);
    if (!response.success) {
      setTransferError(responseError(response));
      return;
    }
    showToast(copy.serversPage.transferCancelled.replace("{id}", compactId(transfer.id)));
    await loadOwnerTransfers();
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={`ws: ${connectionLabel(conn, copy)}`}
          title={copy.serversPage.title}
          detail={copy.serversPage.detail}
          actions={
            <button
              type="button"
              onClick={() => {
                setError(null);
                void loadServers(true);
              }}
              className={buttonClass("secondary")}
            >
              {copy.serversPage.refresh}
            </button>
          }
        />
        <InlineError message={error ?? serverLoadError} />
        {showWsBanner ? (
          <div className="mt-3">
            <InlineNotice tone="yellow">{copy.serversPage.wsDisconnectedBanner}</InlineNotice>
          </div>
        ) : null}
        <ServerSummaryGrid summary={summary} copy={copy} />

        {/* Unified High-Density Control Bar */}
        <div className="mt-5 mb-5 grid gap-3">
          <div className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3.5 shadow-[var(--shadow-xs)]">
            <div className="flex flex-wrap items-center justify-between gap-3">
              {/* Search + Status Segmented Control */}
              <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2.5">
                <div className="relative min-w-[14rem] flex-1 sm:max-w-xs">
                  <svg
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.75"
                    className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-faint)]"
                    aria-hidden="true"
                  >
                    <circle cx="7" cy="7" r="4.5" />
                    <path d="M10.5 10.5L14 14" strokeLinecap="round" />
                  </svg>
                  <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    className={`${inputClass} mt-0 pl-8 py-1.5 text-xs`}
                    placeholder={copy.serversPage.searchPlaceholder}
                  />
                </div>

                <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
                  {(
                    [
                      ["all", copy.serversPage.filterAll],
                      ["online", copy.serversPage.filterOnline],
                      ["offline", copy.serversPage.filterOffline],
                      ["other", copy.serversPage.filterOther],
                    ] as Array<[ServerStatusFilter, string]>
                  ).map(([value, label]) => (
                    <button
                      key={value}
                      type="button"
                      onClick={() => setStatusFilter(value)}
                      className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                        statusFilter === value
                          ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </div>

              {/* Sort & View Switchers */}
              <div className="flex flex-wrap items-center gap-2">
                <select
                  value={groupFilter}
                  onChange={(event) => setGroupFilter(event.target.value)}
                  className={`${selectClass} mt-0 w-auto py-1.5 text-xs`}
                >
                  <option value="all">{copy.serversPage.groupFilterAll}</option>
                  {serverGroups.length ? (
                    <optgroup label="Server groups">
                      {serverGroups.map((group) => (
                        <option key={group.id} value={`group:${group.id}`}>
                          {group.name}
                        </option>
                      ))}
                    </optgroup>
                  ) : null}
                  {tagGroups.length ? (
                    <optgroup label="Tags">
                      {tagGroups.map((group) => (
                        <option key={group} value={`tag:${group}`}>
                          {group}
                        </option>
                      ))}
                    </optgroup>
                  ) : null}
                </select>

                <select
                  value={sortKey}
                  onChange={(event) => setSortKey(event.target.value as ServerSortKey)}
                  className={`${selectClass} mt-0 w-auto py-1.5 text-xs`}
                >
                  <option value="default">{copy.serversPage.sortDefault}</option>
                  <option value="name">{copy.serversPage.sortName}</option>
                  <option value="status">{copy.serversPage.sortStatus}</option>
                  <option value="cpu">CPU</option>
                  <option value="memory">{copy.serversPage.sortMemory}</option>
                  <option value="load">{copy.serversPage.sortLoad}</option>
                  <option value="uptime">{copy.serversPage.sortUptime}</option>
                  <option value="upload">{copy.serversPage.sortUpload}</option>
                  <option value="download">{copy.serversPage.sortDownload}</option>
                  <option value="totalUpload">{copy.serversPage.sortTotalUpload}</option>
                  <option value="totalDownload">{copy.serversPage.sortTotalDownload}</option>
                </select>

                <button
                  type="button"
                  onClick={() => setSortOrder((current) => (current === "asc" ? "desc" : "asc"))}
                  className={buttonClass("secondary")}
                >
                  {sortOrder === "asc" ? copy.serversPage.sortAsc : copy.serversPage.sortDesc}
                </button>

                <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
                  <button
                    type="button"
                    onClick={() => changeViewMode("cards")}
                    className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                      viewMode === "cards"
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {copy.serversPage.viewCards}
                  </button>
                  <button
                    type="button"
                    onClick={() => changeViewMode("compact")}
                    className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                      viewMode === "compact"
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {copy.serversPage.viewCompact}
                  </button>
                </div>

                <button
                  type="button"
                  onClick={toggleMap}
                  className={buttonClass(showMap ? "primary" : "secondary")}
                >
                  {copy.serversPage.toggleMap}
                </button>
                <button
                  type="button"
                  onClick={toggleServices}
                  className={buttonClass(showServices ? "primary" : "secondary")}
                >
                  {copy.serversPage.toggleServices}
                </button>
              </div>
            </div>

            {/* Secondary Row: Quick Group Pills + Drawer Toggles + Page Selection */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-color)] pt-2.5">
              <div className="flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={() => setGroupFilter("all")}
                  className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition ${
                    groupFilter === "all"
                      ? "border-[var(--border-strong)] bg-[var(--bg-muted)] text-[var(--text-main)]"
                      : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                  }`}
                >
                  <span>{copy.serversPage.groupFilterAll}</span>
                  <span className="font-mono text-[10px] opacity-70">{servers.length}</span>
                </button>
                {serverGroups.map((group) => {
                  const key = `group:${group.id}`;
                  const active = groupFilter === key;
                  return (
                    <button
                      key={group.id}
                      type="button"
                      onClick={() => setGroupFilter(active ? "all" : key)}
                      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition ${
                        active
                          ? "border-[var(--border-strong)] bg-[var(--bg-muted)] text-[var(--text-main)]"
                          : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      <span
                        className="h-2 w-2 rounded-full"
                        style={{
                          backgroundColor:
                            sanitizeCssVariableValue(group.color) ?? "var(--accent-color)",
                        }}
                      />
                      <span>{group.name}</span>
                      <span className="font-mono text-[10px] opacity-70">
                        {group.server_ids.length}
                      </span>
                    </button>
                  );
                })}
              </div>

              <div className="flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={toggleFilteredSelection}
                  className={buttonClass(allFilteredSelected ? "primary" : "secondary")}
                >
                  {allFilteredSelected ? copy.serversPage.deselectPage : copy.serversPage.selectPage}
                </button>
                <button
                  type="button"
                  onClick={() => setGroupsDrawerOpen((prev) => !prev)}
                  className={buttonClass(groupsDrawerOpen ? "primary" : "secondary")}
                >
                  <span>Server groups</span>
                  <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.2 font-mono text-[10px] text-[var(--text-muted)]">
                    {serverGroups.length}
                  </span>
                </button>
                {canManageTransfers ? (
                  <button
                    type="button"
                    onClick={() => {
                      const next = !transfersDrawerOpen;
                      setTransfersDrawerOpen(next);
                      if (next && !ownerTransfersLoadedRef.current && !transfersLoading) {
                        void loadOwnerTransfers();
                      }
                    }}
                    className={buttonClass(transfersDrawerOpen ? "primary" : "secondary")}
                  >
                    <span>{copy.serversPage.ownerTransfersTitle}</span>
                    {ownerTransfers.length > 0 ? (
                      <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.2 font-mono text-[10px] text-[var(--text-muted)]">
                        {ownerTransfers.length}
                      </span>
                    ) : null}
                  </button>
                ) : null}
              </div>
            </div>
          </div>

          {groupsDrawerOpen ? (
            <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-4 shadow-[var(--shadow-sm)]">
              <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <h3 className="text-sm font-semibold text-[var(--text-main)]">Server groups</h3>
                  <StatusBadge tone="blue">
                    {copy.serversPage.serverGroupsBadge.replace("{count}", String(serverGroups.length))}
                  </StatusBadge>
                </div>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void loadServerGroups(true)}
                >
                  {copy.serversPage.refreshGroups}
                </button>
              </div>
              <div className="grid gap-3 xl:grid-cols-[minmax(12rem,1fr)_auto_minmax(12rem,1fr)_minmax(10rem,0.8fr)_7rem_7rem_auto_auto] xl:items-end">
                <input
                  value={newServerGroupName}
                  onChange={(event) => setNewServerGroupName(event.target.value)}
                  className={inputClass}
                  placeholder={copy.serversPage.newServerGroupPlaceholder}
                />
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void runGroup(createServerGroup)}
                  disabled={groupBusy}
                >
                  {copy.serversPage.createGroup}
                </button>
                <select
                  value={selectedServerGroupId}
                  onChange={(event) => selectServerGroup(event.target.value)}
                  className={selectClass}
                >
                  <option value="">{copy.serversPage.selectServerGroupOption}</option>
                  {serverGroups.map((group) => (
                    <option key={group.id} value={group.id}>
                      {group.name}
                    </option>
                  ))}
                </select>
                <input
                  value={serverGroupEdit.name}
                  onChange={(event) =>
                    setServerGroupEdit((current) => ({ ...current, name: event.target.value }))
                  }
                  className={inputClass}
                  placeholder={copy.serversPage.groupNamePlaceholder}
                  disabled={!selectedServerGroupId}
                />
                <input
                  value={serverGroupEdit.displayOrder}
                  onChange={(event) =>
                    setServerGroupEdit((current) => ({ ...current, displayOrder: event.target.value }))
                  }
                  className={inputClass}
                  inputMode="numeric"
                  placeholder={copy.serversPage.groupOrderPlaceholder}
                  disabled={!selectedServerGroupId}
                />
                <input
                  type="color"
                  value={serverGroupEdit.color}
                  onChange={(event) =>
                    setServerGroupEdit((current) => ({ ...current, color: event.target.value }))
                  }
                  className="h-9 w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1 disabled:opacity-60"
                  disabled={!selectedServerGroupId}
                  aria-label={copy.serversPage.groupColorAriaLabel}
                />
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => void runGroup(updateSelectedServerGroup)}
                  disabled={!selectedServerGroupId || groupBusy}
                >
                  {copy.serversPage.saveGroup}
                </button>
                <button
                  type="button"
                  onClick={() => void runGroup(deleteSelectedServerGroup)}
                  className={buttonClass("danger")}
                  disabled={!selectedServerGroupId || groupBusy}
                >
                  {copy.serversPage.deleteGroup}
                </button>
              </div>
            </div>
          ) : null}

          {canManageTransfers && transfersDrawerOpen ? (
            <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-4 shadow-[var(--shadow-sm)]">
              <OwnerTransferPanel
                transfers={ownerTransfers}
                loading={transfersLoading}
                busy={transferBusy}
                error={transferError}
                copy={copy}
                onRefresh={() => void loadOwnerTransfers()}
                onRetry={(transfer) => void runTransfer(() => retryOwnerTransfer(transfer))}
                onCancel={(transfer) => void runTransfer(() => cancelOwnerTransfer(transfer))}
              />
            </div>
          ) : null}
        </div>

        {showMap ? (
          <WorldServerMap
            servers={filtered}
            title={copy.serversPage.mapTitle}
            ariaLabel={copy.serversPage.mapAriaLabel}
            serverHref={(server) => `/servers/${encodeURIComponent(server.id)}`}
          />
        ) : null}

        {showServices ? (
          <ServiceTrackerPanel
            loading={servicesLoading || servicesPending}
            error={serviceError}
            trackers={visibleServiceTrackers}
            copy={copy}
          />
        ) : null}

        {loading ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 6 }).map((_, index) => (
              <SkeletonCard key={index} lines={6} />
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <EmptyState
            title={copy.serversPage.serversEmptyTitle}
            detail={copy.serversPage.serversEmptyDetail}
          />
        ) : viewMode === "compact" ? (
          <div className="grid gap-2">
            {filtered.map((server) => (
              <CompactServerRow
                key={server.id}
                server={server}
                selected={selectedSet.has(server.id)}
                onToggle={toggleServerSelection}
                copy={copy}
              />
            ))}
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {filtered.map((server) => (
              <ServerCard
                key={server.id}
                server={server}
                selected={selectedSet.has(server.id)}
                onToggle={toggleServerSelection}
                copy={copy}
              />
            ))}
          </div>
        )}

        {selectedIds.length ? (
          <section
            aria-label={copy.serversPage.selectedCount.replace("{count}", String(selectedIds.length))}
            className="sticky bottom-4 z-30 mt-5 grid gap-3 rounded-xl border border-[var(--border-strong)] bg-[var(--bg-card)]/95 p-4 shadow-[var(--shadow-lg)] backdrop-blur-md"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <StatusBadge tone="green">
                {copy.serversPage.selectedCount.replace("{count}", String(selectedIds.length))}
              </StatusBadge>
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => setBatchCollapsed((current) => !current)}
                  aria-expanded={!batchCollapsed}
                >
                  {batchCollapsed ? copy.serversPage.batchExpand : copy.serversPage.batchCollapse}
                </button>
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => setSelectedIds([])}
                >
                  {copy.serversPage.clearSelection}
                </button>
              </div>
            </div>
            {batchCollapsed ? null : (
              <>
                <div className="grid gap-2.5 xl:grid-cols-[minmax(12rem,1fr)_auto_auto_auto_auto_auto]">
                  <input
                    value={batchTags}
                    onChange={(event) => setBatchTags(event.target.value)}
                    className={inputClass}
                    placeholder={copy.serversPage.batchTagsPlaceholder}
                  />
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(() => runTagBatch("set_tags"))}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.setTags}
                  </button>
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(() => runTagBatch("add_tags"))}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.addTags}
                  </button>
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(() => runTagBatch("remove_tags"))}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.removeTags}
                  </button>
                  <button
                    type="button"
                    className={buttonClass("good")}
                    onClick={() => void runBatch(() => setSelectedDashboardVisible(true))}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.showOnDashboard}
                  </button>
                  <button
                    type="button"
                    className={buttonClass("danger")}
                    onClick={() => void runBatch(() => setSelectedDashboardVisible(false))}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.hideOnDashboard}
                  </button>
                </div>
                <div className="grid gap-2.5 md:grid-cols-[minmax(12rem,1fr)_auto]">
                  <input
                    value={batchOwnerUserId}
                    onChange={(event) => setBatchOwnerUserId(event.target.value)}
                    className={inputClass}
                    placeholder={copy.serversPage.ownerUserIdPlaceholder}
                  />
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(transferSelectedOwner)}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.transferOwner}
                  </button>
                </div>
                <div className="grid gap-2.5 xl:grid-cols-[minmax(12rem,1fr)_auto_auto]">
                  <select
                    value={selectedServerGroupId}
                    onChange={(event) => selectServerGroup(event.target.value)}
                    className={selectClass}
                  >
                    <option value="">{copy.serversPage.selectServerGroupOption}</option>
                    {serverGroups.map((group) => (
                      <option key={group.id} value={group.id}>
                        {group.name}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(addSelectedToServerGroup)}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.addToGroup}
                  </button>
                  <button
                    type="button"
                    className={buttonClass("secondary")}
                    onClick={() => void runBatch(moveSelectedToServerGroup)}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.moveToGroup}
                  </button>
                </div>
                <div className="flex flex-wrap justify-end gap-2">
                  <button
                    type="button"
                    className={buttonClass("danger")}
                    onClick={() => void runBatch(deleteSelectedServers)}
                    disabled={batchBusy}
                  >
                    {copy.serversPage.deleteSelectedServers}
                  </button>
                </div>
              </>
            )}
          </section>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function ServerSummaryGrid({ summary, copy }: { summary: ServerSummary; copy: Copy }) {
  return (
    <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <SummaryTile
        label={copy.serversPage.summaryServers}
        value={String(summary.total)}
        detail={copy.serversPage.summaryOnlineOffline
          .replace("{online}", String(summary.online))
          .replace("{offline}", String(summary.offline))}
      />
      <SummaryTile
        label={copy.serversPage.summaryUpload}
        value={formatRate(summary.uploadSpeed)}
        detail={copy.serversPage.summaryTotal.replace("{total}", formatBytes(summary.totalUpload))}
      />
      <SummaryTile
        label={copy.serversPage.summaryDownload}
        value={formatRate(summary.downloadSpeed)}
        detail={copy.serversPage.summaryTotal.replace("{total}", formatBytes(summary.totalDownload))}
      />
      <SummaryTile
        label={copy.serversPage.summaryStatus}
        value={
          summary.offline > 0 ? copy.serversPage.summaryPartialOffline : copy.serversPage.summaryRunning
        }
        detail={summary.total ? copy.serversPage.summaryWsLive : copy.serversPage.summaryWsWaiting}
      />
    </div>
  );
}

function WorldMapSkeleton() {
  return (
    <section
      className="mb-5 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-4 shadow-[var(--shadow-xs)]"
      aria-busy="true"
    >
      <div className="mb-3 flex items-end justify-between gap-3 border-b border-[var(--border-color)] pb-3">
        <div className="grid gap-2">
          <Skeleton className="h-6 w-44" />
          <Skeleton className="h-4 w-64 max-w-full" />
        </div>
        <Skeleton className="h-6 w-20" />
      </div>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
        <Skeleton className="aspect-[9/5] min-h-72 w-full" />
        <div className="grid content-start gap-3">
          {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-20 w-full" />
          ))}
        </div>
      </div>
    </section>
  );
}

function OwnerTransferPanel({
  transfers,
  loading,
  busy,
  error,
  copy,
  onRefresh,
  onRetry,
  onCancel,
}: {
  transfers: ServerOwnerTransfer[];
  loading: boolean;
  busy: boolean;
  error: string | null;
  copy: Copy;
  onRefresh: () => void;
  onRetry: (transfer: ServerOwnerTransfer) => void;
  onCancel: (transfer: ServerOwnerTransfer) => void;
}) {
  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-semibold tracking-wider uppercase text-[var(--text-muted)]">
            {copy.serversPage.ownerTransfersTitle}
          </span>
          <StatusBadge tone="blue">
            {copy.serversPage.ownerTransfersBadge.replace("{count}", String(transfers.length))}
          </StatusBadge>
          {loading ? <StatusBadge tone="gray">{copy.serversPage.ownerTransfersLoading}</StatusBadge> : null}
        </div>
        <button type="button" className={buttonClass("secondary")} onClick={onRefresh}>
          {copy.serversPage.ownerTransfersRefresh}
        </button>
      </div>
      <InlineError message={error} />
      {transfers.length === 0 ? (
        <EmptyState
          title={copy.serversPage.ownerTransfersEmptyTitle}
          detail={copy.serversPage.ownerTransfersEmptyDetail}
        />
      ) : (
        <div className="grid gap-2">
          {transfers.map((transfer) => (
            <div
              key={transfer.id}
              className="grid gap-3 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3 lg:grid-cols-[minmax(10rem,1fr)_minmax(10rem,1fr)_minmax(12rem,1.4fr)_auto] lg:items-center"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <StatusBadge tone={transferStatusTone(transfer.status)}>{transfer.status}</StatusBadge>
                  <span className="font-mono text-xs text-[var(--text-muted)]">#{transfer.attempts}</span>
                </div>
                <p className="mt-1 truncate font-mono text-sm font-medium text-[var(--text-main)]">
                  {compactId(transfer.server_id)}
                </p>
              </div>
              <div className="min-w-0 font-mono text-xs text-[var(--text-muted)]">
                <p className="truncate">from {compactId(transfer.from_user_id ?? "-")}</p>
                <p className="truncate">to {compactId(transfer.to_user_id)}</p>
              </div>
              <div className="min-w-0 font-mono text-xs text-[var(--text-muted)]">
                <p className="truncate">{formatDate(transfer.last_attempt_at)}</p>
                {transfer.error ? (
                  <p className="truncate text-rose-600 dark:text-rose-400">{transfer.error}</p>
                ) : null}
              </div>
              <div className="flex flex-wrap justify-end gap-2">
                {transfer.status !== "completed" ? (
                  <>
                    <button
                      type="button"
                      className={buttonClass("secondary")}
                      onClick={() => onRetry(transfer)}
                      disabled={busy}
                    >
                      {copy.serversPage.retry}
                    </button>
                    <button
                      type="button"
                      className={buttonClass("danger")}
                      onClick={() => onCancel(transfer)}
                      disabled={busy}
                    >
                      {copy.serversPage.cancel}
                    </button>
                  </>
                ) : (
                  <StatusBadge tone="green">{copy.serversPage.completed}</StatusBadge>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function SummaryTile({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-medium tracking-wider uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2 break-words font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)]">
        {value}
      </div>
      <div className="mt-1 text-xs text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}

function ServiceTrackerPanel({
  loading,
  error,
  trackers,
  copy,
}: {
  loading: boolean;
  error: string | null;
  trackers: ServiceTrackerItem[];
  copy: Copy;
}) {
  return (
    <section className="mb-5 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-4 shadow-[var(--shadow-xs)]">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3">
        <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
          {copy.serversPage.servicesTitle}
        </h2>
        <StatusBadge tone="blue">
          {loading
            ? copy.serversPage.servicesLoading
            : copy.serversPage.servicesItemCount.replace("{count}", String(trackers.length))}
        </StatusBadge>
      </div>
      <InlineError message={error} />
      {loading && trackers.length === 0 ? (
        <BrutalCard>{copy.serversPage.servicesLoadingCard}</BrutalCard>
      ) : trackers.length === 0 ? (
        error ? null : (
          <EmptyState
            title={copy.serversPage.servicesEmptyTitle}
            detail={copy.serversPage.servicesEmptyDetail}
          />
        )
      ) : (
        <div className="grid gap-3 lg:grid-cols-2">
          {trackers.map((tracker) => (
            <ServiceTrackerCard key={tracker.id} tracker={tracker} copy={copy} />
          ))}
        </div>
      )}
      <div className="mt-3 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs text-[var(--text-muted)]">
        {copy.serversPage.servicesTrafficSummary}
      </div>
    </section>
  );
}

function ServiceTrackerCard({ tracker, copy }: { tracker: ServiceTrackerItem; copy: Copy }) {
  return (
    <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-[var(--text-main)]">{tracker.name}</h3>
          <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">
            {formatMs(Math.round(tracker.avgDelay ?? 0))}
          </p>
        </div>
        <StatusBadge tone={tracker.uptime >= 99 ? "green" : tracker.uptime >= 95 ? "yellow" : "red"}>
          {formatPercent(tracker.uptime)}
        </StatusBadge>
      </div>
      <div className="grid grid-cols-[repeat(30,minmax(0,1fr))] gap-1">
        {tracker.days.map((day) => (
          <span
            key={day.key}
            title={`${day.label} ${formatPercent(day.uptime)} ${
              day.avgDelay === undefined ? "" : formatMs(Math.round(day.avgDelay))
            }`}
            className={`h-6 rounded-sm transition-opacity hover:opacity-80 ${serviceDayClass(day)}`}
          />
        ))}
      </div>
      <div className="mt-2 flex justify-between text-[11px] text-[var(--text-muted)]">
        <span>{copy.serversPage.serviceDays30Ago}</span>
        <span>{copy.serversPage.serviceDaysToday}</span>
      </div>
    </div>
  );
}

const ServerCard = memo(function ServerCard({
  server,
  selected,
  onToggle,
  copy,
}: {
  server: Server;
  selected: boolean;
  onToggle: (id: string) => void;
  copy: Copy;
}) {
  const remark = optionalMetaLabel(server.remark) ?? optionalMetaLabel(server.note);
  const expiresAt = server.expires_at || server.expired_at || null;
  const renewalPrice = server.renewal_price ?? server.price ?? null;
  const memoryPercent = memoryPercentValue(server);
  const isOnline = server.status === "online";
  const expired = isExpired(expiresAt);
  const tags = Array.isArray(server.tags) ? server.tags.filter(Boolean) : [];
  const metaRows: Array<{ label: string; value: string | null; danger?: boolean | undefined }> = [
    { label: copy.serversPage.metaRemark, value: remark },
    { label: copy.serversPage.metaProvider, value: optionalMetaLabel(server.provider) },
    { label: copy.serversPage.metaRegion, value: optionalMetaLabel(server.region) },
    { label: copy.serversPage.metaPlan, value: optionalMetaLabel(server.plan) },
    {
      label: copy.serversPage.metaExpires,
      value: optionalMetaLabel(expiresAt ? formatDate(expiresAt) : null),
      danger: expired,
    },
    { label: copy.serversPage.metaRenewal, value: optionalMetaLabel(renewalPrice) },
  ];
  const visibleMetaRows = metaRows.filter(
    (row): row is { label: string; value: string; danger?: boolean | undefined } => row.value !== null,
  );

  return (
    <article
      className={`group relative flex h-full flex-col rounded-xl border bg-[var(--bg-card)] p-4 text-[var(--text-main)] shadow-[var(--shadow-xs)] transition-all [contain-intrinsic-size:auto_28rem] [content-visibility:auto] hover:border-[var(--border-strong)] hover:shadow-[var(--shadow-md)] ${
        selected
          ? "border-[var(--accent-color)] ring-2 ring-[var(--accent-color)]/25"
          : "border-[var(--border-color)]"
      }`}
      style={{
        borderTopColor:
          sanitizeCssVariableValue(server.accent_color) ?? "var(--accent-color)",
        borderTopWidth: "3px",
      }}
    >
      <div className="flex h-full flex-col gap-3.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="mb-1.5 flex items-center gap-2">
              <label
                className="inline-flex cursor-pointer items-center"
                title={copy.serversPage.cardSelect}
              >
                <input
                  type="checkbox"
                  checked={selected}
                  onChange={() => onToggle(server.id)}
                  aria-label={copy.serversPage.cardSelectAria.replace("{name}", server.name)}
                  className="h-3.5 w-3.5 rounded accent-[var(--accent-color)]"
                />
              </label>
              <StatusBadge
                tone={isOnline ? "green" : server.status === "revoked" ? "yellow" : "red"}
              >
                {serverStatusLabel(server.status, copy)}
              </StatusBadge>
              {server.region || server.country ? (
                <span className="truncate rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]">
                  {server.region || server.country}
                </span>
              ) : null}
            </div>
            <Link
              href={`/servers/${encodeURIComponent(server.id)}`}
              className="block break-words text-base font-semibold tracking-tight text-[var(--text-main)] hover:text-[var(--accent-color)]"
              aria-label={copy.serversPage.cardOpenAria.replace("{name}", server.name)}
            >
              {server.name}
            </Link>
            <p className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">
              {compactId(server.id)}
              {server.provider ? ` · ${server.provider}` : ""}
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-1.5">
            <Link
              href={`/terminal?server=${encodeURIComponent(server.id)}`}
              className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-1 font-mono text-[11px] font-medium text-[var(--text-muted)] transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
              title="Web Terminal"
            >
              &gt;_
            </Link>
            <Link
              href={`/servers/${encodeURIComponent(server.id)}`}
              className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-1 text-xs font-medium text-[var(--text-muted)] transition-colors hover:border-[var(--accent-color)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent-color)]"
            >
              {copy.serversPage.cardDetail}
            </Link>
          </div>
        </div>

        {visibleMetaRows.length ? (
          <div className="grid gap-1.5">
            {visibleMetaRows.map((row) => (
              <MetaStrip key={row.label} label={row.label} value={row.value} danger={row.danger} />
            ))}
          </div>
        ) : null}
        {tags.length ? (
          <div className="flex flex-wrap gap-1.5">
            {tags.map((tag) => (
              <span
                key={tag}
                className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 text-[11px] font-medium text-[var(--text-muted)]"
              >
                {tag}
              </span>
            ))}
          </div>
        ) : null}

        <div className="mt-auto grid grid-cols-2 gap-2.5">
          <MetricBlock label={copy.serversPage.metricCpu} value={formatPercent(server.cpu_percent)}>
            <UsageBar value={server.cpu_percent} />
          </MetricBlock>
          <MetricBlock label={copy.serversPage.metricMemory} value={memoryLabel(server)}>
            <UsageBar value={memoryPercent} />
          </MetricBlock>
          <MetricBlock
            label={copy.serversPage.metricLoad}
            value={
              server.load_1 === undefined || server.load_1 === null ? "N/A" : server.load_1.toFixed(2)
            }
          />
          <MetricBlock
            label={copy.serversPage.metricUptime}
            value={durationLabel(server.uptime_seconds)}
          />
          <MetricBlock
            label={copy.serversPage.metricUpload}
            value={formatRate(server.net_tx_bps)}
          />
          <MetricBlock
            label={copy.serversPage.metricDownload}
            value={formatRate(server.net_rx_bps)}
          />
          <MetricBlock
            label={copy.serversPage.metricTotalUpload}
            value={formatBytes(server.network_out_total)}
          />
          <MetricBlock
            label={copy.serversPage.metricTotalDownload}
            value={formatBytes(server.network_in_total)}
          />
        </div>
      </div>
    </article>
  );
});

const CompactServerRow = memo(function CompactServerRow({
  server,
  selected,
  onToggle,
  copy,
}: {
  server: Server;
  selected: boolean;
  onToggle: (id: string) => void;
  copy: Copy;
}) {
  const isOnline = server.status === "online";
  return (
    <div className="grid gap-2 [contain-intrinsic-size:auto_4.5rem] [content-visibility:auto] sm:grid-cols-[auto_minmax(0,1fr)]">
      <label className="flex cursor-pointer items-center justify-center rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-2 transition-colors hover:border-[var(--border-strong)]">
        <input
          type="checkbox"
          checked={selected}
          onChange={() => onToggle(server.id)}
          aria-label={copy.serversPage.cardSelectAria.replace("{name}", server.name)}
          className="h-3.5 w-3.5 accent-[var(--accent-color)]"
        />
      </label>
      <Link
        href={`/servers/${encodeURIComponent(server.id)}`}
        className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3 text-[var(--text-main)] shadow-[var(--shadow-xs)] transition-all hover:border-[var(--border-strong)] hover:shadow-[var(--shadow-md)] lg:grid-cols-[minmax(12rem,1.4fr)_repeat(8,minmax(5.5rem,1fr))]"
        aria-label={copy.serversPage.cardOpenAria.replace("{name}", server.name)}
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span
              className={`h-2 w-2 shrink-0 rounded-full ${
                isOnline ? "bg-emerald-500" : "bg-rose-500"
              }`}
            />
            <span className="truncate text-sm font-semibold text-[var(--text-main)]">
              {server.name}
            </span>
          </div>
          <div className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">
            {compactId(server.id)}
          </div>
        </div>
        <CompactMetric
          label={copy.serversPage.metricStatus}
          value={serverStatusLabel(server.status, copy)}
        />
        <CompactMetric
          label={copy.serversPage.metricCpu}
          value={formatPercent(server.cpu_percent)}
        />
        <CompactMetric label={copy.serversPage.metricMemory} value={memoryLabel(server)} />
        <CompactMetric
          label={copy.serversPage.metricLoad}
          value={server.load_1 === undefined || server.load_1 === null ? "N/A" : server.load_1.toFixed(2)}
        />
        <CompactMetric
          label={copy.serversPage.metricUptimeShort}
          value={durationLabel(server.uptime_seconds)}
        />
        <CompactMetric label={copy.serversPage.metricUpload} value={formatRate(server.net_tx_bps)} />
        <CompactMetric label={copy.serversPage.metricDownload} value={formatRate(server.net_rx_bps)} />
        <CompactMetric
          label={copy.serversPage.metricTraffic}
          value={`↑${formatBytes(server.network_out_total)} ↓${formatBytes(server.network_in_total)}`}
        />
      </Link>
    </div>
  );
});

function CompactMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[11px] text-[var(--text-muted)]">{label}</div>
      <div className="mt-0.5 truncate font-mono text-xs font-medium text-[var(--text-main)]">
        {value}
      </div>
    </div>
  );
}

function MetaStrip({
  label,
  value,
  danger = false,
}: {
  label: string;
  value: string;
  danger?: boolean | undefined;
}) {
  return (
    <div
      className={`grid grid-cols-[4.5rem_minmax(0,1fr)] rounded-md border text-xs ${
        danger
          ? "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300"
          : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-main)]"
      }`}
    >
      <span className="border-r border-[var(--border-color)] px-2 py-1 font-medium text-[var(--text-muted)]">
        {label}
      </span>
      <span className="min-w-0 truncate px-2 py-1 font-medium">{value}</span>
    </div>
  );
}

function MetricBlock({
  label,
  value,
  children,
}: {
  label: string;
  value: string;
  children?: ReactNode | undefined;
}) {
  return (
    <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-2.5">
      <div className="text-[11px] font-medium text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words font-mono text-sm font-semibold text-[var(--text-main)]">
        {value}
      </div>
      {children ? <div className="mt-1.5">{children}</div> : null}
    </div>
  );
}

function UsageBar({ value }: { value?: number | null | undefined }) {
  const percent = clampPercent(value);
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-[var(--border-color)]">
      <div
        className="h-full rounded-full bg-[var(--accent-color)] transition-all duration-300"
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

function memoryLabel(server: Server): string {
  const percent = memoryPercentValue(server);
  return percent === null ? "N/A" : `${percent.toFixed(1)}%`;
}

function memoryPercentValue(server: Server): number | null {
  if (server.memory_used === undefined || server.memory_used === null || !server.memory_total) {
    return null;
  }
  return (server.memory_used / server.memory_total) * 100;
}

function isExpired(value?: string | null): boolean {
  if (!value) return false;
  const date = new Date(value);
  return !Number.isNaN(date.getTime()) && date.getTime() < Date.now();
}

function optionalMetaLabel(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  const normalized = String(value).trim();
  return normalized && normalized !== "N/A" && normalized !== "暂无数据" ? normalized : null;
}

function buildServerGroups(servers: Server[]): string[] {
  return Array.from(new Set(servers.flatMap((server) => server.tags ?? []).filter(Boolean))).sort(
    (a, b) => a.localeCompare(b, "zh-CN"),
  );
}

function serverMatchesGroupFilter(
  server: Server,
  groupFilter: string,
  groupMembership: Map<string, Set<string>>,
): boolean {
  if (groupFilter === "all") return true;
  if (groupFilter.startsWith("group:")) {
    const groupId = groupFilter.slice("group:".length);
    return groupMembership.get(groupId)?.has(server.id) ?? false;
  }
  if (groupFilter.startsWith("tag:")) {
    return (server.tags ?? []).includes(groupFilter.slice("tag:".length));
  }
  return (server.tags ?? []).includes(groupFilter);
}

function buildServerSummary(servers: Server[]): ServerSummary {
  return servers.reduce(
    (summary, server) => ({
      total: summary.total + 1,
      online: summary.online + (server.status === "online" ? 1 : 0),
      offline: summary.offline + (server.status === "offline" ? 1 : 0),
      uploadSpeed: summary.uploadSpeed + (optionalNumber(server.net_tx_bps) ?? 0),
      downloadSpeed: summary.downloadSpeed + (optionalNumber(server.net_rx_bps) ?? 0),
      totalUpload: summary.totalUpload + (optionalNumber(server.network_out_total) ?? 0),
      totalDownload: summary.totalDownload + (optionalNumber(server.network_in_total) ?? 0),
    }),
    {
      total: 0,
      online: 0,
      offline: 0,
      uploadSpeed: 0,
      downloadSpeed: 0,
      totalUpload: 0,
      totalDownload: 0,
    },
  );
}

function transferStatusTone(status: string): "green" | "red" | "yellow" | "gray" | "blue" | "pink" {
  if (status === "completed") return "green";
  if (status === "failed") return "red";
  if (status === "cancelled") return "gray";
  return "yellow";
}

function serverMatchesQuery(server: Server, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return [
    server.name,
    server.id,
    server.status,
    server.remark,
    server.note,
    server.expires_at,
    server.renewal_price,
    server.provider,
    server.region,
    server.plan,
    ...(server.tags ?? []),
  ].some((value) => String(value ?? "").toLowerCase().includes(needle));
}

function serverStatusGroup(status: string): ServerStatusFilter {
  if (status === "online") return "online";
  if (status === "offline") return "offline";
  return "other";
}

function compareServers(a: Server, b: Server, sortKey: ServerSortKey, sortOrder: SortOrder): number {
  const base = compareServerBase(a, b);
  if (sortKey === "default") return base;
  const direction = sortOrder === "asc" ? 1 : -1;
  const valueDiff = compareServerSortValue(a, b, sortKey);
  return (valueDiff || base) * direction;
}

function compareServerBase(a: Server, b: Server): number {
  const orderA = a.display_order ?? Number.MAX_SAFE_INTEGER;
  const orderB = b.display_order ?? Number.MAX_SAFE_INTEGER;
  if (orderA !== orderB) return orderA - orderB;
  if (a.status !== b.status) return statusRank(a.status) - statusRank(b.status);
  return a.name.localeCompare(b.name, "zh-CN");
}

function compareServerSortValue(a: Server, b: Server, sortKey: ServerSortKey): number {
  if (sortKey === "name") return a.name.localeCompare(b.name, "zh-CN");
  if (sortKey === "status") return statusRank(a.status) - statusRank(b.status);
  return serverSortNumber(a, sortKey) - serverSortNumber(b, sortKey);
}

function serverSortNumber(server: Server, sortKey: ServerSortKey): number {
  if (sortKey === "cpu") return optionalNumber(server.cpu_percent) ?? -1;
  if (sortKey === "memory") return memoryPercentValue(server) ?? -1;
  if (sortKey === "load") return optionalNumber(server.load_1) ?? -1;
  if (sortKey === "uptime") return optionalNumber(server.uptime_seconds) ?? -1;
  if (sortKey === "upload") return optionalNumber(server.net_tx_bps) ?? -1;
  if (sortKey === "download") return optionalNumber(server.net_rx_bps) ?? -1;
  if (sortKey === "totalUpload") return optionalNumber(server.network_out_total) ?? -1;
  if (sortKey === "totalDownload") return optionalNumber(server.network_in_total) ?? -1;
  return 0;
}

function statusRank(status: string): number {
  if (status === "online") return 0;
  if (status === "degraded") return 1;
  if (status === "offline") return 2;
  return 3;
}

function normalizeServiceResult(result: unknown): ServiceResult | null {
  const row = asRecord(result);
  const serviceId = asString(row["service_id"]);
  const createdAt = asString(row["created_at"]);
  if (!serviceId || !createdAt) return null;
  return {
    service_id: serviceId,
    server_id: asString(row["server_id"]) || null,
    status: asString(row["status"]),
    delay_ms: optionalNumber(row["delay_ms"]),
    created_at: createdAt,
  };
}

function readEmbeddedHistories(services: ServiceSummary[]): ServiceHistoryEntry[] | null {
  const entries: ServiceHistoryEntry[] = [];
  for (const service of services) {
    const history = asRecord(service)["history"];
    if (!Array.isArray(history)) return null;
    entries.push({
      service,
      results: history
        .map((result) => normalizeServiceResult(result))
        .filter((result): result is ServiceResult => Boolean(result)),
    });
  }
  return entries;
}

function buildServiceTracker(service: ServiceSummary, results: ServiceResult[]): ServiceTrackerItem {
  const days = buildServiceDays(results);
  const totalChecks = results.length;
  const successful = results.filter((result) => serviceResultOk(result.status)).length;
  const delays = results
    .map((result) => optionalNumber(result.delay_ms))
    .filter((value): value is number => value !== undefined);
  return {
    id: service.id,
    name: service.name || compactId(service.id),
    uptime: totalChecks ? (successful / totalChecks) * 100 : 0,
    avgDelay: delays.length
      ? delays.reduce((sum, value) => sum + value, 0) / delays.length
      : undefined,
    days,
  };
}

function buildServiceDays(results: ServiceResult[]): ServiceDay[] {
  const today = startOfDay(new Date());
  const buckets = new Map<string, ServiceResult[]>();
  for (const result of results) {
    const date = new Date(result.created_at);
    if (Number.isNaN(date.getTime())) continue;
    const key = dayKey(date);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(result);
    else buckets.set(key, [result]);
  }

  return Array.from({ length: 30 }, (_, index) => {
    const date = new Date(today);
    date.setDate(today.getDate() - (29 - index));
    const key = dayKey(date);
    const rows = buckets.get(key) ?? [];
    const success = rows.filter((result) => serviceResultOk(result.status)).length;
    const delays = rows
      .map((result) => optionalNumber(result.delay_ms))
      .filter((value): value is number => value !== undefined);
    return {
      key,
      label: date.toLocaleDateString("zh-CN"),
      uptime: rows.length ? (success / rows.length) * 100 : 0,
      avgDelay: delays.length
        ? delays.reduce((sum, value) => sum + value, 0) / delays.length
        : undefined,
      total: rows.length,
    };
  });
}

function serviceBelongsToVisibleServer(service: ServiceSummary, visibleServerIds: Set<string>): boolean {
  const ids = Array.isArray(service.server_ids)
    ? service.server_ids.filter((id) => typeof id === "string" && id.trim().length > 0)
    : [];
  if (service.server_id) ids.push(service.server_id);
  if (!ids.length) return true;
  return ids.some((id) => visibleServerIds.has(id));
}

function serviceResultOk(status: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

function serviceDayClass(day: ServiceDay): string {
  if (day.total === 0) return "bg-[var(--border-color)]";
  if (day.uptime >= 99) return "bg-emerald-500";
  if (day.uptime >= 95) return "bg-amber-400";
  return "bg-rose-500";
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function connectionLabel(conn: ServerConnectionState, copy: Copy): string {
  const labels: Record<ServerConnectionState, string> = {
    connecting: copy.serversPage.connConnecting,
    open: copy.serversPage.connOpen,
    closed: copy.serversPage.connClosed,
    error: copy.serversPage.connError,
  };
  return labels[conn];
}

function currentServerStatus(
  server: Server,
  state: LiveState | undefined,
  now: number,
  serversObservedAt: number | null,
): string {
  if (server.status === "revoked") return server.status;

  if (state?.confirmed_at && timestampIsRecent(state.confirmed_at, now)) return "online";
  if (server.status !== "online") return server.status;
  return serversObservedAt && timestampIsRecent(serversObservedAt, now) ? "online" : "offline";
}

function timestampIsRecent(timestamp: number, now: number): boolean {
  if (!Number.isFinite(timestamp)) return false;
  const age = now - timestamp;
  return age >= -5_000 && age <= 30_000;
}

function serverStatusLabel(status: string, copy: Copy): string {
  const labels: Record<string, string> = {
    online: copy.serversPage.statusOnline,
    offline: copy.serversPage.statusOffline,
    revoked: copy.serversPage.statusRevoked,
    down: copy.serversPage.statusDown,
    degraded: copy.serversPage.statusDegraded,
  };
  return labels[status] ?? status;
}
