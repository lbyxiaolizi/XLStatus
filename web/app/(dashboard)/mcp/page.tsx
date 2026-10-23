"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  KpiCard,
  PageHeader,
  PageShell,
  ProgressBar,
  StatusBadge,
  asNumber,
  asRecord,
  asString,
  buttonClass,
  compactId,
  formatMs,
  formatPercent,
  inputClass,
  responseError,
  selectClass,
  textareaClass,
  type BadgeTone,
} from "@/app/components/M7Primitives";
import { useServerData, type Server } from "@/app/components/ServerDataProvider";
import { showToast } from "@/app/components/Toasts";
import { useSensitiveTotp } from "@/app/(dashboard)/settings/sections/use-sensitive-totp";
import { maskSensitiveToken, redactSensitiveTokens } from "@/app/lib/security";
import { useBusy } from "@/app/lib/use-busy";
import {
  apiClient,
  getApiBaseUrl,
  type McpCatalogResponse,
  type McpPromptDefinition,
  type McpResourceDefinition,
  type McpResourceTemplateDefinition,
  type McpToolDefinition,
} from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";

type WorkspaceTab = "diagnose" | "playground" | "connect";
type PlaygroundKind = "tool" | "resource" | "prompt";
type TransportMode = "studio" | "jsonrpc";
type ClientConfigTab = "streamable-http" | "claude-desktop" | "claude-code" | "curl-smoke";
type PatPresetId = "readonly" | "standard" | "autonomous";

interface PlaybookCardDef {
  id: string;
  titleZh: string;
  titleEn: string;
  subtitleZh: string;
  subtitleEn: string;
  badge: string;
  batchCommand: string;
}

interface NormalizedAnomaly {
  id: string;
  severity: "critical" | "warning" | "info";
  category: string;
  resourceName: string;
  resourceId: string;
  title: string;
  evidence: string;
  recommendation: string;
  suggestedTool: string;
  suggestedArgs: Record<string, unknown>;
}

interface NormalizedDiagnoseReport {
  healthScore: number;
  status: "healthy" | "degraded" | "critical";
  summary: string;
  anomalies: NormalizedAnomaly[];
  checkedAt: string;
  durationMs: number;
  rawOutput: unknown;
}

interface ConsoleOutputState {
  title: string;
  subtitle: string;
  durationMs: number;
  status: "ok" | "error";
  outputText: string;
  raw: unknown;
}

interface PatPreset {
  id: PatPresetId;
  titleZh: string;
  titleEn: string;
  descZh: string;
  descEn: string;
  scopes: string[];
  tone: BadgeTone;
}

const PLAYBOOK_MATRIX: readonly PlaybookCardDef[] = [
  {
    id: "system_vital",
    titleZh: "系统生命体征 / System Vitals",
    titleEn: "System Vitals / 系统生命体征",
    subtitleZh: "内核版本、运行时长、负载均值、CPU 与物理内存快照",
    subtitleEn: "Kernel, uptime, load averages, CPU & memory pressure snapshot",
    badge: "system_vital",
    batchCommand: "uname -a && uptime && (free -m 2>/dev/null || vm_stat 2>/dev/null | head -n 10)",
  },
  {
    id: "top_processes",
    titleZh: "高负载进程排查 / Top Processes",
    titleEn: "Top Processes / 高负载进程排查",
    subtitleZh: "按 CPU 与常驻内存排序定位热点进程与僵尸任务",
    subtitleEn: "Rank top CPU and RSS memory consumers across processes",
    badge: "top_processes",
    batchCommand: "ps aux --sort=-%cpu 2>/dev/null | head -n 15 || ps aux -r | head -n 15",
  },
  {
    id: "disk_and_inodes",
    titleZh: "磁盘与 Inode 审计 / Disk & Inodes",
    titleEn: "Disk & Inodes / 磁盘与 Inode 审计",
    subtitleZh: "检查各挂载点容量水位与 Inode 耗尽风险",
    subtitleEn: "Inspect filesystem capacity watermarks and inode exhaustion",
    badge: "disk_and_inodes",
    batchCommand: "df -h && echo '--- INODES ---' && df -i 2>/dev/null | head -n 15",
  },
  {
    id: "network_sockets",
    titleZh: "网络套接字与监听 / Network Sockets",
    titleEn: "Network Sockets / 网络套接字与监听",
    subtitleZh: "审计当前 TCP/UDP 监听端口与活跃连接状态统计",
    subtitleEn: "Audit listening TCP/UDP sockets and active connection states",
    badge: "network_sockets",
    batchCommand: "ss -tulpn 2>/dev/null | head -n 25 || netstat -an 2>/dev/null | head -n 25",
  },
  {
    id: "docker_containers",
    titleZh: "容器运行状态 / Docker Containers",
    titleEn: "Docker Containers / 容器运行状态",
    subtitleZh: "巡检运行中容器状态、重启次数及资源占用",
    subtitleEn: "Inspect running containers, restart loops, and resource stats",
    badge: "docker_containers",
    batchCommand: "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}' 2>/dev/null || echo 'docker not installed or not accessible'",
  },
  {
    id: "system_logs",
    titleZh: "系统异常日志 / Recent System Logs",
    titleEn: "Recent System Logs / 系统异常日志",
    subtitleZh: "提取近期 systemd journal / syslog 关键错误与告警事件",
    subtitleEn: "Tail recent high-priority systemd journal and kernel warnings",
    badge: "system_logs",
    batchCommand: "journalctl -p err..emerg -n 25 --no-pager 2>/dev/null || tail -n 25 /var/log/syslog /var/log/messages 2>/dev/null || dmesg | tail -n 20",
  },
];

const PAT_PRESETS: readonly PatPreset[] = [
  {
    id: "readonly",
    titleZh: "只读巡检模式 / Read-Only SRE Observer",
    titleEn: "Read-Only SRE Observer / 只读巡检模式",
    descZh: "仅允许读取服务器遥测、服务拨测与只读文件审计，零变更风险。",
    descEn: "Read-only access to fleet telemetry, service probes, and file inspection.",
    scopes: ["server:read", "service:read", "transfer:read"],
    tone: "green",
  },
  {
    id: "standard",
    titleZh: "标准运维排障 / Standard SRE Diagnostics & Exec",
    titleEn: "Standard SRE Diagnostics & Exec / 标准运维排障",
    descZh: "支持 AI 巡检、只读诊断剧本（server.inspect）与受控终端命令执行。",
    descEn: "Enables AI diagnostics, SRE playbooks (server.inspect), and command execution.",
    scopes: ["server:read", "server:exec", "transfer:read", "service:read", "task:read"],
    tone: "blue",
  },
  {
    id: "autonomous",
    titleZh: "全权限集群自动驾驶 / Full Autonomous Ops",
    titleEn: "Full Autonomous Ops / 全权限集群自动驾驶",
    descZh: "授予集群全栈巡检、批量自愈执行、自动化任务及网络拓扑管理权限。",
    descEn: "Full cluster operations across servers, tasks, NAT tunnels, and DDNS records.",
    scopes: ["server:read", "server:exec", "transfer:read", "service:read", "task:*", "nat:*", "ddns:*"],
    tone: "pink",
  },
];

