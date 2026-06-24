"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Navigation from "@/app/components/Navigation";
import { WorldServerMap } from "@/app/components/WorldServerMap";
import {
  asRecord,
  asString,
  BrutalCard,
  EmptyState,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
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
  useStoredUser,
} from "@/app/components/M7Primitives";
import { apiClient, buildWebSocketUrl, type JsonObject, type ServerGroup, type ServerOwnerTransfer } from "@/lib/api";

interface Server {
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

interface ServerLocation {
  source?: string | null;
  provider?: string | null;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  timezone?: string | null;
}

interface LiveState {
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
}

interface ServiceSummary {
  id: string;
  name: string;
  server_id?: string | null;
  server_ids?: string[];
}

interface ServiceResult {
  service_id: string;
  server_id?: string | null;
  status: string;
  delay_ms?: number | null;
  created_at: string;
}

interface ServiceTrackerItem {
  id: string;
  name: string;
  uptime: number;
  avgDelay?: number;
  days: ServiceDay[];
}

interface ServiceDay {
  key: string;
  label: string;
  uptime: number;
  avgDelay?: number;
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

type ConnectionState = "connecting" | "open" | "closed" | "error";
type ServerStatusFilter = "all" | "online" | "offline" | "other";
type ServerViewMode = "cards" | "compact";
type ServerSortKey = "default" | "name" | "status" | "cpu" | "memory" | "load" | "uptime" | "upload" | "download" | "totalUpload" | "totalDownload";
type SortOrder = "asc" | "desc";

export default function ServersPage() {
  const storedUser = useStoredUser();
  const canManageTransfers = isAdmin(storedUser);
  const [servers, setServers] = useState<Server[]>([]);
  const [live, setLive] = useState<Record<string, LiveState>>({});
  const [services, setServices] = useState<ServiceSummary[]>([]);
  const [serviceTrackers, setServiceTrackers] = useState<ServiceTrackerItem[]>([]);
  const [ownerTransfers, setOwnerTransfers] = useState<ServerOwnerTransfer[]>([]);
  const [loading, setLoading] = useState(true);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [transfersLoading, setTransfersLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const [transferError, setTransferError] = useState<string | null>(null);
  const [batchNotice, setBatchNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<ServerStatusFilter>("all");
  const [groupFilter, setGroupFilter] = useState("all");
  const [viewMode, setViewMode] = useState<ServerViewMode>(() => initialServerViewMode());
  const [showServices, setShowServices] = useState(() => initialShowServices());
  const [showMap, setShowMap] = useState(() => initialShowServerMap());
  const [sortKey, setSortKey] = useState<ServerSortKey>("default");
  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [batchTags, setBatchTags] = useState("");
  const [batchOwnerUserId, setBatchOwnerUserId] = useState("");
  const [serverGroups, setServerGroups] = useState<ServerGroup[]>([]);
  const [newServerGroupName, setNewServerGroupName] = useState("");
  const [selectedServerGroupId, setSelectedServerGroupId] = useState("");
  const [serverGroupEdit, setServerGroupEdit] = useState({ name: "", color: "#2563eb", displayOrder: "" });
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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServers();
  }, [loadServers]);

  const loadServerGroups = useCallback(async () => {
    const response = await apiClient.listServerGroups();
    if (response.success && response.data) {
      setServerGroups(response.data.groups ?? []);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServerGroups();
  }, [loadServerGroups]);

  const loadOwnerTransfers = useCallback(async () => {
    if (!canManageTransfers) {
      setOwnerTransfers([]);
      return;
    }
    setTransfersLoading(true);
    setTransferError(null);
    const response = await apiClient.listServerOwnerTransfers(20, 0);
    setTransfersLoading(false);
    if (response.success && response.data) {
      setOwnerTransfers(response.data.transfers ?? []);
    } else {
      setTransferError(responseError(response));
    }
  }, [canManageTransfers]);

  useEffect(() => {
    if (!canManageTransfers) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadOwnerTransfers();
  }, [canManageTransfers, loadOwnerTransfers]);

  const loadServices = useCallback(async () => {
    setServicesLoading(true);
    setServiceError(null);
    const response = await apiClient.listServices(200, 0);
    if (!response.success || !response.data) {
      setServices([]);
      setServiceTrackers([]);
      setServicesLoading(false);
      setServiceError(responseError(response));
      return;
    }

    const nextServices = ((response.data.services as ServiceSummary[]) ?? []).filter((service) => service.id);
    setServices(nextServices);
    const histories = await Promise.all(
      nextServices.map(async (service) => {
        const history = await apiClient.getServiceHistory(service.id, 1200);
        if (!history.success || !history.data) return null;
        const results = Array.isArray(history.data.results) ? history.data.results : [];
        return {
          service,
          results: results
            .map((result) => normalizeServiceResult(result))
            .filter((result): result is ServiceResult => Boolean(result)),
        };
      }),
    );

    setServiceTrackers(
      histories
        .filter((item): item is { service: ServiceSummary; results: ServiceResult[] } => Boolean(item))
        .map(({ service, results }) => buildServiceTracker(service, results)),
    );
    setServicesLoading(false);
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServices();
  }, [loadServices]);

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
                next[item.agent_id] = normalizeLiveState(
                  item.payload,
                  String(item.received_at || new Date().toISOString()),
                  next[item.agent_id],
                );
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
          net_rx_bps: state?.net_rx_bps ?? server.net_rx_bps,
          net_tx_bps: state?.net_tx_bps ?? server.net_tx_bps,
          network_in_total: state?.network_in_total ?? server.network_in_total,
          network_out_total: state?.network_out_total ?? server.network_out_total,
          uptime_seconds: state?.uptime_seconds ?? server.uptime_seconds,
          last_event_at: state?.received_at ?? server.last_event_at,
        };
      }),
    [live, servers],
  );

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
  const selectedServerGroup = useMemo(
    () => serverGroups.find((group) => group.id === selectedServerGroupId),
    [selectedServerGroupId, serverGroups],
  );
  const summary = useMemo(() => buildServerSummary(merged), [merged]);
  const filtered = useMemo(
    () =>
      merged
        .filter((server) => serverMatchesQuery(server, query))
        .filter((server) => statusFilter === "all" || serverStatusGroup(server.status) === statusFilter)
        .filter((server) => serverMatchesGroupFilter(server, groupFilter, groupMembership))
        .sort((a, b) => compareServers(a, b, sortKey, sortOrder)),
    [groupFilter, groupMembership, merged, query, sortKey, sortOrder, statusFilter],
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
    window.localStorage.setItem("xlstatus_server_view", next);
  }

  function toggleServices() {
    setShowServices((current) => {
      const next = !current;
      window.localStorage.setItem("xlstatus_show_services", next ? "1" : "0");
      return next;
    });
  }

  function toggleMap() {
    setShowMap((current) => {
      const next = !current;
      window.localStorage.setItem("xlstatus_show_server_map", next ? "1" : "0");
      return next;
    });
  }

  function toggleServerSelection(id: string) {
    setSelectedIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    );
  }

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
    setBatchNotice(null);
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
    const data = response.data as { updated?: number; failed?: number } | undefined;
    setBatchNotice(`批量操作完成：${data?.updated ?? selectedIds.length} 成功 / ${data?.failed ?? 0} 失败。`);
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
    const code = window.prompt("请输入 6 位 TOTP 验证码");
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError("请输入 6 位 TOTP 验证码。");
      return null;
    }
    return trimmed;
  }

  async function createServerGroup() {
    const name = newServerGroupName.trim();
    if (!name) {
      setError("请填写服务器分组名称。");
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
    setBatchNotice(`服务器分组 ${name} 已创建。`);
    await loadServerGroups();
  }

  async function updateSelectedServerGroup() {
    if (!selectedServerGroupId) {
      setError("请选择要编辑的服务器分组。");
      return;
    }
    const name = serverGroupEdit.name.trim();
    if (!name) {
      setError("请填写服务器分组名称。");
      return;
    }
    const displayOrder = serverGroupEdit.displayOrder.trim();
    const payload: JsonObject = {
      name,
      color: serverGroupEdit.color.trim(),
    };
    if (displayOrder) {
      const parsed = Number.parseInt(displayOrder, 10);
      if (!Number.isFinite(parsed)) {
        setError("排序必须是数字。");
        return;
      }
      payload.display_order = parsed;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.updateServerGroup(selectedServerGroupId, payload, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    setBatchNotice(`服务器分组 ${name} 已更新。`);
    await loadServerGroups();
  }

  async function addSelectedToServerGroup() {
    if (!selectedServerGroupId) {
      setError("请选择目标服务器分组。");
      return;
    }
    if (!selectedIds.length) {
      setError("请先选择服务器。");
      return;
    }
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.addServerGroupMembers(selectedServerGroupId, selectedIds, totpCode);
    if (!response.success) {
      setError(responseError(response));
      return;
    }
    setBatchNotice("已把选中服务器加入分组。");
    await loadServerGroups();
  }

  async function moveSelectedToServerGroup() {
    if (!selectedServerGroupId) {
      setError("请选择目标服务器分组。");
      return;
    }
    if (!selectedIds.length) {
      setError("请先选择服务器。");
      return;
    }
    await runBatchAction("move_group", { group_id: selectedServerGroupId });
    await loadServerGroups();
  }

  async function deleteSelectedServerGroup() {
    if (!selectedServerGroupId) {
      setError("请选择要删除的服务器分组。");
      return;
    }
    const group = serverGroups.find((item) => item.id === selectedServerGroupId);
    if (!confirm(`确定删除服务器分组 ${group?.name ?? selectedServerGroupId}？`)) return;
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
    setBatchNotice("服务器分组已删除。");
    await loadServerGroups();
  }

  async function deleteSelectedServers() {
    if (!selectedIds.length) {
      setError("请先选择服务器。");
      return;
    }
    if (!confirm(`确定删除选中的 ${selectedIds.length} 台服务器？该操作会移除 Agent 记录及关联分组。`)) return;
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
      setError("请填写至少一个分组标签。");
      return;
    }
    await runBatchAction(action, { tags });
  }

  async function transferSelectedOwner() {
    const ownerUserId = batchOwnerUserId.trim();
    if (!ownerUserId) {
      setError("请填写目标用户 ID。");
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
    setBatchNotice(`转移记录 ${compactId(transfer.id)} 已重试。`);
    await loadOwnerTransfers();
    await loadServers();
    await loadServerGroups();
  }

  async function cancelOwnerTransfer(transfer: ServerOwnerTransfer) {
    if (!confirm(`确定取消转移记录 ${compactId(transfer.id)}？`)) return;
    setTransferError(null);
    const totpCode = await sensitiveTotpCodeForAction(true);
    if (totpCode === null) return;
    const response = await apiClient.cancelServerOwnerTransfer(transfer.id, totpCode);
    if (!response.success) {
      setTransferError(responseError(response));
      return;
    }
    setBatchNotice(`转移记录 ${compactId(transfer.id)} 已取消。`);
    await loadOwnerTransfers();
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow={`ws: ${connectionLabel(conn)}`}
          title="服务器"
          detail="接入的 Agent、实时主机状态和远程运维入口。"
          actions={<button type="button" onClick={() => void loadServers()} className={buttonClass("secondary")}>刷新</button>}
        />
        <InlineError message={error} />
        {batchNotice ? <div className="mt-3"><InlineNotice tone="green">{batchNotice}</InlineNotice></div> : null}
        <ServerSummaryGrid summary={summary} />
        <div className="mt-5 mb-5 grid gap-3">
          <input value={query} onChange={(event) => setQuery(event.target.value)} className={inputClass} placeholder="搜索服务器" />
          <div className="flex flex-wrap items-center justify-between gap-3 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal)]">
            <div className="flex flex-wrap gap-2">
              {(
                [
                  ["all", "全部"],
                  ["online", "在线"],
                  ["offline", "离线"],
                  ["other", "其他"],
                ] as Array<[ServerStatusFilter, string]>
              ).map(([value, label]) => (
                <button key={value} type="button" onClick={() => setStatusFilter(value)} className={buttonClass(statusFilter === value ? "primary" : "secondary")}>
                  {label}
                </button>
              ))}
            </div>
            <div className="grid gap-2 sm:grid-cols-[minmax(9rem,auto)_minmax(9rem,auto)_auto]">
              <select value={groupFilter} onChange={(event) => setGroupFilter(event.target.value)} className={inputClass}>
                <option value="all">全部分组</option>
                {serverGroups.length ? (
                  <optgroup label="Server groups">
                    {serverGroups.map((group) => (
                      <option key={group.id} value={`group:${group.id}`}>{group.name}</option>
                    ))}
                  </optgroup>
                ) : null}
                {tagGroups.length ? (
                  <optgroup label="Tags">
                    {tagGroups.map((group) => (
                      <option key={group} value={`tag:${group}`}>{group}</option>
                    ))}
                  </optgroup>
                ) : null}
              </select>
              <select value={sortKey} onChange={(event) => setSortKey(event.target.value as ServerSortKey)} className={inputClass}>
                <option value="default">默认排序</option>
                <option value="name">名称</option>
                <option value="status">状态</option>
                <option value="cpu">CPU</option>
                <option value="memory">内存</option>
                <option value="load">负载</option>
                <option value="uptime">运行时间</option>
                <option value="upload">上传速率</option>
                <option value="download">下载速率</option>
                <option value="totalUpload">累计上传</option>
                <option value="totalDownload">累计下载</option>
              </select>
              <button type="button" onClick={() => setSortOrder((current) => (current === "asc" ? "desc" : "asc"))} className={buttonClass("secondary")}>
                {sortOrder === "asc" ? "升序" : "降序"}
              </button>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={toggleFilteredSelection} className={buttonClass(allFilteredSelected ? "primary" : "secondary")}>
                {allFilteredSelected ? "取消本页" : "选择本页"}
              </button>
              <button type="button" onClick={() => changeViewMode("cards")} className={buttonClass(viewMode === "cards" ? "primary" : "secondary")}>卡片</button>
              <button type="button" onClick={() => changeViewMode("compact")} className={buttonClass(viewMode === "compact" ? "primary" : "secondary")}>紧凑</button>
              <button type="button" onClick={toggleMap} className={buttonClass(showMap ? "primary" : "secondary")}>地图</button>
              <button type="button" onClick={toggleServices} className={buttonClass(showServices ? "primary" : "secondary")}>服务状态</button>
            </div>
          </div>
          <details className="border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <summary className="flex cursor-pointer flex-wrap items-center justify-between gap-3 p-3 text-sm font-black uppercase">
              <span className="flex flex-wrap items-center gap-2">
                <span>Server groups</span>
                <StatusBadge tone="blue">{serverGroups.length} 组</StatusBadge>
                {selectedServerGroup ? <StatusBadge tone="gray">{selectedServerGroup.server_ids.length} 台</StatusBadge> : null}
              </span>
              <span className="text-xs text-[var(--text-muted)]">展开管理</span>
            </summary>
            <div className="grid gap-3 border-t-2 border-black p-3">
              <div className="flex justify-end">
                <button type="button" className={buttonClass("secondary")} onClick={() => void loadServerGroups()}>
                  刷新组
                </button>
              </div>
              <div className="grid gap-3 xl:grid-cols-[minmax(12rem,1fr)_auto_minmax(12rem,1fr)_minmax(10rem,0.8fr)_7rem_7rem_auto_auto] xl:items-end">
                <input value={newServerGroupName} onChange={(event) => setNewServerGroupName(event.target.value)} className={inputClass} placeholder="新服务器分组" />
                <button type="button" className={buttonClass("secondary")} onClick={() => void createServerGroup()}>创建组</button>
                <select value={selectedServerGroupId} onChange={(event) => selectServerGroup(event.target.value)} className={inputClass}>
                  <option value="">选择服务器分组</option>
                  {serverGroups.map((group) => (
                    <option key={group.id} value={group.id}>{group.name}</option>
                  ))}
                </select>
                <input value={serverGroupEdit.name} onChange={(event) => setServerGroupEdit((current) => ({ ...current, name: event.target.value }))} className={inputClass} placeholder="分组名称" disabled={!selectedServerGroupId} />
                <input value={serverGroupEdit.displayOrder} onChange={(event) => setServerGroupEdit((current) => ({ ...current, displayOrder: event.target.value }))} className={inputClass} inputMode="numeric" placeholder="排序" disabled={!selectedServerGroupId} />
                <input
                  type="color"
                  value={serverGroupEdit.color}
                  onChange={(event) => setServerGroupEdit((current) => ({ ...current, color: event.target.value }))}
                  className="h-11 w-full border-2 border-black bg-[var(--accent-bg)] p-1 shadow-[var(--shadow-brutal-sm)] disabled:opacity-60"
                  disabled={!selectedServerGroupId}
                  aria-label="分组颜色"
                />
                <button type="button" className={buttonClass("secondary")} onClick={() => void updateSelectedServerGroup()} disabled={!selectedServerGroupId}>保存组</button>
                <button type="button" className={buttonClass("danger")} onClick={() => void deleteSelectedServerGroup()} disabled={!selectedServerGroupId}>删除组</button>
              </div>
            </div>
          </details>
          {selectedIds.length ? (
            <div className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal)]">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <span className="border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
                  已选择 {selectedIds.length} 台
                </span>
                <button type="button" className={buttonClass("secondary")} onClick={() => setSelectedIds([])}>
                  清空选择
                </button>
              </div>
              <div className="grid gap-3 xl:grid-cols-[minmax(12rem,1fr)_auto_auto_auto_auto_auto]">
                <input value={batchTags} onChange={(event) => setBatchTags(event.target.value)} className={inputClass} placeholder="分组标签，逗号分隔" />
                <button type="button" className={buttonClass("secondary")} onClick={() => void runTagBatch("set_tags")}>设置分组</button>
                <button type="button" className={buttonClass("secondary")} onClick={() => void runTagBatch("add_tags")}>追加分组</button>
                <button type="button" className={buttonClass("secondary")} onClick={() => void runTagBatch("remove_tags")}>移除分组</button>
                <button type="button" className={buttonClass("good")} onClick={() => void runBatchAction("set_dashboard_visible", { dashboard_visible: true })}>显示状态页</button>
                <button type="button" className={buttonClass("danger")} onClick={() => void runBatchAction("set_dashboard_visible", { dashboard_visible: false })}>隐藏状态页</button>
              </div>
              <div className="grid gap-3 md:grid-cols-[minmax(12rem,1fr)_auto]">
                <input value={batchOwnerUserId} onChange={(event) => setBatchOwnerUserId(event.target.value)} className={inputClass} placeholder="目标用户 ID" />
                <button type="button" className={buttonClass("secondary")} onClick={() => void transferSelectedOwner()}>
                  转移所有者
                </button>
              </div>
              <div className="grid gap-3 xl:grid-cols-[minmax(12rem,1fr)_auto_auto]">
                <select value={selectedServerGroupId} onChange={(event) => selectServerGroup(event.target.value)} className={inputClass}>
                  <option value="">选择服务器分组</option>
                  {serverGroups.map((group) => (
                    <option key={group.id} value={group.id}>{group.name}</option>
                  ))}
                </select>
                <button type="button" className={buttonClass("secondary")} onClick={() => void addSelectedToServerGroup()}>加入组</button>
                <button type="button" className={buttonClass("secondary")} onClick={() => void moveSelectedToServerGroup()}>移动到组</button>
              </div>
              <div className="flex flex-wrap justify-end gap-2">
                <button type="button" className={buttonClass("danger")} onClick={() => void deleteSelectedServers()}>
                  删除选中服务器
                </button>
              </div>
            </div>
          ) : null}
          {canManageTransfers ? (
            <details className="border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
              <summary className="flex cursor-pointer flex-wrap items-center justify-between gap-3 p-3 text-sm font-black uppercase">
                <span className="flex flex-wrap items-center gap-2">
                  <span>所有权转移</span>
                  <StatusBadge tone="blue">{ownerTransfers.length} 条</StatusBadge>
                </span>
                <span className="text-xs text-[var(--text-muted)]">展开处理</span>
              </summary>
              <div className="border-t-2 border-black p-3">
                <OwnerTransferPanel
                  transfers={ownerTransfers}
                  loading={transfersLoading}
                  error={transferError}
                  onRefresh={() => void loadOwnerTransfers()}
                  onRetry={(transfer) => void retryOwnerTransfer(transfer)}
                  onCancel={(transfer) => void cancelOwnerTransfer(transfer)}
                />
              </div>
            </details>
          ) : null}
        </div>

        {showMap ? (
          <WorldServerMap
            servers={filtered}
            title="服务器地图"
            ariaLabel="服务器国家和地区分布地图"
            serverHref={(server) => `/servers/${encodeURIComponent(server.id)}`}
          />
        ) : null}

        {showServices ? (
          <ServiceTrackerPanel loading={servicesLoading} error={serviceError} trackers={visibleServiceTrackers} />
        ) : null}

        {loading ? (
          <BrutalCard>正在加载服务器...</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title="暂无服务器" detail="Agent 连接后会显示在这里。" />
        ) : viewMode === "compact" ? (
          <div className="grid gap-2">
            {filtered.map((server) => (
              <CompactServerRow key={server.id} server={server} selected={selectedSet.has(server.id)} onSelect={() => toggleServerSelection(server.id)} />
            ))}
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {filtered.map((server) => (
              <ServerCard key={server.id} server={server} selected={selectedSet.has(server.id)} onSelect={() => toggleServerSelection(server.id)} />
            ))}
          </div>
        )}
      </PageShell>
    </div>
  );
}

