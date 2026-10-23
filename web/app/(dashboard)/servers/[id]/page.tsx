"use client";

import Link from "next/link";
import { FormEvent, use, useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import {
  MiniDualAxisChart,
  MiniLineChart,
  chartHasData,
  thinPoints,
  type ChartPoint,
  type ChartSeries,
} from "@/app/components/charts";
import { showToast } from "@/app/components/Toasts";
import { apiClient, getApiBaseUrl, type FileEntry, type JsonObject } from "@/lib/api";
import { durationLabel, formatRate, maxOf, minOf, optionalNumber } from "@/app/lib/format";
import {
  normalizeRemotePosixPath,
  resolveSafeApiUrl,
  sanitizeCssVariableValue,
  sanitizeExternalHttpUrl,
} from "@/app/lib/security";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";
import { useStoredPreference } from "@/app/lib/client-store";
import { mapWithConcurrency } from "@/app/lib/concurrency";
import { useBusy } from "@/app/lib/use-busy";

const BINARY_PREFERENCE_VALUES = ["0", "1"] as const;

interface ServerDetail {
  id: string;
  name: string;
  remark?: string | null | undefined;
  public_note?: string | null | undefined;
  expires_at?: string | null | undefined;
  renewal_price?: string | number | null | undefined;
  price?: string | number | null | undefined;
  currency?: string | null | undefined;
  billing_cycle?: string | null | undefined;
  auto_renew?: boolean | null | undefined;
  traffic_quota_bytes?: number | string | null | undefined;
  traffic_quota_type?: string | null | undefined;
  provider?: string | null | undefined;
  region?: string | null | undefined;
  country?: string | null | undefined;
  city?: string | null | undefined;
  latitude?: number | null | undefined;
  longitude?: number | null | undefined;
  location?: ServerLocation | null | undefined;
  plan?: string | null | undefined;
  tags?: string[] | undefined;
  accent_color?: string | null | undefined;
  dashboard_visible?: boolean | null | undefined;
  hide_for_guest?: boolean | null | undefined;
  display_order?: number | null | undefined;
  status: string;
  last_seen_at?: string | null | undefined;
  last_state?: Record<string, unknown> | null | undefined;
  last_info?: Record<string, unknown> | null | undefined;
}

interface ServerLocation {
  source?: string | null | undefined;
  provider?: string | null | undefined;
  country?: string | null | undefined;
  region?: string | null | undefined;
  city?: string | null | undefined;
  latitude?: number | null | undefined;
  longitude?: number | null | undefined;
  timezone?: string | null | undefined;
}

interface MetricSample {
  sample_at: string;
  fields_json?: Record<string, unknown> | null | undefined;
}

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

interface ProbeHistoryEntry {
  service: ServiceSummary;
  results: ServiceResult[];
}

interface ProbeHistory {
  id: string;
  label: string;
  color: string;
  latency: ChartPoint[];
  loss: ChartPoint[];
  lossRate: number;
  minLatency?: number | undefined;
  maxLatency?: number | undefined;
  latestLatency?: number | undefined;
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
  accent_color: "#10b981",
  dashboard_visible: false,
  hide_for_guest: false,
  display_order: "",
};

function metadataFormFromDetail(detail: ServerDetail): ServerMetadataForm {
  const info = detail.last_info ?? {};
  return {
    name: detail.name || "",
    remark: detail.remark || metadataFromRecord(info, ["remark", "note", "description"]) || "",
    public_note:
      detail.public_note || metadataFromRecord(info, ["public_note", "public_description"]) || "",
    expires_at:
      detail.expires_at ||
      metadataFromRecord(info, ["expires_at", "expired_at", "expire_at", "due_at", "end_at"]) ||
      "",
    renewal_price:
      detail.renewal_price === null || detail.renewal_price === undefined
        ? ""
        : String(detail.renewal_price),
    price:
      detail.price === null || detail.price === undefined
        ? metadataFromRecord(info, ["price", "billing_price", "monthly_price", "amount"]) || ""
        : String(detail.price),
    currency: detail.currency || metadataFromRecord(info, ["currency", "currency_code"]) || "",
    billing_cycle:
      detail.billing_cycle ||
      metadataFromRecord(info, ["billing_cycle", "cycle", "billing_period", "period"]) ||
      "",
    auto_renew: detail.auto_renew ?? false,
    traffic_quota_bytes:
      detail.traffic_quota_bytes === null || detail.traffic_quota_bytes === undefined
        ? ""
        : String(detail.traffic_quota_bytes),
    traffic_quota_type:
      detail.traffic_quota_type ||
      metadataFromRecord(info, [
        "traffic_quota_type",
        "quota_type",
        "traffic_type",
        "bandwidth_type",
      ]) ||
      "",
    provider:
      detail.provider || metadataFromRecord(info, ["provider", "vendor", "datacenter", "isp"]) || "",
    region:
      detail.region ||
      metadataFromRecord(info, ["region", "geo_region", "state", "province", "location"]) ||
      "",
    country:
      detail.country || metadataFromRecord(info, ["country", "geo_country", "country_name"]) || "",
    city: detail.city || metadataFromRecord(info, ["city", "geo_city"]) || "",
    latitude:
      detail.latitude === null || detail.latitude === undefined
        ? metadataFromRecord(info, ["latitude", "lat", "geo_latitude"]) || ""
        : String(detail.latitude),
    longitude:
      detail.longitude === null || detail.longitude === undefined
        ? metadataFromRecord(info, ["longitude", "lon", "lng", "geo_longitude"]) || ""
        : String(detail.longitude),
    plan:
      detail.plan ||
      metadataFromRecord(info, ["plan", "package", "sku", "product", "instance_type"]) ||
      "",
    tags: Array.isArray(detail.tags) ? detail.tags.join(", ") : "",
    accent_color: detail.accent_color || "#10b981",
    dashboard_visible: detail.dashboard_visible ?? false,
    hide_for_guest: detail.hide_for_guest ?? false,
    display_order:
      detail.display_order === null || detail.display_order === undefined
        ? ""
        : String(detail.display_order),
  };
}

function probeHistoryKey(serverId: string, range: MetricsRange): string {
  return `${serverId}:${range}`;
}

export default function ServerDetailPage({ params }: PageProps) {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const resolved = params instanceof Promise ? use(params) : params;
  const serverId = resolved.id;
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
  const [metadataBusy, runMetadata] = useBusy();
  const [filesBusy, runFiles] = useBusy();
  const [opsBusy, runOps] = useBusy();
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
  const [showMissingInfoPreference, setShowMissingInfoPreference] = useStoredPreference(
    "xlstatus_detail_show_missing",
    "0",
    BINARY_PREFERENCE_VALUES,
  );
  const showMissingInfo = showMissingInfoPreference === "1";
  const [probeLoading, setProbeLoading] = useState(false);
  const [monitorError, setMonitorError] = useState<string | null>(null);
  const loadedProbeKeyRef = useRef<string | null>(null);

  const loadServer = useCallback(
    async (signal?: AbortSignal) => {
      setLoading(true);
      setError(null);
      const response = await apiClient.getServer(serverId, signal ? { signal } : {});
      if (signal?.aborted) return;
      setLoading(false);
      if (response.success && response.data) {
        const detail = response.data as unknown as ServerDetail;
        setServer(detail);
        setMetadataForm(metadataFormFromDetail(detail));
      } else {
        setError(responseError(response));
      }
    },
    [serverId],
  );

  const loadFiles = useCallback(
    async (nextPath: string) => {
      const safePath = normalizeRemotePosixPath(nextPath);
      if (!safePath) {
        setError("Invalid remote directory path");
        return;
      }
      setFilesLoading(true);
      setError(null);
      const response = await apiClient.listServerFiles(serverId, safePath);
      setFilesLoading(false);
      if (response.success && response.data) {
        setPath(normalizeRemotePosixPath(response.data.path) ?? safePath);
        setFiles(response.data.entries ?? []);
      } else {
        setError(responseError(response));
      }
    },
    [serverId],
  );

  const loadMetrics = useCallback(
    async (signal?: AbortSignal) => {
      setMetricsLoading(true);
      setMonitorError(null);
      const response = await apiClient.getServerMetrics(
        serverId,
        metricsRange,
        signal ? { signal } : {},
      );
      if (signal?.aborted) return;
      setMetricsLoading(false);
      if (response.success && response.data) {
        const series = asRecord(response.data["series"]);
        const samples = Array.isArray(series["samples"]) ? series["samples"] : [];
        setMetrics(
          samples.map((sample) => normalizeMetricSample(sample)).filter(Boolean) as MetricSample[],
        );
      } else {
        setMetrics([]);
        setMonitorError(responseError(response));
      }
    },
    [metricsRange, serverId],
  );

  const loadProbeHistory = useCallback(
    async (signal?: AbortSignal) => {
      setProbeLoading(true);
      setMonitorError(null);
      const historyLimit = historyLimitForRange(metricsRange);
      const servicesResponse = await apiClient.listServices(200, 0, false, {
        ...(signal ? { signal } : {}),
        includeHistory: true,
        historyLimit,
      });
      if (signal?.aborted) return;
      if (!servicesResponse.success || !servicesResponse.data) {
        setProbeHistories([]);
        setProbeLoading(false);
        setMonitorError(responseError(servicesResponse));
        return;
      }

      const services = ((servicesResponse.data["services"] as ServiceSummary[]) ?? []).filter(
        (service) => service.id && serviceBelongsToServer(service, serverId),
      );
      const embedded = readEmbeddedHistories(services, serverId);
      const histories: (ProbeHistoryEntry | null)[] =
        embedded ??
        (await mapWithConcurrency(services, 6, async (service) => {
          if (signal?.aborted) return null;
          const response = await apiClient.getServiceHistory(
            service.id,
            historyLimit,
            signal ? { signal } : {},
          );
          if (!response.success || !response.data) return null;
          const results = Array.isArray(response.data["results"]) ? response.data["results"] : [];
          return {
            service,
            results: results
              .map((result) => normalizeServiceResult(result))
              .filter((result): result is ServiceResult => Boolean(result))
              .filter((result) => result.server_id === serverId),
          };
        }));
      if (signal?.aborted) return;

      const nextHistories = histories
        .filter(
          (item): item is { service: ServiceSummary; results: ServiceResult[] } =>
            Boolean(item && item.results.length),
        )
        .map((item, index) => {
          const sortedResults = item.results
            .slice()
            .sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime());
          const latency = thinPoints(
            sortedResults
              .filter((result) => typeof result.delay_ms === "number")
              .map((result) => ({
                time: new Date(result.created_at).getTime(),
                value: result.delay_ms ?? 0,
              }))
              .filter((point) => Number.isFinite(point.time) && Number.isFinite(point.value)),
          );
          const loss = thinPoints(buildProbeLossPoints(sortedResults));
          return {
            id: item.service.id,
            label: item.service.name || compactId(item.service.id),
            color: chartColors[index % chartColors.length] ?? "var(--accent-color)",
            latency,
            loss,
            lossRate: averagePointValue(loss) ?? packetLossRate(item.results),
            minLatency: minPointValue(latency),
            maxLatency: maxPointValue(latency),
            latestLatency: latestDelay(item.results),
          };
        });

      loadedProbeKeyRef.current = probeHistoryKey(serverId, metricsRange);
      setProbeHistories(nextHistories);
      setSelectedProbeIds((current) =>
        current.filter((id) => nextHistories.some((history) => history.id === id)),
      );
      setProbeLoading(false);
    },
    [metricsRange, serverId],
  );

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
    if (chartTab !== "network") return;
    if (loadedProbeKeyRef.current === probeHistoryKey(serverId, metricsRange)) return;
    const controller = new AbortController();
    void loadProbeHistory(controller.signal);
    return () => controller.abort();
  }, [chartTab, loadProbeHistory, metricsRange, serverId]);

  const statusTone =
    server?.status === "online" ? "green" : server?.status === "revoked" ? "yellow" : "red";
  const currentDir = useMemo(() => parentPath(path), [path]);
  const metricCharts = useMemo(
    () => buildMetricCharts(metrics, server?.last_state),
    [metrics, server?.last_state],
  );
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
    if (next !== "files") setNotice(null);
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
    setShowMissingInfoPreference(next ? "1" : "0");
  }

  async function readSelectedFile(nextPath: string) {
    const safePath = normalizeRemotePosixPath(nextPath);
    if (!safePath) {
      setError(copy.serverDetailPage.selectFilePathFirst);
      return;
    }
    setSelectedPath(safePath);
    const response = await apiClient.readServerFile(serverId, safePath, "utf8");
    if (response.success && response.data) {
      setWritePath(safePath);
      setWriteContent(response.data.content);
      showToast(
        copy.serverDetailPage.readFileNotice
          .replace("{path}", safePath)
          .replace("{bytes}", String(response.data.bytes)),
      );
    } else {
      setError(responseError(response));
    }
  }

  async function writeFile(event: FormEvent) {
    event.preventDefault();
    await runFiles(async () => {
      const safePath = normalizeRemotePosixPath(writePath);
      if (!safePath) {
        setError(copy.serverDetailPage.writePathRequired);
        return;
      }
      const totpCode = await sensitiveTotpCode();
      if (totpCode === null) return;
      const response = await apiClient.writeServerFile(
        serverId,
        {
          path: safePath,
          content: writeContent,
          encoding: "utf8",
          create_dirs: true,
        },
        totpCode,
      );
      if (response.success) {
        setWritePath(safePath);
        showToast(copy.serverDetailPage.writeFileNotice.replace("{path}", safePath));
        await loadFiles(path);
      } else {
        setError(responseError(response));
      }
    });
  }

  async function deleteEntry(entryPath: string, recursive: boolean) {
    await runFiles(async () => {
      const safePath = normalizeRemotePosixPath(entryPath);
      if (!safePath || safePath === "/") {
        setError(copy.serverDetailPage.selectFilePathFirst);
        return;
      }
      if (
        !(await dialogs.confirm({
          message: copy.serverDetailPage.deleteEntryConfirm.replace("{path}", safePath),
          danger: true,
        }))
      ) {
        return;
      }
      const totpCode = await sensitiveTotpCode();
      if (totpCode === null) return;
      const response = await apiClient.deleteServerFile(
        serverId,
        { path: safePath, recursive },
        totpCode,
      );
      if (response.success) {
        showToast(copy.serverDetailPage.deleteEntryNotice.replace("{path}", safePath));
        await loadFiles(path);
      } else {
        setError(responseError(response));
      }
    });
  }

  async function createTempUrl(kind: "download" | "upload") {
    await runFiles(async () => {
      setNotice(null);
      const target = kind === "download" ? selectedPath || writePath : writePath;
      const safeTarget = normalizeRemotePosixPath(target);
      if (!safeTarget) {
        setError(copy.serverDetailPage.selectFilePathFirst);
        return;
      }
      const totpCode = kind === "upload" ? await sensitiveTotpCode() : undefined;
      if (totpCode === null) return;
      const response =
        kind === "download"
          ? await apiClient.getServerDownloadUrl(serverId, safeTarget)
          : await apiClient.getServerUploadUrl(serverId, safeTarget, totpCode);
      if (response.success && response.data) {
        const rawMethod = String(response.data.method ?? "").trim().toUpperCase();
        const safeMethod =
          rawMethod === "GET" || rawMethod === "POST" || rawMethod === "PUT"
            ? rawMethod
            : kind === "download"
              ? "GET"
              : "PUT";
        const safeUrl = resolveSafeApiUrl(response.data.url, getApiBaseUrl());
        if (!safeUrl) {
          setError("Untrusted or malformed transfer URL returned by server");
          return;
        }
        setNotice(`${safeMethod} ${safeUrl}`);
      } else {
        setError(responseError(response));
      }
    });
  }

  async function applyConfig(event: FormEvent) {
    event.preventDefault();
    await runOps(async () => {
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
      const response = await apiClient.applyServerConfig(serverId, config, totpCode);
      if (response.success) {
        showToast(copy.serverDetailPage.configPatchSent);
        await loadServer();
      } else {
        setError(responseError(response));
      }
    });
  }

  async function forceUpdate(event: FormEvent) {
    event.preventDefault();
    await runOps(async () => {
      const version = updateForm.version.trim();
      const rawDownloadUrl = updateForm.download_url.trim();
      const checksum = updateForm.checksum.trim();
      if (!version || !rawDownloadUrl || !checksum) {
        setError(copy.serverDetailPage.updateFieldsRequired);
        return;
      }
      const safeDownloadUrl = sanitizeExternalHttpUrl(rawDownloadUrl);
      if (!safeDownloadUrl) {
        setError("Invalid download URL: only http:// or https:// URLs without credentials are allowed");
        return;
      }
      const totpCode = await sensitiveTotpCode();
      if (totpCode === null) return;
      const response = await apiClient.forceUpdateServer(
        serverId,
        {
          version,
          download_url: safeDownloadUrl,
          checksum,
        },
        totpCode,
      );
      if (response.success) {
        showToast(copy.serverDetailPage.forceUpdateSent);
        setUpdateForm({ version: "", download_url: "", checksum: "" });
      } else {
        setError(responseError(response));
      }
    });
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
    await runMetadata(async () => {
      const rawAccentColor = metadataForm.accent_color.trim();
      const safeAccentColor = rawAccentColor ? sanitizeCssVariableValue(rawAccentColor) : null;
      if (rawAccentColor && !safeAccentColor) {
        setError("Invalid accent color value");
        return;
      }
      const totpCode = await sensitiveTotpCode();
      if (totpCode === null) return;
      const response = await apiClient.updateServer(
        serverId,
        {
          name: metadataForm.name.trim(),
          remark: metadataForm.remark.trim() || null,
          public_note: metadataForm.public_note.trim() || null,
          expires_at: metadataForm.expires_at.trim() || null,
          renewal_price: metadataForm.renewal_price.trim() || null,
          price: metadataForm.price.trim() || null,
          currency: metadataForm.currency.trim() || null,
          billing_cycle: metadataForm.billing_cycle.trim() || null,
          auto_renew: metadataForm.auto_renew,
          traffic_quota_bytes: metadataForm.traffic_quota_bytes.trim()
            ? Number(metadataForm.traffic_quota_bytes)
            : null,
          traffic_quota_type: metadataForm.traffic_quota_type.trim() || null,
          provider: metadataForm.provider.trim() || null,
          region: metadataForm.region.trim() || null,
          country: metadataForm.country.trim() || null,
          city: metadataForm.city.trim() || null,
          latitude: metadataForm.latitude.trim() ? Number(metadataForm.latitude) : null,
          longitude: metadataForm.longitude.trim() ? Number(metadataForm.longitude) : null,
          plan: metadataForm.plan.trim() || null,
          tags: splitTags(metadataForm.tags),
          accent_color: safeAccentColor,
          dashboard_visible: metadataForm.dashboard_visible,
          hide_for_guest: metadataForm.hide_for_guest,
          display_order: metadataForm.display_order.trim()
            ? Number(metadataForm.display_order)
            : null,
        },
        totpCode,
      );
      if (response.success && response.data) {
        const detail = response.data as unknown as ServerDetail;
        setServer(detail);
        setMetadataForm(metadataFormFromDetail(detail));
        showToast(copy.serverDetailPage.metadataSaved);
      } else {
        setError(responseError(response));
      }
    });
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.serverDetailPage.eyebrow}
          title={server?.name || compactId(serverId)}
          detail={copy.serverDetailPage.lastSeen.replace("{time}", formatDate(server?.last_seen_at))}
          actions={
            <div className="flex flex-wrap items-center gap-2">
              {server ? (
                <StatusBadge tone={statusTone}>{serverStatusLabel(server.status, copy)}</StatusBadge>
              ) : null}
              <Link
                href={`/terminal?server_id=${encodeURIComponent(serverId)}`}
                className={buttonClass("primary")}
              >
                &gt;_ Web Terminal
              </Link>
              <Link href="/servers" className={buttonClass("secondary")}>
                {copy.serverDetailPage.back}
              </Link>
            </div>
          }
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading && !server ? <EmptyState title={copy.serverDetailPage.loadingServer} /> : null}

        {server ? (
          <div className="mb-6 grid gap-5">
            <ServerOverview
              server={server}
              showMissing={showMissingInfo}
              onShowMissingChange={changeShowMissingInfo}
            />
            <SecondaryPanelNav active={secondaryPanel} onOpen={openSecondaryPanel} />
            {secondaryPanel === "custom" ? (
              <ServerMetadataEditor
                form={metadataForm}
                saving={metadataBusy}
                onChange={setMetadataForm}
                onSubmit={saveServerMetadata}
              />
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
                saving={filesBusy}
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
                saving={opsBusy}
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
  "#3b82f6",
  "#0ea5e9",
  "#f97316",
  "#10b981",
  "#a855f7",
];

const formatChartPercent = (value: number) => `${value.toFixed(1)}%`;
const formatChartFixed2 = (value: number) => value.toFixed(2);
const formatChartRound = (value: number) => String(Math.round(value));
const formatChartTemperature = (value: number) => `${value.toFixed(1)} °C`;

function SecondaryPanelNav({
  active,
  onOpen,
}: {
  active: SecondaryPanel;
  onOpen: (panel: SecondaryPanel) => void;
}) {
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const panels: Array<{
    id: Exclude<SecondaryPanel, null>;
    label: string;
    hint: string;
    icon: string;
  }> = [
    {
      id: "custom",
      label: copy.serverDetailPage.panelCustom,
      hint: isZh ? "地区、账单、标签与公开展示" : "Geo, billing, tags & visibility",
      icon: "✎",
    },
    {
      id: "files",
      label: copy.serverDetailPage.panelFiles,
      hint: isZh ? "远程文件浏览、读写与临时直链" : "Remote file browser & signed URLs",
      icon: "⊞",
    },
    {
      id: "ops",
      label: copy.serverDetailPage.panelOps,
      hint: isZh ? "遥测上报周期、能力开关与远程热更新" : "Telemetry intervals, flags & OTA update",
      icon: "⚙",
    },
  ];
  return (
    <div className="grid gap-2.5 sm:grid-cols-3">
      {panels.map((item) => {
        const isActive = active === item.id;
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onOpen(item.id)}
            className={`group flex items-center justify-between gap-3 rounded-xl border px-4 py-3 text-left transition-all ${
              isActive
                ? "border-[var(--accent-color)] bg-[var(--accent-soft)] shadow-[var(--shadow-xs)]"
                : "border-[var(--border-color)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]"
            }`}
          >
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span
                  className={`inline-flex h-6 w-6 items-center justify-center rounded-md text-xs font-bold ${
                    isActive
                      ? "bg-[var(--accent-color)] text-white"
                      : "bg-[var(--bg-subtle)] text-[var(--text-secondary)]"
                  }`}
                >
                  {item.icon}
                </span>
                <span className="text-sm font-semibold text-[var(--text-main)]">{item.label}</span>
              </div>
              <p className="mt-1 truncate text-xs text-[var(--text-muted)]">{item.hint}</p>
            </div>
            <span
              className={`shrink-0 rounded-md px-2 py-0.5 text-[11px] font-semibold ${
                isActive
                  ? "bg-[var(--accent-color)] text-white"
                  : "bg-[var(--bg-subtle)] text-[var(--text-muted)] group-hover:text-[var(--text-main)]"
              }`}
            >
              {isActive ? (isZh ? "收起" : "Close") : isZh ? "展开" : "Open"}
            </span>
          </button>
        );
      })}
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
            <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
              {copy.serverDetailPage.fileSystem}
            </h2>
            <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">{path}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button className={buttonClass("secondary")} onClick={() => void onLoad(currentDir)}>
              {copy.serverDetailPage.parentDir}
            </button>
            <button className={buttonClass("secondary")} onClick={() => void onLoad(path)}>
              {copy.serverDetailPage.refresh}
            </button>
          </div>
        </div>
        {filesLoading ? (
          <p className="text-sm text-[var(--text-muted)]">{copy.serverDetailPage.loadingFiles}</p>
        ) : files.length === 0 ? (
          <EmptyState
            title={copy.serverDetailPage.noFilesTitle}
            detail={copy.serverDetailPage.noFilesDetail}
          />
        ) : (
          <div className="overflow-x-auto rounded-lg border border-[var(--border-color)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>{copy.serverDetailPage.colName}</th>
                  <th className={thClass}>{copy.serverDetailPage.colType}</th>
                  <th className={thClass}>{copy.serverDetailPage.colSize}</th>
                  <th className={thClass}>{copy.serverDetailPage.colActions}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-color)]">
                {files.map((entry) => {
                  const nextPath = joinPath(path, entry.name);
                  return (
                    <tr
                      key={`${entry.file_type}-${entry.name}`}
                      className="transition-colors hover:bg-[var(--bg-subtle)]"
                    >
                      <td className={`${tdClass} font-mono text-xs`}>{entry.name}</td>
                      <td className={tdClass}>
                        <StatusBadge tone={entry.file_type === "dir" ? "blue" : "gray"}>
                          {entry.file_type}
                        </StatusBadge>
                      </td>
                      <td className={`${tdClass} font-mono text-xs text-[var(--text-muted)]`}>
                        {formatBytes(entry.size)}
                      </td>
                      <td className={tdClass}>
                        <div className="flex flex-wrap gap-2">
                          {entry.file_type === "dir" ? (
                            <button
                              className={buttonClass("secondary")}
                              onClick={() => void onLoad(nextPath)}
                            >
                              {copy.serverDetailPage.open}
                            </button>
                          ) : (
                            <button
                              className={buttonClass("primary")}
                              onClick={() => void onRead(nextPath)}
                            >
                              {copy.serverDetailPage.read}
                            </button>
                          )}
                          <button
                            className={buttonClass("danger")}
                            onClick={() => void onDelete(nextPath, entry.file_type === "dir")}
                            disabled={saving}
                          >
                            {copy.serverDetailPage.delete}
                          </button>
                        </div>
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
        <h2 className="mb-4 text-base font-semibold tracking-tight text-[var(--text-main)]">
          {copy.serverDetailPage.writeFileTitle}
        </h2>
        <form onSubmit={onWrite} className="space-y-4">
          <Field label={copy.serverDetailPage.pathLabel}>
            <input
              className={inputClass}
              value={writePath}
              onChange={(event) => onWritePathChange(event.target.value)}
              placeholder="/tmp/xlstatus.txt"
            />
          </Field>
          <Field label={copy.serverDetailPage.contentLabel}>
            <textarea
              className={`${textareaClass} min-h-40 font-mono text-xs`}
              value={writeContent}
              onChange={(event) => onWriteContentChange(event.target.value)}
            />
          </Field>
          <div className="flex flex-wrap gap-2">
            <button disabled={saving} className={buttonClass("primary")}>
              {copy.serverDetailPage.writeFileButton}
            </button>
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void onTempUrl("download")}
              disabled={saving}
            >
              {copy.serverDetailPage.downloadUrl}
            </button>
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void onTempUrl("upload")}
              disabled={saving}
            >
              {copy.serverDetailPage.uploadUrl}
            </button>
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
        <h2 className="mb-4 text-base font-semibold tracking-tight text-[var(--text-main)]">
          {copy.serverDetailPage.applyConfigTitle}
        </h2>
        <form onSubmit={onApplyConfig} className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label={copy.serverDetailPage.reportInterval}>
              <input
                className={inputClass}
                value={configForm.report_interval_seconds}
                onChange={(e) =>
                  onConfigChange({ ...configForm, report_interval_seconds: e.target.value })
                }
              />
            </Field>
            <Field label={copy.serverDetailPage.ipReportInterval}>
              <input
                className={inputClass}
                value={configForm.ip_report_interval_seconds}
                onChange={(e) =>
                  onConfigChange({ ...configForm, ip_report_interval_seconds: e.target.value })
                }
              />
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
              <label key={key} className="flex items-center gap-2.5 text-sm font-medium text-[var(--text-main)]">
                <input
                  type="checkbox"
                  checked={configForm[key]}
                  onChange={(e) => onConfigChange({ ...configForm, [key]: e.target.checked })}
                />
                {label}
              </label>
            ))}
          </div>
          <button disabled={saving} className={buttonClass("primary")}>
            {copy.serverDetailPage.applyConfigButton}
          </button>
        </form>
      </BrutalCard>

      <BrutalCard>
        <h2 className="mb-4 text-base font-semibold tracking-tight text-[var(--text-main)]">
          {copy.serverDetailPage.sendUpdateTitle}
        </h2>
        <form onSubmit={onForceUpdate} className="space-y-4">
          <Field label={copy.serverDetailPage.versionLabel}>
            <input
              className={inputClass}
              value={updateForm.version}
              onChange={(e) => onUpdateChange({ ...updateForm, version: e.target.value })}
              placeholder="v0.2"
            />
          </Field>
          <Field label={copy.serverDetailPage.downloadUrlLabel}>
            <input
              className={inputClass}
              value={updateForm.download_url}
              onChange={(e) => onUpdateChange({ ...updateForm, download_url: e.target.value })}
              placeholder="https://github.com/lbyxiaolizi/XLStatus/releases/download/v0.2/xlstatus-agent-linux-x86_64"
            />
          </Field>
          <Field label="SHA-256">
            <input
              className={inputClass}
              value={updateForm.checksum}
              onChange={(e) => onUpdateChange({ ...updateForm, checksum: e.target.value })}
              placeholder={copy.serverDetailPage.checksumPlaceholder}
            />
          </Field>
          <button disabled={saving} className={buttonClass("danger")}>
            {copy.serverDetailPage.sendUpdateButton}
          </button>
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
  const resourceCharts = useMemo(
    () => [
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
        series: [
          {
            id: "memory",
            label: copy.serverDetailPage.chartMemory,
            color: "#3b82f6",
            points: metricCharts.memory,
          },
        ],
        maxValue: 100,
        formatValue: formatChartPercent,
      },
      {
        key: "load",
        title: copy.serverDetailPage.chartLoad,
        value:
          metricCharts.latestLoad === undefined ? "N/A" : metricCharts.latestLoad.toFixed(2),
        series: [
          {
            id: "load",
            label: copy.serverDetailPage.chartLoad,
            color: "#0ea5e9",
            points: metricCharts.load,
          },
        ],
        formatValue: formatChartFixed2,
      },
      {
        key: "disk",
        title: copy.serverDetailPage.chartDisk,
        value: formatPercent(metricCharts.latestDisk),
        series: [
          {
            id: "disk",
            label: copy.serverDetailPage.chartDisk,
            color: "#f97316",
            points: metricCharts.disk,
          },
        ],
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
        value:
          metricCharts.latestProcess === undefined
            ? "N/A"
            : String(Math.round(metricCharts.latestProcess)),
        series: [
          {
            id: "process",
            label: copy.serverDetailPage.chartProcess,
            color: "#10b981",
            points: metricCharts.process,
          },
        ],
        formatValue: formatChartRound,
      },
      {
        key: "connection",
        title: copy.serverDetailPage.chartConnection,
        value: `TCP ${
          metricCharts.latestTcp === undefined ? "N/A" : Math.round(metricCharts.latestTcp)
        } / UDP ${
          metricCharts.latestUdp === undefined ? "N/A" : Math.round(metricCharts.latestUdp)
        }`,
        series: [
          { id: "tcp", label: "TCP", color: "var(--accent-color)", points: metricCharts.tcp },
          { id: "udp", label: "UDP", color: "#64748b", points: metricCharts.udp },
        ],
        formatValue: formatChartRound,
      },
      {
        key: "temperature",
        title: copy.serverDetailPage.chartTemperature,
        value:
          metricCharts.latestTemperature === undefined
            ? "N/A"
            : `${metricCharts.latestTemperature.toFixed(1)} °C`,
        series: [
          {
            id: "temperature",
            label: copy.serverDetailPage.chartTemperature,
            color: "#dc2626",
            points: metricCharts.temperature,
          },
        ],
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
    ],
    [copy, metricCharts],
  );
  const visibleResourceCharts = metricsLoading
    ? resourceCharts
    : resourceCharts.filter((chart) => chartHasData(chart.series));

  return (
    <section className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-xs)]">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3.5">
        <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
          {copy.serverDetailPage.monitoringCharts}
        </h2>
        <div className="flex flex-wrap gap-1.5">
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
      <div className="mb-4 flex items-center gap-1.5 border-b border-[var(--border-color)] pb-3">
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
            className={buttonClass(activeTab === tab ? "primary" : "secondary")}
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
          <EmptyState
            title={copy.serverDetailPage.noResourceChartsTitle}
            detail={copy.serverDetailPage.noResourceChartsDetail}
          />
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
  const displayedLatencySeries = useMemo(
    () =>
      peakCutEnabled
        ? latencySeries.map((item) => ({ ...item, points: smoothPeakPoints(item.points) }))
        : latencySeries,
    [latencySeries, peakCutEnabled],
  );
  const networkSeries = useMemo(
    () => [
      {
        id: "tx",
        label: copy.serverDetailPage.seriesUpload,
        color: "var(--accent-color)",
        points: metricCharts.tx,
      },
      {
        id: "rx",
        label: copy.serverDetailPage.seriesDownload,
        color: "#3b82f6",
        points: metricCharts.rx,
      },
    ],
    [copy, metricCharts],
  );
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
                  className={`inline-flex items-center gap-2.5 rounded-lg border px-3 py-2 text-left text-xs font-medium transition-colors ${
                    active
                      ? "border-[var(--border-strong)] bg-[var(--bg-subtle)] text-[var(--text-main)]"
                      : "border-[var(--border-color)] bg-transparent text-[var(--text-muted)] opacity-60"
                  }`}
                >
                  <span
                    className="h-2.5 w-2.5 shrink-0 rounded-full"
                    style={{ background: history.color }}
                  />
                  <span className="grid gap-0.5">
                    <span className="font-semibold">{history.label}</span>
                    <span className="font-mono text-[11px] text-[var(--text-muted)]">
                      {history.latestLatency === undefined
                        ? "N/A"
                        : formatMs(Math.round(history.latestLatency))}
                      {" / "}↓
                      {history.minLatency === undefined ? "N/A" : Math.round(history.minLatency)}{" "}
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
            <label className="inline-flex h-9 items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 text-xs font-medium text-[var(--text-main)]">
              <input
                type="checkbox"
                checked={peakCutEnabled}
                onChange={(event) => onPeakCutChange(event.target.checked)}
                className="h-3.5 w-3.5 accent-[var(--accent-color)]"
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
        <EmptyState
          title={copy.serverDetailPage.noNetworkChartsTitle}
          detail={copy.serverDetailPage.noNetworkChartsDetail}
        />
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
  const publicNote =
    server.public_note || metadataFromRecord(info, ["public_note", "public_description"]);
  const expiresAt =
    server.expires_at ||
    metadataFromRecord(info, ["expires_at", "expired_at", "expire_at", "due_at", "end_at"]);
  const renewalPrice =
    server.renewal_price ??
    metadataFromRecord(info, ["renewal_price", "renew_price", "renewal", "price", "billing_price"]);
  const price =
    server.price ??
    metadataFromRecord(info, ["price", "billing_price", "monthly_price", "amount"]);
  const currency = server.currency || metadataFromRecord(info, ["currency", "currency_code"]);
  const billingCycle =
    server.billing_cycle ||
    metadataFromRecord(info, ["billing_cycle", "cycle", "billing_period", "period"]);
  const trafficQuotaType =
    server.traffic_quota_type ||
    metadataFromRecord(info, [
      "traffic_quota_type",
      "quota_type",
      "traffic_type",
      "bandwidth_type",
    ]);
  const trafficQuotaBytes =
    server.traffic_quota_bytes ??
    metadataFromRecord(info, [
      "traffic_quota_bytes",
      "traffic_quota",
      "quota_bytes",
      "bandwidth_quota_bytes",
      "monthly_traffic_bytes",
    ]);
  const provider =
    server.provider || metadataFromRecord(info, ["provider", "vendor", "datacenter", "isp"]);
  const region = locationLabel(server, info);
  const plan =
    server.plan ||
    metadataFromRecord(info, ["plan", "package", "sku", "product", "instance_type"]);
  const tags = Array.isArray(server.tags) ? server.tags.filter(Boolean) : [];
  const platformDetails = parsePlatformDetails(info, copy);
  const platform = joinLabels([platformDetails.os, platformDetails.version]);
  const load = [
    optionalNumber(state["load_1"]),
    optionalNumber(state["load_5"]),
    optionalNumber(state["load_15"]),
  ]
    .map((value) => (value === undefined ? null : value.toFixed(2)))
    .filter(Boolean)
    .join(" / ");
  const tcp = optionalNumber(state["tcp_connections"]);
  const udp = optionalNumber(state["udp_connections"]);
  const tiles = filterInfoTiles(
    [
      { label: copy.serverDetailPage.tilePrivateRemark, value: remark, wide: true },
      { label: copy.serverDetailPage.tilePublicNote, value: publicNote, wide: true },
      { label: copy.serverDetailPage.tileProvider, value: provider },
      { label: copy.serverDetailPage.tileRegion, value: region },
      { label: copy.serverDetailPage.tilePlan, value: plan },
      {
        label: copy.serverDetailPage.tileExpires,
        value: expiresAt ? formatDate(expiresAt) : null,
        danger: isExpired(expiresAt),
      },
      {
        label: copy.serverDetailPage.tileRenewal,
        value:
          renewalPrice === null || renewalPrice === undefined || renewalPrice === ""
            ? null
            : String(renewalPrice),
      },
      { label: copy.serverDetailPage.tilePrice, value: billingLabel(price, currency, billingCycle) },
      { label: copy.serverDetailPage.tileAutoRenew, value: booleanLabel(server.auto_renew, copy) },
      {
        label: copy.serverDetailPage.tileTrafficQuota,
        value: trafficQuotaLabel(trafficQuotaBytes, trafficQuotaType),
      },
      { label: copy.serverDetailPage.tilePublicAccess, value: publicVisibilityLabel(server, copy) },
    ],
    showMissing,
  );
  const hostRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowHostname, stringValue(info["hostname"])],
      [copy.serverDetailPage.rowSystem, platform],
      [copy.serverDetailPage.rowKernel, stringValue(info["kernel_version"])],
      [copy.serverDetailPage.rowArch, stringValue(info["arch"])],
      ["Agent", platformDetails.agent],
    ],
    showMissing,
  );
  const resourceRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowCpuCores, numberLabel(info["cpu_cores"])],
      [
        copy.serverDetailPage.rowMemory,
        resourceBytes(state["memory_used"], state["memory_total"], info["total_memory"]),
      ],
      ["Swap", resourceBytes(state["swap_used"], state["swap_total"], info["total_swap"])],
      [copy.serverDetailPage.rowDisk, diskLabel(state, info)],
    ],
    showMissing,
  );
  const runtimeRows = filterInfoRows(
    [
      [copy.serverDetailPage.rowUptime, durationLabel(state["uptime_seconds"])],
      [copy.serverDetailPage.rowLoad, load],
      [copy.serverDetailPage.rowProcess, numberLabel(state["process_count"])],
      [
        copy.serverDetailPage.rowConnection,
        tcp === undefined && udp === undefined ? "" : `TCP ${tcp ?? 0} / UDP ${udp ?? 0}`,
      ],
      [copy.serverDetailPage.rowReport, platformDetails.report],
      [copy.serverDetailPage.rowFlags, platformDetails.flags],
    ],
    showMissing,
  );

  return (
    <section className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-xs)]">
      <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="break-words text-xl font-semibold tracking-tight text-[var(--text-main)]">
            {server.name}
          </h2>
          <p className="mt-1 break-all font-mono text-xs text-[var(--text-muted)]">{server.id}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="inline-flex h-9 items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 text-xs font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={showMissing}
              onChange={(event) => onShowMissingChange(event.target.checked)}
              className="h-3.5 w-3.5 accent-[var(--accent-color)]"
            />
            {copy.serverDetailPage.showMissing}
          </label>
          <StatusBadge
            tone={
              server.status === "online" ? "green" : server.status === "revoked" ? "yellow" : "red"
            }
          >
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
        <div className="mt-3 flex flex-wrap gap-1.5">
          {tags.map((tag) => (
            <span
              key={tag}
              className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 text-xs font-medium text-[var(--text-muted)]"
            >
              {tag}
            </span>
          ))}
        </div>
      ) : null}

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        {hostRows.length || showMissing ? (
          <InfoGroup title={copy.serverDetailPage.groupHost} rows={hostRows} />
        ) : null}
        {resourceRows.length || showMissing ? (
          <InfoGroup title={copy.serverDetailPage.groupResource} rows={resourceRows} />
        ) : null}
        {runtimeRows.length || showMissing ? (
          <InfoGroup title={copy.serverDetailPage.groupRuntime} rows={runtimeRows} />
        ) : null}
      </div>
    </section>
  );
}

