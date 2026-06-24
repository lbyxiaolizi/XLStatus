"use client";

import Link from "next/link";
import { FormEvent, MouseEvent, memo, useCallback, useEffect, useMemo, useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
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
import { durationLabel, formatRate, maxOf, minOf, optionalNumber } from "@/app/lib/format";
import { useI18n } from "@/lib/use-i18n";

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
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
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

  const loadServer = useCallback(async (signal?: AbortSignal) => {
    if (!serverId) return;
    setLoading(true);
    setError(null);
    const response = await apiClient.getServer(serverId, { signal });
    if (signal?.aborted) return;
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

  const loadMetrics = useCallback(async (signal?: AbortSignal) => {
    if (!serverId) return;
    setMetricsLoading(true);
    setMonitorError(null);
    const response = await apiClient.getServerMetrics(serverId, metricsRange, { signal });
    if (signal?.aborted) return;
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

  const loadProbeHistory = useCallback(async (signal?: AbortSignal) => {
    if (!serverId) return;
    setProbeLoading(true);
    setMonitorError(null);
    const servicesResponse = await apiClient.listServices(200, 0, false, { signal });
    if (signal?.aborted) return;
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
        const response = await apiClient.getServiceHistory(service.id, historyLimitForRange(metricsRange), { signal });
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
    if (signal?.aborted) return;

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
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServer(controller.signal);
    return () => controller.abort();
  }, [loadServer]);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadMetrics(controller.signal);
    return () => controller.abort();
  }, [loadMetrics]);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadProbeHistory(controller.signal);
    return () => controller.abort();
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
      setNotice(copy.serverDetailPage.readFileNotice.replace("{path}", nextPath).replace("{bytes}", String(response.data.bytes)));
    } else {
      setError(responseError(response));
    }
  }

  async function writeFile(event: FormEvent) {
    event.preventDefault();
    if (!writePath.trim()) {
      setError(copy.serverDetailPage.writePathRequired);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setSaving(true);
    const response = await apiClient.writeServerFile(serverId, {
      path: writePath.trim(),
      content: writeContent,
      encoding: "utf8",
      create_dirs: true,
    }, totpCode);
    setSaving(false);
    if (response.success) {
      setNotice(copy.serverDetailPage.writeFileNotice.replace("{path}", writePath.trim()));
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteEntry(entryPath: string, recursive: boolean) {
    if (!(await dialogs.confirm({ message: copy.serverDetailPage.deleteEntryConfirm.replace("{path}", entryPath), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteServerFile(serverId, { path: entryPath, recursive }, totpCode);
    if (response.success) {
      setNotice(copy.serverDetailPage.deleteEntryNotice.replace("{path}", entryPath));
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function createTempUrl(kind: "download" | "upload") {
    const target = kind === "download" ? selectedPath || writePath : writePath;
    if (!target.trim()) {
      setError(copy.serverDetailPage.selectFilePathFirst);
      return;
    }
    const totpCode = kind === "upload" ? await sensitiveTotpCode() : undefined;
    if (totpCode === null) return;
    const response =
      kind === "download"
        ? await apiClient.getServerDownloadUrl(serverId, target.trim())
        : await apiClient.getServerUploadUrl(serverId, target.trim(), totpCode);
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
      setNotice(copy.serverDetailPage.configPatchSent);
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
      setError(copy.serverDetailPage.updateFieldsRequired);
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
      setNotice(copy.serverDetailPage.forceUpdateSent);
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
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.serverDetailPage.totpRequired);
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
      setNotice(copy.serverDetailPage.metadataSaved);
      await loadServer();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.serverDetailPage.eyebrow}
          title={server?.name || compactId(serverId)}
          detail={copy.serverDetailPage.lastSeen.replace("{time}", formatDate(server?.last_seen_at))}
          actions={
            <>
              <Link href="/servers" className={buttonClass("secondary")}>{copy.serverDetailPage.back}</Link>
              {server ? <StatusBadge tone={statusTone}>{serverStatusLabel(server.status, copy)}</StatusBadge> : null}
            </>
          }
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading && !server ? <EmptyState title={copy.serverDetailPage.loadingServer} /> : null}

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
      {dialogs.element}
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

// Module-level chart value formatters: stable references so memoized chart
// components don't re-render when the parent rebuilds its chart config array.
const formatChartPercent = (value: number) => `${value.toFixed(1)}%`;
const formatChartFixed2 = (value: number) => value.toFixed(2);
const formatChartRound = (value: number) => String(Math.round(value));
const formatChartTemperature = (value: number) => `${value.toFixed(1)} °C`;

function SecondaryPanelNav({ active, onOpen }: { active: SecondaryPanel; onOpen: (panel: SecondaryPanel) => void }) {
  const { t: copy } = useI18n();
  return (
    <div className="flex flex-wrap gap-3 border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal)]">
      {(
        [
          ["custom", copy.serverDetailPage.panelCustom],
          ["files", copy.serverDetailPage.panelFiles],
          ["ops", copy.serverDetailPage.panelOps],
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
  const { t: copy } = useI18n();
  return (
    <div className="grid gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.serverDetailPage.fileSystem}</h2>
            <p className="text-sm font-bold text-[var(--text-muted)]">{path}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button className={buttonClass("secondary")} onClick={() => void onLoad(currentDir)}>{copy.serverDetailPage.parentDir}</button>
            <button className={buttonClass("secondary")} onClick={() => void onLoad(path)}>{copy.serverDetailPage.refresh}</button>
          </div>
        </div>
        {filesLoading ? (
          <p className="font-bold">{copy.serverDetailPage.loadingFiles}</p>
        ) : files.length === 0 ? (
          <EmptyState title={copy.serverDetailPage.noFilesTitle} detail={copy.serverDetailPage.noFilesDetail} />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>{copy.serverDetailPage.colName}</th>
                  <th className={thClass}>{copy.serverDetailPage.colType}</th>
                  <th className={thClass}>{copy.serverDetailPage.colSize}</th>
                  <th className={thClass}>{copy.serverDetailPage.colActions}</th>
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
                          <button className={buttonClass("secondary")} onClick={() => void onLoad(nextPath)}>{copy.serverDetailPage.open}</button>
                        ) : (
                          <button className={buttonClass("primary")} onClick={() => void onRead(nextPath)}>{copy.serverDetailPage.read}</button>
                        )}
                        <button className={buttonClass("danger")} onClick={() => void onDelete(nextPath, entry.file_type === "dir")}>{copy.serverDetailPage.delete}</button>
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
        <h2 className="mb-4 text-xl font-black uppercase">{copy.serverDetailPage.writeFileTitle}</h2>
        <form onSubmit={onWrite} className="space-y-4">
          <Field label={copy.serverDetailPage.pathLabel}>
            <input className={inputClass} value={writePath} onChange={(event) => onWritePathChange(event.target.value)} placeholder="/tmp/xlstatus.txt" />
          </Field>
          <Field label={copy.serverDetailPage.contentLabel}>
            <textarea className={`${textareaClass} min-h-40`} value={writeContent} onChange={(event) => onWriteContentChange(event.target.value)} />
          </Field>
          <div className="flex flex-wrap gap-2">
            <button disabled={saving} className={buttonClass("primary")}>{copy.serverDetailPage.writeFileButton}</button>
            <button type="button" className={buttonClass("secondary")} onClick={() => void onTempUrl("download")}>{copy.serverDetailPage.downloadUrl}</button>
            <button type="button" className={buttonClass("secondary")} onClick={() => void onTempUrl("upload")}>{copy.serverDetailPage.uploadUrl}</button>
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
  const { t: copy } = useI18n();
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <BrutalCard>
        <h2 className="mb-4 text-xl font-black uppercase">{copy.serverDetailPage.applyConfigTitle}</h2>
        <form onSubmit={onApplyConfig} className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label={copy.serverDetailPage.reportInterval}>
              <input className={inputClass} value={configForm.report_interval_seconds} onChange={(e) => onConfigChange({ ...configForm, report_interval_seconds: e.target.value })} />
            </Field>
            <Field label={copy.serverDetailPage.ipReportInterval}>
              <input className={inputClass} value={configForm.ip_report_interval_seconds} onChange={(e) => onConfigChange({ ...configForm, ip_report_interval_seconds: e.target.value })} />
            </Field>
          </div>
          <div className="grid gap-2">
            {(
              [
                ["disable_auto_update", copy.serverDetailPage.flagDisableAutoUpdate],
                ["disable_force_update", copy.serverDetailPage.flagDisableForceUpdate],
                ["disable_command_execute", copy.serverDetailPage.flagDisableCommandExecute],
                ["disable_nat", copy.serverDetailPage.flagDisableNat],
                ["disable_send_query", copy.serverDetailPage.flagDisableSendQuery],
              ] as const
            ).map(([key, label]) => (
              <label key={key} className="flex items-center gap-2 text-sm font-black">
                <input type="checkbox" checked={configForm[key]} onChange={(e) => onConfigChange({ ...configForm, [key]: e.target.checked })} />
                {label}
              </label>
            ))}
          </div>
          <button disabled={saving} className={buttonClass("primary")}>{copy.serverDetailPage.applyConfigButton}</button>
        </form>
      </BrutalCard>

      <BrutalCard>
        <h2 className="mb-4 text-xl font-black uppercase">{copy.serverDetailPage.sendUpdateTitle}</h2>
        <form onSubmit={onForceUpdate} className="space-y-4">
          <Field label={copy.serverDetailPage.versionLabel}>
            <input className={inputClass} value={updateForm.version} onChange={(e) => onUpdateChange({ ...updateForm, version: e.target.value })} placeholder="v0.1" />
          </Field>
          <Field label={copy.serverDetailPage.downloadUrlLabel}>
            <input className={inputClass} value={updateForm.download_url} onChange={(e) => onUpdateChange({ ...updateForm, download_url: e.target.value })} placeholder="https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.1/xlstatus-agent-linux-amd64.tar.gz" />
          </Field>
          <Field label="SHA-256">
            <input className={inputClass} value={updateForm.checksum} onChange={(e) => onUpdateChange({ ...updateForm, checksum: e.target.value })} placeholder={copy.serverDetailPage.checksumPlaceholder} />
          </Field>
          <button disabled={saving} className={buttonClass("danger")}>{copy.serverDetailPage.sendUpdateButton}</button>
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
  const { t: copy } = useI18n();
  const resourceCharts = [
    {
      key: "cpu",
      title: "CPU",
      value: formatPercent(metricCharts.latestCpu),
      series: [{ id: "cpu", label: "CPU", color: "var(--accent-color)", points: metricCharts.cpu }],
      maxValue: 100,
      formatValue: formatChartPercent,
    },
    {
      key: "memory",
      title: copy.serverDetailPage.chartMemory,
      value: formatPercent(metricCharts.latestMemory),
      series: [{ id: "memory", label: copy.serverDetailPage.chartMemory, color: "var(--btn-bg)", points: metricCharts.memory }],
      maxValue: 100,
      formatValue: formatChartPercent,
    },
    {
      key: "load",
      title: copy.serverDetailPage.chartLoad,
      value: metricCharts.latestLoad === undefined ? "N/A" : metricCharts.latestLoad.toFixed(2),
      series: [{ id: "load", label: copy.serverDetailPage.chartLoad, color: "#0ea5e9", points: metricCharts.load }],
      formatValue: formatChartFixed2,
    },
    {
      key: "disk",
      title: copy.serverDetailPage.chartDisk,
      value: formatPercent(metricCharts.latestDisk),
      series: [{ id: "disk", label: copy.serverDetailPage.chartDisk, color: "#f97316", points: metricCharts.disk }],
      maxValue: 100,
      formatValue: formatChartPercent,
    },
    {
      key: "swap",
      title: "Swap",
      value: formatPercent(metricCharts.latestSwap),
      series: [{ id: "swap", label: "Swap", color: "#a855f7", points: metricCharts.swap }],
      maxValue: 100,
      formatValue: formatChartPercent,
    },
    {
      key: "process",
      title: copy.serverDetailPage.chartProcess,
      value: metricCharts.latestProcess === undefined ? "N/A" : String(Math.round(metricCharts.latestProcess)),
      series: [{ id: "process", label: copy.serverDetailPage.chartProcess, color: "#10b981", points: metricCharts.process }],
      formatValue: formatChartRound,
    },
    {
      key: "connection",
      title: copy.serverDetailPage.chartConnection,
      value: `TCP ${metricCharts.latestTcp === undefined ? "N/A" : Math.round(metricCharts.latestTcp)} / UDP ${metricCharts.latestUdp === undefined ? "N/A" : Math.round(metricCharts.latestUdp)}`,
      series: [
        { id: "tcp", label: "TCP", color: "var(--accent-color)", points: metricCharts.tcp },
        { id: "udp", label: "UDP", color: "var(--border-color)", points: metricCharts.udp },
      ],
      formatValue: formatChartRound,
    },
    {
      key: "temperature",
      title: copy.serverDetailPage.chartTemperature,
      value: metricCharts.latestTemperature === undefined ? "N/A" : `${metricCharts.latestTemperature.toFixed(1)} °C`,
      series: [{ id: "temperature", label: copy.serverDetailPage.chartTemperature, color: "#dc2626", points: metricCharts.temperature }],
      formatValue: formatChartTemperature,
    },
    {
      key: "gpu",
      title: "GPU",
      value: formatPercent(metricCharts.latestGpu),
      series: [{ id: "gpu", label: "GPU", color: "#ef4444", points: metricCharts.gpu }],
      maxValue: 100,
      formatValue: formatChartPercent,
    },
  ];
  const visibleResourceCharts = metricsLoading ? resourceCharts : resourceCharts.filter((chart) => chartHasData(chart.series));

  return (
    <section className="border-2 border-black bg-[var(--accent-bg)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-4 flex flex-wrap items-end justify-between gap-3 border-b-4 border-black pb-3">
        <h2 className="text-2xl font-black uppercase">{copy.serverDetailPage.monitoringCharts}</h2>
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
            ["resources", copy.serverDetailPage.tabResources],
            ["network", copy.serverDetailPage.tabNetwork],
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
          <EmptyState title={copy.serverDetailPage.noResourceChartsTitle} detail={copy.serverDetailPage.noResourceChartsDetail} />
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
  const { t: copy } = useI18n();
  const visibleHistories = selectedIds.length
    ? histories.filter((history) => selectedIds.includes(history.id))
    : histories;
  const displayedLatencySeries = peakCutEnabled
    ? latencySeries.map((item) => ({ ...item, points: smoothPeakPoints(item.points) }))
    : latencySeries;
  const networkSeries = [
    { id: "tx", label: copy.serverDetailPage.seriesUpload, color: "var(--accent-color)", points: metricCharts.tx },
    { id: "rx", label: copy.serverDetailPage.seriesDownload, color: "var(--border-color)", points: metricCharts.rx },
  ];
  const hasNetworkData = chartHasData(networkSeries);
  const hasProbeData = chartHasData(displayedLatencySeries) || chartHasData(lossSeries);
  const showNetworkChart = metricsLoading || hasNetworkData;
  const showProbeChart = probeLoading || hasProbeData;

  return (
    <div className="grid gap-4">
      {showNetworkChart ? (
        <MetricChartCard
          title={copy.serverDetailPage.chartNetwork}
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
                {copy.serverDetailPage.clearSelection.replace("{count}", String(selectedIds.length))}
              </button>
            ) : null}
            <label className="inline-flex min-h-10 items-center gap-2 border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
              <input
                type="checkbox"
                checked={peakCutEnabled}
                onChange={(event) => onPeakCutChange(event.target.checked)}
                className="h-4 w-4 accent-black"
              />
              {copy.serverDetailPage.peakCut}
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
        <EmptyState title={copy.serverDetailPage.noNetworkChartsTitle} detail={copy.serverDetailPage.noNetworkChartsDetail} />
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
  const { t: copy } = useI18n();
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
  const platformDetails = parsePlatformDetails(info, copy);
  const platform = joinLabels([platformDetails.os, platformDetails.version]);
  const load = [optionalNumber(state.load_1), optionalNumber(state.load_5), optionalNumber(state.load_15)]
    .map((value) => (value === undefined ? null : value.toFixed(2)))
    .filter(Boolean)
    .join(" / ");
  const tcp = optionalNumber(state.tcp_connections);
  const udp = optionalNumber(state.udp_connections);
  const tiles = filterInfoTiles(
    [
      { label: copy.serverDetailPage.tilePrivateRemark, value: remark, wide: true },
      { label: copy.serverDetailPage.tilePublicNote, value: publicNote, wide: true },
      { label: copy.serverDetailPage.tileProvider, value: provider },
      { label: copy.serverDetailPage.tileRegion, value: region },
      { label: copy.serverDetailPage.tilePlan, value: plan },
      { label: copy.serverDetailPage.tileExpires, value: expiresAt ? formatDate(expiresAt) : null, danger: isExpired(expiresAt) },
      {
        label: copy.serverDetailPage.tileRenewal,
        value: renewalPrice === null || renewalPrice === undefined || renewalPrice === "" ? null : String(renewalPrice),
      },
      { label: copy.serverDetailPage.tilePrice, value: billingLabel(price, currency, billingCycle) },
      { label: copy.serverDetailPage.tileAutoRenew, value: booleanLabel(server.auto_renew, copy) },
      { label: copy.serverDetailPage.tileTrafficQuota, value: trafficQuotaLabel(trafficQuotaBytes, trafficQuotaType) },
      { label: copy.serverDetailPage.tilePublicAccess, value: publicVisibilityLabel(server, copy) },
    ],
    showMissing,
  );
  const hostRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowHostname, stringValue(info.hostname)],
      [copy.serverDetailPage.rowSystem, platform],
      [copy.serverDetailPage.rowKernel, stringValue(info.kernel_version)],
      [copy.serverDetailPage.rowArch, stringValue(info.arch)],
      ["Agent", platformDetails.agent],
    ],
    showMissing,
  );
  const resourceRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowCpuCores, numberLabel(info.cpu_cores)],
      [copy.serverDetailPage.rowMemory, resourceBytes(state.memory_used, state.memory_total, info.total_memory)],
      ["Swap", resourceBytes(state.swap_used, state.swap_total, info.total_swap)],
      [copy.serverDetailPage.rowDisk, diskLabel(state, info)],
    ],
    showMissing,
  );
  const runtimeRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowUptime, durationLabel(state.uptime_seconds)],
      [copy.serverDetailPage.rowLoad, load],
      [copy.serverDetailPage.rowProcess, numberLabel(state.process_count)],
      [copy.serverDetailPage.rowConnection, tcp === undefined && udp === undefined ? "" : `TCP ${tcp ?? 0} / UDP ${udp ?? 0}`],
      [copy.serverDetailPage.rowReport, platformDetails.report],
      [copy.serverDetailPage.rowFlags, platformDetails.flags],
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
            {copy.serverDetailPage.showMissing}
          </label>
          <StatusBadge tone={server.status === "online" ? "green" : server.status === "revoked" ? "yellow" : "red"}>
            {serverStatusLabel(server.status, copy)}
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
        {hostRows.length || showMissing ? <InfoGroup title={copy.serverDetailPage.groupHost} rows={hostRows} /> : null}
        {resourceRows.length || showMissing ? <InfoGroup title={copy.serverDetailPage.groupResource} rows={resourceRows} /> : null}
        {runtimeRows.length || showMissing ? <InfoGroup title={copy.serverDetailPage.groupRuntime} rows={runtimeRows} /> : null}
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

function booleanLabel(value: boolean | null | undefined, copy: import("@/lib/i18n").Translations): string | null {
  if (value === undefined || value === null) return null;
  return value ? copy.serverDetailPage.yes : copy.serverDetailPage.no;
}

function trafficQuotaLabel(value?: string | number | null, type?: string | null): string | null {
  const quota = optionalNumber(value);
  const quotaLabel = quota === undefined ? "" : formatBytes(quota);
  const typeLabel = String(type ?? "").trim();
  const parts = [quotaLabel, typeLabel].filter(Boolean);
  return parts.length ? parts.join(" / ") : null;
}

function publicVisibilityLabel(server: ServerDetail, copy: import("@/lib/i18n").Translations): string {
  if (server.hide_for_guest) return copy.serverDetailPage.visibilityGuestHidden;
  if (server.dashboard_visible !== true) return copy.serverDetailPage.visibilityStatusHidden;
  return copy.serverDetailPage.visibilityPublic;
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
  const { t: copy } = useI18n();
  return (
    <BrutalCard>
      <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-xl font-black uppercase">{copy.serverDetailPage.customInfo}</h2>
        </div>
      </div>
      <form onSubmit={onSubmit} className="space-y-4">
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldName}>
            <input className={inputClass} value={form.name} onChange={(event) => onChange({ ...form, name: event.target.value })} required />
          </Field>
          <Field label={copy.serverDetailPage.fieldProvider}>
            <input className={inputClass} value={form.provider} onChange={(event) => onChange({ ...form, provider: event.target.value })} placeholder="Wawo" />
          </Field>
          <Field label={copy.serverDetailPage.fieldRegion}>
            <input className={inputClass} value={form.region} onChange={(event) => onChange({ ...form, region: event.target.value })} placeholder="HK" />
          </Field>
          <Field label={copy.serverDetailPage.fieldPlan}>
            <input className={inputClass} value={form.plan} onChange={(event) => onChange({ ...form, plan: event.target.value })} placeholder="Dedicated" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldCountry}>
            <input className={inputClass} value={form.country} onChange={(event) => onChange({ ...form, country: event.target.value })} placeholder="Hong Kong" />
          </Field>
          <Field label={copy.serverDetailPage.fieldCity}>
            <input className={inputClass} value={form.city} onChange={(event) => onChange({ ...form, city: event.target.value })} placeholder="Hong Kong" />
          </Field>
          <Field label={copy.serverDetailPage.fieldLatitude}>
            <input type="number" min="-90" max="90" step="0.000001" className={inputClass} value={form.latitude} onChange={(event) => onChange({ ...form, latitude: event.target.value })} placeholder="22.3193" />
          </Field>
          <Field label={copy.serverDetailPage.fieldLongitude}>
            <input type="number" min="-180" max="180" step="0.000001" className={inputClass} value={form.longitude} onChange={(event) => onChange({ ...form, longitude: event.target.value })} placeholder="114.1694" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldRemark}>
            <input className={inputClass} value={form.remark} onChange={(event) => onChange({ ...form, remark: event.target.value })} placeholder={copy.serverDetailPage.fieldRemarkPlaceholder} />
          </Field>
          <Field label={copy.serverDetailPage.fieldPublicNote}>
            <input className={inputClass} value={form.public_note} onChange={(event) => onChange({ ...form, public_note: event.target.value })} placeholder={copy.serverDetailPage.fieldPublicNotePlaceholder} />
          </Field>
          <Field label={copy.serverDetailPage.fieldExpiresAt}>
            <input className={inputClass} value={form.expires_at} onChange={(event) => onChange({ ...form, expires_at: event.target.value })} placeholder="2026-12-31" />
          </Field>
          <Field label={copy.serverDetailPage.fieldRenewalPrice}>
            <input className={inputClass} value={form.renewal_price} onChange={(event) => onChange({ ...form, renewal_price: event.target.value })} placeholder={copy.serverDetailPage.fieldRenewalPricePlaceholder} />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldPrice}>
            <input className={inputClass} value={form.price} onChange={(event) => onChange({ ...form, price: event.target.value })} placeholder="12" />
          </Field>
          <Field label={copy.serverDetailPage.fieldCurrency}>
            <input className={inputClass} value={form.currency} onChange={(event) => onChange({ ...form, currency: event.target.value })} placeholder="USD" />
          </Field>
          <Field label={copy.serverDetailPage.fieldBillingCycle}>
            <input className={inputClass} value={form.billing_cycle} onChange={(event) => onChange({ ...form, billing_cycle: event.target.value })} placeholder="monthly" />
          </Field>
          <Field label={copy.serverDetailPage.fieldTrafficQuotaBytes}>
            <input type="number" min="0" step="1" className={inputClass} value={form.traffic_quota_bytes} onChange={(event) => onChange({ ...form, traffic_quota_bytes: event.target.value })} placeholder="1099511627776" />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldQuotaType}>
            <input className={inputClass} value={form.traffic_quota_type} onChange={(event) => onChange({ ...form, traffic_quota_type: event.target.value })} placeholder="monthly / total" />
          </Field>
          <Field label={copy.serverDetailPage.fieldDisplayOrder}>
            <input className={inputClass} value={form.display_order} onChange={(event) => onChange({ ...form, display_order: event.target.value })} placeholder="10" />
          </Field>
          <label className="flex min-h-12 items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input type="checkbox" checked={form.auto_renew} onChange={(event) => onChange({ ...form, auto_renew: event.target.checked })} />
            {copy.serverDetailPage.fieldAutoRenew}
          </label>
          <label className="flex min-h-12 items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
            <input type="checkbox" checked={form.hide_for_guest} onChange={(event) => onChange({ ...form, hide_for_guest: event.target.checked })} />
            {copy.serverDetailPage.fieldHideForGuest}
          </label>
        </div>
        <div className="grid gap-4 md:grid-cols-[minmax(0,1fr)_12rem_12rem] md:items-end">
          <Field label={copy.serverDetailPage.fieldTags}>
            <input className={inputClass} value={form.tags} onChange={(event) => onChange({ ...form, tags: event.target.value })} placeholder={copy.serverDetailPage.fieldTagsPlaceholder} />
          </Field>
          <Field label={copy.serverDetailPage.fieldAccentColor}>
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
            {copy.serverDetailPage.fieldDashboardVisible}
          </label>
        </div>
        <button type="submit" disabled={saving} className={buttonClass("primary")}>
          {copy.serverDetailPage.save}
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
  const { t: copy } = useI18n();
  const hasData = series.some((item) => item.points.length > 0);

  return (
    <section className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-xl font-black uppercase">{title}</h3>
          <p className="mt-1 text-sm font-black text-[var(--text-muted)]">{value}</p>
        </div>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {loading ? copy.serverDetailPage.loading : "LIVE"}
        </span>
      </div>
      {loading ? (
        <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          {copy.serverDetailPage.loadingChart}
        </div>
      ) : hasData ? (
        <MiniLineChart series={series} maxValue={maxValue} formatValue={formatValue} />
      ) : (
        <div className="flex h-44 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          {copy.serverDetailPage.noHistory}
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
  const { t: copy } = useI18n();
  const hasData = latencySeries.some((item) => item.points.length > 0) || lossSeries.some((item) => item.points.length > 0);

  return (
    <section className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-xl font-black uppercase">{copy.serverDetailPage.probeLatencyLoss}</h3>
          <p className="mt-1 text-sm font-black text-[var(--text-muted)]">
            {latestLatency} / {formatPercent(averageLoss)}
          </p>
        </div>
        <span className="border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black shadow-[var(--shadow-brutal-sm)]">
          {loading ? copy.serverDetailPage.loading : "LIVE"}
        </span>
      </div>
      {loading ? (
        <div className="flex h-56 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          {copy.serverDetailPage.loadingChart}
        </div>
      ) : hasData ? (
        <MiniDualAxisChart latencySeries={latencySeries} lossSeries={lossSeries} />
      ) : (
        <div className="flex h-56 items-center justify-center border-2 border-black bg-[var(--accent-bg)] text-sm font-black">
          {copy.serverDetailPage.noProbeHistory}
        </div>
      )}
      {hasData ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {latencySeries.map((item) => (
            <span key={item.id ?? item.label} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
              <span className="h-2.5 w-2.5 border-2 border-black" style={{ background: item.color }} />
              {copy.serverDetailPage.seriesLatencySuffix.replace("{label}", item.label)}
            </span>
          ))}
          {lossSeries.map((item) => (
            <span key={`${item.id ?? item.label}-loss`} className="inline-flex items-center gap-2 border-2 border-black bg-[var(--accent-bg)] px-2 py-1 text-xs font-black">
              <span className="h-2.5 w-2.5 border-2 border-black bg-[#facc15]" />
              {copy.serverDetailPage.seriesLossSuffix.replace("{label}", item.label)}
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
}

const MiniLineChart = memo(function MiniLineChart({ series, maxValue, formatValue }: { series: ChartSeries[]; maxValue?: number; formatValue: (value: number) => string }) {
  const { t: copy } = useI18n();
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
        {copy.serverDetailPage.noHistory}
      </div>
    );
  }
  const timeValues = Array.from(new Set(allPoints.map((point) => point.time))).sort((a, b) => a - b);
  const minTime = timeValues[0];
  const maxTime = timeValues[timeValues.length - 1];
  const maxSeriesValue = maxOf(allPoints.map((point) => point.value), 1);
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
      aria-label={copy.serverDetailPage.metricChartAria}
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
});

const MiniDualAxisChart = memo(function MiniDualAxisChart({ latencySeries, lossSeries }: { latencySeries: ChartSeries[]; lossSeries: ChartSeries[] }) {
  const { t: copy } = useI18n();
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
        {copy.serverDetailPage.noProbeHistory}
      </div>
    );
  }

  const timeValues = Array.from(new Set(allPoints.map((point) => point.time))).sort((a, b) => a - b);
  const minTime = timeValues[0];
  const maxTime = timeValues[timeValues.length - 1];
  const maxLatency = maxOf(latencyPoints.map((point) => point.value), 1);
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
        return point ? { label: copy.serverDetailPage.seriesLatencySuffix.replace("{label}", item.label), color: item.color, value: point.value, time: point.time, kind: "latency" as const } : null;
      }),
      ...lossSeries.map((item) => {
        const point = nearestPoint(item.points, hoverTime);
        return point ? { label: copy.serverDetailPage.seriesLossSuffix.replace("{label}", item.label), color: "#facc15", value: point.value, time: point.time, kind: "loss" as const } : null;
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
      aria-label={copy.serverDetailPage.probeChartAria}
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
});

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
  return values.length ? maxOf(values) : undefined;
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
  return points.length ? minOf(points.map((point) => point.value)) : undefined;
}

function maxPointValue(points: ChartPoint[]): number | undefined {
  return points.length ? maxOf(points.map((point) => point.value)) : undefined;
}

function latestProbeLatency(series: ChartSeries[]): string {
  const latest = series
    .flatMap((item) => item.points)
    .sort((a, b) => a.time - b.time)
    .at(-1);
  return latest ? formatMs(Math.round(latest.value)) : "N/A";
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

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function parsePlatformDetails(info: Record<string, unknown>, copy: import("@/lib/i18n").Translations): { os: string; version: string; agent: string; report: string; flags: string } {
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
    kv.get("report") ? `${copy.serverDetailPage.reportHostPrefix} ${kv.get("report")}` : "",
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
      flagLabel("auto_update_disabled", copy.serverDetailPage.flagAutoUpdate, kv, copy),
      flagLabel("force_update_disabled", copy.serverDetailPage.flagForceUpdate, kv, copy),
      flagLabel("command_disabled", copy.serverDetailPage.flagCommand, kv, copy),
      flagLabel("nat_disabled", "NAT", kv, copy),
      flagLabel("send_query_disabled", copy.serverDetailPage.flagSendQuery, kv, copy),
    ]
      .filter(Boolean)
      .join(" / "),
  };
}

function flagLabel(key: string, label: string, values: Map<string, string>, copy: import("@/lib/i18n").Translations): string {
  const value = values.get(key);
  if (value === undefined) return "";
  return `${label}${value === "true" ? copy.serverDetailPage.flagOff : copy.serverDetailPage.flagOn}`;
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

function serverStatusLabel(status: string, copy: import("@/lib/i18n").Translations): string {
  const labels: Record<string, string> = {
    online: copy.serverDetailPage.statusOnline,
    offline: copy.serverDetailPage.statusOffline,
    revoked: copy.serverDetailPage.statusRevoked,
    down: copy.serverDetailPage.statusDown,
    degraded: copy.serverDetailPage.statusDegraded,
  };
  return labels[status] || status;
}

function initialShowMissingInfo(): boolean {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem("xlstatus_detail_show_missing") === "1";
}