function ServerSummaryGrid({ summary }: { summary: ServerSummary }) {
  return (
    <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <SummaryTile label="服务器" value={String(summary.total)} detail={`${summary.online} 在线 / ${summary.offline} 离线`} />
      <SummaryTile label="上传" value={formatRate(summary.uploadSpeed)} detail={`累计 ${formatBytes(summary.totalUpload)}`} />
      <SummaryTile label="下载" value={formatRate(summary.downloadSpeed)} detail={`累计 ${formatBytes(summary.totalDownload)}`} />
      <SummaryTile label="状态" value={summary.offline > 0 ? "部分离线" : "运行中"} detail={`ws ${summary.total ? "实时更新" : "等待数据"}`} />
    </div>
  );
}

function OwnerTransferPanel({
  transfers,
  loading,
  error,
  onRefresh,
  onRetry,
  onCancel,
}: {
  transfers: ServerOwnerTransfer[];
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  onRetry: (transfer: ServerOwnerTransfer) => void;
  onCancel: (transfer: ServerOwnerTransfer) => void;
}) {
  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
            所有权转移
          </span>
          <StatusBadge tone="blue">{transfers.length} 条</StatusBadge>
          {loading ? <StatusBadge tone="gray">加载中</StatusBadge> : null}
        </div>
        <button type="button" className={buttonClass("secondary")} onClick={onRefresh}>
          刷新记录
        </button>
      </div>
      <InlineError message={error} />
      {transfers.length === 0 ? (
        <EmptyState title="暂无转移记录" detail="批量转移服务器所有者后会显示在这里。" />
      ) : (
        <div className="grid gap-2">
          {transfers.map((transfer) => (
            <div key={transfer.id} className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3 lg:grid-cols-[minmax(10rem,1fr)_minmax(10rem,1fr)_minmax(12rem,1.4fr)_auto] lg:items-center">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <StatusBadge tone={transferStatusTone(transfer.status)}>{transfer.status}</StatusBadge>
                  <span className="text-xs font-black uppercase text-[var(--text-muted)]">#{transfer.attempts}</span>
                </div>
                <p className="mt-1 truncate text-sm font-black">{compactId(transfer.server_id)}</p>
              </div>
              <div className="min-w-0 text-xs font-bold text-[var(--text-muted)]">
                <p className="truncate">from {compactId(transfer.from_user_id ?? "-")}</p>
                <p className="truncate">to {compactId(transfer.to_user_id)}</p>
              </div>
              <div className="min-w-0 text-xs font-bold text-[var(--text-muted)]">
                <p className="truncate">{formatDate(transfer.last_attempt_at)}</p>
                {transfer.error ? <p className="truncate text-[var(--danger)]">{transfer.error}</p> : null}
              </div>
              <div className="flex flex-wrap justify-end gap-2">
                {transfer.status !== "completed" ? (
                  <>
                    <button type="button" className={buttonClass("secondary")} onClick={() => onRetry(transfer)}>
                      重试
                    </button>
                    <button type="button" className={buttonClass("danger")} onClick={() => onCancel(transfer)}>
                      取消
                    </button>
                  </>
                ) : (
                  <StatusBadge tone="green">完成</StatusBadge>
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
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2 break-words text-2xl font-black">{value}</div>
      <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}

function ServiceTrackerPanel({ loading, error, trackers }: { loading: boolean; error: string | null; trackers: ServiceTrackerItem[] }) {
  return (
    <section className="mb-5 border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3 border-b-4 border-black pb-3">
        <h2 className="text-xl font-black uppercase">服务状态</h2>
        <span className="border-2 border-black bg-[var(--bg-card)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {loading ? "加载中" : `${trackers.length} 项`}
        </span>
      </div>
      <InlineError message={error} />
      {loading && trackers.length === 0 ? (
        <BrutalCard>正在加载服务状态...</BrutalCard>
      ) : trackers.length === 0 ? (
        <EmptyState title="暂无服务状态" detail="创建服务监控后会显示最近状态。" />
      ) : (
        <div className="grid gap-3 lg:grid-cols-2">
          {trackers.map((tracker) => (
            <ServiceTrackerCard key={tracker.id} tracker={tracker} />
          ))}
        </div>
      )}
      <div className="mt-3 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black text-[var(--text-muted)] shadow-[var(--shadow-brutal-sm)]">
        周期流量统计：暂无数据
      </div>
    </section>
  );
}

function ServiceTrackerCard({ tracker }: { tracker: ServiceTrackerItem }) {
  return (
    <div className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
        <div>
          <h3 className="text-sm font-black">{tracker.name}</h3>
          <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{formatMs(Math.round(tracker.avgDelay ?? 0))}</p>
        </div>
        <StatusBadge tone={tracker.uptime >= 99 ? "green" : tracker.uptime >= 95 ? "yellow" : "red"}>
          {formatPercent(tracker.uptime)}
        </StatusBadge>
      </div>
      <div className="grid grid-cols-[repeat(30,minmax(0,1fr))] gap-1">
        {tracker.days.map((day) => (
          <span
            key={day.key}
            title={`${day.label} ${formatPercent(day.uptime)} ${day.avgDelay === undefined ? "" : formatMs(Math.round(day.avgDelay))}`}
            className={`h-7 border-2 border-black ${serviceDayClass(day)}`}
          />
        ))}
      </div>
      <div className="mt-2 flex justify-between text-xs font-black text-[var(--text-muted)]">
        <span>30 天前</span>
        <span>今天</span>
      </div>
    </div>
  );
}

function ServerCard({
  server,
  selected,
  onSelect,
}: {
  server: Server;
  selected: boolean;
  onSelect: () => void;
}) {
  const remark = optionalMetaLabel(server.remark) ?? optionalMetaLabel(server.note);
  const expiresAt = server.expires_at || server.expired_at || null;
  const renewalPrice = server.renewal_price ?? server.price ?? null;
  const memoryPercent = memoryPercentValue(server);
  const isOnline = server.status === "online";
  const expired = isExpired(expiresAt);
  const tags = Array.isArray(server.tags) ? server.tags.filter(Boolean) : [];
  const metaRows = [
    { label: "备注", value: remark },
    { label: "供应商", value: optionalMetaLabel(server.provider) },
    { label: "地区", value: optionalMetaLabel(server.region) },
    { label: "套餐", value: optionalMetaLabel(server.plan) },
    { label: "到期", value: optionalMetaLabel(expiresAt ? formatDate(expiresAt) : null), danger: expired },
    { label: "续费", value: optionalMetaLabel(renewalPrice) },
  ].filter((row): row is { label: string; value: string; danger?: boolean } => row.value !== null);

  return (
    <article className="grid h-full gap-2">
      <label className="flex items-center gap-2 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
        <input type="checkbox" checked={selected} onChange={onSelect} />
        选择
      </label>
      <Link
        href={`/servers/${encodeURIComponent(server.id)}`}
        className="group block h-full border-2 border-black bg-[var(--bg-card)] p-4 text-[var(--text-main)] shadow-[var(--shadow-brutal)] transition hover:-translate-x-1 hover:-translate-y-1 hover:shadow-[8px_8px_0_0_var(--border-color)] focus:outline-none focus:ring-4 focus:ring-[var(--accent-color)]"
        style={{ borderTopColor: server.accent_color || "var(--border-color)", borderTopWidth: "8px" }}
        aria-label={`打开服务器 ${server.name}`}
      >
        <div className="flex h-full flex-col gap-4">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="mb-2 flex items-center gap-2">
              <span className={`h-3 w-3 shrink-0 border-2 border-black ${isOnline ? "bg-[var(--accent-color)]" : "bg-[var(--btn-bg)]"}`} />
              <StatusBadge tone={isOnline ? "green" : server.status === "revoked" ? "yellow" : "red"}>
                {serverStatusLabel(server.status)}
              </StatusBadge>
            </div>
            <h2 className="break-words text-2xl font-black uppercase">{server.name}</h2>
            <p className="mt-1 break-all font-mono text-[11px] font-bold text-[var(--text-muted)]">{server.id}</p>
          </div>
          <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)] transition group-hover:bg-[var(--accent-color)] group-hover:text-[var(--btn-text)]">
            详情
          </span>
        </div>

        {metaRows.length ? (
          <div className="grid gap-2">
            {metaRows.map((row) => (
              <MetaStrip key={row.label} label={row.label} value={row.value} danger={row.danger} />
            ))}
          </div>
        ) : null}
        {tags.length ? (
          <div className="flex flex-wrap gap-2">
            {tags.map((tag) => (
              <span key={tag} className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-[11px] font-black shadow-[var(--shadow-brutal-sm)]">
                {tag}
              </span>
            ))}
          </div>
        ) : null}

        <div className="mt-auto grid grid-cols-2 gap-3">
          <MetricBlock label="CPU" value={formatPercent(server.cpu_percent)}>
            <UsageBar value={server.cpu_percent} />
          </MetricBlock>
          <MetricBlock label="内存" value={memoryLabel(server)}>
            <UsageBar value={memoryPercent} />
          </MetricBlock>
          <MetricBlock label="负载" value={server.load_1 === undefined || server.load_1 === null ? "N/A" : server.load_1.toFixed(2)} />
          <MetricBlock label="运行时间" value={durationLabel(server.uptime_seconds)} />
          <MetricBlock label="上传" value={formatRate(server.net_tx_bps)} />
          <MetricBlock label="下载" value={formatRate(server.net_rx_bps)} />
          <MetricBlock label="累计上传" value={formatBytes(server.network_out_total)} />
          <MetricBlock label="累计下载" value={formatBytes(server.network_in_total)} />
        </div>
        </div>
      </Link>
    </article>
  );
}

function CompactServerRow({
  server,
  selected,
  onSelect,
}: {
  server: Server;
  selected: boolean;
  onSelect: () => void;
}) {
  const isOnline = server.status === "online";
  return (
    <div className="grid gap-2 sm:grid-cols-[auto_minmax(0,1fr)]">
      <label className="flex items-center justify-center border-2 border-black bg-[var(--bg-card)] px-3 py-2 shadow-[var(--shadow-brutal-sm)]">
        <input type="checkbox" checked={selected} onChange={onSelect} aria-label={`选择 ${server.name}`} />
      </label>
      <Link
        href={`/servers/${encodeURIComponent(server.id)}`}
        className="grid gap-3 border-2 border-black bg-[var(--bg-card)] p-3 text-[var(--text-main)] shadow-[var(--shadow-brutal-sm)] transition hover:-translate-x-0.5 hover:-translate-y-0.5 hover:shadow-[var(--shadow-brutal)] lg:grid-cols-[minmax(12rem,1.4fr)_repeat(8,minmax(5.5rem,1fr))]"
        aria-label={`打开服务器 ${server.name}`}
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className={`h-3 w-3 shrink-0 border-2 border-black ${isOnline ? "bg-[var(--accent-color)]" : "bg-[var(--btn-bg)]"}`} />
            <span className="truncate text-sm font-black">{server.name}</span>
          </div>
          <div className="mt-1 truncate font-mono text-[11px] font-bold text-[var(--text-muted)]">{compactId(server.id)}</div>
        </div>
        <CompactMetric label="状态" value={serverStatusLabel(server.status)} />
        <CompactMetric label="CPU" value={formatPercent(server.cpu_percent)} />
        <CompactMetric label="内存" value={memoryLabel(server)} />
        <CompactMetric label="负载" value={server.load_1 === undefined || server.load_1 === null ? "N/A" : server.load_1.toFixed(2)} />
        <CompactMetric label="运行" value={durationLabel(server.uptime_seconds)} />
        <CompactMetric label="上传" value={formatRate(server.net_tx_bps)} />
        <CompactMetric label="下载" value={formatRate(server.net_rx_bps)} />
        <CompactMetric label="流量" value={`↑${formatBytes(server.network_out_total)} ↓${formatBytes(server.network_in_total)}`} />
      </Link>
    </div>
  );
}

function CompactMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-[11px] font-black text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 truncate text-xs font-black">{value}</div>
    </div>
  );
}

function MetaStrip({ label, value, danger = false }: { label: string; value: string; danger?: boolean }) {
  return (
    <div className={`grid grid-cols-[4.5rem_minmax(0,1fr)] border-2 border-black ${danger ? "bg-[var(--btn-bg)] text-[var(--btn-text)]" : "bg-[var(--accent-bg)] text-[var(--text-main)]"} shadow-[var(--shadow-brutal-sm)]`}>
      <span className="border-r-2 border-black px-2 py-1.5 text-xs font-black">{label}</span>
      <span className="min-w-0 truncate px-2 py-1.5 text-xs font-black">{value}</span>
    </div>
  );
}

function MetricBlock({ label, value, children }: { label: string; value: string; children?: ReactNode }) {
  return (
    <div className="min-h-20 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <div className="text-xs font-black text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words text-lg font-black">{value}</div>
      {children ? <div className="mt-2">{children}</div> : null}
    </div>
  );
}

function UsageBar({ value }: { value?: number | null }) {
  const percent = clampPercent(value);
  return (
    <div className="h-3 border-2 border-black bg-[var(--bg-page)]">
      <div className="h-full bg-[var(--accent-color)]" style={{ width: `${percent}%` }} />
    </div>
  );
}

function memoryLabel(server: Server): string {
  const percent = memoryPercentValue(server);
  return percent === null ? "N/A" : `${percent.toFixed(1)}%`;
}

function memoryPercentValue(server: Server): number | null {
  if (server.memory_used === undefined || server.memory_used === null || !server.memory_total) return null;
  return (server.memory_used / server.memory_total) * 100;
}

function clampPercent(value?: number | null): number {
  if (value === undefined || value === null || Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(100, value));
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
  return Array.from(new Set(servers.flatMap((server) => server.tags ?? []).filter(Boolean))).sort((a, b) => a.localeCompare(b, "zh-CN"));
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
    { total: 0, online: 0, offline: 0, uploadSpeed: 0, downloadSpeed: 0, totalUpload: 0, totalDownload: 0 },
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

function normalizeLiveState(payload: unknown, receivedAt: string, previous?: LiveState): LiveState {
  const state = asRecord(payload);
  const netInTotal = optionalNumber(state.network_in_total) ?? netIoTotal(state, "bytes_recv");
  const netOutTotal = optionalNumber(state.network_out_total) ?? netIoTotal(state, "bytes_sent");
  const netRxBps = optionalNumber(state.net_rx_bps) ?? rateFromTotalDelta(netInTotal, previous?.network_in_total, receivedAt, previous?.received_at);
  const netTxBps = optionalNumber(state.net_tx_bps) ?? rateFromTotalDelta(netOutTotal, previous?.network_out_total, receivedAt, previous?.received_at);
  return {
    cpu_percent: optionalNumber(state.cpu_percent),
    memory_used: optionalNumber(state.memory_used),
    memory_total: optionalNumber(state.memory_total),
    load_1: optionalNumber(state.load_1),
    net_rx_bps: netRxBps,
    net_tx_bps: netTxBps,
    network_in_total: netInTotal,
    network_out_total: netOutTotal,
    uptime_seconds: optionalNumber(state.uptime_seconds),
    received_at: receivedAt,
  };
}

function normalizeServiceResult(result: unknown): ServiceResult | null {
  const row = asRecord(result);
  const serviceId = asString(row.service_id);
  const createdAt = asString(row.created_at);
  if (!serviceId || !createdAt) return null;
  return {
    service_id: serviceId,
    server_id: asString(row.server_id) || null,
    status: asString(row.status),
    delay_ms: optionalNumber(row.delay_ms),
    created_at: createdAt,
  };
}

function buildServiceTracker(service: ServiceSummary, results: ServiceResult[]): ServiceTrackerItem {
  const days = buildServiceDays(results);
  const totalChecks = results.length;
  const successful = results.filter((result) => serviceResultOk(result.status)).length;
  const delays = results.map((result) => optionalNumber(result.delay_ms)).filter((value): value is number => value !== undefined);
  return {
    id: service.id,
    name: service.name || compactId(service.id),
    uptime: totalChecks ? (successful / totalChecks) * 100 : 0,
    avgDelay: delays.length ? delays.reduce((sum, value) => sum + value, 0) / delays.length : undefined,
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
    buckets.set(key, [...(buckets.get(key) ?? []), result]);
  }

  return Array.from({ length: 30 }, (_, index) => {
    const date = new Date(today);
    date.setDate(today.getDate() - (29 - index));
    const key = dayKey(date);
    const rows = buckets.get(key) ?? [];
    const success = rows.filter((result) => serviceResultOk(result.status)).length;
    const delays = rows.map((result) => optionalNumber(result.delay_ms)).filter((value): value is number => value !== undefined);
    return {
      key,
      label: date.toLocaleDateString("zh-CN"),
      uptime: rows.length ? (success / rows.length) * 100 : 0,
      avgDelay: delays.length ? delays.reduce((sum, value) => sum + value, 0) / delays.length : undefined,
      total: rows.length,
    };
  });
}

function serviceBelongsToVisibleServer(service: ServiceSummary, visibleServerIds: Set<string>): boolean {
  const ids = Array.isArray(service.server_ids) ? service.server_ids : [];
  if (service.server_id) ids.push(service.server_id);
  if (!ids.length) return true;
  return ids.some((id) => visibleServerIds.has(id));
}

function serviceResultOk(status: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

function serviceDayClass(day: ServiceDay): string {
  if (day.total === 0) return "bg-[var(--accent-bg)]";
  if (day.uptime >= 99) return "bg-[var(--accent-color)]";
  if (day.uptime >= 95) return "bg-yellow-300";
  return "bg-[var(--btn-bg)]";
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function formatRate(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  return `${formatBytes(value)}/s`;
}

function durationLabel(value?: number | null): string {
  if (value === undefined || value === null || Number.isNaN(value)) return "N/A";
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

function optionalNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

function netIoTotal(state: Record<string, unknown>, field: "bytes_recv" | "bytes_sent"): number | undefined {
  const netIo = Array.isArray(state.net_io) ? state.net_io : Array.isArray(state.network_interfaces) ? state.network_interfaces : [];
  if (!netIo.length) return undefined;
  return netIo.reduce((sum, item) => sum + (optionalNumber(asRecord(item)[field]) ?? 0), 0);
}

function rateFromTotalDelta(
  current?: number,
  previous?: number,
  currentAt?: string,
  previousAt?: string,
): number | undefined {
  if (current === undefined || previous === undefined || !currentAt || !previousAt) return undefined;
  const delta = current - previous;
  const elapsedMs = new Date(currentAt).getTime() - new Date(previousAt).getTime();
  if (delta < 0 || elapsedMs <= 0 || Number.isNaN(elapsedMs)) return undefined;
  return delta / (elapsedMs / 1000);
}

function initialServerViewMode(): ServerViewMode {
  if (typeof window === "undefined") return "cards";
  const stored = window.localStorage.getItem("xlstatus_server_view");
  return stored === "compact" ? "compact" : "cards";
}

function initialShowServices(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_show_services") === "1";
}

function initialShowServerMap(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_show_server_map") === "1";
}

function connectionLabel(conn: ConnectionState): string {
  const labels: Record<ConnectionState, string> = {
    connecting: "连接中",
    open: "已连接",
    closed: "已关闭",
    error: "错误",
  };
  return labels[conn];
}

function serverStatusLabel(status: string): string {
  const labels: Record<string, string> = {
    online: "在线",
    offline: "离线",
    revoked: "已撤销",
    down: "异常",
    degraded: "降级",
  };
  return labels[status] || status;
}

function getCookie(name: string): string | null {
  return document.cookie.split("; ").find((row) => row.startsWith(`${name}=`))?.split("=")[1] ?? null;
}

function hasBrowserSessionSignal(): boolean {
  return Boolean(getCookie("xlstatus_csrf"));
}

function buildWsUrl(): string {
  return buildWebSocketUrl("/ws/servers");
}
