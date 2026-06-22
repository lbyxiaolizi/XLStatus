"use client";

import Link from "next/link";
import { FormEvent, MouseEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  asRecord,
  asString,
  BrutalCard,
  EmptyState,
  Field,
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
  inputClass,
  responseError,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type FileEntry, type JsonObject } from "@/lib/api";

interface ServerDetail {
  id: string;
  name: string;
  remark?: string | null;
  public_note?: string | null;
  expires_at?: string | null;
  renewal_price?: string | number | null;
  price?: string | number | null;
  currency?: string | null;
  billing_cycle?: string | null;
  auto_renew?: boolean | null;
  traffic_quota_bytes?: number | string | null;
  traffic_quota_type?: string | null;
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
  hide_for_guest?: boolean | null;
  display_order?: number | null;
  status: string;
  last_seen_at?: string | null;
  last_state?: Record<string, unknown> | null;
  last_info?: Record<string, unknown> | null;
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

interface MetricSample {
  sample_at: string;
  fields_json?: Record<string, unknown> | null;
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

interface ChartPoint {
  time: number;
  value: number;
}

interface ChartSeries {
  id?: string;
  label: string;
  color: string;
  points: ChartPoint[];
  axis?: ChartAxis;
  kind?: ChartKind;
  opacity?: number;
  strokeDasharray?: string;
  strokeWidth?: number;
}

interface ProbeHistory {
  id: string;
  label: string;
  color: string;
  latency: ChartPoint[];
  loss: ChartPoint[];
  lossRate: number;
  minLatency?: number;
  maxLatency?: number;
  latestLatency?: number;
}

interface ServerMetadataForm {
  name: string;
  remark: string;
  public_note: string;
  expires_at: string;
  renewal_price: string;
  price: string;
  currency: string;
  billing_cycle: string;
  auto_renew: boolean;
  traffic_quota_bytes: string;
  traffic_quota_type: string;
  provider: string;
  region: string;
  country: string;
  city: string;
  latitude: string;
  longitude: string;
  plan: string;
  tags: string;
  accent_color: string;
  dashboard_visible: boolean;
  hide_for_guest: boolean;
  display_order: string;
}

interface PageProps {
  params: Promise<{ id: string }> | { id: string };
}

type MetricsRange = "1d" | "7d" | "30d";
type SecondaryPanel = "custom" | "files" | "ops" | null;
type ChartTab = "resources" | "network";
type ChartAxis = "left" | "right";
type ChartKind = "line" | "area";

const blankConfig = {
  report_interval_seconds: "3",
  ip_report_interval_seconds: "60",
  disable_auto_update: false,
  disable_force_update: false,
  disable_command_execute: false,
  disable_nat: false,
  disable_send_query: false,
};

const blankMetadataForm: ServerMetadataForm = {
  name: "",
  remark: "",
  public_note: "",
  expires_at: "",
  renewal_price: "",
  price: "",
  currency: "",
  billing_cycle: "",
  auto_renew: false,
  traffic_quota_bytes: "",
  traffic_quota_type: "",
  provider: "",
  region: "",
  country: "",
  city: "",
  latitude: "",
  longitude: "",
  plan: "",
  tags: "",
  accent_color: "#db2777",
  dashboard_visible: false,
  hide_for_guest: false,
  display_order: "",
};

export default function ServerDetailPage({ params }: PageProps) {
  const [serverId, setServerId] = useState("");
  const [server, setServer] = useState<ServerDetail | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [path, setPath] = useState("/");
  const [selectedPath, setSelectedPath] = useState("");
  const [writePath, setWritePath] = useState("");
  const [writeContent, setWriteContent] = useState("");
  const [configForm, setConfigForm] = useState(blankConfig);
  const [updateForm, setUpdateForm] = useState({ version: "", download_url: "", checksum: "" });
  const [loading, setLoading] = useState(true);
  const [filesLoading, setFilesLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [metadataForm, setMetadataForm] = useState<ServerMetadataForm>(blankMetadataForm);
  const [metricsRange, setMetricsRange] = useState<MetricsRange>("1d");
  const [secondaryPanel, setSecondaryPanel] = useState<SecondaryPanel>(null);
  const [chartTab, setChartTab] = useState<ChartTab>("resources");
  const [metrics, setMetrics] = useState<MetricSample[]>([]);
  const [metricsLoading, setMetricsLoading] = useState(false);
  const [probeHistories, setProbeHistories] = useState<ProbeHistory[]>([]);
  const [selectedProbeIds, setSelectedProbeIds] = useState<string[]>([]);
  const [probePeakCutEnabled, setProbePeakCutEnabled] = useState(false);
  const [showMissingInfo, setShowMissingInfo] = useState(() => initialShowMissingInfo());
  const [probeLoading, setProbeLoading] = useState(false);
  const [monitorError, setMonitorError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.resolve(params).then((value) => setServerId(value.id));
  }, [params]);

  const loadServer = useCallback(async () => {
    if (!serverId) return;
    setLoading(true);
    setError(null);
    const response = await apiClient.getServer(serverId);
    setLoading(false);
    if (response.success && response.data) {
      const detail = response.data as unknown as ServerDetail;
      setServer(detail);
      setMetadataForm({
        name: detail.name || "",
        remark: detail.remark || metadataFromRecord(detail.last_info ?? {}, ["remark", "note", "description"]) || "",
        public_note: detail.public_note || metadataFromRecord(detail.last_info ?? {}, ["public_note", "public_description"]) || "",
        expires_at: detail.expires_at || metadataFromRecord(detail.last_info ?? {}, ["expires_at", "expired_at", "expire_at", "due_at", "end_at"]) || "",
        renewal_price: detail.renewal_price === null || detail.renewal_price === undefined ? "" : String(detail.renewal_price),
        price: detail.price === null || detail.price === undefined ? metadataFromRecord(detail.last_info ?? {}, ["price", "billing_price", "monthly_price", "amount"]) || "" : String(detail.price),
        currency: detail.currency || metadataFromRecord(detail.last_info ?? {}, ["currency", "currency_code"]) || "",
        billing_cycle: detail.billing_cycle || metadataFromRecord(detail.last_info ?? {}, ["billing_cycle", "cycle", "billing_period", "period"]) || "",
        auto_renew: detail.auto_renew ?? false,
        traffic_quota_bytes: detail.traffic_quota_bytes === null || detail.traffic_quota_bytes === undefined ? "" : String(detail.traffic_quota_bytes),
        traffic_quota_type: detail.traffic_quota_type || metadataFromRecord(detail.last_info ?? {}, ["traffic_quota_type", "quota_type", "traffic_type", "bandwidth_type"]) || "",
        provider: detail.provider || metadataFromRecord(detail.last_info ?? {}, ["provider", "vendor", "datacenter", "isp"]) || "",
        region: detail.region || metadataFromRecord(detail.last_info ?? {}, ["region", "geo_region", "state", "province", "location"]) || "",
        country: detail.country || metadataFromRecord(detail.last_info ?? {}, ["country", "geo_country", "country_name"]) || "",
        city: detail.city || metadataFromRecord(detail.last_info ?? {}, ["city", "geo_city"]) || "",
        latitude: detail.latitude === null || detail.latitude === undefined ? metadataFromRecord(detail.last_info ?? {}, ["latitude", "lat", "geo_latitude"]) || "" : String(detail.latitude),
        longitude: detail.longitude === null || detail.longitude === undefined ? metadataFromRecord(detail.last_info ?? {}, ["longitude", "lon", "lng", "geo_longitude"]) || "" : String(detail.longitude),
        plan: detail.plan || metadataFromRecord(detail.last_info ?? {}, ["plan", "package", "sku", "product", "instance_type"]) || "",
        tags: Array.isArray(detail.tags) ? detail.tags.join(", ") : "",
        accent_color: detail.accent_color || "#db2777",
        dashboard_visible: detail.dashboard_visible ?? false,
        hide_for_guest: detail.hide_for_guest ?? false,
        display_order: detail.display_order === null || detail.display_order === undefined ? "" : String(detail.display_order),
      });
    } else {
      setError(responseError(response));
    }
  }, [serverId]);

  const loadFiles = useCallback(
    async (nextPath: string) => {
      if (!serverId) return;
      setFilesLoading(true);
      setError(null);
      const response = await apiClient.listServerFiles(serverId, nextPath);
      setFilesLoading(false);
      if (response.success && response.data) {
        setPath(response.data.path);
        setFiles(response.data.entries ?? []);
      } else {
        setError(responseError(response));
      }
    },
    [serverId],
  );

  const loadMetrics = useCallback(async () => {
    if (!serverId) return;
    setMetricsLoading(true);
    setMonitorError(null);
    const response = await apiClient.getServerMetrics(serverId, metricsRange);
    setMetricsLoading(false);
    if (response.success && response.data) {
      const series = asRecord(response.data.series);
      const samples = Array.isArray(series.samples) ? series.samples : [];
      setMetrics(samples.map((sample) => normalizeMetricSample(sample)).filter(Boolean) as MetricSample[]);
    } else {
      setMetrics([]);
      setMonitorError(responseError(response));
    }
  }, [metricsRange, serverId]);

  const loadProbeHistory = useCallback(async () => {
    if (!serverId) return;
    setProbeLoading(true);
    setMonitorError(null);
    const servicesResponse = await apiClient.listServices(200, 0);
    if (!servicesResponse.success || !servicesResponse.data) {
      setProbeHistories([]);
      setProbeLoading(false);
      setMonitorError(responseError(servicesResponse));
      return;
    }

    const services = ((servicesResponse.data.services as ServiceSummary[]) ?? []).filter(
      (service) => service.id && serviceBelongsToServer(service, serverId),
    );
    const histories = await Promise.all(
      services.map(async (service) => {
        const response = await apiClient.getServiceHistory(service.id, historyLimitForRange(metricsRange));
        if (!response.success || !response.data) return null;
        const results = Array.isArray(response.data.results) ? response.data.results : [];
        return {
          service,
          results: results
            .map((result) => normalizeServiceResult(result))
            .filter((result): result is ServiceResult => Boolean(result))
            .filter((result) => result.server_id === serverId),
        };
      }),
    );

    const nextHistories = histories
      .filter((item): item is { service: ServiceSummary; results: ServiceResult[] } => Boolean(item && item.results.length))
      .map((item, index) => {
        const sortedResults = item.results
          .slice()
          .sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime());
        const latency = thinPoints(
          sortedResults
            .filter((result) => typeof result.delay_ms === "number")
            .map((result) => ({ time: new Date(result.created_at).getTime(), value: result.delay_ms ?? 0 }))
            .filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value)),
        );
        const loss = thinPoints(buildProbeLossPoints(sortedResults));
        return {
          id: item.service.id,
          label: item.service.name || compactId(item.service.id),
          color: chartColors[index % chartColors.length],
          latency,
          loss,
          lossRate: averagePointValue(loss) ?? packetLossRate(item.results),
          minLatency: minPointValue(latency),
          maxLatency: maxPointValue(latency),
          latestLatency: latestDelay(item.results),
        };
      });