const FALLBACK_CATALOG: McpCatalogResponse = {
  protocol_version: "2024-11-05",
  server_info: {
    name: "XLStatus AI-Native MCP Server",
    version: "0.1.0",
  },
  tools: [
    {
      name: "cluster.diagnose",
      description:
        "Run an AI SRE automated anomaly & health scan across all accessible servers, service probes, and alert events. Returns a cluster health score (0-100), prioritized anomalies with root-cause hints, and recommended follow-up tools",
      category: "diagnostics",
      input_schema: {
        type: "object",
        properties: {
          cpu_threshold: { type: "number", description: "CPU usage percentage threshold", default: 85 },
          memory_threshold: { type: "number", description: "Memory usage percentage threshold", default: 85 },
          disk_threshold: { type: "number", description: "Disk usage percentage threshold", default: 85 },
        },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "cluster.overview",
      description:
        "Get a comprehensive fleet-wide summary: online/offline/revoked server counts, aggregate CPU/RAM utilization, network RX/TX throughput, service monitor SLA health, active alerts, and regional breakdown",
      category: "cluster",
      input_schema: { type: "object", properties: {}, required: [] },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "server.inspect",
      description:
        "Run a safe, read-only SRE diagnostic playbook on a live online server (system vitals, top CPU/RAM processes, disk & inode usage, listening sockets, Docker containers, or recent system logs)",
      category: "diagnostics",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Canonical Server UUID" },
          playbook: {
            type: "string",
            enum: [
              "system_vital",
              "top_processes",
              "disk_and_inodes",
              "network_sockets",
              "docker_containers",
              "system_logs",
            ],
            default: "system_vital",
          },
        },
        required: ["server_id"],
      },
      annotations: { readOnlyHint: false, destructiveHint: false },
    },
    {
      name: "server.list",
      description:
        "List all servers accessible to the current user with live CPU, memory, disk, load, network throughput, geo-location, and tags",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          status: { type: "string", enum: ["all", "online", "offline"], default: "all" },
          search: { type: "string", description: "Optional substring filter" },
          limit: { type: "integer", default: 50 },
          offset: { type: "integer", default: 0 },
        },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "server.get",
      description:
        "Get detailed hardware specifications, metadata, and real-time telemetry for a specific server by canonical UUID",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Canonical Server UUID" },
        },
        required: ["server_id"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "server.resolve",
      description:
        "Fuzzy-resolve a server by name, partial UUID, hostname, region, country, provider, or tag into canonical server_id UUIDs",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          query: { type: "string", description: "Search query (server name, partial ID, region, provider, or tag)" },
        },
        required: ["query"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "server.metrics",
      description:
        "Query time-series telemetry (CPU, memory, load_1, network RX/TX) for a server with statistical min/max/avg/latest aggregates",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Canonical Server UUID" },
          range: { type: "string", enum: ["15m", "1h", "6h", "24h"], default: "1h" },
        },
        required: ["server_id"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "server.exec",
      description: "Execute a shell command on a single online server over the mTLS gRPC agent stream",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Canonical Server UUID" },
          command: { type: "string", description: "Command to execute" },
          timeout: { type: "integer", default: 30 },
        },
        required: ["server_id", "command"],
      },
      annotations: { readOnlyHint: false, destructiveHint: true },
    },
    {
      name: "server.batch_exec",
      description:
        "Execute a shell command concurrently across multiple online servers (up to 10 nodes) and aggregate exit codes and stdout/stderr",
      category: "server",
      input_schema: {
        type: "object",
        properties: {
          target: { type: "string", enum: ["selected", "all_online"], default: "all_online" },
          server_ids: { type: "array", items: { type: "string" } },
          command: { type: "string", description: "Shell command to run on each target server" },
          timeout: { type: "integer", default: 20 },
        },
        required: ["command"],
      },
      annotations: { readOnlyHint: false, destructiveHint: true },
    },
    {
      name: "service.list",
      description:
        "List HTTP, TCP, and ICMP service monitors with latest probe status, latency, and TLS certificate expiry",
      category: "service",
      input_schema: {
        type: "object",
        properties: { limit: { type: "integer", default: 50 } },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "service.history",
      description:
        "Get recent probe check results, availability percentage, and average latency for a specific service monitor",
      category: "service",
      input_schema: {
        type: "object",
        properties: {
          service_id: { type: "string", description: "Service Monitor UUID" },
          limit: { type: "integer", default: 30 },
        },
        required: ["service_id"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "alert.events",
      description: "List recent alert events (firing and recovered incidents) across visible servers and services",
      category: "alert",
      input_schema: {
        type: "object",
        properties: {
          kind: { type: "string", default: "all" },
          limit: { type: "integer", default: 25 },
        },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "alert.rules",
      description: "List configured alert rules, trigger conditions, and automated remediation task bindings",
      category: "alert",
      input_schema: {
        type: "object",
        properties: { limit: { type: "integer", default: 50 } },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "task.list",
      description: "List scheduled cron jobs and automation tasks configured in the cluster",
      category: "automation",
      input_schema: {
        type: "object",
        properties: { limit: { type: "integer", default: 25 } },
        required: [],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "network.topology",
      description:
        "Inspect NAT reverse-tunnel port mappings and Dynamic DNS (DDNS) domain records across visible servers",
      category: "network",
      input_schema: { type: "object", properties: {}, required: [] },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "fs.list",
      description: "List files in a directory on a server",
      category: "filesystem",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Server ID" },
          path: { type: "string", description: "Directory path" },
        },
        required: ["server_id", "path"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "fs.read",
      description: "Read a file from a server",
      category: "filesystem",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Server ID" },
          path: { type: "string", description: "File path" },
          max_size: { type: "integer", default: 1048576 },
        },
        required: ["server_id", "path"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "fs.download_url",
      description: "Generate a temporary download URL for a file",
      category: "filesystem",
      input_schema: {
        type: "object",
        properties: {
          server_id: { type: "string", description: "Server ID" },
          path: { type: "string", description: "File path" },
          expires_in: { type: "integer", default: 3600 },
        },
        required: ["server_id", "path"],
      },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
    {
      name: "meta.whoami",
      description: "Get current authenticated identity, scopes, server allowlist, and XLStatus MCP capabilities",
      category: "meta",
      input_schema: { type: "object", properties: {}, required: [] },
      annotations: { readOnlyHint: true, destructiveHint: false },
    },
  ],
  resources: [
    {
      uri: "xlstatus://cluster/overview",
      name: "Cluster Health & Anomaly Overview",
      description:
        "Real-time fleet summary, aggregate resource saturation, network bandwidth, service probe SLAs, and active incidents",
      mime_type: "application/json",
    },
    {
      uri: "xlstatus://servers/inventory",
      name: "Live Server Fleet Inventory & Telemetry",
      description:
        "Full list of accessible servers enriched with CPU, RAM, disk, load average, network RX/TX, geo-location, and tags",
      mime_type: "application/json",
    },
    {
      uri: "xlstatus://services/health",
      name: "Service Probes & SLA Status",
      description:
        "HTTP, TCP, and ICMP service monitors with latest check status, latency, and TLS certificate expiration",
      mime_type: "application/json",
    },
    {
      uri: "xlstatus://alerts/active",
      name: "Active Incidents, Alert Events & Rules",
      description: "Recent alert events and configured threshold rules across the cluster",
      mime_type: "application/json",
    },
    {
      uri: "xlstatus://network/topology",
      name: "NAT Port Mappings & DDNS Records",
      description: "Active reverse-tunnel NAT port mappings and Dynamic DNS domain bindings",
      mime_type: "application/json",
    },
  ],
  resource_templates: [
    {
      uri_template: "xlstatus://servers/{server_id}/telemetry",
      name: "Deep Server Hardware, Live State & 1h TSDB Summary",
      description:
        "Hardware specifications, live host telemetry, and 1-hour time-series statistical aggregates for a specific server UUID",
      mime_type: "application/json",
    },
  ],
  prompts: [
    {
      name: "sre.cluster_health_audit",
      description:
        "Comprehensive AI SRE audit of fleet health, resource bottlenecks, offline nodes, and active incidents",
      arguments: [],
    },
    {
      name: "sre.troubleshoot_node",
      description:
        "Deep-dive root-cause analysis for a specific server node with live telemetry and diagnostic playbook guidance",
      arguments: [
        {
          name: "server_id",
          description: "Canonical Server UUID or server name to troubleshoot",
          required: true,
        },
        {
          name: "symptom",
          description: "Observed symptom (e.g. high_cpu, memory_pressure, disk_full, network_spike, offline)",
          required: false,
        },
      ],
    },
    {
      name: "sre.incident_triage",
      description: "Triage active alerts and degraded service probes with actionable remediation commands",
      arguments: [],
    },
    {
      name: "sre.capacity_and_security",
      description: "Audit cluster resource saturation, expiring nodes, and exposed NAT/DDNS endpoints",
      arguments: [],
    },
  ],
};

export default function McpWorkbenchPage() {
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const dialogs = useDialogs();
  const { servers, loading: serversLoading, refresh: refreshServers } = useServerData();
  const [busy, runBusy] = useBusy();

  const [activeTab, setActiveTab] = useState<WorkspaceTab>("diagnose");
  const [error, setError] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);

  // Catalog state
  const [catalog, setCatalog] = useState<McpCatalogResponse>(FALLBACK_CATALOG);
  const [catalogLoading, setCatalogLoading] = useState(true);

  // Diagnose state
  const [diagnoseLoading, setDiagnoseLoading] = useState(true);
  const [diagnoseReport, setDiagnoseReport] = useState<NormalizedDiagnoseReport | null>(null);
  const [selectedPlaybookServerId, setSelectedPlaybookServerId] = useState<string>("");
  const [activePlaybookId, setActivePlaybookId] = useState<string | null>(null);
  const [consoleOutput, setConsoleOutput] = useState<ConsoleOutputState | null>(null);

  // Prompt generator state
  const [activePromptName, setActivePromptName] = useState<string>("sre.cluster_health_audit");
  const [promptSymptom, setPromptSymptom] = useState<string>("high_cpu");
  const [generatedPromptText, setGeneratedPromptText] = useState<string>("");
  const [promptLoading, setPromptLoading] = useState(false);

  // Playground state
  const [playgroundKind, setPlaygroundKind] = useState<PlaygroundKind>("tool");
  const [playgroundSearch, setPlaygroundSearch] = useState<string>("");
  const [selectedItemName, setSelectedItemName] = useState<string>("cluster.diagnose");
  const [argsEditorText, setArgsEditorText] = useState<string>("{\n  \"cpu_threshold\": 85,\n  \"memory_threshold\": 85,\n  \"disk_threshold\": 85\n}");
  const [transportMode, setTransportMode] = useState<TransportMode>("studio");
  const [livePatToken, setLivePatToken] = useState<string>("");
  const [playgroundRunning, setPlaygroundRunning] = useState(false);
  const [playgroundResult, setPlaygroundResult] = useState<{
    durationMs: number;
    ok: boolean;
    payload: unknown;
  } | null>(null);

  // Connect / PAT Onboarding state
  const [selectedPatPreset, setSelectedPatPreset] = useState<PatPresetId>("standard");
  const [patTokenName, setPatTokenName] = useState<string>("ai-mcp-copilot");
  const [patAllowlistServerIds, setPatAllowlistServerIds] = useState<string[]>([]);
  const [clientConfigTab, setClientConfigTab] = useState<ClientConfigTab>("streamable-http");

  const onlineServers = useMemo(
    () => servers.filter((server) => server.status === "online"),
    [servers],
  );
  const firstOnlineServer = onlineServers[0] ?? servers[0] ?? null;
  const effectivePlaybookServerId =
    selectedPlaybookServerId || firstOnlineServer?.id || "all_online";

  const buildDefaultArgsForItem = useCallback(
    (kind: PlaygroundKind, name: string): Record<string, unknown> => {
      const sampleServerId = firstOnlineServer?.id ?? "00000000-0000-0000-0000-000000000001";
      if (kind === "resource") {
        if (name.includes("{server_id}")) {
          return {
            uri: name.replace("{server_id}", sampleServerId),
            server_id: sampleServerId,
          };
        }
        return { uri: name };
      }
      if (kind === "prompt") {
        if (name === "sre.troubleshoot_node") {
          return { server_id: sampleServerId, symptom: "high_cpu" };
        }
        return {};
      }
      switch (name) {
        case "cluster.diagnose":
          return { cpu_threshold: 85, memory_threshold: 85, disk_threshold: 85 };
        case "server.list":
          return { status: "all", limit: 20, offset: 0 };
        case "server.get":
          return { server_id: sampleServerId };
        case "server.resolve":
          return { query: firstOnlineServer?.name ?? "node" };
        case "server.metrics":
          return { server_id: sampleServerId, range: "1h" };
        case "server.inspect":
          return { server_id: sampleServerId, playbook: "system_vital" };
        case "server.exec":
          return { server_id: sampleServerId, command: "uname -a && uptime", timeout: 20 };
        case "server.batch_exec":
          return { target: "all_online", command: "uptime", timeout: 20 };
        case "service.list":
          return { limit: 25 };
        case "service.history":
          return { service_id: "00000000-0000-0000-0000-000000000001", limit: 20 };
        case "alert.events":
          return { kind: "all", limit: 20 };
        case "alert.rules":
          return { limit: 25 };
        case "task.list":
          return { limit: 25 };
        case "fs.list":
          return { server_id: sampleServerId, path: "/var/log" };
        case "fs.read":
          return { server_id: sampleServerId, path: "/etc/os-release", max_size: 65536 };
        case "fs.download_url":
          return { server_id: sampleServerId, path: "/etc/hosts", expires_in: 300 };
        default:
          return {};
      }
    },
    [firstOnlineServer],
  );

  const applyCatalogAndDiagnoseResponses = useCallback(
    (
      catalogRes: Awaited<ReturnType<typeof apiClient.getMcpCatalog>>,
      diagnoseRes: Awaited<ReturnType<typeof apiClient.executeMcpStudio>>,
    ) => {
      setCatalogLoading(false);
      setDiagnoseLoading(false);

      if (catalogRes.success && catalogRes.data) {
        const data = catalogRes.data;
        setCatalog({
          protocol_version: data.protocol_version || "2024-11-05",
          server_info: data.server_info ?? {},
          tools: Array.isArray(data.tools) && data.tools.length > 0 ? data.tools : FALLBACK_CATALOG.tools,
          resources:
            Array.isArray(data.resources) && data.resources.length > 0
              ? data.resources
              : FALLBACK_CATALOG.resources,
          resource_templates:
            Array.isArray(data.resource_templates) && data.resource_templates.length > 0
              ? data.resource_templates
              : FALLBACK_CATALOG.resource_templates,
          prompts:
            Array.isArray(data.prompts) && data.prompts.length > 0
              ? data.prompts
              : FALLBACK_CATALOG.prompts,
        });
      }

      if (diagnoseRes.success && diagnoseRes.data) {
        setDiagnoseReport(
          normalizeDiagnoseOutput(
            diagnoseRes.data.output,
            diagnoseRes.data.duration_ms,
            servers,
            isZh,
          ),
        );
      } else {
        setDiagnoseReport(normalizeDiagnoseOutput(null, 0, servers, isZh));
      }
    },
    [isZh, servers],
  );

  const loadCatalogAndDiagnose = useCallback(
    async (bypassCache = false) => {
      setCatalogLoading(true);
      setDiagnoseLoading(true);
      setError(null);
      const [catalogRes, diagnoseRes] = await Promise.all([
        apiClient.getMcpCatalog({ bypassClientCache: bypassCache }),
        apiClient.executeMcpStudio({
          kind: "tool",
          name: "cluster.diagnose",
          arguments: { cpu_threshold: 85, memory_threshold: 85, disk_threshold: 85 },
        }),
      ]);
      applyCatalogAndDiagnoseResponses(catalogRes, diagnoseRes);
    },
    [applyCatalogAndDiagnoseResponses],
  );

  useEffect(() => {
    let cancelled = false;
    void Promise.all([
      apiClient.getMcpCatalog({ bypassClientCache: false }),
      apiClient.executeMcpStudio({
        kind: "tool",
        name: "cluster.diagnose",
        arguments: { cpu_threshold: 85, memory_threshold: 85, disk_threshold: 85 },
      }),
    ]).then(([catalogRes, diagnoseRes]) => {
      if (cancelled) return;
      applyCatalogAndDiagnoseResponses(catalogRes, diagnoseRes);
    });
    return () => {
      cancelled = true;
    };
  }, [applyCatalogAndDiagnoseResponses]);

  const effectiveReport = useMemo(
    () => diagnoseReport ?? normalizeDiagnoseOutput(null, 0, servers, isZh),
    [diagnoseReport, isZh, servers],
  );

  const totalResourcesCount =
    catalog.resources.length + catalog.resource_templates.length;

  // Run any suggested tool from an anomaly card or manual trigger
  async function handleRunSuggestedTool(toolName: string, args: Record<string, unknown>, label?: string) {
    setError(null);
    const { result: res, elapsedMs: elapsed } = await runTimed(() =>
      apiClient.executeMcpStudio({
        kind: "tool",
        name: toolName,
        arguments: args,
      }),
    );

    if (res.success && res.data) {
      const duration = res.data.duration_ms || elapsed;
      const formatted = formatOutputPreview(res.data.output);
      setConsoleOutput({
        title: label ?? `${toolName}`,
        subtitle: `${JSON.stringify(args)}`,
        durationMs: duration,
        status: "ok",
        outputText: formatted,
        raw: res.data.output,
      });
      showToast(
        isZh
          ? `已完成诊断工具 ${toolName} (${duration} ms)`
          : `Executed ${toolName} (${duration} ms)`,
      );
    } else {
      const errMsg = redactSensitiveTokens(responseError(res), [livePatToken]);
      setConsoleOutput({
        title: label ?? `${toolName}`,
        subtitle: `${JSON.stringify(args)}`,
        durationMs: elapsed,
        status: "error",
        outputText: errMsg,
        raw: res,
      });
      setError(errMsg);
    }
  }

  // Run one-click SRE Playbook on target server or all_online
  async function handleRunPlaybook(playbook: PlaybookCardDef) {
    setActivePlaybookId(playbook.id);
    setError(null);
    const target = effectivePlaybookServerId;
    const isBatch = target === "all_online";
    const toolName = isBatch ? "server.batch_exec" : "server.inspect";
    const toolArgs: Record<string, unknown> = isBatch
      ? {
          target: "all_online",
          command: playbook.batchCommand,
          timeout: 20,
        }
      : {
          server_id: target,
          playbook: playbook.id,
        };

    const serverLabel = isBatch
      ? isZh
        ? `全部在线节点 (${onlineServers.length})`
        : `All Online Nodes (${onlineServers.length})`
      : servers.find((s) => s.id === target)?.name ?? compactId(target);

    try {
      await handleRunSuggestedTool(
        toolName,
        toolArgs,
        `${isZh ? playbook.titleZh : playbook.titleEn} @ ${serverLabel}`,
      );
    } finally {
      setActivePlaybookId(null);
    }
  }

  // Generate AI SRE Prompt
  async function handleGeneratePrompt(promptName: string) {
    setActivePromptName(promptName);
    setPromptLoading(true);
    setError(null);
    const targetServerId =
      selectedPlaybookServerId && selectedPlaybookServerId !== "all_online"
        ? selectedPlaybookServerId
        : firstOnlineServer?.id ?? "";
    const args: Record<string, unknown> =
      promptName === "sre.troubleshoot_node"
        ? {
            ...(targetServerId ? { server_id: targetServerId } : {}),
            symptom: promptSymptom || "high_cpu",
          }
        : {};

    const res = await apiClient.executeMcpStudio({
      kind: "prompt",
      name: promptName,
      arguments: args,
    });
    setPromptLoading(false);

    if (res.success && res.data) {
      setGeneratedPromptText(extractPromptMarkdown(res.data.output, promptName, args, effectiveReport, servers));
      showToast(
        isZh
          ? `已生成 ${promptName} 实时遥测提示词`
          : `Generated live SRE prompt: ${promptName}`,
      );
    } else {
      // Fallback rich client-side prompt generation if studio endpoint returns an error
      setGeneratedPromptText(extractPromptMarkdown(null, promptName, args, effectiveReport, servers));
    }
  }

  // Select item in Playground
  function handleSelectPlaygroundItem(kind: PlaygroundKind, name: string) {
    setPlaygroundKind(kind);
    setSelectedItemName(name);
    const defaultArgs = buildDefaultArgsForItem(kind, name);
    setArgsEditorText(JSON.stringify(defaultArgs, null, 2));
    setPlaygroundResult(null);
  }

  // Build JSON-RPC 2.0 request preview for Playground
  const jsonRpcPreviewPayload = useMemo((): Record<string, unknown> => {
    let parsedArgs: Record<string, unknown> = {};
    try {
      const candidate = JSON.parse(argsEditorText) as unknown;
      if (candidate && typeof candidate === "object" && !Array.isArray(candidate)) {
        parsedArgs = candidate as Record<string, unknown>;
      }
    } catch {
      parsedArgs = {};
    }

    if (playgroundKind === "tool") {
      return {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: selectedItemName,
          arguments: parsedArgs,
        },
      };
    }
    if (playgroundKind === "resource") {
      const resolvedUri =
        typeof parsedArgs["uri"] === "string" && parsedArgs["uri"].trim()
          ? parsedArgs["uri"].trim()
          : selectedItemName.replace(
              "{server_id}",
              String(parsedArgs["server_id"] ?? firstOnlineServer?.id ?? "00000000-0000-0000-0000-000000000001"),
            );
      return {
        jsonrpc: "2.0",
        id: 1,
        method: "resources/read",
        params: {
          uri: resolvedUri,
        },
      };
    }
    return {
      jsonrpc: "2.0",
      id: 1,
      method: "prompts/get",
      params: {
        name: selectedItemName,
        arguments: parsedArgs,
      },
    };
  }, [argsEditorText, firstOnlineServer?.id, playgroundKind, selectedItemName]);

  // Execute in Playground
  async function handleExecutePlayground() {
    setError(null);
    let parsedArgs: Record<string, unknown> = {};
    try {
      const raw = argsEditorText.trim() ? (JSON.parse(argsEditorText) as unknown) : {};
      if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
        setError(isZh ? "参数必须是合法的 JSON 对象" : "Arguments must be a valid JSON object");
        return;
      }
      parsedArgs = raw as Record<string, unknown>;
    } catch (err) {
      setError(
        isZh
          ? `JSON 参数语法错误: ${err instanceof Error ? err.message : String(err)}`
          : `Invalid JSON arguments: ${err instanceof Error ? err.message : String(err)}`,
      );
      return;
    }

    setPlaygroundRunning(true);

    if (transportMode === "jsonrpc") {
      if (!livePatToken.trim()) {
        setPlaygroundRunning(false);
        setError(
          isZh
            ? "Raw JSON-RPC `/mcp` 模式需要提供 PAT 令牌（可在「Agent 一键接入」标签页一键生成）"
            : "Raw JSON-RPC `/mcp` mode requires a PAT token (generate one in the Connect tab)",
        );
        return;
      }
      const { result: res, elapsedMs: elapsed } = await runTimed(() =>
        apiClient.callMcpJsonRpc(livePatToken, jsonRpcPreviewPayload),
      );
      setPlaygroundRunning(false);
      const safeError = res.error ? redactSensitiveTokens(res.error, [livePatToken]) : undefined;
      setPlaygroundResult({
        durationMs: elapsed,
        ok: res.success,
        payload: res.data ?? { error: safeError },
      });
      if (!res.success && safeError) {
        setError(safeError);
      }
      return;
    }

    const targetName =
      playgroundKind === "resource" && typeof parsedArgs["uri"] === "string" && parsedArgs["uri"].trim()
        ? parsedArgs["uri"].trim()
        : selectedItemName;

    const { result: res, elapsedMs: elapsed } = await runTimed(() =>
      apiClient.executeMcpStudio({
        kind: playgroundKind,
        name: targetName,
        arguments: parsedArgs,
      }),
    );
    setPlaygroundRunning(false);

    if (res.success && res.data) {
      setPlaygroundResult({
        durationMs: res.data.duration_ms || elapsed,
        ok: true,
        payload: res.data.output,
      });
    } else {
      const errMsg = redactSensitiveTokens(responseError(res), [livePatToken]);
      setPlaygroundResult({
        durationMs: elapsed,
        ok: false,
        payload: res.data ?? { error: errMsg },
      });
      setError(errMsg);
    }
  }

  // Generate 1-Click MCP PAT
  async function handleGenerateMcpPat() {
    setError(null);
    const preset = PAT_PRESETS.find((item) => item.id === selectedPatPreset) ?? PAT_PRESETS[1]!;
    if (
      patAllowlistServerIds.length === 0 &&
      !(await dialogs.confirm({
        title: isZh ? "确认生成全局 MCP 令牌" : "Confirm Global MCP Token",
        message: isZh
          ? "未选择特定服务器白名单，该 MCP 令牌将可访问您名下所有可见服务器。是否继续？"
          : "No server allowlist selected; this MCP token will have access to all visible servers. Continue?",
      }))
    ) {
      return;
    }

    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;

    const expiresAt = new Date(Date.now() + 90 * 24 * 60 * 60 * 1000).toISOString();
    const cleanName = patTokenName.trim() || `mcp-${preset.id}`;

    const res = await apiClient.createPat(
      {
        name: cleanName,
        scopes: preset.scopes,
        expires_at: expiresAt,
        ...(patAllowlistServerIds.length > 0 ? { server_ids: patAllowlistServerIds } : {}),
      },
      totpCode,
    );

    if (res.success && res.data) {
      setLivePatToken(res.data.token);
      showToast(
        isZh
          ? "已生成 MCP 专属令牌并自动注入下方所有客户端配置！"
          : "Generated MCP token and injected into all client configurations below!",
      );
    } else {
      setError(responseError(res));
    }
  }

  function copyText(value: string, label: string) {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(value);
      showToast(isZh ? `已复制 ${label}` : `Copied ${label}`);
    }
  }

  const healthBadgeTone: BadgeTone =
    effectiveReport.healthScore >= 85
      ? "green"
      : effectiveReport.healthScore >= 60
        ? "yellow"
        : "red";
  const healthStatusLabel =
    effectiveReport.healthScore >= 85
      ? isZh
        ? "健康 / Healthy"
        : "Healthy"
      : effectiveReport.healthScore >= 60
        ? isZh
          ? "亚健康 / Degraded"
          : "Degraded"
        : isZh
          ? "告警 / Critical"
          : "Critical";

  const criticalCount = effectiveReport.anomalies.filter((a) => a.severity === "critical").length;
  const warningCount = effectiveReport.anomalies.filter((a) => a.severity === "warning").length;

  const filteredTools = useMemo(() => {
    const q = playgroundSearch.trim().toLowerCase();
    if (!q) return catalog.tools;
    return catalog.tools.filter(
      (tool) =>
        tool.name.toLowerCase().includes(q) ||
        tool.description.toLowerCase().includes(q) ||
        (tool.category ?? "").toLowerCase().includes(q),
    );
  }, [catalog.tools, playgroundSearch]);

  const allResources = useMemo(() => {
    const items: Array<{
      uri: string;
      name: string;
      description: string;
      mime_type: string;
      isTemplate: boolean;
    }> = [
      ...catalog.resources.map((r: McpResourceDefinition) => ({
        uri: r.uri,
        name: r.name,
        description: r.description,
        mime_type: r.mime_type,
        isTemplate: false,
      })),
      ...catalog.resource_templates.map((t: McpResourceTemplateDefinition) => ({
        uri: t.uri_template,
        name: t.name,
        description: t.description,
        mime_type: t.mime_type,
        isTemplate: true,
      })),
    ];
    const q = playgroundSearch.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (item) =>
        item.uri.toLowerCase().includes(q) ||
        item.name.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q),
    );
  }, [catalog.resource_templates, catalog.resources, playgroundSearch]);

  const filteredPrompts = useMemo(() => {
    const q = playgroundSearch.trim().toLowerCase();
    if (!q) return catalog.prompts;
    return catalog.prompts.filter(
      (p) => p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q),
    );
  }, [catalog.prompts, playgroundSearch]);

  const selectedToolDef: McpToolDefinition | undefined =
    playgroundKind === "tool"
      ? catalog.tools.find((t) => t.name === selectedItemName)
      : undefined;
  const selectedResourceDef =
    playgroundKind === "resource"
      ? allResources.find((r) => r.uri === selectedItemName)
      : undefined;
  const selectedPromptDef: McpPromptDefinition | undefined =
    playgroundKind === "prompt"
      ? catalog.prompts.find((p) => p.name === selectedItemName)
      : undefined;

  const apiBase = getApiBaseUrl();
  const displayPat = livePatToken.trim() || "xlp_YOUR_MCP_PERSONAL_ACCESS_TOKEN";

  const clientConfigSnippet = useMemo(() => {
    switch (clientConfigTab) {
      case "streamable-http":
        return JSON.stringify(
          {
            mcpServers: {
              xlstatus: {
                url: `${apiBase}/mcp`,
                headers: {
                  Authorization: `Bearer ${displayPat}`,
                },
              },
            },
          },
          null,
          2,
        );
      case "claude-desktop":
        return JSON.stringify(
          {
            mcpServers: {
              xlstatus: {
                command: "node",
                args: ["/path/to/XLStatus/scripts/xlstatus-mcp-stdio.mjs"],
                env: {
                  XLSTATUS_URL: apiBase,
                  XLSTATUS_PAT: displayPat,
                },
              },
            },
          },
          null,
          2,
        );
      case "claude-code":
        return `claude mcp add --transport http xlstatus "${apiBase}/mcp" \\\n  --header "Authorization: Bearer ${displayPat}"`;
      case "curl-smoke":
        return [
          `# 1. MCP Protocol Handshake (initialize)`,
          `curl -sS -X POST "${apiBase}/mcp" \\`,
          `  -H "Content-Type: application/json" \\`,
          `  -H "Authorization: Bearer ${displayPat}" \\`,
          `  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | jq .`,
          ``,
          `# 2. List Registered MCP Tools (19 tools)`,
          `curl -sS -X POST "${apiBase}/mcp" \\`,
          `  -H "Content-Type: application/json" \\`,
          `  -H "Authorization: Bearer ${displayPat}" \\`,
          `  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | jq .`,
          ``,
          `# 3. Run AI Cluster Anomaly Radar (cluster.diagnose)`,
          `curl -sS -X POST "${apiBase}/mcp" \\`,
          `  -H "Content-Type: application/json" \\`,
          `  -H "Authorization: Bearer ${displayPat}" \\`,
          `  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cluster.diagnose","arguments":{"cpu_threshold":85}}}' | jq .`,
        ].join("\n");
    }
  }, [apiBase, clientConfigTab, displayPat]);

  return (
    <PageShell wide>
      <PageHeader
        eyebrow={isZh ? "AI-NATIVE SRE COPILOT · MCP 2024-11-05" : "AI-NATIVE SRE COPILOT · MCP 2024-11-05"}
        title={isZh ? "AI 智能集群运维与 MCP 工作台" : "AI-Native Cluster Operations & MCP Studio"}
        detail={
          isZh
            ? "集自动化异常雷达、一键节点 SRE 诊断剧本、交互式 MCP 协议演练场（Tools / Resources / Prompts）与 IDE 智能体零门槛接入于一体的 AI 运维中枢。"
            : "Unified AI SRE anomaly radar, one-click node diagnostic playbooks, interactive MCP protocol inspector (Tools / Resources / Prompts), and zero-friction agent onboarding."
        }
        actions={
          <>
            <button
              type="button"
              className={buttonClass("secondary")}
              disabled={diagnoseLoading || catalogLoading || serversLoading}
              onClick={() => {
                void refreshServers(true);
                void loadCatalogAndDiagnose(true);
              }}
            >
              {diagnoseLoading || catalogLoading
                ? copy.common.loading
                : isZh
                  ? "重新扫描集群"
                  : "Rescan Cluster"}
            </button>
            <button
              type="button"
              className={buttonClass("primary")}
              onClick={() => setActiveTab("connect")}
            >
              {isZh ? "一键接入 Claude / Cursor" : "Connect AI Agent / IDE"}
            </button>
          </>
        }
      />

      {error ? (
        <div className="mb-5">
          <InlineError message={error} />
        </div>
      ) : null}

      {/* Top KPI Strip */}
      <div className="mb-6 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <KpiCard
          label={isZh ? "集群健康评分 / Cluster Health Score" : "Cluster Health Score"}
          value={`${effectiveReport.healthScore} / 100`}
          badge={<StatusBadge tone={healthBadgeTone}>{healthStatusLabel}</StatusBadge>}
          hint={
            <div className="mt-1 space-y-1.5">
              <ProgressBar
                value={effectiveReport.healthScore}
                tone={
                  effectiveReport.healthScore >= 85
                    ? "emerald"
                    : effectiveReport.healthScore >= 60
                      ? "amber"
                      : "rose"
                }
              />
              <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                <span>cluster.diagnose</span>
                <span>{formatMs(effectiveReport.durationMs || 12)}</span>
              </div>
            </div>
          }
        />

        <KpiCard
          label={isZh ? "异常检测雷达 / Detected Anomalies" : "Detected Anomalies"}
          value={effectiveReport.anomalies.length}
          badge={
            <StatusBadge
              tone={
                criticalCount > 0
                  ? "red"
                  : warningCount > 0
                    ? "yellow"
                    : "green"
              }
            >
              {criticalCount > 0
                ? `${criticalCount} Critical`
                : warningCount > 0
                  ? `${warningCount} Warning`
                  : isZh
                    ? "零异常"
                    : "All Clear"}
            </StatusBadge>
          }
          hint={
            isZh
              ? `严重 ${criticalCount} · 警告 ${warningCount} · 覆盖节点与拨测探针`
              : `${criticalCount} critical · ${warningCount} warning across nodes & probes`
          }
        />

        <KpiCard
          label={isZh ? "MCP 协议能力矩阵 / MCP Capabilities" : "MCP Capabilities"}
          value={`${catalog.tools.length} Tools`}
          badge={<StatusBadge tone="blue">{catalog.protocol_version}</StatusBadge>}
          hint={
            isZh
              ? `${catalog.tools.length} 工具 · ${totalResourcesCount} 资源 · ${catalog.prompts.length} SRE 提示词`
              : `${catalog.tools.length} Tools · ${totalResourcesCount} Resources · ${catalog.prompts.length} SRE Prompts`
          }
        />

        <KpiCard
          label={isZh ? "在线巡检覆盖率 / Online Fleet Coverage" : "Online Fleet Coverage"}
          value={`${onlineServers.length} / ${servers.length}`}
          badge={
            <StatusBadge tone={onlineServers.length > 0 ? "green" : "gray"}>
              {servers.length > 0
                ? formatPercent((onlineServers.length / servers.length) * 100)
                : "0.0%"}
            </StatusBadge>
          }
          hint={
            isZh
              ? `${onlineServers.length} 台在线节点可随时执行实时 mTLS 诊断剧本`
              : `${onlineServers.length} online nodes ready for live mTLS SRE playbooks`
          }
        />
      </div>

      {/* Segmented Workspace Tabs */}
      <div
        role="tablist"
        aria-label={isZh ? "AI 运维工作台切换" : "AI Operations Workbench Tabs"}
        className="mb-6 grid gap-2 rounded-2xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1.5 sm:grid-cols-3"
      >
        {(
          [
            {
              id: "diagnose" as const,
              title: isZh ? "1. AI 智能集群巡检与自愈台" : "1. AI Cluster Diagnostics & Playbooks",
              subtitle: isZh
                ? "异常根因雷达 · 6 大节点 SRE 剧本 · 提示词生成器"
                : "Anomaly Radar · 6 Node SRE Playbooks · Prompt Builder",
              badge: `${effectiveReport.anomalies.length}`,
            },
            {
              id: "playground" as const,
              title: isZh ? "2. MCP 协议演练场" : "2. MCP Protocol Playground",
              subtitle: isZh
                ? "交互式调试 19 Tools · 6 Resources · 4 Prompts"
                : "Inspect & Execute 19 Tools · 6 Resources · 4 Prompts",
              badge: "JSON-RPC 2.0",
            },
            {
              id: "connect" as const,
              title: isZh ? "3. Agent / IDE 一键接入向导" : "3. Agent & IDE Onboarding",
              subtitle: isZh
                ? "一键签发 MCP 令牌 · Cursor / Claude / CLI 配置生成"
                : "1-Click MCP PAT · Cursor / Claude / CLI Configs",
              badge: livePatToken ? (isZh ? "已就绪" : "Token Ready") : "PAT",
            },
          ] as const
        ).map((tab) => {
          const active = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => setActiveTab(tab.id)}
              className={`flex flex-col justify-between rounded-xl px-4 py-3 text-left transition cursor-pointer ${
                active
                  ? "border border-[var(--border-strong)] bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                  : "border border-transparent text-[var(--text-muted)] hover:bg-[var(--bg-card)]/60 hover:text-[var(--text-main)]"
              }`}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-semibold tracking-tight">{tab.title}</span>
                <span
                  className={`rounded-full px-2 py-0.5 font-mono text-[10px] font-semibold ${
                    active
                      ? "bg-[var(--accent-color)]/15 text-[var(--accent-color)]"
                      : "bg-[var(--bg-muted)] text-[var(--text-muted)]"
                  }`}
                >
                  {tab.badge}
                </span>
              </div>
              <span className="mt-1 text-xs text-[var(--text-muted)]">{tab.subtitle}</span>
            </button>
          );
        })}
      </div>

      {/* TAB 1: DIAGNOSE */}
      {activeTab === "diagnose" ? (
        <div className="space-y-6">
          {/* Automated Anomaly Radar */}
          <BrutalCard>
            <div className="flex flex-wrap items-start justify-between gap-3 border-b border-[var(--border-color)] pb-4">
              <div>
                <div className="flex items-center gap-2">
                  <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
                    {isZh
                      ? "自动化集群异常雷达 (cluster.diagnose)"
                      : "Automated Cluster Anomaly Radar (cluster.diagnose)"}
                  </h2>
                  <StatusBadge tone={healthBadgeTone}>{healthStatusLabel}</StatusBadge>
                </div>
                <p className="mt-1 text-xs text-[var(--text-muted)]">
                  {effectiveReport.summary}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  disabled={diagnoseLoading}
                  onClick={() => void loadCatalogAndDiagnose(true)}
                >
                  {diagnoseLoading
                    ? copy.common.loading
                    : isZh
                      ? "立即执行深度巡检"
                      : "Run Deep Scan Now"}
                </button>
              </div>
            </div>

            {effectiveReport.anomalies.length === 0 ? (
              <div className="mt-4">
                <InlineNotice tone="green">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <div className="font-semibold">
                        {isZh
                          ? "全集群运行平稳 — 未检测到 CPU/内存/磁盘热点、离线节点或拨测故障"
                          : "Cluster is operating normally — no CPU/RAM/disk hotspots, offline nodes, or probe failures detected"}
                      </div>
                      <div className="mt-0.5 text-xs opacity-85">
                        {isZh
                          ? "您仍可在下方选择任意在线节点运行只读 SRE 诊断剧本，或生成 AI 巡检报告。"
                          : "You can still run read-only SRE playbooks on any online node below or generate an AI audit prompt."}
                      </div>
                    </div>
                    <button
                      type="button"
                      className={buttonClass("secondary")}
                      onClick={() =>
                        void handleRunSuggestedTool(
                          "cluster.overview",
                          {},
                          isZh ? "集群全景概览 (cluster.overview)" : "Cluster Overview (cluster.overview)",
                        )
                      }
                    >
                      {isZh ? "查看集群全景快照" : "Inspect Cluster Snapshot"}
                    </button>
                  </div>
                </InlineNotice>
              </div>
            ) : (
              <div className="mt-4 grid gap-3 md:grid-cols-2">
                {effectiveReport.anomalies.map((anomaly) => {
                  const cardStyle =
                    anomaly.severity === "critical"
                      ? "border-rose-500/35 bg-[var(--status-danger-bg)]/45"
                      : anomaly.severity === "warning"
                        ? "border-amber-500/35 bg-[var(--status-warn-bg)]/45"
                        : "border-indigo-500/30 bg-[var(--status-info-bg)]/45";
                  const badgeTone: BadgeTone =
                    anomaly.severity === "critical"
                      ? "red"
                      : anomaly.severity === "warning"
                        ? "yellow"
                        : "blue";

                  return (
                    <div
                      key={anomaly.id}
                      className={`flex flex-col justify-between rounded-xl border p-4 ${cardStyle}`}
                    >
                      <div>
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div className="flex items-center gap-2">
                            <StatusBadge tone={badgeTone}>{anomaly.severity.toUpperCase()}</StatusBadge>
                            <span className="font-mono text-xs font-semibold text-[var(--text-main)]">
                              {anomaly.resourceName}
                            </span>
                          </div>
                          <span className="rounded border border-[var(--border-color)] bg-[var(--bg-card)] px-2 py-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                            {anomaly.category}
                          </span>
                        </div>

                        <div className="mt-2.5 text-sm font-semibold text-[var(--text-main)]">
                          {anomaly.title}
                        </div>

                        <div className="mt-1.5 font-mono text-xs text-[var(--text-muted)]">
                          {isZh ? "证据指标: " : "Evidence: "}
                          <span className="text-[var(--text-main)]">{anomaly.evidence}</span>
                        </div>

                        <div className="mt-2 text-xs leading-relaxed text-[var(--text-muted)]">
                          {isZh ? "建议处置: " : "Remediation: "}
                          {anomaly.recommendation}
                        </div>
                      </div>

                      <div className="mt-3.5 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-color)]/60 pt-3">
                        <code className="font-mono text-[11px] text-[var(--text-muted)]">
                          {anomaly.suggestedTool}
                        </code>
                        <button
                          type="button"
                          className={buttonClass("primary")}
                          onClick={() =>
                            void handleRunSuggestedTool(
                              anomaly.suggestedTool,
                              anomaly.suggestedArgs,
                              `${anomaly.suggestedTool} (${anomaly.resourceName})`,
                            )
                          }
                        >
                          {isZh ? "一键运行建议诊断工具" : "1-Click Run Suggested Tool"}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </BrutalCard>

          {/* One-Click Node SRE Playbook Matrix */}
          <BrutalCard>
            <div className="flex flex-wrap items-end justify-between gap-4 border-b border-[var(--border-color)] pb-4">
              <div>
                <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
                  {isZh
                    ? "一键节点 SRE 诊断剧本矩阵 (server.inspect / server.batch_exec)"
                    : "One-Click Node SRE Playbook Matrix (server.inspect / server.batch_exec)"}
                </h2>
                <p className="mt-1 text-xs text-[var(--text-muted)]">
                  {isZh
                    ? "选择单台在线服务器或「全部在线节点」并发执行只读 SRE 排障剧本，通过 mTLS 通道实时采集底层状态。"
                    : "Select a target server or 'All Online Nodes' to dispatch read-only SRE diagnostic playbooks over live mTLS streams."}
                </p>
              </div>

              <div className="w-full sm:w-80">
                <Field label={isZh ? "目标执行节点 / Target Server" : "Target Server"}>
                  <select
                    className={selectClass}
                    value={effectivePlaybookServerId}
                    onChange={(e) => setSelectedPlaybookServerId(e.target.value)}
                  >
                    <option value="all_online">
                      {isZh
                        ? `⚡ 全部在线节点并发执行 (all_online · ${onlineServers.length} 台)`
                        : `⚡ All Online Nodes Batch (all_online · ${onlineServers.length})`}
                    </option>
                    {servers.map((srv: Server) => (
                      <option key={srv.id} value={srv.id}>
                        {srv.status === "online" ? "🟢 " : "🔴 "}
                        {srv.name} ({compactId(srv.id)}) — CPU {formatPercent(srv.cpu_percent)}
                      </option>
                    ))}
                  </select>
                </Field>
              </div>
            </div>

            <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {PLAYBOOK_MATRIX.map((playbook) => {
                const isRunning = activePlaybookId === playbook.id;
                return (
                  <div
                    key={playbook.id}
                    className="flex flex-col justify-between rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4 transition hover:border-[var(--border-strong)]"
                  >
                    <div>
                      <div className="flex items-center justify-between gap-2">
                        <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2 py-0.5 font-mono text-[11px] font-semibold text-[var(--accent-color)]">
                          {playbook.badge}
                        </span>
                        <StatusBadge tone="blue">Read-Only</StatusBadge>
                      </div>
                      <h3 className="mt-2.5 text-sm font-semibold text-[var(--text-main)]">
                        {isZh ? playbook.titleZh : playbook.titleEn}
                      </h3>
                      <p className="mt-1 text-xs leading-relaxed text-[var(--text-muted)]">
                        {isZh ? playbook.subtitleZh : playbook.subtitleEn}
                      </p>
                    </div>

                    <div className="mt-4 pt-2">
                      <button
                        type="button"
                        disabled={isRunning}
                        onClick={() => void handleRunPlaybook(playbook)}
                        className={`${buttonClass("primary")} w-full`}
                      >
                        {isRunning
                          ? copy.common.loading
                          : isZh
                            ? "立即执行剧本"
                            : "Run Playbook"}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Live Execution Terminal / Structured Console Card */}
            {consoleOutput ? (
              <div className="mt-5 overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-100 shadow-[var(--shadow-lg)]">
                <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800 bg-zinc-900/90 px-4 py-2.5">
                  <div className="flex items-center gap-2.5">
                    <span
                      className={`h-2.5 w-2.5 rounded-full ${
                        consoleOutput.status === "ok" ? "bg-emerald-400" : "bg-rose-400"
                      }`}
                    />
                    <span className="font-mono text-xs font-semibold text-zinc-100">
                      {consoleOutput.title}
                    </span>
                    <span className="rounded bg-zinc-800 px-2 py-0.5 font-mono text-[11px] text-emerald-300">
                      {formatMs(consoleOutput.durationMs)}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => copyText(consoleOutput.outputText, consoleOutput.title)}
                      className="rounded border border-zinc-700 bg-zinc-800 px-2.5 py-1 font-mono text-[11px] text-zinc-200 transition hover:bg-zinc-700 cursor-pointer"
                    >
                      {isZh ? "复制输出 / Copy" : "Copy Output"}
                    </button>
                    <button
                      type="button"
                      onClick={() => setConsoleOutput(null)}
                      className="rounded border border-zinc-700 bg-zinc-800 px-2.5 py-1 font-mono text-[11px] text-zinc-400 transition hover:bg-zinc-700 hover:text-zinc-200 cursor-pointer"
                    >
                      {copy.common.close}
                    </button>
                  </div>
                </div>
                <div className="border-b border-zinc-900 bg-zinc-950/60 px-4 py-1.5 font-mono text-[11px] text-zinc-400">
                  args: {consoleOutput.subtitle}
                </div>
                <pre className="max-h-96 overflow-auto p-4 font-mono text-xs leading-relaxed text-emerald-300 whitespace-pre-wrap break-words">
                  {consoleOutput.outputText}
                </pre>
              </div>
            ) : null}
          </BrutalCard>

          {/* AI SRE Prompt Generator */}
          <BrutalCard>
            <div className="flex flex-wrap items-end justify-between gap-4 border-b border-[var(--border-color)] pb-4">
              <div>
                <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
                  {isZh
                    ? "AI SRE 实时遥测提示词生成器 (MCP Prompts)"
                    : "AI SRE Telemetry-Enriched Prompt Generator (MCP Prompts)"}
                </h2>
                <p className="mt-1 text-xs text-[var(--text-muted)]">
                  {isZh
                    ? "将当前集群实时遥测、告警上下文与可用 MCP 工具编排为结构化 Markdown 提示词，一键复制到任意大模型或直接审阅。"
                    : "Compile live fleet telemetry, active incidents, and MCP tool schemas into a structured Markdown prompt ready for any LLM."}
                </p>
              </div>
              <div className="w-full sm:w-64">
                <Field label={isZh ? "故障症状标签 (用于单节点排障)" : "Symptom Tag (for node troubleshoot)"}>
                  <select
                    className={selectClass}
                    value={promptSymptom}
                    onChange={(e) => setPromptSymptom(e.target.value)}
                  >
                    <option value="high_cpu">high_cpu (CPU 飙升)</option>
                    <option value="memory_pressure">memory_pressure (内存压力 / OOM)</option>
                    <option value="disk_full">disk_full (磁盘空间 / Inode 告警)</option>
                    <option value="network_spike">network_spike (网络流量激增)</option>
                    <option value="offline">offline (节点失联 / 掉线)</option>
                  </select>
                </Field>
              </div>
            </div>

            <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {catalog.prompts.map((prompt) => {
                const isSelected = activePromptName === prompt.name;
                return (
                  <button
                    key={prompt.name}
                    type="button"
                    disabled={promptLoading}
                    onClick={() => void handleGeneratePrompt(prompt.name)}
                    className={`flex flex-col justify-between rounded-xl border p-4 text-left transition cursor-pointer ${
                      isSelected
                        ? "border-[var(--accent-color)] bg-[var(--bg-subtle)] shadow-[var(--shadow-xs)]"
                        : "border-[var(--border-color)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]"
                    }`}
                  >
                    <div>
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-xs font-semibold text-[var(--accent-color)]">
                          {prompt.name}
                        </span>
                        <span className="rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                          Prompt
                        </span>
                      </div>
                      <p className="mt-2 text-xs leading-relaxed text-[var(--text-muted)]">
                        {prompt.description}
                      </p>
                    </div>
                    <div className="mt-3 pt-2 text-xs font-semibold text-[var(--text-main)]">
                      {promptLoading && isSelected
                        ? copy.common.loading
                        : isZh
                          ? "生成实时提示词 →"
                          : "Generate Live Prompt →"}
                    </div>
                  </button>
                );
              })}
            </div>

            {generatedPromptText ? (
              <div className="mt-4 overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-100">
                <div className="flex items-center justify-between gap-2 border-b border-zinc-800 bg-zinc-900 px-4 py-2.5">
                  <span className="font-mono text-xs font-semibold text-zinc-200">
                    {activePromptName} · Markdown Prompt
                  </span>
                  <button
                    type="button"
                    onClick={() => copyText(generatedPromptText, activePromptName)}
                    className="rounded border border-zinc-700 bg-zinc-800 px-2.5 py-1 font-mono text-[11px] text-emerald-300 transition hover:bg-zinc-700 cursor-pointer"
                  >
                    {isZh ? "一键复制 Markdown" : "Copy Markdown"}
                  </button>
                </div>
                <pre className="max-h-80 overflow-auto p-4 font-mono text-xs leading-relaxed text-zinc-200 whitespace-pre-wrap break-words">
                  {generatedPromptText}
                </pre>
              </div>
            ) : null}
          </BrutalCard>
        </div>
      ) : null}

      {/* TAB 2: PLAYGROUND */}
      {activeTab === "playground" ? (
        <div className="grid gap-6 lg:grid-cols-[22rem_minmax(0,1fr)]">
          {/* Left Column: Filterable Catalog of Tools, Resources & Prompts */}
          <BrutalCard className="flex flex-col justify-between">
            <div>
              <div className="grid grid-cols-3 gap-1 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1">
                {(
                  [
                    { id: "tool" as const, label: `Tools (${catalog.tools.length})` },
                    { id: "resource" as const, label: `Resources (${totalResourcesCount})` },
                    { id: "prompt" as const, label: `Prompts (${catalog.prompts.length})` },
                  ] as const
                ).map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => {
                      setPlaygroundKind(item.id);
                      if (item.id === "tool" && catalog.tools[0]) {
                        handleSelectPlaygroundItem("tool", catalog.tools[0].name);
                      } else if (item.id === "resource" && allResources[0]) {
                        handleSelectPlaygroundItem("resource", allResources[0].uri);
                      } else if (item.id === "prompt" && catalog.prompts[0]) {
                        handleSelectPlaygroundItem("prompt", catalog.prompts[0].name);
                      }
                    }}
                    className={`rounded-lg px-2 py-1.5 font-mono text-xs font-semibold transition cursor-pointer ${
                      playgroundKind === item.id
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {item.label}
                  </button>
                ))}
              </div>

              <div className="mt-3">
                <input
                  type="search"
                  className={inputClass}
                  placeholder={
                    isZh ? "搜索 MCP 工具、资源 URI 或提示词..." : "Filter tools, resources, or prompts..."
                  }
                  value={playgroundSearch}
                  onChange={(e) => setPlaygroundSearch(e.target.value)}
                />
              </div>

              <div className="mt-3 max-h-[34rem] space-y-1.5 overflow-y-auto pr-1">
                {playgroundKind === "tool" ? (
                  filteredTools.length === 0 ? (
                    <EmptyState title={copy.common.noMatches} />
                  ) : (
                    filteredTools.map((tool) => {
                      const active = selectedItemName === tool.name;
                      const annotations = asRecord(tool.annotations);
                      const isReadOnly = annotations["readOnlyHint"] !== false;
                      return (
                        <button
                          key={tool.name}
                          type="button"
                          onClick={() => handleSelectPlaygroundItem("tool", tool.name)}
                          className={`w-full rounded-xl border p-3 text-left transition cursor-pointer ${
                            active
                              ? "border-[var(--accent-color)] bg-[var(--bg-subtle)] shadow-[var(--shadow-xs)]"
                              : "border-[var(--border-color)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]"
                          }`}
                        >
                          <div className="flex flex-wrap items-center justify-between gap-1.5">
                            <span className="font-mono text-xs font-semibold text-[var(--text-main)]">
                              {tool.name}
                            </span>
                            <div className="flex items-center gap-1">
                              {tool.category ? (
                                <span className="rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                                  {tool.category}
                                </span>
                              ) : null}
                              <span
                                className={`rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold ${
                                  isReadOnly
                                    ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300"
                                    : "bg-amber-500/15 text-amber-600 dark:text-amber-300"
                                }`}
                              >
                                {isReadOnly ? "readOnly" : "exec"}
                              </span>
                            </div>
                          </div>
                          <p className="mt-1 line-clamp-2 text-[11px] leading-relaxed text-[var(--text-muted)]">
                            {tool.description}
                          </p>
                        </button>
                      );
                    })
                  )
                ) : null}

                {playgroundKind === "resource" ? (
                  allResources.length === 0 ? (
                    <EmptyState title={copy.common.noMatches} />
                  ) : (
                    allResources.map((res) => {
                      const active = selectedItemName === res.uri;
                      return (
                        <button
                          key={res.uri}
                          type="button"
                          onClick={() => handleSelectPlaygroundItem("resource", res.uri)}
                          className={`w-full rounded-xl border p-3 text-left transition cursor-pointer ${
                            active
                              ? "border-[var(--accent-color)] bg-[var(--bg-subtle)] shadow-[var(--shadow-xs)]"
                              : "border-[var(--border-color)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]"
                          }`}
                        >
                          <div className="flex items-center justify-between gap-1.5">
                            <span className="truncate font-mono text-xs font-semibold text-[var(--text-main)]">
                              {res.uri}
                            </span>
                            <span className="shrink-0 rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                              {res.isTemplate ? "template" : "static"}
                            </span>
                          </div>
                          <div className="mt-1 text-xs font-medium text-[var(--text-main)]">
                            {res.name}
                          </div>
                          <p className="mt-0.5 line-clamp-2 text-[11px] text-[var(--text-muted)]">
                            {res.description}
                          </p>
                        </button>
                      );
                    })
                  )
                ) : null}

                {playgroundKind === "prompt" ? (
                  filteredPrompts.length === 0 ? (
                    <EmptyState title={copy.common.noMatches} />
                  ) : (
                    filteredPrompts.map((prompt) => {
                      const active = selectedItemName === prompt.name;
                      return (
                        <button
                          key={prompt.name}
                          type="button"
                          onClick={() => handleSelectPlaygroundItem("prompt", prompt.name)}
                          className={`w-full rounded-xl border p-3 text-left transition cursor-pointer ${
                            active
                              ? "border-[var(--accent-color)] bg-[var(--bg-subtle)] shadow-[var(--shadow-xs)]"
                              : "border-[var(--border-color)] bg-[var(--bg-card)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-subtle)]"
                          }`}
                        >
                          <div className="flex items-center justify-between gap-1.5">
                            <span className="font-mono text-xs font-semibold text-[var(--text-main)]">
                              {prompt.name}
                            </span>
                            <span className="rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                              {prompt.arguments.length} args
                            </span>
                          </div>
                          <p className="mt-1 line-clamp-2 text-[11px] text-[var(--text-muted)]">
                            {prompt.description}
                          </p>
                        </button>
                      );
                    })
                  )
                ) : null}
              </div>
            </div>
          </BrutalCard>

          {/* Right Column: Inspector, Arguments Editor, JSON-RPC Preview & Live Output */}
          <div className="space-y-6">
            <BrutalCard>
              <div className="flex flex-wrap items-start justify-between gap-4 border-b border-[var(--border-color)] pb-4">
                <div>
                  <div className="flex flex-wrap items-center gap-2">
                    <StatusBadge tone="blue">{playgroundKind.toUpperCase()}</StatusBadge>
                    <h2 className="font-mono text-lg font-semibold text-[var(--text-main)]">
                      {selectedItemName}
                    </h2>
                  </div>
                  <p className="mt-1.5 text-xs leading-relaxed text-[var(--text-muted)]">
                    {selectedToolDef?.description ??
                      selectedResourceDef?.description ??
                      selectedPromptDef?.description ??
                      ""}
                  </p>
                </div>

                {/* Transport Mode Switcher */}
                <div className="inline-flex rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1">
                  <button
                    type="button"
                    onClick={() => setTransportMode("studio")}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition cursor-pointer ${
                      transportMode === "studio"
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {isZh ? "Session Studio 模式" : "Session Studio Mode"}
                  </button>
                  <button
                    type="button"
                    onClick={() => setTransportMode("jsonrpc")}
                    className={`rounded-lg px-3 py-1.5 text-xs font-medium transition cursor-pointer ${
                      transportMode === "jsonrpc"
                        ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    {isZh ? "Raw JSON-RPC (/mcp) 模式" : "Raw JSON-RPC (/mcp)"}
                  </button>
                </div>
              </div>

              {transportMode === "jsonrpc" ? (
                <div className="mt-4 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5">
                  <Field
                    label={
                      isZh
                        ? "Bearer PAT 令牌 (用于直连 POST /mcp JSON-RPC 2.0 端点)"
                        : "Bearer PAT Token (for direct POST /mcp JSON-RPC 2.0 calls)"
                    }
                    hint={
                      isZh
                        ? "可在此粘贴您的 xlp_... 令牌，或前往「3. Agent / IDE 一键接入向导」一键生成。"
                        : "Paste an xlp_... token here, or generate one with 1 click in Tab 3."
                    }
                  >
                    <input
                      type="password"
                      autoComplete="off"
                      spellCheck={false}
                      className={inputClass}
                      placeholder="xlp_..."
                      value={livePatToken}
                      onChange={(e) => setLivePatToken(e.target.value)}
                    />
                  </Field>
                </div>
              ) : null}

              {/* Schema Properties Summary */}
              {selectedToolDef ? (
                <div className="mt-4">
                  <div className="text-xs font-semibold text-[var(--text-main)]">
                    {isZh ? "输入 Schema 属性 (Input Schema Properties)" : "Input Schema Properties"}
                  </div>
                  <div className="mt-2 flex flex-wrap gap-2">
                    {Object.entries(asRecord(selectedToolDef.input_schema["properties"])).length ===
                    0 ? (
                      <span className="text-xs text-[var(--text-muted)]">
                        {isZh ? "无必填参数（可直接运行）" : "No parameters required"}
                      </span>
                    ) : (
                      Object.entries(asRecord(selectedToolDef.input_schema["properties"])).map(
                        ([propName, propSpec]) => {
                          const spec = asRecord(propSpec);
                          const requiredList = Array.isArray(selectedToolDef.input_schema["required"])
                            ? (selectedToolDef.input_schema["required"] as unknown[])
                            : [];
                          const isReq = requiredList.includes(propName);
                          return (
                            <span
                              key={propName}
                              className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-1 font-mono text-xs"
                            >
                              <span className="font-semibold text-[var(--text-main)]">{propName}</span>
                              <span className="text-[11px] text-[var(--text-muted)]">
                                :{asString(spec["type"], "any")}
                              </span>
                              {isReq ? (
                                <span className="text-[10px] font-bold text-rose-500">req</span>
                              ) : null}
                            </span>
                          );
                        },
                      )
                    )}
                  </div>
                </div>
              ) : null}

              {/* Arguments Editor + Quick Preset Fill Buttons */}
              <div className="mt-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-xs font-semibold text-[var(--text-main)]">
                    {isZh ? "调用参数 JSON 编辑器 / Arguments JSON" : "Arguments JSON Editor"}
                  </span>
                  <div className="flex flex-wrap items-center gap-1.5">
                    {firstOnlineServer ? (
                      <button
                        type="button"
                        onClick={() => {
                          try {
                            const current = JSON.parse(argsEditorText) as Record<string, unknown>;
                            const next: Record<string, unknown> = {
                              ...current,
                              server_id: firstOnlineServer.id,
                            };
                            if (
                              playgroundKind === "resource" &&
                              selectedItemName.includes("{server_id}")
                            ) {
                              next["uri"] = selectedItemName.replace(
                                "{server_id}",
                                firstOnlineServer.id,
                              );
                            }
                            setArgsEditorText(JSON.stringify(next, null, 2));
                          } catch {
                            setArgsEditorText(
                              JSON.stringify({ server_id: firstOnlineServer.id }, null, 2),
                            );
                          }
                        }}
                        className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-1 font-mono text-[11px] text-[var(--text-main)] hover:bg-[var(--bg-muted)] cursor-pointer"
                      >
                        {isZh
                          ? `填入在线节点: ${firstOnlineServer.name}`
                          : `Fill Online Server: ${firstOnlineServer.name}`}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={() =>
                        setArgsEditorText(
                          JSON.stringify(
                            buildDefaultArgsForItem(playgroundKind, selectedItemName),
                            null,
                            2,
                          ),
                        )
                      }
                      className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-1 font-mono text-[11px] text-[var(--text-muted)] hover:text-[var(--text-main)] cursor-pointer"
                    >
                      {isZh ? "重置默认参数" : "Reset Preset"}
                    </button>
                  </div>
                </div>

                <textarea
                  className={`${textareaClass} mt-2 min-h-36`}
                  value={argsEditorText}
                  onChange={(e) => setArgsEditorText(e.target.value)}
                />

                <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
                  <span className="font-mono text-[11px] text-[var(--text-muted)]">
                    {transportMode === "studio"
                      ? "POST /api/v1/mcp/studio"
                      : "POST /mcp (Authorization: Bearer xlp_...)"}
                  </span>
                  <button
                    type="button"
                    disabled={playgroundRunning}
                    onClick={() => void handleExecutePlayground()}
                    className={buttonClass("primary")}
                  >
                    {playgroundRunning
                      ? copy.common.loading
                      : isZh
                        ? `发送 MCP 请求 (${selectedItemName})`
                        : `Execute MCP Call (${selectedItemName})`}
                  </button>
                </div>
              </div>
            </BrutalCard>

            {/* Side-by-Side JSON-RPC 2.0 Request & Live Response Output */}
            <div className="grid gap-6 xl:grid-cols-2">
              <div className="overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-100">
                <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-900 px-4 py-2.5">
                  <span className="font-mono text-xs font-semibold text-zinc-200">
                    JSON-RPC 2.0 Request Payload
                  </span>
                  <button
                    type="button"
                    onClick={() =>
                      copyText(JSON.stringify(jsonRpcPreviewPayload, null, 2), "JSON-RPC Request")
                    }
                    className="rounded border border-zinc-700 bg-zinc-800 px-2 py-0.5 font-mono text-[11px] text-zinc-200 hover:bg-zinc-700 cursor-pointer"
                  >
                    Copy
                  </button>
                </div>
                <pre className="max-h-96 overflow-auto p-4 font-mono text-xs leading-relaxed text-sky-300 whitespace-pre-wrap break-words">
                  {JSON.stringify(jsonRpcPreviewPayload, null, 2)}
                </pre>
              </div>

              <div className="overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-100">
                <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-900 px-4 py-2.5">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-semibold text-zinc-200">
                      Live Response Output
                    </span>
                    {playgroundResult ? (
                      <span
                        className={`rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold ${
                          playgroundResult.ok
                            ? "bg-emerald-500/20 text-emerald-300"
                            : "bg-rose-500/20 text-rose-300"
                        }`}
                      >
                        {formatMs(playgroundResult.durationMs)}
                      </span>
                    ) : null}
                  </div>
                  {playgroundResult ? (
                    <button
                      type="button"
                      onClick={() =>
                        copyText(
                          formatOutputPreview(playgroundResult.payload),
                          "MCP Response",
                        )
                      }
                      className="rounded border border-zinc-700 bg-zinc-800 px-2 py-0.5 font-mono text-[11px] text-zinc-200 hover:bg-zinc-700 cursor-pointer"
                    >
                      Copy
                    </button>
                  ) : null}
                </div>
                <pre className="max-h-96 overflow-auto p-4 font-mono text-xs leading-relaxed text-emerald-300 whitespace-pre-wrap break-words">
                  {playgroundResult
                    ? formatOutputPreview(playgroundResult.payload)
                    : isZh
                      ? "// 点击上方「发送 MCP 请求」查看实时返回结果..."
                      : "// Click 'Execute MCP Call' above to inspect live response..."}
                </pre>
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {/* TAB 3: CONNECT */}
      {activeTab === "connect" ? (
        <div className="space-y-6">
          {/* 1-Click MCP PAT Generator */}
          <BrutalCard accent>
            <div className="border-b border-[var(--border-color)] pb-4">
              <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
                {isZh
                  ? "第一步：一键签发 MCP 专属个人访问令牌 (PAT)"
                  : "Step 1: 1-Click MCP Personal Access Token (PAT) Generator"}
              </h2>
              <p className="mt-1 text-xs text-[var(--text-muted)]">
                {isZh
                  ? "选择 AI 智能体权限预设与可选的服务器白名单，生成后将自动注入下方所有 IDE 与 CLI 配置文件中。"
                  : "Choose a permission preset and optional server allowlist. The generated token is automatically injected into all client configs below."}
              </p>
            </div>

            {/* Permission Presets */}
            <div className="mt-4 grid gap-3 md:grid-cols-3">
              {PAT_PRESETS.map((preset) => {
                const selected = selectedPatPreset === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    onClick={() => setSelectedPatPreset(preset.id)}
                    className={`flex flex-col justify-between rounded-xl border p-4 text-left transition cursor-pointer ${
                      selected
                        ? "border-[var(--accent-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                        : "border-[var(--border-color)] bg-[var(--bg-card)]/60 hover:border-[var(--border-strong)]"
                    }`}
                  >
                    <div>
                      <div className="flex items-center justify-between gap-2">
                        <StatusBadge tone={preset.tone}>{preset.id.toUpperCase()}</StatusBadge>
                        {selected ? (
                          <span className="font-mono text-[11px] font-semibold text-[var(--accent-color)]">
                            ✓ {isZh ? "已选" : "Selected"}
                          </span>
                        ) : null}
                      </div>
                      <div className="mt-2.5 text-sm font-semibold text-[var(--text-main)]">
                        {isZh ? preset.titleZh : preset.titleEn}
                      </div>
                      <p className="mt-1 text-xs leading-relaxed text-[var(--text-muted)]">
                        {isZh ? preset.descZh : preset.descEn}
                      </p>
                    </div>
                    <div className="mt-3 flex flex-wrap gap-1 border-t border-[var(--border-color)] pt-2.5">
                      {preset.scopes.map((scope) => (
                        <code
                          key={scope}
                          className="rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-main)]"
                        >
                          {scope}
                        </code>
                      ))}
                    </div>
                  </button>
                );
              })}
            </div>

            {/* Token Name & Server Allowlist Pills */}
            <div className="mt-4 grid gap-4 lg:grid-cols-[20rem_minmax(0,1fr)]">
              <Field label={isZh ? "令牌标识名称 / Token Name" : "Token Name"}>
                <input
                  className={inputClass}
                  value={patTokenName}
                  onChange={(e) => setPatTokenName(e.target.value)}
                  placeholder="ai-mcp-copilot"
                />
              </Field>

              <div>
                <div className="flex items-center justify-between">
                  <span className="text-xs font-medium text-[var(--text-main)]">
                    {isZh
                      ? "服务器访问白名单 (可选，不选则默认允许访问所有可见节点)"
                      : "Server Allowlist (Optional — leave empty to allow all visible servers)"}
                  </span>
                  {patAllowlistServerIds.length > 0 ? (
                    <button
                      type="button"
                      onClick={() => setPatAllowlistServerIds([])}
                      className="text-xs font-medium text-[var(--accent-color)] hover:underline cursor-pointer"
                    >
                      {isZh ? "清空选择 (允许全部)" : "Clear (Allow All)"}
                    </button>
                  ) : null}
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {servers.map((srv) => {
                    const checked = patAllowlistServerIds.includes(srv.id);
                    return (
                      <button
                        key={srv.id}
                        type="button"
                        onClick={() =>
                          setPatAllowlistServerIds((prev) =>
                            prev.includes(srv.id)
                              ? prev.filter((id) => id !== srv.id)
                              : [...prev, srv.id],
                          )
                        }
                        className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium transition cursor-pointer ${
                          checked
                            ? "border-[var(--accent-color)] bg-[var(--accent-color)]/15 text-[var(--text-main)]"
                            : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                        }`}
                      >
                        <span
                          className={`h-1.5 w-1.5 rounded-full ${
                            srv.status === "online" ? "bg-emerald-500" : "bg-rose-500"
                          }`}
                        />
                        <span>{srv.name}</span>
                        <span className="font-mono text-[10px] opacity-75">
                          ({compactId(srv.id)})
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>

            <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-color)] pt-4">
              <div className="text-xs text-[var(--text-muted)]">
                {livePatToken ? (
                  <span className="font-mono text-emerald-600 dark:text-emerald-400">
                    ✓ {isZh ? "已生成实时令牌并注入下方配置: " : "Live PAT injected: "}
                    {maskSensitiveToken(livePatToken)}
                  </span>
                ) : (
                  <span>
                    {isZh
                      ? "生成后令牌仅明文显示一次，并自动填入下方配置模板。"
                      : "Token is shown once upon creation and automatically injected into snippets below."}
                  </span>
                )}
              </div>
              <button
                type="button"
                disabled={busy}
                onClick={() => void runBusy(handleGenerateMcpPat)}
                className={buttonClass("primary")}
              >
                {busy
                  ? copy.common.loading
                  : isZh
                    ? "一键生成 MCP 令牌并注入下方配置"
                    : "1-Click Generate MCP Token & Inject Below"}
              </button>
            </div>
          </BrutalCard>

          {/* Ready-to-Copy Client Config Tabs */}
          <BrutalCard>
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-4">
              <div>
                <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">
                  {isZh
                    ? "第二步：复制 AI 客户端 / IDE 接入配置"
                    : "Step 2: Copy Ready-to-Use AI Client & IDE Configuration"}
                </h2>
                <p className="mt-1 text-xs text-[var(--text-muted)]">
                  {isZh
                    ? "支持 Streamable HTTP 直连 `/mcp` 或通过内置 `scripts/xlstatus-mcp-stdio.mjs` 桥接器接入。"
                    : "Connect directly via Streamable HTTP `/mcp` or via the bundled `scripts/xlstatus-mcp-stdio.mjs` stdio bridge."}
                </p>
              </div>
              <button
                type="button"
                onClick={() => copyText(clientConfigSnippet, clientConfigTab)}
                className={buttonClass("primary")}
              >
                {isZh ? "一键复制当前配置" : "Copy Configuration"}
              </button>
            </div>

            <div className="mt-4 flex flex-wrap gap-2">
              {(
                [
                  {
                    id: "streamable-http" as const,
                    label: isZh
                      ? "Streamable HTTP 直连 (Antigravity / Cursor / Windsurf / Cline)"
                      : "Streamable HTTP Direct (/mcp)",
                  },
                  {
                    id: "claude-desktop" as const,
                    label: isZh
                      ? "Claude Desktop (stdio 桥接器)"
                      : "Claude Desktop (stdio Bridge)",
                  },
                  {
                    id: "claude-code" as const,
                    label: "Claude Code CLI",
                  },
                  {
                    id: "curl-smoke" as const,
                    label: isZh ? "cURL JSON-RPC 冒烟测试" : "cURL JSON-RPC Smoke Test",
                  },
                ] as const
              ).map((tab) => (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setClientConfigTab(tab.id)}
                  className={`rounded-lg border px-3 py-1.5 text-xs font-medium transition cursor-pointer ${
                    clientConfigTab === tab.id
                      ? "border-[var(--accent-color)] bg-[var(--accent-color)]/15 font-semibold text-[var(--text-main)]"
                      : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                  }`}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            <div className="mt-4 overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-100">
              <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-900 px-4 py-2">
                <span className="font-mono text-xs text-zinc-300">{clientConfigTab}</span>
                <span className="font-mono text-[11px] text-zinc-400">{apiBase}/mcp</span>
              </div>
              <pre className="overflow-x-auto p-4 font-mono text-xs leading-relaxed text-emerald-300">
                {clientConfigSnippet}
              </pre>
            </div>
          </BrutalCard>
        </div>
      ) : null}

      {dialogs.element}
    </PageShell>
  );
}

async function runTimed<T>(task: () => Promise<T>): Promise<{ result: T; elapsedMs: number }> {
  const started = performance.now();
  const result = await task();
  const elapsedMs = Math.max(1, Math.round(performance.now() - started));
  return { result, elapsedMs };
}

function formatOutputPreview(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined || value === null) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function normalizeDiagnoseOutput(
  output: unknown,
  durationMs: number,
  servers: Server[],
  isZh: boolean,
): NormalizedDiagnoseReport {
  const obj = asRecord(output);
  const rawAnomalies = Array.isArray(obj["anomalies"])
    ? obj["anomalies"]
    : Array.isArray(obj["findings"])
      ? obj["findings"]
      : [];

  const parsedAnomalies: NormalizedAnomaly[] = rawAnomalies.map((item, index) => {
    const row = asRecord(item);
    const rawSeverity = asString(row["severity"], "warning").toLowerCase();
    const severity: "critical" | "warning" | "info" =
      rawSeverity === "critical" || rawSeverity === "high"
        ? "critical"
        : rawSeverity === "info" || rawSeverity === "low"
          ? "info"
          : "warning";

    const suggestedToolObj = asRecord(row["suggested_tool"]);
    const suggestedToolName =
      typeof row["suggested_tool"] === "string"
        ? row["suggested_tool"]
        : asString(
            suggestedToolObj["name"] ?? row["recommended_tool"],
            "server.inspect",
          );
    const suggestedArgs =
      typeof row["suggested_arguments"] === "object" && row["suggested_arguments"] !== null
        ? asRecord(row["suggested_arguments"])
        : typeof suggestedToolObj["arguments"] === "object" &&
            suggestedToolObj["arguments"] !== null
          ? asRecord(suggestedToolObj["arguments"])
          : row["server_id"]
            ? { server_id: asString(row["server_id"]), playbook: "system_vital" }
            : {};

    return {
      id: asString(row["id"], `anomaly-${index}`),
      severity,
      category: asString(row["category"] ?? row["kind"], "node"),
      resourceName: asString(
        row["resource_name"] ?? row["server_name"] ?? row["service_name"] ?? row["name"],
        "Cluster Resource",
      ),
      resourceId: asString(row["resource_id"] ?? row["server_id"] ?? row["service_id"], ""),
      title: asString(row["title"] ?? row["summary"] ?? row["message"], "Detected anomaly"),
      evidence: asString(row["evidence"] ?? row["metric"] ?? row["detail"], "threshold exceeded"),
      recommendation: asString(
        row["recommendation"] ?? row["remediation"] ?? row["hint"],
        isZh ? "运行建议的诊断工具进一步排查根因。" : "Run the suggested diagnostic tool to inspect root cause.",
      ),
      suggestedTool: suggestedToolName,
      suggestedArgs,
    };
  });

  // If backend didn't return anomalies yet, synthesize client-side telemetry anomalies from live servers
  if (parsedAnomalies.length === 0 && output === null && servers.length > 0) {
    for (const srv of servers) {
      if (srv.status !== "online") {
        parsedAnomalies.push({
          id: `offline-${srv.id}`,
          severity: "critical",
          category: "connectivity",
          resourceName: srv.name,
          resourceId: srv.id,
          title: isZh ? "服务器节点离线失联" : "Server Node Offline",
          evidence: `status=${srv.status}, last_seen=${srv.last_seen_at ?? "unknown"}`,
          recommendation: isZh
            ? "检查节点宿主网络或 systemd xlstatus-agent 服务状态。"
            : "Verify host network reachability or xlstatus-agent systemd service.",
          suggestedTool: "server.get",
          suggestedArgs: { server_id: srv.id },
        });
      } else if ((srv.cpu_percent ?? 0) >= 85) {
        parsedAnomalies.push({
          id: `cpu-${srv.id}`,
          severity: "warning",
          category: "cpu_hotspot",
          resourceName: srv.name,
          resourceId: srv.id,
          title: isZh ? "节点 CPU 负载越界" : "High CPU Saturation",
          evidence: `cpu_percent=${formatPercent(srv.cpu_percent)}, load_1=${srv.load_1 ?? 0}`,
          recommendation: isZh
            ? "运行 top_processes 诊断剧本定位高占用进程。"
            : "Run top_processes playbook to identify runaway CPU consumers.",
          suggestedTool: "server.inspect",
          suggestedArgs: { server_id: srv.id, playbook: "top_processes" },
        });
      }
    }
  }

  const rawScore = obj["health_score"] ?? obj["score"];
  const computedScore =
    typeof rawScore === "number" && Number.isFinite(rawScore)
      ? Math.max(0, Math.min(100, Math.round(rawScore)))
      : Math.max(
          0,
          100 -
            parsedAnomalies.filter((a) => a.severity === "critical").length * 20 -
            parsedAnomalies.filter((a) => a.severity === "warning").length * 8,
        );

  const status: "healthy" | "degraded" | "critical" =
    computedScore >= 85 ? "healthy" : computedScore >= 60 ? "degraded" : "critical";

  const defaultSummary = isZh
    ? `已扫描 ${servers.length} 台节点（在线 ${servers.filter((s) => s.status === "online").length} 台），发现 ${parsedAnomalies.length} 项待关注异常。`
    : `Scanned ${servers.length} nodes (${servers.filter((s) => s.status === "online").length} online); detected ${parsedAnomalies.length} anomalies.`;

  return {
    healthScore: asNumber(rawScore, computedScore),
    status,
    summary: asString(obj["summary"], defaultSummary),
    anomalies: parsedAnomalies,
    checkedAt: asString(obj["timestamp"] ?? obj["checked_at"], new Date().toISOString()),
    durationMs,
    rawOutput: output,
  };
}

function extractPromptMarkdown(
  output: unknown,
  promptName: string,
  args: Record<string, unknown>,
  report: NormalizedDiagnoseReport,
  servers: Server[],
): string {
  const obj = asRecord(output);
  if (typeof obj["prompt"] === "string" && obj["prompt"].trim()) {
    return obj["prompt"];
  }
  if (Array.isArray(obj["messages"]) && obj["messages"].length > 0) {
    const chunks: string[] = [];
    for (const msg of obj["messages"]) {
      const m = asRecord(msg);
      const content = asRecord(m["content"]);
      const text =
        typeof m["content"] === "string"
          ? m["content"]
          : asString(content["text"], "");
      if (text) chunks.push(text);
    }
    if (chunks.length > 0) return chunks.join("\n\n");
  }
  const onlineCount = servers.filter((s) => s.status === "online").length;
  return [
    `# XLStatus AI SRE Prompt — ${promptName}`,
    ``,
    `## Context & Parameters`,
    `- Arguments: \`${JSON.stringify(args)}\``,
    `- Cluster Health Score: **${report.healthScore}/100** (${report.status.toUpperCase()})`,
    `- Fleet Coverage: **${onlineCount}/${servers.length}** nodes online`,
    `- Active Anomalies: **${report.anomalies.length}**`,
    ``,
    `## Detected Anomalies`,
    ...(report.anomalies.length === 0
      ? [`- None (All monitored metrics within nominal thresholds)`]
      : report.anomalies.map(
          (a) =>
            `- **[${a.severity.toUpperCase()}] ${a.resourceName}**: ${a.title} (${a.evidence}) → Suggested MCP Tool: \`${a.suggestedTool}(${JSON.stringify(a.suggestedArgs)})\``,
        )),
    ``,
    `## Instructions for AI SRE Agent`,
    `1. Review the anomalies and live telemetry above.`,
    `2. Call \`server.inspect\` with playbooks (\`system_vital\`, \`top_processes\`, \`disk_and_inodes\`, \`network_sockets\`, \`docker_containers\`, \`system_logs\`) to gather deeper evidence.`,
    `3. Provide a concise root-cause analysis and safe, step-by-step remediation commands.`,
  ].join("\n");
}