function InfoGroup({ title, rows }: { title: string; rows: Array<[string, string]> }) {
  return (
    <div className="rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
      <h3 className="mb-3 text-xs font-semibold tracking-wider uppercase text-[var(--text-muted)]">
        {title}
      </h3>
      <dl className="grid gap-2">
        {rows.map(([label, value]) => (
          <div
            key={label}
            className="grid grid-cols-[5.5rem_minmax(0,1fr)] gap-2 border-t border-[var(--border-color)] pt-2 first:border-t-0 first:pt-0"
          >
            <dt className="text-xs font-medium text-[var(--text-muted)]">{label}</dt>
            <dd className="min-w-0 break-words font-mono text-xs font-medium text-[var(--text-main)]">
              {value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function InfoTile({
  label,
  value,
  danger = false,
  wide = false,
}: {
  label: string;
  value: string;
  danger?: boolean | undefined;
  wide?: boolean | undefined;
}) {
  return (
    <div
      className={`rounded-lg border p-3 ${
        danger
          ? "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300"
          : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-main)]"
      } ${wide ? "sm:col-span-2" : ""}`}
    >
      <div className="text-[11px] font-medium text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 break-words text-sm font-semibold">{value}</div>
    </div>
  );
}

function filterInfoTiles<
  T extends { value?: string | null | undefined; danger?: boolean | undefined; wide?: boolean | undefined },
>(tiles: T[], showMissing: boolean): T[] {
  return showMissing ? tiles : tiles.filter((tile) => !isMissingInfoValue(tile.value));
}

function filterInfoRows(
  rows: Array<[string, string | null | undefined]>,
  showMissing: boolean,
): Array<[string, string]> {
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

function booleanLabel(value: boolean | null | undefined, copy: Translations): string | null {
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

function publicVisibilityLabel(server: ServerDetail, copy: Translations): string {
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
          <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
            {copy.serverDetailPage.customInfo}
          </h2>
        </div>
      </div>
      <form onSubmit={onSubmit} className="space-y-4">
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldName}>
            <input
              className={inputClass}
              value={form.name}
              onChange={(event) => onChange({ ...form, name: event.target.value })}
              required
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldProvider}>
            <input
              className={inputClass}
              value={form.provider}
              onChange={(event) => onChange({ ...form, provider: event.target.value })}
              placeholder="Wawo"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldRegion}>
            <input
              className={inputClass}
              value={form.region}
              onChange={(event) => onChange({ ...form, region: event.target.value })}
              placeholder="HK"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldPlan}>
            <input
              className={inputClass}
              value={form.plan}
              onChange={(event) => onChange({ ...form, plan: event.target.value })}
              placeholder="Dedicated"
            />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldCountry}>
            <input
              className={inputClass}
              value={form.country}
              onChange={(event) => onChange({ ...form, country: event.target.value })}
              placeholder="Hong Kong"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldCity}>
            <input
              className={inputClass}
              value={form.city}
              onChange={(event) => onChange({ ...form, city: event.target.value })}
              placeholder="Hong Kong"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldLatitude}>
            <input
              type="number"
              min="-90"
              max="90"
              step="0.000001"
              className={inputClass}
              value={form.latitude}
              onChange={(event) => onChange({ ...form, latitude: event.target.value })}
              placeholder="22.3193"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldLongitude}>
            <input
              type="number"
              min="-180"
              max="180"
              step="0.000001"
              className={inputClass}
              value={form.longitude}
              onChange={(event) => onChange({ ...form, longitude: event.target.value })}
              placeholder="114.1694"
            />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldRemark}>
            <input
              className={inputClass}
              value={form.remark}
              onChange={(event) => onChange({ ...form, remark: event.target.value })}
              placeholder={copy.serverDetailPage.fieldRemarkPlaceholder}
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldPublicNote}>
            <input
              className={inputClass}
              value={form.public_note}
              onChange={(event) => onChange({ ...form, public_note: event.target.value })}
              placeholder={copy.serverDetailPage.fieldPublicNotePlaceholder}
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldExpiresAt}>
            <input
              className={inputClass}
              value={form.expires_at}
              onChange={(event) => onChange({ ...form, expires_at: event.target.value })}
              placeholder="2026-12-31"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldRenewalPrice}>
            <input
              className={inputClass}
              value={form.renewal_price}
              onChange={(event) => onChange({ ...form, renewal_price: event.target.value })}
              placeholder={copy.serverDetailPage.fieldRenewalPricePlaceholder}
            />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldPrice}>
            <input
              className={inputClass}
              value={form.price}
              onChange={(event) => onChange({ ...form, price: event.target.value })}
              placeholder="12"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldCurrency}>
            <input
              className={inputClass}
              value={form.currency}
              onChange={(event) => onChange({ ...form, currency: event.target.value })}
              placeholder="USD"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldBillingCycle}>
            <input
              className={inputClass}
              value={form.billing_cycle}
              onChange={(event) => onChange({ ...form, billing_cycle: event.target.value })}
              placeholder="monthly"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldTrafficQuotaBytes}>
            <input
              type="number"
              min="0"
              step="1"
              className={inputClass}
              value={form.traffic_quota_bytes}
              onChange={(event) => onChange({ ...form, traffic_quota_bytes: event.target.value })}
              placeholder="1099511627776"
            />
          </Field>
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={copy.serverDetailPage.fieldQuotaType}>
            <input
              className={inputClass}
              value={form.traffic_quota_type}
              onChange={(event) => onChange({ ...form, traffic_quota_type: event.target.value })}
              placeholder="monthly / total"
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldDisplayOrder}>
            <input
              className={inputClass}
              value={form.display_order}
              onChange={(event) => onChange({ ...form, display_order: event.target.value })}
              placeholder="10"
            />
          </Field>
          <label className="flex min-h-10 items-center gap-2.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-sm font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={form.auto_renew}
              onChange={(event) => onChange({ ...form, auto_renew: event.target.checked })}
            />
            {copy.serverDetailPage.fieldAutoRenew}
          </label>
          <label className="flex min-h-10 items-center gap-2.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-sm font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={form.hide_for_guest}
              onChange={(event) => onChange({ ...form, hide_for_guest: event.target.checked })}
            />
            {copy.serverDetailPage.fieldHideForGuest}
          </label>
        </div>
        <div className="grid gap-4 md:grid-cols-[minmax(0,1fr)_12rem_12rem] md:items-end">
          <Field label={copy.serverDetailPage.fieldTags}>
            <input
              className={inputClass}
              value={form.tags}
              onChange={(event) => onChange({ ...form, tags: event.target.value })}
              placeholder={copy.serverDetailPage.fieldTagsPlaceholder}
            />
          </Field>
          <Field label={copy.serverDetailPage.fieldAccentColor}>
            <div className="grid grid-cols-[3rem_minmax(0,1fr)] gap-2">
              <input
                type="color"
                className="h-9 w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                value={isHexColor(form.accent_color) ? form.accent_color : "#10b981"}
                onChange={(event) => onChange({ ...form, accent_color: event.target.value })}
              />
              <input
                className={inputClass}
                value={form.accent_color}
                onChange={(event) => onChange({ ...form, accent_color: event.target.value })}
                placeholder="#10b981"
              />
            </div>
          </Field>
          <label className="flex min-h-10 items-center gap-2.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-sm font-medium text-[var(--text-main)]">
            <input
              type="checkbox"
              checked={form.dashboard_visible}
              onChange={(event) => onChange({ ...form, dashboard_visible: event.target.checked })}
            />
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
  maxValue?: number | undefined;
  formatValue: (value: number) => string;
}) {
  const { t: copy } = useI18n();
  const hasData = series.some((item) => item.points.length > 0);

  return (
    <section className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-[var(--text-main)]">{title}</h3>
          <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">{value}</p>
        </div>
        <StatusBadge tone="green">{loading ? copy.serverDetailPage.loading : "LIVE"}</StatusBadge>
      </div>
      {loading ? (
        <div className="flex h-44 items-center justify-center rounded-lg border border-dashed border-[var(--border-color)] bg-[var(--bg-card)] text-xs text-[var(--text-muted)]">
          {copy.serverDetailPage.loadingChart}
        </div>
      ) : hasData ? (
        <MiniLineChart
          series={series}
          maxValue={maxValue}
          formatValue={formatValue}
          emptyLabel={copy.serverDetailPage.noHistory}
          ariaLabel={copy.serverDetailPage.metricChartAria}
        />
      ) : (
        <div className="flex h-44 items-center justify-center rounded-lg border border-dashed border-[var(--border-color)] bg-[var(--bg-card)] text-xs text-[var(--text-muted)]">
          {copy.serverDetailPage.noHistory}
        </div>
      )}
      {hasData ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {series.map((item) => (
            <span
              key={item.id ?? item.label}
              className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2 py-0.5 text-xs font-medium text-[var(--text-muted)]"
            >
              <span className="h-2 w-2 rounded-full" style={{ background: item.color }} />
              {item.label}
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
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
  averageLoss?: number | undefined;
}) {
  const { t: copy } = useI18n();
  const hasData =
    latencySeries.some((item) => item.points.length > 0) ||
    lossSeries.some((item) => item.points.length > 0);

  return (
    <section className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-[var(--text-main)]">
            {copy.serverDetailPage.probeLatencyLoss}
          </h3>
          <p className="mt-0.5 font-mono text-xs text-[var(--text-muted)]">
            {latestLatency} / {formatPercent(averageLoss)}
          </p>
        </div>
        <StatusBadge tone="green">{loading ? copy.serverDetailPage.loading : "LIVE"}</StatusBadge>
      </div>
      {loading ? (
        <div className="flex h-56 items-center justify-center rounded-lg border border-dashed border-[var(--border-color)] bg-[var(--bg-card)] text-xs text-[var(--text-muted)]">
          {copy.serverDetailPage.loadingChart}
        </div>
      ) : hasData ? (
        <MiniDualAxisChart
          latencySeries={latencySeries}
          lossSeries={lossSeries}
          emptyLabel={copy.serverDetailPage.noProbeHistory}
          ariaLabel={copy.serverDetailPage.probeChartAria}
          latencyLabelTemplate={copy.serverDetailPage.seriesLatencySuffix}
          lossLabelTemplate={copy.serverDetailPage.seriesLossSuffix}
        />
      ) : (
        <div className="flex h-56 items-center justify-center rounded-lg border border-dashed border-[var(--border-color)] bg-[var(--bg-card)] text-xs text-[var(--text-muted)]">
          {copy.serverDetailPage.noProbeHistory}
        </div>
      )}
      {hasData ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {latencySeries.map((item) => (
            <span
              key={item.id ?? item.label}
              className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2 py-0.5 text-xs font-medium text-[var(--text-muted)]"
            >
              <span className="h-2 w-2 rounded-full" style={{ background: item.color }} />
              {copy.serverDetailPage.seriesLatencySuffix.replace("{label}", item.label)}
            </span>
          ))}
          {lossSeries.map((item) => (
            <span
              key={`${item.id ?? item.label}-loss`}
              className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2 py-0.5 text-xs font-medium text-[var(--text-muted)]"
            >
              <span className="h-2 w-2 rounded-full bg-amber-400" />
              {copy.serverDetailPage.seriesLossSuffix.replace("{label}", item.label)}
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
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
  let previous: {
    time: number;
    rxTotal?: number | undefined;
    txTotal?: number | undefined;
  } | null = null;

  for (const row of rows) {
    const state = row.state;
    const cpuValue = optionalNumber(state["cpu_percent"]);
    const memoryUsed = optionalNumber(state["memory_used"]);
    const memoryTotal = optionalNumber(state["memory_total"]);
    const diskPercent = diskUsagePercent(state);
    const swapPercent = percentFromUsedTotal(state["swap_used"], state["swap_total"]);
    const loadValue = optionalNumber(state["load_1"]) ?? optionalNumber(state["load1"]);
    const processValue = optionalNumber(state["process_count"]);
    const tcpValue = optionalNumber(state["tcp_connections"]);
    const udpValue = optionalNumber(state["udp_connections"]);
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
      rx.push({
        time: row.time,
        value: rateFromPrevious(row.time, rxTotal, previous?.time, previous?.rxTotal),
      });
    }
    if (txTotal !== undefined) {
      tx.push({
        time: row.time,
        value: rateFromPrevious(row.time, txTotal, previous?.time, previous?.txTotal),
      });
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
    const valid = window.filter(
      (value) => Math.abs(value - median) <= 3 * mad && value <= Math.max(median * 3, median + mad),
    );
    const next = valid.length
      ? valid.reduce((sum, value) => sum + value, 0) / valid.length
      : median;
    previous = previous === null ? next : alpha * next + (1 - alpha) * previous;
    return { ...point, value: previous };
  });
}

function medianValue(values: number[]): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2
    : (sorted[middle] ?? 0);
}

function normalizeMetricSample(sample: unknown): MetricSample | null {
  const row = asRecord(sample);
  const sampleAt = asString(row["sample_at"]);
  if (!sampleAt) return null;
  return {
    sample_at: sampleAt,
    fields_json: asRecord(row["fields_json"]),
  };
}

function normalizeServiceResult(result: unknown): ServiceResult | null {
  const row = asRecord(result);
  const serviceId = asString(row["service_id"]);
  const createdAt = asString(row["created_at"]);
  if (!serviceId || !createdAt) return null;
  return {
    service_id: serviceId,
    server_id: stringValue(row["server_id"]) || null,
    status: asString(row["status"]),
    delay_ms: optionalNumber(row["delay_ms"]),
    created_at: createdAt,
  };
}

function readEmbeddedHistories(
  services: ServiceSummary[],
  serverId: string,
): ProbeHistoryEntry[] | null {
  const entries: ProbeHistoryEntry[] = [];
  for (const service of services) {
    const history = asRecord(service)["history"];
    if (!Array.isArray(history)) return null;
    entries.push({
      service,
      results: history
        .map((result) => normalizeServiceResult(result))
        .filter((result): result is ServiceResult => Boolean(result))
        .filter((result) => result.server_id === serverId),
    });
  }
  return entries;
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
    value: serviceResultOk(row.status) ? (derivedLoss[index] ?? 0) : 100,
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
        const variance =
          windowDelays.reduce((sum, delay) => sum + (delay - mean) ** 2, 0) / windowDelays.length;
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
      lossRate = 0.3 * lossRate + 0.7 * (packetLossRates[index - 1] ?? 0);
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
  return (
    results
      .filter((result) => typeof result.delay_ms === "number")
      .sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime())
      .at(-1)?.delay_ms ?? undefined
  );
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

function rateFromPrevious(
  time: number,
  total: number,
  previousTime?: number,
  previousTotal?: number,
): number {
  if (
    previousTime === undefined ||
    previousTotal === undefined ||
    time <= previousTime ||
    total < previousTotal
  ) {
    return 0;
  }
  return (total - previousTotal) / ((time - previousTime) / 1000);
}

function percentFromUsedTotal(usedValue: unknown, totalValue: unknown): number | undefined {
  const used = optionalNumber(usedValue);
  const total = optionalNumber(totalValue);
  if (used === undefined || !total || total <= 0) return undefined;
  return (used / total) * 100;
}

function diskUsagePercent(state: Record<string, unknown>): number | undefined {
  const direct =
    optionalNumber(state["disk_percent"]) ??
    percentFromUsedTotal(state["disk_used"], state["disk_total"]);
  if (direct !== undefined) return direct;
  const disks = Array.isArray(state["disks"]) ? state["disks"] : [];
  const used = disks.reduce(
    (sum: number, item: unknown) => sum + (optionalNumber(asRecord(item)["used"]) ?? 0),
    0,
  );
  const total = disks.reduce(
    (sum: number, item: unknown) => sum + (optionalNumber(asRecord(item)["total"]) ?? 0),
    0,
  );
  return total > 0 ? (used / total) * 100 : undefined;
}

function gpuUsagePercent(state: Record<string, unknown>): number | undefined {
  return (
    optionalNumber(state["gpu_percent"]) ??
    optionalNumber(state["gpu_usage"]) ??
    optionalNumber(state["gpu"])
  );
}

function maxTemperature(state: Record<string, unknown>): number | undefined {
  const direct = optionalNumber(state["temperature"]) ?? optionalNumber(state["temp"]);
  if (direct !== undefined) return direct;
  const sensors = Array.isArray(state["temperatures"]) ? state["temperatures"] : [];
  const values = sensors
    .map((item: unknown) => {
      if (typeof item === "number") return item;
      const sensor = asRecord(item);
      return (
        optionalNumber(sensor["temperature"]) ??
        optionalNumber(sensor["temp"]) ??
        optionalNumber(sensor["value"])
      );
    })
    .filter((value): value is number => value !== undefined);
  return values.length ? maxOf(values) : undefined;
}

function netTotal(
  state: Record<string, unknown>,
  field: "bytes_recv" | "bytes_sent",
  fallbackKey: string,
): number | undefined {
  const fallback = optionalNumber(state[fallbackKey]);
  if (fallback !== undefined) return fallback;
  const netIo = Array.isArray(state["net_io"]) ? state["net_io"] : [];
  const total = netIo.reduce(
    (sum: number, item: unknown) => sum + (optionalNumber(asRecord(item)[field]) ?? 0),
    0,
  );
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
    server.location?.country ??
      server.country ??
      metadataFromRecord(info, ["country", "geo_country", "country_name"]),
    server.location?.region ??
      server.region ??
      metadataFromRecord(info, ["region", "geo_region", "state", "province", "location"]),
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

function parsePlatformDetails(
  info: Record<string, unknown>,
  copy: Translations,
): { os: string; version: string; agent: string; report: string; flags: string } {
  const os = stringValue(info["platform"]) || stringValue(info["os"]);
  const rawVersion = stringValue(info["platform_version"]) || stringValue(info["os_version"]);
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

function flagLabel(
  key: string,
  label: string,
  values: Map<string, string>,
  copy: Translations,
): string {
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
  if (used !== undefined && total !== undefined && total > 0) {
    return `${formatBytes(used)} / ${formatBytes(total)}`;
  }
  return total === undefined ? "N/A" : formatBytes(total);
}

function diskLabel(state: Record<string, unknown>, info: Record<string, unknown>): string {
  const stateDisks = Array.isArray(state["disks"]) ? state["disks"] : [];
  const infoDisks = Array.isArray(info["disks"]) ? info["disks"] : [];
  const disks = stateDisks.length ? stateDisks : infoDisks;
  const used = disks.reduce(
    (sum: number, item: unknown) => sum + (optionalNumber(asRecord(item)["used"]) ?? 0),
    0,
  );
  const total = disks.reduce(
    (sum: number, item: unknown) => sum + (optionalNumber(asRecord(item)["total"]) ?? 0),
    0,
  );
  if (used > 0 && total > 0) return `${formatBytes(used)} / ${formatBytes(total)}`;
  return total > 0 ? formatBytes(total) : "N/A";
}

function isExpired(value?: string | null): boolean {
  if (!value) return false;
  const date = new Date(value);
  return !Number.isNaN(date.getTime()) && date.getTime() < Date.now();
}

function joinPath(base: string, name: string): string {
  const safeBase = normalizeRemotePosixPath(base) ?? "/";
  const cleanName = name.replace(/[/\\]+/g, "").trim();
  if (!cleanName || cleanName === "." || cleanName === "..") {
    return safeBase;
  }
  const combined = safeBase === "/" ? `/${cleanName}` : `${safeBase}/${cleanName}`;
  return normalizeRemotePosixPath(combined) ?? safeBase;
}

function parentPath(value: string): string {
  const safeValue = normalizeRemotePosixPath(value);
  if (!safeValue || safeValue === "/") return "/";
  const index = safeValue.lastIndexOf("/");
  return index <= 0 ? "/" : safeValue.slice(0, index);
}

function serverStatusLabel(status: string, copy: Translations): string {
  const labels: Record<string, string> = {
    online: copy.serverDetailPage.statusOnline,
    offline: copy.serverDetailPage.statusOffline,
    revoked: copy.serverDetailPage.statusRevoked,
    down: copy.serverDetailPage.statusDown,
    degraded: copy.serverDetailPage.statusDegraded,
  };
  return labels[status] ?? status;
}