    setProbeHistories(nextHistories);
    setSelectedProbeIds((current) =>
      current.filter((id) => nextHistories.some((history) => history.id === id)),
    );
    setProbeLoading(false);
  }, [metricsRange, serverId]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServer();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServer]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadMetrics();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadMetrics]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadProbeHistory();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadProbeHistory]);

  const statusTone = server?.status === "online" ? "green" : server?.status === "revoked" ? "yellow" : "red";
  const currentDir = useMemo(() => parentPath(path), [path]);
  const metricCharts = useMemo(() => buildMetricCharts(metrics, server?.last_state), [metrics, server?.last_state]);
  const visibleProbeHistories = useMemo(
    () =>
      selectedProbeIds.length
        ? probeHistories.filter((history) => selectedProbeIds.includes(history.id))
        : probeHistories,
    [probeHistories, selectedProbeIds],
  );
  const probeLatencySeries = useMemo(
    () =>
      visibleProbeHistories.map((history) => ({
        id: `${history.id}:latency`,
        label: history.label,
        color: history.color,
        points: history.latency,
        axis: "left" as const,
      })),
    [visibleProbeHistories],
  );
  const probeLossSeries = useMemo(
    () =>
      visibleProbeHistories.map((history) => ({
        id: `${history.id}:loss`,
        label: history.label,
        color: history.color,
        points: history.loss,
        axis: "right" as const,
        kind: "area" as const,
        opacity: 0.18,
        strokeDasharray: "5 4",
        strokeWidth: 2,
      })),
    [visibleProbeHistories],
  );

  function openSecondaryPanel(panel: SecondaryPanel) {
    const next = secondaryPanel === panel ? null : panel;
    setSecondaryPanel(next);
    if (next === "files" && files.length === 0 && !filesLoading) {
      void loadFiles(path || "/");
    }
  }

  function toggleProbeSelection(id: string) {
    setSelectedProbeIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    );
  }

  function changeShowMissingInfo(next: boolean) {
    setShowMissingInfo(next);
    window.localStorage.setItem("xlstatus_detail_show_missing", next ? "1" : "0");
  }

  async function readSelectedFile(nextPath: string) {
    setSelectedPath(nextPath);
    const response = await apiClient.readServerFile(serverId, nextPath, "utf8");
    if (response.success && response.data) {
      setWritePath(nextPath);
      setWriteContent(response.data.content);
      setNotice(`已从 ${nextPath} 读取 ${response.data.bytes} 字节。`);
    } else {
      setError(responseError(response));
    }
  }

  async function writeFile(event: FormEvent) {
    event.preventDefault();
    if (!writePath.trim()) {
      setError("请填写写入路径。");
      return;
    }
    setSaving(true);
    const response = await apiClient.writeServerFile(serverId, {
      path: writePath.trim(),
      content: writeContent,
      encoding: "utf8",
      create_dirs: true,
    });
    setSaving(false);
    if (response.success) {
      setNotice(`已写入 ${writePath.trim()}。`);
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteEntry(entryPath: string, recursive: boolean) {
    if (!confirm(`确定删除 ${entryPath}？`)) return;
    const response = await apiClient.deleteServerFile(serverId, { path: entryPath, recursive });
    if (response.success) {
      setNotice(`已删除 ${entryPath}。`);
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function createTempUrl(kind: "download" | "upload") {
    const target = kind === "download" ? selectedPath || writePath : writePath;
    if (!target.trim()) {
      setError("请先选择或输入文件路径。");
      return;
    }
    const response =
      kind === "download"
        ? await apiClient.getServerDownloadUrl(serverId, target.trim())
        : await apiClient.getServerUploadUrl(serverId, target.trim());
    if (response.success && response.data) {
      setNotice(`${response.data.method} ${response.data.url}`);
    } else {
      setError(responseError(response));
    }
  }

  async function applyConfig(event: FormEvent) {
    event.preventDefault();
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const config: JsonObject = {
      report_interval_seconds: Number(configForm.report_interval_seconds),
      ip_report_interval_seconds: Number(configForm.ip_report_interval_seconds),
      disable_auto_update: configForm.disable_auto_update,
      disable_force_update: configForm.disable_force_update,
      disable_command_execute: configForm.disable_command_execute,
      disable_nat: configForm.disable_nat,
      disable_send_query: configForm.disable_send_query,
    };
    setSaving(true);
    const response = await apiClient.applyServerConfig(serverId, config, totpCode);
    setSaving(false);
    if (response.success) {
      setNotice("配置补丁已发送到 Agent。");
      await loadServer();
    } else {
      setError(responseError(response));
    }
  }

  async function forceUpdate(event: FormEvent) {
    event.preventDefault();
    const version = updateForm.version.trim();
    const downloadUrl = updateForm.download_url.trim();
    const checksum = updateForm.checksum.trim();
    if (!version || !downloadUrl || !checksum) {
      setError("版本、下载 URL 和 SHA-256 校验和均为必填。");
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSaving(true);
    const response = await apiClient.forceUpdateServer(serverId, {
      version,
      download_url: downloadUrl,
      checksum,
    }, totpCode);
    setSaving(false);
    if (response.success) {
      setNotice("强制更新请求已发送。");
      setUpdateForm({ version: "", download_url: "", checksum: "" });
    } else {
      setError(responseError(response));
    }
  }

  async function sensitiveTotpCode(): Promise<string | undefined | null> {
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

  async function saveServerMetadata(event: FormEvent) {
    event.preventDefault();
    if (!serverId) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSaving(true);
    const response = await apiClient.updateServer(serverId, {
      name: metadataForm.name.trim(),
      remark: metadataForm.remark.trim() || null,
      public_note: metadataForm.public_note.trim() || null,
      expires_at: metadataForm.expires_at.trim() || null,
      renewal_price: metadataForm.renewal_price.trim() || null,
      price: metadataForm.price.trim() || null,
      currency: metadataForm.currency.trim() || null,
      billing_cycle: metadataForm.billing_cycle.trim() || null,
      auto_renew: metadataForm.auto_renew,
      traffic_quota_bytes: metadataForm.traffic_quota_bytes.trim() ? Number(metadataForm.traffic_quota_bytes) : null,
      traffic_quota_type: metadataForm.traffic_quota_type.trim() || null,
      provider: metadataForm.provider.trim() || null,
      region: metadataForm.region.trim() || null,
      country: metadataForm.country.trim() || null,
      city: metadataForm.city.trim() || null,
      latitude: metadataForm.latitude.trim() ? Number(metadataForm.latitude) : null,
      longitude: metadataForm.longitude.trim() ? Number(metadataForm.longitude) : null,
      plan: metadataForm.plan.trim() || null,
      tags: splitTags(metadataForm.tags),
      accent_color: metadataForm.accent_color.trim() || null,
      dashboard_visible: metadataForm.dashboard_visible,
      hide_for_guest: metadataForm.hide_for_guest,
      display_order: metadataForm.display_order.trim() ? Number(metadataForm.display_order) : null,
    }, totpCode);
    setSaving(false);
    if (response.success && response.data) {
      const detail = response.data as unknown as ServerDetail;
      setServer(detail);
      setNotice("服务器自定义信息已保存。");
      await loadServer();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="服务器详情"
          title={server?.name || compactId(serverId)}
          detail={`最后在线 ${formatDate(server?.last_seen_at)}`}
          actions={
            <>
              <Link href="/servers" className={buttonClass("secondary")}>返回</Link>
              {server ? <StatusBadge tone={statusTone}>{serverStatusLabel(server.status)}</StatusBadge> : null}
            </>
          }
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading && !server ? <EmptyState title="正在加载服务器" /> : null}

        {server ? (
          <div className="mb-6 grid gap-5">
            <ServerOverview server={server} showMissing={showMissingInfo} onShowMissingChange={changeShowMissingInfo} />
            <SecondaryPanelNav active={secondaryPanel} onOpen={openSecondaryPanel} />
            {secondaryPanel === "custom" ? (
              <ServerMetadataEditor form={metadataForm} saving={saving} onChange={setMetadataForm} onSubmit={saveServerMetadata} />
            ) : null}
            {secondaryPanel === "files" ? (
              <FileSystemPanel
                path={path}
                currentDir={currentDir}
                files={files}
                filesLoading={filesLoading}
                selectedPath={selectedPath}
                writePath={writePath}
                writeContent={writeContent}
                saving={saving}
                onLoad={loadFiles}
                onRead={readSelectedFile}
                onDelete={deleteEntry}
                onWrite={writeFile}
                onWritePathChange={setWritePath}
                onWriteContentChange={setWriteContent}
                onTempUrl={createTempUrl}
              />
            ) : null}
            {secondaryPanel === "ops" ? (
              <AgentOpsPanel
                configForm={configForm}
                updateForm={updateForm}
                saving={saving}
                onConfigChange={setConfigForm}
                onUpdateChange={setUpdateForm}
                onApplyConfig={applyConfig}
                onForceUpdate={forceUpdate}
              />
            ) : null}
            <MonitoringCharts
              activeTab={chartTab}
              onTabChange={setChartTab}
              metricsRange={metricsRange}
              onRangeChange={setMetricsRange}
              monitorError={monitorError}
              metricCharts={metricCharts}
              metricsLoading={metricsLoading}
              probeLoading={probeLoading}
              probeHistories={probeHistories}
              selectedProbeIds={selectedProbeIds}
              probePeakCutEnabled={probePeakCutEnabled}
              probeLatencySeries={probeLatencySeries}
              probeLossSeries={probeLossSeries}
              onToggleProbe={toggleProbeSelection}
              onClearProbes={() => setSelectedProbeIds([])}
              onPeakCutChange={setProbePeakCutEnabled}
            />
          </div>
        ) : null}
      </PageShell>
    </div>
  );
}

const chartColors = [
  "var(--accent-color)",
  "var(--btn-bg)",
  "#0ea5e9",
  "#f97316",
  "#10b981",
  "#a855f7",
];

function SecondaryPanelNav({ active, onOpen }: { active: SecondaryPanel; onOpen: (panel: SecondaryPanel) => void }) {
  return (
    <div className="flex flex-wrap gap-3 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal)]">
      {(
        [
          ["custom", "自定义信息"],
          ["files", "文件系统"],
          ["ops", "Agent 设置"],
        ] as Array<[Exclude<SecondaryPanel, null>, string]>
      ).map(([panel, label]) => (
        <button
          key={panel}
          type="button"
          onClick={() => onOpen(panel)}
          className={buttonClass(active === panel ? "primary" : "secondary")}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

function FileSystemPanel({
  path,
  currentDir,
  files,
  filesLoading,
  writePath,
  writeContent,
  saving,
  onLoad,
  onRead,
  onDelete,
  onWrite,
  onWritePathChange,
  onWriteContentChange,
  onTempUrl,
}: {
  path: string;
  currentDir: string;
  files: FileEntry[];
  filesLoading: boolean;
  selectedPath: string;
  writePath: string;
  writeContent: string;
  saving: boolean;
  onLoad: (path: string) => void | Promise<void>;
  onRead: (path: string) => void | Promise<void>;
  onDelete: (path: string, recursive: boolean) => void | Promise<void>;
  onWrite: (event: FormEvent) => void | Promise<void>;
  onWritePathChange: (value: string) => void;
  onWriteContentChange: (value: string) => void;
  onTempUrl: (kind: "download" | "upload") => void | Promise<void>;
}) {
  return (
    <div className="grid gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">文件系统</h2>
            <p className="text-sm font-bold text-[var(--text-muted)]">{path}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button className={buttonClass("secondary")} onClick={() => void onLoad(currentDir)}>上级</button>
            <button className={buttonClass("secondary")} onClick={() => void onLoad(path)}>刷新</button>
          </div>
        </div>
        {filesLoading ? (
          <p className="font-bold">正在加载文件...</p>
        ) : files.length === 0 ? (
          <EmptyState title="没有返回文件" detail="Agent 可能没有暴露这个路径。" />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>名称</th>
                  <th className={thClass}>类型</th>
                  <th className={thClass}>大小</th>
                  <th className={thClass}>操作</th>
                </tr>
              </thead>
              <tbody>
                {files.map((entry) => {
                  const nextPath = joinPath(path, entry.name);
                  return (
                    <tr key={`${entry.file_type}-${entry.name}`}>
                      <td className={tdClass}>{entry.name}</td>
                      <td className={tdClass}>{entry.file_type}</td>
                      <td className={tdClass}>{formatBytes(entry.size)}</td>
                      <td className={`${tdClass} flex flex-wrap gap-2`}>
                        {entry.file_type === "dir" ? (
                          <button className={buttonClass("secondary")} onClick={() => void onLoad(nextPath)}>打开</button>
                        ) : (
                          <button className={buttonClass("primary")} onClick={() => void onRead(nextPath)}>读取</button>
                        )}
                        <button className={buttonClass("danger")} onClick={() => void onDelete(nextPath, entry.file_type === "dir")}>删除</button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </BrutalCard>

      <BrutalCard accent>
        <h2 className="mb-4 text-xl font-black uppercase">写入文件</h2>
        <form onSubmit={onWrite} className="space-y-4">
          <Field label="路径">
            <input className={inputClass} value={writePath} onChange={(event) => onWritePathChange(event.target.value)} placeholder="/tmp/xlstatus.txt" />
          </Field>
          <Field label="内容">
            <textarea className={`${textareaClass} min-h-40`} value={writeContent} onChange={(event) => onWriteContentChange(event.target.value)} />
          </Field>
          <div className="flex flex-wrap gap-2">
            <button disabled={saving} className={buttonClass("primary")}>写入文件</button>
            <button type="button" className={buttonClass("secondary")} onClick={() => void onTempUrl("download")}>下载 URL</button>
            <button type="button" className={buttonClass("secondary")} onClick={() => void onTempUrl("upload")}>上传 URL</button>
          </div>
        </form>
      </BrutalCard>
    </div>
  );
}

function AgentOpsPanel({
  configForm,
  updateForm,
  saving,
  onConfigChange,
  onUpdateChange,
  onApplyConfig,
  onForceUpdate,
}: {
  configForm: typeof blankConfig;
  updateForm: { version: string; download_url: string; checksum: string };
  saving: boolean;
  onConfigChange: (next: typeof blankConfig) => void;
  onUpdateChange: (next: { version: string; download_url: string; checksum: string }) => void;
  onApplyConfig: (event: FormEvent) => void | Promise<void>;
  onForceUpdate: (event: FormEvent) => void | Promise<void>;
}) {
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <BrutalCard>
        <h2 className="mb-4 text-xl font-black uppercase">应用配置</h2>
        <form onSubmit={onApplyConfig} className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="上报间隔">
              <input className={inputClass} value={configForm.report_interval_seconds} onChange={(e) => onConfigChange({ ...configForm, report_interval_seconds: e.target.value })} />
            </Field>
            <Field label="IP 上报间隔">
              <input className={inputClass} value={configForm.ip_report_interval_seconds} onChange={(e) => onConfigChange({ ...configForm, ip_report_interval_seconds: e.target.value })} />
            </Field>
          </div>
          <div className="grid gap-2">
            {(["disable_auto_update", "disable_force_update", "disable_command_execute", "disable_nat", "disable_send_query"] as const).map((key) => (
              <label key={key} className="flex items-center gap-2 text-sm font-black">
                <input type="checkbox" checked={configForm[key]} onChange={(e) => onConfigChange({ ...configForm, [key]: e.target.checked })} />
                {configFlagLabel(key)}
              </label>
            ))}
          </div>
          <button disabled={saving} className={buttonClass("primary")}>应用配置</button>
        </form>
      </BrutalCard>

      <BrutalCard>
        <h2 className="mb-4 text-xl font-black uppercase">发送更新</h2>
        <form onSubmit={onForceUpdate} className="space-y-4">
          <Field label="版本">
            <input className={inputClass} value={updateForm.version} onChange={(e) => onUpdateChange({ ...updateForm, version: e.target.value })} placeholder="v0.1.0-alpha.3" />
          </Field>
          <Field label="下载 URL">
            <input className={inputClass} value={updateForm.download_url} onChange={(e) => onUpdateChange({ ...updateForm, download_url: e.target.value })} placeholder="https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1.0-alpha.3/xlstatus-agent-linux-amd64.tar.gz" />
          </Field>
          <Field label="SHA-256">
            <input className={inputClass} value={updateForm.checksum} onChange={(e) => onUpdateChange({ ...updateForm, checksum: e.target.value })} placeholder="64 位十六进制摘要" />
          </Field>
          <button disabled={saving} className={buttonClass("danger")}>发送更新</button>
        </form>
      </BrutalCard>
    </div>
  );
}

function MonitoringCharts({
  activeTab,
  onTabChange,
  metricsRange,
  onRangeChange,
  monitorError,
  metricCharts,
  metricsLoading,
  probeLoading,
  probeHistories,
  selectedProbeIds,
  probePeakCutEnabled,
  probeLatencySeries,
  probeLossSeries,
  onToggleProbe,
  onClearProbes,
  onPeakCutChange,
}: {
  activeTab: ChartTab;
  onTabChange: (tab: ChartTab) => void;
  metricsRange: MetricsRange;
  onRangeChange: (range: MetricsRange) => void;
  monitorError: string | null;
  metricCharts: ReturnType<typeof buildMetricCharts>;
  metricsLoading: boolean;
  probeLoading: boolean;
  probeHistories: ProbeHistory[];
  selectedProbeIds: string[];
  probePeakCutEnabled: boolean;
  probeLatencySeries: ChartSeries[];
  probeLossSeries: ChartSeries[];
  onToggleProbe: (id: string) => void;
  onClearProbes: () => void;
  onPeakCutChange: (enabled: boolean) => void;
}) {
  const resourceCharts = [
    {
      key: "cpu",
      title: "CPU",
      value: formatPercent(metricCharts.latestCpu),
      series: [{ id: "cpu", label: "CPU", color: "var(--accent-color)", points: metricCharts.cpu }],
      maxValue: 100,
      formatValue: (value: number) => `${value.toFixed(1)}%`,
    },
    {
      key: "memory",
      title: "内存",
      value: formatPercent(metricCharts.latestMemory),
      series: [{ id: "memory", label: "内存", color: "var(--btn-bg)", points: metricCharts.memory }],
      maxValue: 100,
      formatValue: (value: number) => `${value.toFixed(1)}%`,
    },
    {
      key: "load",
      title: "负载",
      value: metricCharts.latestLoad === undefined ? "N/A" : metricCharts.latestLoad.toFixed(2),
      series: [{ id: "load", label: "负载", color: "#0ea5e9", points: metricCharts.load }],
      formatValue: (value: number) => value.toFixed(2),
    },
    {
      key: "disk",
      title: "磁盘",
      value: formatPercent(metricCharts.latestDisk),
      series: [{ id: "disk", label: "磁盘", color: "#f97316", points: metricCharts.disk }],
      maxValue: 100,
      formatValue: (value: number) => `${value.toFixed(1)}%`,
    },
    {
      key: "swap",
      title: "Swap",
      value: formatPercent(metricCharts.latestSwap),
      series: [{ id: "swap", label: "Swap", color: "#a855f7", points: metricCharts.swap }],
      maxValue: 100,
      formatValue: (value: number) => `${value.toFixed(1)}%`,
    },
    {
      key: "process",
      title: "进程",
      value: metricCharts.latestProcess === undefined ? "N/A" : String(Math.round(metricCharts.latestProcess)),
      series: [{ id: "process", label: "进程", color: "#10b981", points: metricCharts.process }],
      formatValue: (value: number) => String(Math.round(value)),
    },
    {
      key: "connection",
      title: "连接",
      value: `TCP ${metricCharts.latestTcp === undefined ? "N/A" : Math.round(metricCharts.latestTcp)} / UDP ${metricCharts.latestUdp === undefined ? "N/A" : Math.round(metricCharts.latestUdp)}`,
      series: [
        { id: "tcp", label: "TCP", color: "var(--accent-color)", points: metricCharts.tcp },
        { id: "udp", label: "UDP", color: "var(--border-color)", points: metricCharts.udp },
      ],
      formatValue: (value: number) => String(Math.round(value)),
    },
    {
      key: "temperature",
      title: "温度",
      value: metricCharts.latestTemperature === undefined ? "N/A" : `${metricCharts.latestTemperature.toFixed(1)} °C`,
      series: [{ id: "temperature", label: "温度", color: "#dc2626", points: metricCharts.temperature }],
      formatValue: (value: number) => `${value.toFixed(1)} °C`,
    },
    {
      key: "gpu",
      title: "GPU",
      value: formatPercent(metricCharts.latestGpu),
      series: [{ id: "gpu", label: "GPU", color: "#ef4444", points: metricCharts.gpu }],
      maxValue: 100,
      formatValue: (value: number) => `${value.toFixed(1)}%`,
    },
  ];
  const visibleResourceCharts = metricsLoading ? resourceCharts : resourceCharts.filter((chart) => chartHasData(chart.series));

  return (
    <section className="border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-4 flex flex-wrap items-end justify-between gap-3 border-b-4 border-black pb-3">
        <h2 className="text-2xl font-black uppercase">监控图表</h2>
        <div className="flex flex-wrap gap-2">
          {(["1d", "7d", "30d"] as const).map((range) => (
            <button
              key={range}
              type="button"
              onClick={() => onRangeChange(range)}
              className={buttonClass(metricsRange === range ? "primary" : "secondary")}
            >
              {range}
            </button>
          ))}
        </div>
      </div>
      <InlineError message={monitorError} />
      <div className="mb-4 flex items-end gap-1 overflow-x-auto border-b-4 border-black">
        {(
          [
            ["resources", "资源"],
            ["network", "网络 / 探测"],
          ] as Array<[ChartTab, string]>
        ).map(([tab, label]) => (
          <button
            key={tab}
            type="button"
            onClick={() => onTabChange(tab)}
            className={`border-2 border-b-0 border-black px-4 py-2 text-sm font-black uppercase shadow-[var(--shadow-brutal-sm)] ${
              activeTab === tab
                ? "translate-y-1 bg-[var(--bg-card)] text-[var(--text-main)]"
                : "bg-[var(--btn-bg)] text-[var(--btn-text)]"
            }`}
          >
            {label}
          </button>
        ))}
      </div>
      {activeTab === "resources" ? (
        visibleResourceCharts.length ? (
          <div className="grid gap-4 lg:grid-cols-2">
            {visibleResourceCharts.map((chart) => (
              <MetricChartCard
                key={chart.key}
                title={chart.title}
                value={chart.value}
                loading={metricsLoading}
                series={chart.series}
                maxValue={chart.maxValue}
                formatValue={chart.formatValue}
              />
            ))}
          </div>
        ) : (
          <EmptyState title="暂无资源图表" detail="服务器暂未上报可绘制的资源指标。" />
        )
      ) : null}
      {activeTab === "network" ? (
        <NetworkProbePanel
          metricsLoading={metricsLoading}
          metricCharts={metricCharts}
          histories={probeHistories}
          selectedIds={selectedProbeIds}
          probeLoading={probeLoading}
          peakCutEnabled={probePeakCutEnabled}
          latencySeries={probeLatencySeries}
          lossSeries={probeLossSeries}
          onToggle={onToggleProbe}
          onClear={onClearProbes}
          onPeakCutChange={onPeakCutChange}
        />
      ) : null}
    </section>
  );
}

function NetworkProbePanel({
  metricsLoading,
  metricCharts,
  histories,
  selectedIds,
  probeLoading,
  peakCutEnabled,
  latencySeries,
  lossSeries,
  onToggle,
  onClear,
  onPeakCutChange,
}: {
  metricsLoading: boolean;
  metricCharts: ReturnType<typeof buildMetricCharts>;
  histories: ProbeHistory[];
  selectedIds: string[];
  probeLoading: boolean;
  peakCutEnabled: boolean;
  latencySeries: ChartSeries[];
  lossSeries: ChartSeries[];
  onToggle: (id: string) => void;
  onClear: () => void;
  onPeakCutChange: (enabled: boolean) => void;
}) {
  const visibleHistories = selectedIds.length
    ? histories.filter((history) => selectedIds.includes(history.id))
    : histories;
  const displayedLatencySeries = peakCutEnabled
    ? latencySeries.map((item) => ({ ...item, points: smoothPeakPoints(item.points) }))
    : latencySeries;
  const networkSeries = [
    { id: "tx", label: "上传", color: "var(--accent-color)", points: metricCharts.tx },
    { id: "rx", label: "下载", color: "var(--border-color)", points: metricCharts.rx },
  ];
  const hasNetworkData = chartHasData(networkSeries);
  const hasProbeData = chartHasData(displayedLatencySeries) || chartHasData(lossSeries);
  const showNetworkChart = metricsLoading || hasNetworkData;
  const showProbeChart = probeLoading || hasProbeData;

  return (
    <div className="grid gap-4">
      {showNetworkChart ? (
        <MetricChartCard
          title="网络"
          value={`↑ ${formatRate(metricCharts.latestTx)} / ↓ ${formatRate(metricCharts.latestRx)}`}
          loading={metricsLoading}
          series={networkSeries}
          formatValue={formatRate}
        />
      ) : null}
      {histories.length ? (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap gap-2">
            {histories.map((history) => {
              const active = selectedIds.length === 0 || selectedIds.includes(history.id);
              return (
                <button
                  key={history.id}
                  type="button"
                  onClick={() => onToggle(history.id)}
                  className={`inline-flex min-h-12 items-center gap-2 border-2 border-black px-3 py-2 text-left text-xs font-black shadow-[var(--shadow-brutal-sm)] ${
                    active
                      ? "bg-[var(--bg-card)] text-[var(--text-main)]"
                      : "bg-[var(--btn-bg)] text-[var(--btn-text)]"
                  }`}
                >
                  <span className="h-3 w-3 shrink-0 border-2 border-black" style={{ background: history.color }} />
                  <span className="grid gap-0.5">
                    <span>{history.label}</span>
                    <span className="text-[11px] text-[var(--text-muted)]">
                      {history.latestLatency === undefined ? "N/A" : formatMs(Math.round(history.latestLatency))}
                      {" / "}
                      ↓{history.minLatency === undefined ? "N/A" : Math.round(history.minLatency)}
                      {" "}
                      ↑{history.maxLatency === undefined ? "N/A" : Math.round(history.maxLatency)}
                      {" / "}
                      {formatPercent(history.lossRate)}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {selectedIds.length ? (
              <button type="button" className={buttonClass("secondary")} onClick={onClear}>
                清空选择 ({selectedIds.length})
              </button>
            ) : null}
            <label className="inline-flex min-h-10 items-center gap-2 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
              <input
                type="checkbox"
                checked={peakCutEnabled}
                onChange={(event) => onPeakCutChange(event.target.checked)}
                className="h-4 w-4 accent-black"
              />
              削峰
            </label>
          </div>
        </div>
      ) : null}
      {showProbeChart ? (
        <ProbeOverlayChartCard
          loading={probeLoading}
          latencySeries={displayedLatencySeries}
          lossSeries={lossSeries}
          latestLatency={latestProbeLatency(latencySeries)}
          averageLoss={averageLossRate(visibleHistories)}
        />
      ) : null}
      {!showNetworkChart && !showProbeChart ? (
        <EmptyState title="暂无网络 / 探测图表" detail="服务器暂未上报网络历史，且没有关联探测历史。" />
      ) : null}
    </div>
  );
}

function ServerOverview({
  server,
  showMissing,
  onShowMissingChange,
}: {
  server: ServerDetail;
  showMissing: boolean;
  onShowMissingChange: (next: boolean) => void;
}) {
  const info = server.last_info ?? {};
  const state = server.last_state ?? {};
  const remark = server.remark || metadataFromRecord(info, ["remark", "note", "description"]);
  const publicNote = server.public_note || metadataFromRecord(info, ["public_note", "public_description"]);
  const expiresAt = server.expires_at || metadataFromRecord(info, ["expires_at", "expired_at", "expire_at", "due_at", "end_at"]);
  const renewalPrice = server.renewal_price ?? metadataFromRecord(info, ["renewal_price", "renew_price", "renewal", "price", "billing_price"]);
  const price = server.price ?? metadataFromRecord(info, ["price", "billing_price", "monthly_price", "amount"]);
  const currency = server.currency || metadataFromRecord(info, ["currency", "currency_code"]);
  const billingCycle = server.billing_cycle || metadataFromRecord(info, ["billing_cycle", "cycle", "billing_period", "period"]);
  const trafficQuotaType = server.traffic_quota_type || metadataFromRecord(info, ["traffic_quota_type", "quota_type", "traffic_type", "bandwidth_type"]);
  const trafficQuotaBytes = server.traffic_quota_bytes ?? metadataFromRecord(info, ["traffic_quota_bytes", "traffic_quota", "quota_bytes", "bandwidth_quota_bytes", "monthly_traffic_bytes"]);
  const provider = server.provider || metadataFromRecord(info, ["provider", "vendor", "datacenter", "isp"]);
  const region = locationLabel(server, info);
  const plan = server.plan || metadataFromRecord(info, ["plan", "package", "sku", "product", "instance_type"]);
  const tags = Array.isArray(server.tags) ? server.tags.filter(Boolean) : [];
  const platformDetails = parsePlatformDetails(info);
  const platform = joinLabels([platformDetails.os, platformDetails.version]);
  const load = [optionalNumber(state.load_1), optionalNumber(state.load_5), optionalNumber(state.load_15)]
    .map((value) => (value === undefined ? null : value.toFixed(2)))
    .filter(Boolean)
    .join(" / ");
  const tcp = optionalNumber(state.tcp_connections);
  const udp = optionalNumber(state.udp_connections);
  const tiles = filterInfoTiles(
    [
      { label: "私有备注", value: remark, wide: true },
      { label: "公开说明", value: publicNote, wide: true },
      { label: "供应商", value: provider },
      { label: "地区", value: region },
      { label: "套餐", value: plan },
      { label: "到期", value: expiresAt ? formatDate(expiresAt) : null, danger: isExpired(expiresAt) },
      {
        label: "续费",
        value: renewalPrice === null || renewalPrice === undefined || renewalPrice === "" ? null : String(renewalPrice),
      },
      { label: "价格", value: billingLabel(price, currency, billingCycle) },
      { label: "自动续费", value: booleanLabel(server.auto_renew) },
      { label: "流量额度", value: trafficQuotaLabel(trafficQuotaBytes, trafficQuotaType) },
      { label: "公开访问", value: publicVisibilityLabel(server) },
    ],
    showMissing,
  );
  const hostRows = filterInfoRows(
    [
      ["主机名", stringValue(info.hostname)],
      ["系统", platform],
      ["内核", stringValue(info.kernel_version)],
      ["架构", stringValue(info.arch)],
      ["Agent", platformDetails.agent],
    ],
    showMissing,
  );
  const resourceRows = filterInfoRows(
    [
      ["CPU 核心", numberLabel(info.cpu_cores)],
      ["内存", resourceBytes(state.memory_used, state.memory_total, info.total_memory)],
      ["Swap", resourceBytes(state.swap_used, state.swap_total, info.total_swap)],
      ["磁盘", diskLabel(state, info)],
    ],
    showMissing,
  );
  const runtimeRows = filterInfoRows(
    [
      ["运行时间", durationLabel(state.uptime_seconds)],
      ["负载", load],
      ["进程", numberLabel(state.process_count)],
      ["连接", tcp === undefined && udp === undefined ? "" : `TCP ${tcp ?? 0} / UDP ${udp ?? 0}`],
      ["上报", platformDetails.report],
      ["限制", platformDetails.flags],
    ],
    showMissing,
  );

  return (
    <section className="border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="break-words text-2xl font-black uppercase">{server.name}</h2>
          <p className="mt-1 break-all font-mono text-xs font-bold text-[var(--text-muted)]">{server.id}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="inline-flex min-h-10 items-center gap-2 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
            <input
              type="checkbox"
              checked={showMissing}
              onChange={(event) => onShowMissingChange(event.target.checked)}
              className="h-4 w-4 accent-black"
            />
            显示缺失
          </label>
          <StatusBadge tone={server.status === "online" ? "green" : server.status === "revoked" ? "yellow" : "red"}>
            {serverStatusLabel(server.status)}
          </StatusBadge>
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-6">
        {tiles.map((tile) => (
          <InfoTile
            key={tile.label}
            label={tile.label}
            value={displayInfoValue(tile.value)}
            danger={tile.danger}
            wide={tile.wide}
          />
        ))}
      </div>

      {tags.length ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {tags.map((tag) => (
            <span key={tag} className="border-2 border-black bg-[var(--bg-card)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
              {tag}
            </span>
          ))}
        </div>
      ) : null}

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        {hostRows.length || showMissing ? <InfoGroup title="主机" rows={hostRows} /> : null}
        {resourceRows.length || showMissing ? <InfoGroup title="资源" rows={resourceRows} /> : null}
        {runtimeRows.length || showMissing ? <InfoGroup title="运行/网络" rows={runtimeRows} /> : null}
      </div>
    </section>
  );
}

function InfoGroup({ title, rows }: { title: string; rows: Array<[string, string]> }) {
  return (
    <div className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
      <h3 className="mb-3 text-sm font-black uppercase">{title}</h3>
      <dl className="grid gap-2">
        {rows.map(([label, value]) => (
          <div key={label} className="grid grid-cols-[5.5rem_minmax(0,1fr)] gap-2 border-t-2 border-black pt-2 first:border-t-0 first:pt-0">
            <dt className="text-xs font-black text-[var(--text-muted)]">{label}</dt>
            <dd className="min-w-0 break-words text-sm font-black">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function InfoTile({ label, value, danger = false, wide = false }: { label: string; value: string; danger?: boolean; wide?: boolean }) {
  return (
    <div className={`border-2 border-black ${danger ? "bg-[var(--btn-bg)] text-[var(--btn-text)]" : "bg-[var(--bg-card)] text-[var(--text-main)]"} p-3 shadow-[var(--shadow-brutal-sm)] ${wide ? "sm:col-span-2" : ""}`}>
      <div className="text-xs font-black text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words text-sm font-black">{value}</div>
    </div>
  );
}

function filterInfoTiles<T extends { value?: string | null }>(tiles: T[], showMissing: boolean): T[] {
  return showMissing ? tiles : tiles.filter((tile) => !isMissingInfoValue(tile.value));
}

function filterInfoRows(rows: Array<[string, string | null | undefined]>, showMissing: boolean): Array<[string, string]> {
  return rows
    .filter(([, value]) => showMissing || !isMissingInfoValue(value))
    .map(([label, value]) => [label, displayInfoValue(value)]);
}

function displayInfoValue(value?: string | null): string {
  return isMissingInfoValue(value) ? "N/A" : String(value);
}

function isMissingInfoValue(value?: string | null): boolean {
  const normalized = String(value ?? "").trim();
  return !normalized || normalized === "N/A" || normalized === "暂无数据";
}

function billingLabel(
  price?: string | number | null,
  currency?: string | null,
  cycle?: string | null,
): string | null {
  const parts = [price === null || price === undefined ? "" : String(price), currency, cycle]
    .map((part) => String(part ?? "").trim())
    .filter(Boolean);
  return parts.length ? parts.join(" / ") : null;
}

function booleanLabel(value?: boolean | null): string | null {
  if (value === undefined || value === null) return null;
  return value ? "是" : "否";
}

function trafficQuotaLabel(value?: string | number | null, type?: string | null): string | null {
  const quota = optionalNumber(value);
  const quotaLabel = quota === undefined ? "" : formatBytes(quota);
  const typeLabel = String(type ?? "").trim();
  const parts = [quotaLabel, typeLabel].filter(Boolean);
  return parts.length ? parts.join(" / ") : null;
}

function publicVisibilityLabel(server: ServerDetail): string {
  if (server.hide_for_guest) return "游客隐藏";
  if (server.dashboard_visible !== true) return "状态页隐藏";
  return "公开显示";
}

function ServerMetadataEditor({
  form,
  saving,
  onChange,
  onSubmit,
}: {
  form: ServerMetadataForm;
  saving: boolean;
  onChange: (next: ServerMetadataForm) => void;
  onSubmit: (event: FormEvent) => void;
}) {
  return (
    <BrutalCard>
      <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-xl font-black uppercase">自定义信息</h2>
        </div>
      </div>
      <form onSubmit={onSubmit} className="space-y-4">
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label="名称">
            <input className={inputClass} value={form.name} onChange={(event) => onChange({ ...form, name: event.target.value })} required />
          </Field>
          <Field label="供应商">
            <input className={inputClass} value={form.provider} onChange={(event) => onChange({ ...form, provider: event.target.value })} placeholder="Wawo" />
          </Field>
          <Field label="地区">
            <input className={inputClass} value={form.region} onChange={(event) => onChange({ ...form, region: event.target.value })} placeholder="HK" />
          </Field>
          <Field label="套餐">
            <input className={inputClass} value={form.plan} onChange={(event) => onChange({ ...form, plan: event.target.value })} placeholder="Dedicated" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label="国家">
            <input className={inputClass} value={form.country} onChange={(event) => onChange({ ...form, country: event.target.value })} placeholder="Hong Kong" />
          </Field>
          <Field label="城市">
            <input className={inputClass} value={form.city} onChange={(event) => onChange({ ...form, city: event.target.value })} placeholder="Hong Kong" />
          </Field>
          <Field label="纬度">
            <input type="number" min="-90" max="90" step="0.000001" className={inputClass} value={form.latitude} onChange={(event) => onChange({ ...form, latitude: event.target.value })} placeholder="22.3193" />
          </Field>
          <Field label="经度">
            <input type="number" min="-180" max="180" step="0.000001" className={inputClass} value={form.longitude} onChange={(event) => onChange({ ...form, longitude: event.target.value })} placeholder="114.1694" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label="备注">
            <input className={inputClass} value={form.remark} onChange={(event) => onChange({ ...form, remark: event.target.value })} placeholder="线路、套餐、用途" />
          </Field>
          <Field label="公开说明">
            <input className={inputClass} value={form.public_note} onChange={(event) => onChange({ ...form, public_note: event.target.value })} placeholder="状态页展示文案" />
          </Field>
          <Field label="到期时间">
            <input className={inputClass} value={form.expires_at} onChange={(event) => onChange({ ...form, expires_at: event.target.value })} placeholder="2026-12-31" />
          </Field>
          <Field label="续费价格">
            <input className={inputClass} value={form.renewal_price} onChange={(event) => onChange({ ...form, renewal_price: event.target.value })} placeholder="$12/月" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label="价格">
            <input className={inputClass} value={form.price} onChange={(event) => onChange({ ...form, price: event.target.value })} placeholder="12" />
          </Field>
          <Field label="币种">
            <input className={inputClass} value={form.currency} onChange={(event) => onChange({ ...form, currency: event.target.value })} placeholder="USD" />
          </Field>
          <Field label="账单周期">
            <input className={inputClass} value={form.billing_cycle} onChange={(event) => onChange({ ...form, billing_cycle: event.target.value })} placeholder="monthly" />
          </Field>
          <Field label="流量额度 Bytes">
            <input type="number" min="0" step="1" className={inputClass} value={form.traffic_quota_bytes} onChange={(event) => onChange({ ...form, traffic_quota_bytes: event.target.value })} placeholder="1099511627776" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label="额度类型">
            <input className={inputClass} value={form.traffic_quota_type} onChange={(event) => onChange({ ...form, traffic_quota_type: event.target.value })} placeholder="monthly / total" />
          </Field>
          <Field label="排序">
            <input className={inputClass} value={form.display_order} onChange={(event) => onChange({ ...form, display_order: event.target.value })} placeholder="10" />
          </Field>
          <label className="flex min-h-12 items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input type="checkbox" checked={form.auto_renew} onChange={(event) => onChange({ ...form, auto_renew: event.target.checked })} />
            自动续费
          </label>
          <label className="flex min-h-12 items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input type="checkbox" checked={form.hide_for_guest} onChange={(event) => onChange({ ...form, hide_for_guest: event.target.checked })} />
            游客隐藏
          </label>
        </div>
        <div className="grid gap-4 md:grid-cols-[minmax(0,1fr)_12rem_12rem] md:items-end">
          <Field label="标签">
            <input className={inputClass} value={form.tags} onChange={(event) => onChange({ ...form, tags: event.target.value })} placeholder="生产, 香港, 高防" />
          </Field>
          <Field label="强调色">
            <div className="grid grid-cols-[3.5rem_minmax(0,1fr)] gap-2">
              <input
                type="color"
                className="h-12 w-full border-2 border-black bg-[var(--bg-card)] p-1 shadow-[var(--shadow-brutal-sm)]"
                value={isHexColor(form.accent_color) ? form.accent_color : "#db2777"}
                onChange={(event) => onChange({ ...form, accent_color: event.target.value })}
              />
              <input className={inputClass} value={form.accent_color} onChange={(event) => onChange({ ...form, accent_color: event.target.value })} placeholder="#db2777" />
            </div>
          </Field>
          <label className="flex min-h-12 items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input type="checkbox" checked={form.dashboard_visible} onChange={(event) => onChange({ ...form, dashboard_visible: event.target.checked })} />
            公开状态页显示
          </label>
        </div>
        <button type="submit" disabled={saving} className={buttonClass("primary")}>
          保存
        </button>
      </form>
    </BrutalCard>
  );
}

function MetricChartCard({
  title,
  value,
  loading,
  series,
  maxValue,
  formatValue,
}: {
  title: string;
  value: string;
  loading: boolean;
  series: ChartSeries[];
  maxValue?: number;
  formatValue: (value: number) => string;
}) {
  const hasData = series.some((item) => item.points.length > 0);

  return (
    <section className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-xl font-black uppercase">{title}</h3>
          <p className="mt-1 text-sm font-black text-[var(--text-muted)]">{value}</p>
        </div>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {loading ? "加载中" : "LIVE"}
        </span>
      </div>
      {loading ? (
        <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          正在加载图表...
        </div>
      ) : hasData ? (
        <MiniLineChart series={series} maxValue={maxValue} formatValue={formatValue} />
      ) : (
        <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          暂无历史数据
        </div>
      )}
      {hasData ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {series.map((item) => (
            <span key={item.id ?? item.label} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
              <span className="h-2.5 w-2.5 border-2 border-black" style={{ background: item.color }} />
              {item.label}
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function chartHasData(series: ChartSeries[]): boolean {
  return series.some((item) => item.points.some((point) => Number.isFinite(point.time) && Number.isFinite(point.value)));
}

function ProbeOverlayChartCard({
  loading,
  latencySeries,
  lossSeries,
  latestLatency,
  averageLoss,
}: {
  loading: boolean;
  latencySeries: ChartSeries[];
  lossSeries: ChartSeries[];
  latestLatency: string;
  averageLoss?: number;
}) {
  const hasData = latencySeries.some((item) => item.points.length > 0) || lossSeries.some((item) => item.points.length > 0);

  return (
    <section className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-xl font-black uppercase">探测延迟 / 丢包率</h3>
          <p className="mt-1 text-sm font-black text-[var(--text-muted)]">
            {latestLatency} / {formatPercent(averageLoss)}
          </p>
        </div>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {loading ? "加载中" : "LIVE"}
        </span>
      </div>
      {loading ? (
        <div className="flex h-56 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          正在加载图表...
        </div>
      ) : hasData ? (
        <MiniDualAxisChart latencySeries={latencySeries} lossSeries={lossSeries} />
      ) : (
        <div className="flex h-56 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          暂无探测历史
        </div>
      )}
      {hasData ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {latencySeries.map((item) => (
            <span key={item.id ?? item.label} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
              <span className="h-2.5 w-2.5 border-2 border-black" style={{ background: item.color }} />
              {item.label} 延迟
            </span>
          ))}
          {lossSeries.map((item) => (
            <span key={`${item.id ?? item.label}-loss`} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
              <span className="h-2.5 w-2.5 border-2 border-black bg-[#facc15]" />
              {item.label} 丢包
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function MiniLineChart({ series, maxValue, formatValue }: { series: ChartSeries[]; maxValue?: number; formatValue: (value: number) => string }) {
  const [hover, setHover] = useState<{
    x: number;
    time: number;
    values: Array<{ label: string; color: string; value: number }>;
  } | null>(null);
  const width = 640;
  const height = 190;
  const padding = { top: 14, right: 16, bottom: 30, left: 46 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const allPoints = series.flatMap((item) => item.points).filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
  if (allPoints.length === 0) {
    return (
      <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
        暂无历史数据
      </div>
    );
  }
  const timeValues = Array.from(new Set(allPoints.map((point) => point.time))).sort((a, b) => a - b);
  const minTime = timeValues[0];
  const maxTime = timeValues[timeValues.length - 1];
  const maxSeriesValue = Math.max(...allPoints.map((point) => point.value), 1);
  const yMax = maxValue ?? Math.ceil(maxSeriesValue * 1.2);
  const timeSpan = Math.max(maxTime - minTime, 1);

  function x(time: number): number {
    if (allPoints.length === 1) return padding.left + plotWidth / 2;
    return padding.left + ((time - minTime) / timeSpan) * plotWidth;
  }

  function y(value: number): number {
    const clamped = Math.max(0, Math.min(yMax, value));
    return padding.top + plotHeight - (clamped / yMax) * plotHeight;
  }

  function handleMove(event: MouseEvent<SVGSVGElement>) {
    const localX = svgPointerX(event, width);
    if (localX < padding.left || localX > width - padding.right) {
      setHover(null);
      return;
    }
    const targetTime = minTime + ((localX - padding.left) / plotWidth) * timeSpan;
    const hoverTime = nearestValue(timeValues, targetTime);
    const values = series
      .map((item) => {
        const point = nearestPoint(item.points, hoverTime);
        return point ? { label: item.label, color: item.color, value: point.value, time: point.time } : null;
      })
      .filter((item): item is { label: string; color: string; value: number; time: number } => Boolean(item));
    if (!values.length) {
      setHover(null);
      return;
    }
    setHover({
      x: x(hoverTime),
      time: hoverTime,
      values: values.map(({ label, color, value }) => ({ label, color, value })),
    });
  }

  const tooltipWidth = 210;
  const tooltipHeight = hover ? 28 + hover.values.length * 20 : 0;
  const tooltipX = hover ? Math.min(Math.max(hover.x + 12, padding.left), width - padding.right - tooltipWidth) : 0;
  const tooltipY = hover ? Math.max(padding.top + 4, padding.top + plotHeight - tooltipHeight - 8) : 0;

  return (
    <svg
      className="h-48 w-full border-2 border-black bg-[var(--bg-page)]"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="指标趋势图"
      onMouseMove={handleMove}
      onMouseLeave={() => setHover(null)}
    >
      {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
        const lineY = padding.top + plotHeight * ratio;
        return (
          <line
            key={ratio}
            x1={padding.left}
            x2={width - padding.right}
            y1={lineY}
            y2={lineY}
            stroke="var(--border-color)"
            strokeOpacity="0.18"
            strokeWidth="2"
          />
        );
      })}
      <text x={8} y={padding.top + 10} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatValue(yMax)}
      </text>
      <text x={8} y={padding.top + plotHeight} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatValue(0)}
      </text>
      <text x={padding.left} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatChartTime(minTime)}
      </text>
      <text x={width - padding.right} y={height - 9} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
        {formatChartTime(maxTime)}
      </text>
      {series.map((item) => {
        const points = item.points.filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
        const path = points.map((point, index) => `${index === 0 ? "M" : "L"} ${x(point.time).toFixed(2)} ${y(point.value).toFixed(2)}`).join(" ");
        return (
          <g key={item.id ?? item.label}>
            <path
              d={path}
              fill="none"
              stroke={item.color}
              strokeLinecap="square"
              strokeLinejoin="round"
              strokeWidth="4"
            />
            {points.length === 1 ? (
              <circle cx={x(points[0].time)} cy={y(points[0].value)} r="5" fill={item.color} stroke="var(--border-color)" strokeWidth="2" />
            ) : null}
          </g>
        );
      })}
      {hover ? (
        <g>
          <line x1={hover.x} x2={hover.x} y1={padding.top} y2={padding.top + plotHeight} stroke="var(--border-color)" strokeDasharray="6 5" strokeWidth="2" />
          <rect x={tooltipX} y={tooltipY} width={tooltipWidth} height={tooltipHeight} fill="var(--bg-card)" stroke="var(--border-color)" strokeWidth="2" />
          <text x={tooltipX + 10} y={tooltipY + 18} fill="var(--text-main)" fontSize="12" fontWeight="900">
            {formatChartTime(hover.time)}
          </text>
          {hover.values.map((item, index) => (
            <g key={item.label} transform={`translate(${tooltipX + 10}, ${tooltipY + 36 + index * 20})`}>
              <rect width="9" height="9" y="-8" fill={item.color} stroke="var(--border-color)" strokeWidth="1.5" />
              <text x="16" y="0" fill="var(--text-main)" fontSize="12" fontWeight="900">
                {item.label}: {formatValue(item.value)}
              </text>
            </g>
          ))}
        </g>
      ) : null}
    </svg>
  );
}

function MiniDualAxisChart({ latencySeries, lossSeries }: { latencySeries: ChartSeries[]; lossSeries: ChartSeries[] }) {
  const [hover, setHover] = useState<{
    x: number;
    time: number;
    values: Array<{ label: string; color: string; value: number; kind: "latency" | "loss" }>;
  } | null>(null);
  const width = 760;
  const height = 240;
  const padding = { top: 16, right: 52, bottom: 34, left: 52 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const allPoints = [...latencySeries, ...lossSeries]
    .flatMap((item) => item.points)
    .filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
  const latencyPoints = latencySeries.flatMap((item) => item.points).filter((point) => Number.isFinite(point.value));
  if (allPoints.length === 0) {
    return (
      <div className="flex h-56 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
        暂无探测历史
      </div>
    );
  }

  const timeValues = Array.from(new Set(allPoints.map((point) => point.time))).sort((a, b) => a - b);
  const minTime = timeValues[0];
  const maxTime = timeValues[timeValues.length - 1];
  const maxLatency = Math.max(...latencyPoints.map((point) => point.value), 1);
  const latencyMax = Math.max(1, Math.ceil(maxLatency * 1.2));
  const lossMax = 100;
  const timeSpan = Math.max(maxTime - minTime, 1);

  function x(time: number): number {
    if (allPoints.length === 1) return padding.left + plotWidth / 2;
    return padding.left + ((time - minTime) / timeSpan) * plotWidth;
  }

  function yLatency(value: number): number {
    const clamped = Math.max(0, Math.min(latencyMax, value));
    return padding.top + plotHeight - (clamped / latencyMax) * plotHeight;
  }

  function yLoss(value: number): number {
    const clamped = Math.max(0, Math.min(lossMax, value));
    return padding.top + plotHeight - (clamped / lossMax) * plotHeight;
  }

  function handleMove(event: MouseEvent<SVGSVGElement>) {
    const localX = svgPointerX(event, width);
    if (localX < padding.left || localX > width - padding.right) {
      setHover(null);
      return;
    }
    const targetTime = minTime + ((localX - padding.left) / plotWidth) * timeSpan;
    const hoverTime = nearestValue(timeValues, targetTime);
    const values = [
      ...latencySeries.map((item) => {
        const point = nearestPoint(item.points, hoverTime);
        return point ? { label: `${item.label} 延迟`, color: item.color, value: point.value, time: point.time, kind: "latency" as const } : null;
      }),
      ...lossSeries.map((item) => {
        const point = nearestPoint(item.points, hoverTime);
        return point ? { label: `${item.label} 丢包`, color: "#facc15", value: point.value, time: point.time, kind: "loss" as const } : null;
      }),
    ].filter((item): item is { label: string; color: string; value: number; time: number; kind: "latency" | "loss" } => Boolean(item));
    if (!values.length) {
      setHover(null);
      return;
    }
    setHover({
      x: x(hoverTime),
      time: hoverTime,
      values: values.map(({ label, color, value, kind }) => ({ label, color, value, kind })),
    });
  }

  const tooltipWidth = 240;
  const tooltipHeight = hover ? 28 + hover.values.length * 20 : 0;
  const tooltipX = hover ? Math.min(Math.max(hover.x + 12, padding.left), width - padding.right - tooltipWidth) : 0;
  const tooltipY = hover ? Math.max(padding.top + 4, padding.top + plotHeight - tooltipHeight - 8) : 0;
  const lossBaseline = padding.top + plotHeight;

  return (
    <svg
      className="h-60 w-full border-2 border-black bg-[var(--bg-page)]"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="探测延迟与丢包率趋势图"
      onMouseMove={handleMove}
      onMouseLeave={() => setHover(null)}
    >
      {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
        const lineY = padding.top + plotHeight * ratio;
        return (
          <line
            key={ratio}
            x1={padding.left}
            x2={width - padding.right}
            y1={lineY}
            y2={lineY}
            stroke="var(--border-color)"
            strokeOpacity="0.18"
            strokeWidth="2"
          />
        );
      })}
      <text x={8} y={padding.top + 10} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatMs(latencyMax)}
      </text>
      <text x={8} y={padding.top + plotHeight} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        0 ms
      </text>
      <text x={width - 8} y={padding.top + 10} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
        100%
      </text>
      <text x={width - 8} y={padding.top + plotHeight} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
        0%
      </text>
      <text x={padding.left} y={height - 10} fill="var(--text-muted)" fontSize="12" fontWeight="900">
        {formatChartTime(minTime)}
      </text>
      <text x={width - padding.right} y={height - 10} fill="var(--text-muted)" fontSize="12" fontWeight="900" textAnchor="end">
        {formatChartTime(maxTime)}
      </text>
      {lossSeries.map((item) => {
        const points = item.points.filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
        const linePath = points.map((point, index) => `${index === 0 ? "M" : "L"} ${x(point.time).toFixed(2)} ${yLoss(point.value).toFixed(2)}`).join(" ");
        const areaPath = points.length
          ? `${linePath} L ${x(points[points.length - 1].time).toFixed(2)} ${lossBaseline.toFixed(2)} L ${x(points[0].time).toFixed(2)} ${lossBaseline.toFixed(2)} Z`
          : "";
        return (
          <g key={`${item.id ?? item.label}-loss`}>
            {areaPath ? <path d={areaPath} fill="#facc15" fillOpacity="0.14" stroke="none" /> : null}
            <path d={linePath} fill="none" stroke="#ca8a04" strokeDasharray="6 5" strokeLinecap="square" strokeLinejoin="round" strokeWidth="2.5" />
          </g>
        );
      })}
      {latencySeries.map((item) => {
        const points = item.points.filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value));
        const path = points.map((point, index) => `${index === 0 ? "M" : "L"} ${x(point.time).toFixed(2)} ${yLatency(point.value).toFixed(2)}`).join(" ");
        return (
          <g key={item.id ?? item.label}>
            <path d={path} fill="none" stroke={item.color} strokeLinecap="square" strokeLinejoin="round" strokeWidth="4" />
            {points.length === 1 ? (
              <circle cx={x(points[0].time)} cy={yLatency(points[0].value)} r="5" fill={item.color} stroke="var(--border-color)" strokeWidth="2" />
            ) : null}
          </g>
        );
      })}
      {hover ? (
        <g>
          <line x1={hover.x} x2={hover.x} y1={padding.top} y2={padding.top + plotHeight} stroke="var(--border-color)" strokeDasharray="6 5" strokeWidth="2" />
          <rect x={tooltipX} y={tooltipY} width={tooltipWidth} height={tooltipHeight} fill="var(--bg-card)" stroke="var(--border-color)" strokeWidth="2" />
          <text x={tooltipX + 10} y={tooltipY + 18} fill="var(--text-main)" fontSize="12" fontWeight="900">
            {formatChartTime(hover.time)}
          </text>
          {hover.values.map((item, index) => (
            <g key={`${item.label}-${index}`} transform={`translate(${tooltipX + 10}, ${tooltipY + 36 + index * 20})`}>
              <rect width="9" height="9" y="-8" fill={item.color} stroke="var(--border-color)" strokeWidth="1.5" />
              <text x="16" y="0" fill="var(--text-main)" fontSize="12" fontWeight="900">
                {item.label}: {item.kind === "latency" ? formatMs(Math.round(item.value)) : `${item.value.toFixed(1)}%`}
              </text>
            </g>
          ))}
        </g>
      ) : null}
    </svg>
  );
}

function nearestPoint(points: ChartPoint[], targetTime: number): ChartPoint | null {
  let best: ChartPoint | null = null;
  for (const point of points) {
    if (!best || Math.abs(point.time - targetTime) < Math.abs(best.time - targetTime)) {
      best = point;
    }
  }
  return best;
}

function nearestValue(values: number[], target: number): number {
  let best = values[0] ?? target;
  for (const value of values) {
    if (Math.abs(value - target) < Math.abs(best - target)) best = value;
  }
  return best;
}

function svgPointerX(event: MouseEvent<SVGSVGElement>, fallbackWidth: number): number {
  const svg = event.currentTarget;
  const matrix = svg.getScreenCTM();
  if (matrix) {
    const point = svg.createSVGPoint();
    point.x = event.clientX;
    point.y = event.clientY;
    return point.matrixTransform(matrix.inverse()).x;
  }

  const rect = svg.getBoundingClientRect();
  return ((event.clientX - rect.left) / rect.width) * fallbackWidth;
}

function buildMetricCharts(samples: MetricSample[], lastState?: Record<string, unknown> | null) {
  const rows = samples
    .map((sample) => ({
      time: new Date(sample.sample_at).getTime(),
      state: sample.fields_json ?? {},
    }))
    .filter((row) => Number.isFinite(row.time));

  if (rows.length === 0 && lastState) {
    rows.push({ time: Date.now(), state: lastState });
  }

  const cpu: ChartPoint[] = [];
  const memory: ChartPoint[] = [];
  const disk: ChartPoint[] = [];
  const swap: ChartPoint[] = [];
  const load: ChartPoint[] = [];
  const process: ChartPoint[] = [];
  const tcp: ChartPoint[] = [];
  const udp: ChartPoint[] = [];
  const gpu: ChartPoint[] = [];
  const temperature: ChartPoint[] = [];
  const rx: ChartPoint[] = [];
  const tx: ChartPoint[] = [];
  let previous: { time: number; rxTotal?: number; txTotal?: number } | null = null;

  for (const row of rows) {
    const state = row.state;
    const cpuValue = optionalNumber(state.cpu_percent);
    const memoryUsed = optionalNumber(state.memory_used);
    const memoryTotal = optionalNumber(state.memory_total);
    const diskPercent = diskUsagePercent(state);
    const swapPercent = percentFromUsedTotal(state.swap_used, state.swap_total);
    const loadValue = optionalNumber(state.load_1) ?? optionalNumber(state.load1);
    const processValue = optionalNumber(state.process_count);
    const tcpValue = optionalNumber(state.tcp_connections);
    const udpValue = optionalNumber(state.udp_connections);
    const gpuValue = gpuUsagePercent(state);
    const temperatureValue = maxTemperature(state);
    const rxTotal = netTotal(state, "bytes_recv", "network_in_total");
    const txTotal = netTotal(state, "bytes_sent", "network_out_total");

    if (cpuValue !== undefined) cpu.push({ time: row.time, value: cpuValue });
    if (memoryUsed !== undefined && memoryTotal && memoryTotal > 0) {
      memory.push({ time: row.time, value: (memoryUsed / memoryTotal) * 100 });
    }
    if (diskPercent !== undefined) disk.push({ time: row.time, value: diskPercent });
    if (swapPercent !== undefined) swap.push({ time: row.time, value: swapPercent });
    if (loadValue !== undefined) load.push({ time: row.time, value: loadValue });
    if (processValue !== undefined) process.push({ time: row.time, value: processValue });
    if (tcpValue !== undefined) tcp.push({ time: row.time, value: tcpValue });
    if (udpValue !== undefined) udp.push({ time: row.time, value: udpValue });
    if (gpuValue !== undefined) gpu.push({ time: row.time, value: gpuValue });
    if (temperatureValue !== undefined) temperature.push({ time: row.time, value: temperatureValue });
    if (rxTotal !== undefined) {
      rx.push({ time: row.time, value: rateFromPrevious(row.time, rxTotal, previous?.time, previous?.rxTotal) });
    }
    if (txTotal !== undefined) {
      tx.push({ time: row.time, value: rateFromPrevious(row.time, txTotal, previous?.time, previous?.txTotal) });
    }

    previous = { time: row.time, rxTotal, txTotal };
  }

  return {
    cpu: thinPoints(cpu),
    memory: thinPoints(memory),
    disk: thinPoints(disk),
    swap: thinPoints(swap),
    load: thinPoints(load),
    process: thinPoints(process),
    tcp: thinPoints(tcp),
    udp: thinPoints(udp),
    gpu: thinPoints(gpu),
    temperature: thinPoints(temperature),
    rx: thinPoints(rx),
    tx: thinPoints(tx),
    latestCpu: latestValue(cpu),
    latestMemory: latestValue(memory),
    latestDisk: latestValue(disk),
    latestSwap: latestValue(swap),
    latestLoad: latestValue(load),
    latestProcess: latestValue(process),
    latestTcp: latestValue(tcp),
    latestUdp: latestValue(udp),
    latestGpu: latestValue(gpu),
    latestTemperature: latestValue(temperature),
    latestRx: latestValue(rx),
    latestTx: latestValue(tx),
  };
}

function historyLimitForRange(range: MetricsRange): number {
  if (range === "30d") return 1440;
  if (range === "7d") return 720;
  return 240;
}

function thinPoints(points: ChartPoint[], limit = 240): ChartPoint[] {
  if (points.length <= limit) return points;
  const lastIndex = points.length - 1;
  const step = lastIndex / (limit - 1);
  const seen = new Set<number>();
  const out: ChartPoint[] = [];
  for (let index = 0; index < limit; index += 1) {
    const point = points[Math.round(index * step)];
    if (point && !seen.has(point.time)) {
      seen.add(point.time);
      out.push(point);
    }
  }
  const last = points[lastIndex];
  if (last && !seen.has(last.time)) out.push(last);
  return out;
}

function smoothPeakPoints(points: ChartPoint[]): ChartPoint[] {
  if (points.length < 7) return points;
  const windowSize = 7;
  const alpha = 0.35;
  let previous: number | null = null;
  return points.map((point, index) => {
    if (index < windowSize - 1) return point;
    const window = points.slice(index - windowSize + 1, index + 1).map((item) => item.value);
    const median = medianValue(window);
    const deviations = window.map((value) => Math.abs(value - median));
    const mad = Math.max(medianValue(deviations) * 1.4826, 1);
    const valid = window.filter((value) => Math.abs(value - median) <= 3 * mad && value <= Math.max(median * 3, median + mad));
    const next = valid.length ? valid.reduce((sum, value) => sum + value, 0) / valid.length : median;
    previous = previous === null ? next : alpha * next + (1 - alpha) * previous;
    return { ...point, value: previous };
  });
}

function medianValue(values: number[]): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function normalizeMetricSample(sample: unknown): MetricSample | null {
  const row = asRecord(sample);
  const sampleAt = asString(row.sample_at);
  if (!sampleAt) return null;
  return {
    sample_at: sampleAt,
    fields_json: asRecord(row.fields_json),
  };
}

function normalizeServiceResult(result: unknown): ServiceResult | null {
  const row = asRecord(result);
  const serviceId = asString(row.service_id);
  const createdAt = asString(row.created_at);
  if (!serviceId || !createdAt) return null;
  return {
    service_id: serviceId,
    server_id: stringValue(row.server_id) || null,
    status: asString(row.status),
    delay_ms: optionalNumber(row.delay_ms),
    created_at: createdAt,
  };
}

function serviceBelongsToServer(service: ServiceSummary, serverId: string): boolean {
  const serverIds = Array.isArray(service.server_ids) ? service.server_ids : [];
  return service.server_id === serverId || serverIds.includes(serverId);
}

function serviceResultOk(status: string): boolean {
  return status === "success" || status === "up" || status === "ok";
}

function buildProbeLossPoints(results: ServiceResult[]): ChartPoint[] {
  const rows = results
    .map((result) => ({
      time: new Date(result.created_at).getTime(),
      delay: result.delay_ms,
      status: result.status,
    }))
    .filter((row) => Number.isFinite(row.time));
  if (!rows.length) return [];

  const derivedLoss = calculatePacketLossFromDelays(rows.map((row) => row.delay ?? 0));
  return rows.map((row, index) => ({
    time: row.time,
    value: serviceResultOk(row.status) ? derivedLoss[index] ?? 0 : 100,
  }));
}

function calculatePacketLossFromDelays(delays: number[]): number[] {
  if (!delays.length) return [];
  const windowSize = Math.min(10, Math.max(3, Math.floor(delays.length / 10)));
  const timeoutThreshold = 3000;
  const extremeDelayThreshold = 10000;
  const packetLossRates: number[] = [];

  for (let index = 0; index < delays.length; index += 1) {
    const currentDelay = delays[index];
    let lossRate = 0;

    if (!currentDelay || currentDelay <= 0) {
      lossRate = 100;
    } else if (currentDelay >= extremeDelayThreshold) {
      lossRate = Math.min(95, 60 + (currentDelay - extremeDelayThreshold) / 1000);
    } else if (currentDelay >= timeoutThreshold) {
      lossRate = Math.min(50, (currentDelay - timeoutThreshold) / 200);
    } else {
      const start = Math.max(0, index - Math.floor(windowSize / 2));
      const end = Math.min(delays.length, index + Math.ceil(windowSize / 2));
      const windowDelays = delays.slice(start, end).filter((delay) => delay > 0);
      if (windowDelays.length > 2) {
        const mean = windowDelays.reduce((sum, delay) => sum + delay, 0) / windowDelays.length;
        const variance = windowDelays.reduce((sum, delay) => sum + (delay - mean) ** 2, 0) / windowDelays.length;
        const coefficientOfVariation = Math.sqrt(variance) / mean;
        if (coefficientOfVariation > 0.8) {
          lossRate = Math.min(25, coefficientOfVariation * 15);
        } else if (coefficientOfVariation > 0.5) {
          lossRate = Math.min(10, coefficientOfVariation * 8);
        } else if (coefficientOfVariation > 0.3) {
          lossRate = Math.min(5, coefficientOfVariation * 5);
        }
        if (currentDelay > mean * 2.5) {
          lossRate += Math.min(15, (currentDelay / mean - 2.5) * 10);
        }
      }
    }

    if (index > 0) {
      lossRate = 0.3 * lossRate + 0.7 * packetLossRates[index - 1];
    }
    packetLossRates.push(Math.max(0, Math.min(100, Number(lossRate.toFixed(2)))));
  }

  return packetLossRates;
}

function packetLossRate(results: ServiceResult[]): number {
  if (!results.length) return 0;
  const lost = results.filter((result) => !serviceResultOk(result.status)).length;
  return (lost / results.length) * 100;
}

function latestDelay(results: ServiceResult[]): number | undefined {
  return results
    .filter((result) => typeof result.delay_ms === "number")
    .sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime())
    .at(-1)?.delay_ms ?? undefined;
}

function averageLossRate(histories: ProbeHistory[]): number | undefined {
  if (!histories.length) return undefined;
  return histories.reduce((sum, history) => sum + history.lossRate, 0) / histories.length;
}

function averagePointValue(points: ChartPoint[]): number | undefined {
  if (!points.length) return undefined;
  return points.reduce((sum, point) => sum + point.value, 0) / points.length;
}

function splitTags(value: string): string[] {
  return value
    .split(/[，,;；、]/)
    .map((item) => item.trim())
    .filter((item, index, all) => item && all.indexOf(item) === index)
    .slice(0, 8);
}

function isHexColor(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}

function rateFromPrevious(time: number, total: number, previousTime?: number, previousTotal?: number): number {
  if (previousTime === undefined || previousTotal === undefined || time <= previousTime || total < previousTotal) return 0;
  return (total - previousTotal) / ((time - previousTime) / 1000);
}

function percentFromUsedTotal(usedValue: unknown, totalValue: unknown): number | undefined {
  const used = optionalNumber(usedValue);
  const total = optionalNumber(totalValue);
  if (used === undefined || !total || total <= 0) return undefined;
  return (used / total) * 100;
}

function diskUsagePercent(state: Record<string, unknown>): number | undefined {
  const direct = optionalNumber(state.disk_percent) ?? percentFromUsedTotal(state.disk_used, state.disk_total);
  if (direct !== undefined) return direct;
  const disks = Array.isArray(state.disks) ? state.disks : [];
  const used = disks.reduce((sum, item) => sum + (optionalNumber(asRecord(item).used) ?? 0), 0);
  const total = disks.reduce((sum, item) => sum + (optionalNumber(asRecord(item).total) ?? 0), 0);
  return total > 0 ? (used / total) * 100 : undefined;
}

function gpuUsagePercent(state: Record<string, unknown>): number | undefined {
  return optionalNumber(state.gpu_percent) ?? optionalNumber(state.gpu_usage) ?? optionalNumber(state.gpu);
}

function maxTemperature(state: Record<string, unknown>): number | undefined {
  const direct = optionalNumber(state.temperature) ?? optionalNumber(state.temp);
  if (direct !== undefined) return direct;
  const sensors = Array.isArray(state.temperatures) ? state.temperatures : [];
  const values = sensors
    .map((item) => {
      if (typeof item === "number") return item;
      const sensor = asRecord(item);
      return optionalNumber(sensor.temperature) ?? optionalNumber(sensor.temp) ?? optionalNumber(sensor.value);
    })
    .filter((value): value is number => value !== undefined);
  return values.length ? Math.max(...values) : undefined;
}

function netTotal(state: Record<string, unknown>, field: "bytes_recv" | "bytes_sent", fallbackKey: string): number | undefined {
  const fallback = optionalNumber(state[fallbackKey]);
  if (fallback !== undefined) return fallback;
  const netIo = Array.isArray(state.net_io) ? state.net_io : [];
  const total = netIo.reduce((sum, item) => sum + (optionalNumber(asRecord(item)[field]) ?? 0), 0);
  return netIo.length > 0 ? total : undefined;
}

function latestValue(points: ChartPoint[]): number | undefined {
  return points.length ? points[points.length - 1]?.value : undefined;
}

function minPointValue(points: ChartPoint[]): number | undefined {
  return points.length ? Math.min(...points.map((point) => point.value)) : undefined;
}

function maxPointValue(points: ChartPoint[]): number | undefined {
  return points.length ? Math.max(...points.map((point) => point.value)) : undefined;
}

function latestProbeLatency(series: ChartSeries[]): string {
  const latest = series
    .flatMap((item) => item.points)
    .sort((a, b) => a.time - b.time)
    .at(-1);
  return latest ? formatMs(Math.round(latest.value)) : "N/A";
}

function formatRate(value?: number): string {
  if (value === undefined || Number.isNaN(value)) return "N/A";
  return `${formatBytes(value)}/s`;
}

function formatChartTime(value: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function metadataFromRecord(record: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = valueLabel(record[key]);
    if (value) return value;
  }
  for (const container of ["billing", "plan", "metadata", "custom"]) {
    const child = asRecord(record[container]);
    for (const key of keys) {
      const value = valueLabel(child[key]);
      if (value) return value;
    }
  }
  return null;
}

function locationLabel(server: ServerDetail, info: Record<string, unknown>): string | null {
  const parts = [
    server.location?.country ?? server.country ?? metadataFromRecord(info, ["country", "geo_country", "country_name"]),
    server.location?.region ?? server.region ?? metadataFromRecord(info, ["region", "geo_region", "state", "province", "location"]),
    server.location?.city ?? server.city ?? metadataFromRecord(info, ["city", "geo_city"]),
  ]
    .map((value) => value?.trim())
    .filter(Boolean);
  return parts.length ? Array.from(new Set(parts)).join(" / ") : null;
}

function valueLabel(value: unknown): string | null {
  if (typeof value === "string") return value.trim() || null;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "boolean") return String(value);
  return null;
}

function optionalNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function parsePlatformDetails(info: Record<string, unknown>): { os: string; version: string; agent: string; report: string; flags: string } {
  const os = stringValue(info.platform) || stringValue(info.os);
  const rawVersion = stringValue(info.platform_version) || stringValue(info.os_version);
  const parts = rawVersion
    .split("|")
    .map((part) => part.trim())
    .filter(Boolean);
  const version = parts.find((part) => !part.includes("=")) || rawVersion;
  const kv = new Map<string, string>();
  for (const part of parts) {
    const [key, ...rest] = part.split("=");
    if (key && rest.length) kv.set(key.trim(), rest.join("=").trim());
  }
  const report = [
    kv.get("report") ? `主机 ${kv.get("report")}` : "",
    kv.get("ip_report") ? `IP ${kv.get("ip_report")}` : "",
  ]
    .filter(Boolean)
    .join(" / ");
  return {
    os,
    version,
    agent: kv.get("agent") || "",
    report,
    flags: [
      flagLabel("auto_update_disabled", "自动更新", kv),
      flagLabel("force_update_disabled", "强制更新", kv),
      flagLabel("command_disabled", "命令", kv),
      flagLabel("nat_disabled", "NAT", kv),
      flagLabel("send_query_disabled", "查询上报", kv),
    ]
      .filter(Boolean)
      .join(" / "),
  };
}

function flagLabel(key: string, label: string, values: Map<string, string>): string {
  const value = values.get(key);
  if (value === undefined) return "";
  return `${label}${value === "true" ? "关" : "开"}`;
}

function joinLabels(values: Array<string | undefined>): string {
  return values.filter(Boolean).join(" ");
}

function numberLabel(value: unknown): string {
  const parsed = optionalNumber(value);
  return parsed === undefined ? "N/A" : String(parsed);
}

function resourceBytes(usedValue: unknown, totalValue: unknown, infoTotalValue?: unknown): string {
  const used = optionalNumber(usedValue);
  const total = optionalNumber(totalValue) ?? optionalNumber(infoTotalValue);
  if (used !== undefined && total !== undefined && total > 0) return `${formatBytes(used)} / ${formatBytes(total)}`;
  return total === undefined ? "N/A" : formatBytes(total);
}

function diskLabel(state: Record<string, unknown>, info: Record<string, unknown>): string {
  const stateDisks = Array.isArray(state.disks) ? state.disks : [];
  const infoDisks = Array.isArray(info.disks) ? info.disks : [];
  const disks = stateDisks.length ? stateDisks : infoDisks;
  const used = disks.reduce((sum, item) => sum + (optionalNumber(asRecord(item).used) ?? 0), 0);
  const total = disks.reduce((sum, item) => sum + (optionalNumber(asRecord(item).total) ?? 0), 0);
  if (used > 0 && total > 0) return `${formatBytes(used)} / ${formatBytes(total)}`;
  return total > 0 ? formatBytes(total) : "N/A";
}

function durationLabel(value: unknown): string {
  const seconds = optionalNumber(value);
  if (seconds === undefined) return "N/A";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

function isExpired(value?: string | null): boolean {
  if (!value) return false;
  const date = new Date(value);
  return !Number.isNaN(date.getTime()) && date.getTime() < Date.now();
}

function joinPath(base: string, name: string): string {
  if (base === "/") return `/${name}`;
  return `${base.replace(/\/$/, "")}/${name}`;
}

function parentPath(value: string): string {
  if (!value || value === "/") return "/";
  const trimmed = value.replace(/\/$/, "");
  const index = trimmed.lastIndexOf("/");
  return index <= 0 ? "/" : trimmed.slice(0, index);
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

function configFlagLabel(key: keyof typeof blankConfig): string {
  const labels: Partial<Record<keyof typeof blankConfig, string>> = {
    disable_auto_update: "禁用自动更新",
    disable_force_update: "禁用强制更新",
    disable_command_execute: "禁用命令执行",
    disable_nat: "禁用 NAT",
    disable_send_query: "禁用查询上报",
  };
  return labels[key] || key;
}

function initialShowMissingInfo(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_detail_show_missing") === "1";
}
