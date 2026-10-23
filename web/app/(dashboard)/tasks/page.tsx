"use client";

import { FormEvent, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  Modal,
  PageHeader,
  PageShell,
  StatusBadge,
  TableSkeleton,
  buttonClass,
  compactId,
  formatDate,
  formatMs,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type ServerGroup, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useServerData, type Server } from "@/app/components/ServerDataProvider";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";
import type { tasksPage } from "@/lib/i18n/pages/tasks";

type TaskType = "shell" | "http_get" | "icmp_ping" | "tcp_ping";
type CoverMode = "all" | "any" | "specific";
type StatusFilter = "ALL" | "pending/running" | "completed/success" | "failed";

interface NotificationGroup {
  id: string;
  name: string;
}

interface Task {
  id: string;
  name: string;
  task_type: TaskType;
  schedule?: string | null | undefined;
  command?: string | null | undefined;
  payload_json?: string | null | undefined;
  cover_mode: CoverMode;
  server_selector_json: string;
  push_successful: boolean;
  notification_group_id?: string | null | undefined;
  enabled: boolean;
  last_executed_at?: string | null | undefined;
  last_result?: string | null | undefined;
}

interface TaskRun {
  id: string;
  server_id: string;
  status: string;
  exit_code?: number | null | undefined;
  delay_ms?: number | null | undefined;
  output?: string | null | undefined;
  stdout?: string | null | undefined;
  stderr?: string | null | undefined;
  error?: string | null | undefined;
  created_at: string;
}

interface TaskForm {
  name: string;
  task_type: TaskType;
  schedule: string;
  command: string;
  timeout_seconds: number;
  payload_json: string;
  cover_mode: CoverMode;
  selected_server_ids: string[];
  excluded_server_ids: string[];
  selected_group_ids: string[];
  tag_names: string[];
  source_server: boolean;
  push_successful: boolean;
  notification_group_id: string;
  enabled: boolean;
}

interface PlaybookPreset {
  id: string;
  label: string;
  defaultName: string;
  command: string;
  timeoutSeconds: number;
}

const PLAYBOOK_PRESETS: readonly PlaybookPreset[] = [
  {
    id: "sys-info",
    label: "系统概览 / System Info",
    defaultName: "System Overview",
    command: "uname -a && uptime",
    timeoutSeconds: 15,
  },
  {
    id: "disk-usage",
    label: "磁盘空间 / Disk Usage",
    defaultName: "Disk Space Audit",
    command: "df -h",
    timeoutSeconds: 15,
  },
  {
    id: "mem-top",
    label: "内存与负载 / Memory & Top",
    defaultName: "Memory & Load Snapshot",
    command: "top -l 1 -n 5 2>/dev/null || free -m",
    timeoutSeconds: 20,
  },
  {
    id: "net-ports",
    label: "网络监听 / Listening Ports",
    defaultName: "Listening Ports Check",
    command: "netstat -an | head -n 30",
    timeoutSeconds: 20,
  },
  {
    id: "docker-ps",
    label: "Docker 容器 / Containers",
    defaultName: "Docker Containers Status",
    command: 'docker ps --format "table {{.Names}}\\t{{.Status}}\\t{{.Ports}}"',
    timeoutSeconds: 30,
  },
];

const STATUS_FILTER_PILLS: readonly { value: StatusFilter; label: string }[] = [
  { value: "ALL", label: "ALL" },
  { value: "pending/running", label: "pending/running" },
  { value: "completed/success", label: "completed/success" },
  { value: "failed", label: "failed" },
];

const blankForm: TaskForm = {
  name: "",
  task_type: "shell",
  schedule: "",
  command: "",
  timeout_seconds: 30,
  payload_json: '{"timeout_secs":30}',
  cover_mode: "specific",
  selected_server_ids: [],
  excluded_server_ids: [],
  selected_group_ids: [],
  tag_names: [],
  source_server: false,
  push_successful: false,
  notification_group_id: "",
  enabled: true,
};

export default function TasksPage() {
  const dialogs = useDialogs();
  const { t: copy, locale } = useI18n();
  const isZh = locale === "zh-CN";
  const {
    servers,
    error: serverLoadError,
    revalidate: revalidateServers,
  } = useServerData();
  const [tasks, setTasks] = useState<Task[]>([]);
  const [serverGroups, setServerGroups] = useState<ServerGroup[]>([]);
  const [notificationGroups, setNotificationGroups] = useState<NotificationGroup[]>([]);
  const [runs, setRuns] = useState<TaskRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [optionsLoading, setOptionsLoading] = useState(false);
  const [optionsError, setOptionsError] = useState<string | null>(null);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "edit" | "runs" | null>(null);
  const [editing, setEditing] = useState<Task | null>(null);
  const [inspectedTask, setInspectedTask] = useState<Task | null>(null);
  const [form, setForm] = useState<TaskForm>(blankForm);
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("ALL");
  const [runStatusFilter, setRunStatusFilter] = useState<StatusFilter>("ALL");
  const [runSearchQuery, setRunSearchQuery] = useState("");
  const [copiedRunId, setCopiedRunId] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();
  const runsRequestId = useRef(0);
  const runsAbortRef = useRef<AbortController | null>(null);
  const initialLoadRef = useRef(true);
  const optionsLoadedRef = useRef(false);
  const tasksLoadRequestIdRef = useRef(0);

  const serverMap = useMemo(() => {
    const map = new Map<string, Server>();
    for (const s of servers) {
      map.set(s.id, s);
    }
    return map;
  }, [servers]);

  const onlineServerCount = useMemo(
    () => servers.filter((s) => isServerOnline(s.status)).length,
    [servers],
  );

  const loadTasks = useCallback(async (signal?: AbortSignal) => {
    const requestId = tasksLoadRequestIdRef.current + 1;
    tasksLoadRequestIdRef.current = requestId;
    if (initialLoadRef.current) setLoading(true);
    setError(null);
    const tasksResponse = await apiClient.listTasks(200, 0, signal ? { signal } : {});
    if (signal?.aborted || requestId !== tasksLoadRequestIdRef.current) return;
    if (tasksResponse.success && tasksResponse.data) {
      setTasks((tasksResponse.data["tasks"] as Task[]) ?? []);
    } else {
      setError(responseError(tasksResponse));
    }
    initialLoadRef.current = false;
    setLoading(false);
  }, []);

  const loadTaskOptions = useCallback(async () => {
    if (optionsLoadedRef.current) return;
    setOptionsLoading(true);
    setOptionsError(null);
    const [groupsResponse, notificationGroupsResponse] = await Promise.all([
      apiClient.listServerGroups(),
      apiClient.listNotificationGroups(200, 0),
    ]);
    const optionErrors: string[] = [];
    if (groupsResponse.success && groupsResponse.data) {
      setServerGroups(groupsResponse.data.groups ?? []);
    } else {
      optionErrors.push(responseError(groupsResponse));
    }
    if (notificationGroupsResponse.success && notificationGroupsResponse.data) {
      setNotificationGroups((notificationGroupsResponse.data["groups"] as NotificationGroup[]) ?? []);
    } else {
      optionErrors.push(responseError(notificationGroupsResponse));
    }
    setOptionsError(optionErrors[0] ?? null);
    optionsLoadedRef.current = optionErrors.length === 0;
    setOptionsLoading(false);
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadTasks(controller.signal);
    return () => {
      controller.abort();
      tasksLoadRequestIdRef.current += 1;
      runsAbortRef.current?.abort();
    };
  }, [loadTasks]);

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  const deferredQuery = useDeferredValue(query);

  const statusCounts = useMemo(() => {
    const counts: Record<StatusFilter, number> = {
      ALL: tasks.length,
      "pending/running": 0,
      "completed/success": 0,
      failed: 0,
    };
    for (const task of tasks) {
      const bucket = classifyStatusFilter(task.last_result, runningTaskId === task.id);
      counts[bucket] += 1;
    }
    return counts;
  }, [tasks, runningTaskId]);

  const filtered = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return tasks.filter((task) => {
      if (statusFilter !== "ALL") {
        const bucket = classifyStatusFilter(task.last_result, runningTaskId === task.id);
        if (bucket !== statusFilter) return false;
      }
      if (!needle) return true;
      return [task.name, task.task_type, task.schedule, task.command, task.last_result]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle));
    });
  }, [deferredQuery, tasks, statusFilter, runningTaskId]);

  const filteredRuns = useMemo(() => {
    const needle = runSearchQuery.trim().toLowerCase();
    return runs.filter((run) => {
      if (runStatusFilter !== "ALL") {
        const bucket = classifyStatusFilter(run.status, false);
        if (bucket !== runStatusFilter) return false;
      }
      if (!needle) return true;
      const serverName = serverMap.get(run.server_id)?.name ?? "";
      return [
        run.server_id,
        serverName,
        run.status,
        run.output,
        run.stdout,
        run.stderr,
        run.error,
      ]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle));
    });
  }, [runs, runStatusFilter, runSearchQuery, serverMap]);

  function openCreate() {
    setEditing(null);
    const onlineIds = servers.filter((s) => isServerOnline(s.status)).map((s) => s.id);
    setForm({
      ...blankForm,
      selected_server_ids: onlineIds,
    });
    setModal("create");
    void loadTaskOptions();
  }

  function openCreateWithPreset(preset: PlaybookPreset) {
    setEditing(null);
    const onlineIds = servers.filter((s) => isServerOnline(s.status)).map((s) => s.id);
    setForm({
      ...blankForm,
      name: preset.defaultName,
      task_type: "shell",
      command: preset.command,
      timeout_seconds: preset.timeoutSeconds,
      payload_json: JSON.stringify({ timeout_secs: preset.timeoutSeconds }),
      cover_mode: "specific",
      selected_server_ids: onlineIds,
    });
    setModal("create");
    void loadTaskOptions();
  }

  function openEdit(task: Task) {
    const selector = parseSelector(task.server_selector_json);
    const parsedTimeout = parseTimeoutSeconds(task.payload_json);
    setEditing(task);
    setForm({
      name: task.name || "",
      task_type: task.task_type || "shell",
      schedule: task.schedule || "",
      command: task.command || "",
      timeout_seconds: parsedTimeout,
      payload_json: task.payload_json || JSON.stringify({ timeout_secs: parsedTimeout }),
      cover_mode: task.cover_mode || "specific",
      selected_server_ids: selector.server_ids,
      excluded_server_ids: selector.exclude_server_ids,
      selected_group_ids: selector.group_ids,
      tag_names: selector.tag_names,
      source_server: selector.source_server,
      push_successful: Boolean(task.push_successful),
      notification_group_id: task.notification_group_id || "",
      enabled: task.enabled,
    });
    setModal("edit");
    void loadTaskOptions();
  }

  const fetchRunsForTask = useCallback(async (task: Task, openModalDialog: boolean) => {
    runsAbortRef.current?.abort();
    const controller = new AbortController();
    runsAbortRef.current = controller;
    const requestId = runsRequestId.current + 1;
    runsRequestId.current = requestId;
    setEditing(task);
    setInspectedTask(task);
    setRuns([]);
    setRunsError(null);
    setRunsLoading(true);
    if (openModalDialog) {
      setModal("runs");
    }
    const response = await apiClient.listTaskRuns(task.id, 30, { signal: controller.signal });
    if (controller.signal.aborted || requestId !== runsRequestId.current) return;
    if (response.success && response.data) {
      setRuns((response.data["runs"] as TaskRun[]) ?? []);
    } else {
      setRunsError(responseError(response));
    }
    setRunsLoading(false);
  }, []);

  async function openRuns(task: Task) {
    await fetchRunsForTask(task, true);
  }

  async function inspectTaskInline(task: Task) {
    await fetchRunsForTask(task, false);
  }

  async function copyRunOutput(run: TaskRun) {
    const text = formatRunConsoleText(run);
    try {
      await navigator.clipboard.writeText(text);
      setCopiedRunId(run.id);
      showToast(isZh ? "终端输出已复制到剪贴板" : "Terminal output copied to clipboard");
      window.setTimeout(() => {
        setCopiedRunId((current) => (current === run.id ? null : current));
      }, 2000);
    } catch {
      showToast(isZh ? "复制失败，请手动选择复制" : "Copy failed, please copy manually");
    }
  }

  async function submitForm() {
    setSaving(true);
    const payload = formToPayload(form);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) {
      setSaving(false);
      return;
    }
    const response =
      modal === "edit" && editing
        ? await apiClient.updateTask(editing.id, payload, totpCode)
        : await apiClient.createTask(payload, totpCode);
    setSaving(false);
    if (response.success) {
      setModal(null);
      showToast(modal === "edit" ? copy.tasksPage.noticeUpdated : copy.tasksPage.noticeCreated);
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  async function runTask(task: Task) {
    if (!(await dialogs.confirm({ message: copy.tasksPage.runConfirm.replace("{name}", String(task.name)) }))) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setRunningTaskId(task.id);
    const response = await apiClient.runTask(task.id, totpCode);
    setRunningTaskId(null);
    if (response.success) {
      showToast(copy.tasksPage.noticeRunRequested);
      await loadTasks();
      void fetchRunsForTask(task, false);
    } else {
      setError(responseError(response));
    }
  }

  async function sensitiveTotpCode(): Promise<string | undefined | null> {
    let enabled = totpStatus?.enabled;
    if (totpStatus === null) {
      const response = await apiClient.getTotpStatus();
      if (!response.success || !response.data) {
        setError(responseError(response));
        return null;
      }
      setTotpStatus(response.data);
      enabled = response.data.enabled;
    }
    if (!enabled) return undefined;
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.tasksPage.totpInvalid);
      return null;
    }
    return trimmed;
  }

  async function deleteTask(task: Task) {
    if (
      !(await dialogs.confirm({
        message: copy.tasksPage.deleteConfirm.replace("{name}", String(task.name)),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteTask(task.id, totpCode);
    if (response.success) {
      showToast(copy.tasksPage.noticeDeleted);
      if (inspectedTask?.id === task.id) {
        setInspectedTask(null);
        setRuns([]);
      }
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.tasksPage.eyebrow}
          title={copy.tasksPage.title}
          detail={copy.tasksPage.detail}
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge tone={onlineServerCount > 0 ? "green" : "gray"}>
                {isZh
                  ? `${onlineServerCount}/${servers.length} 在线节点`
                  : `${onlineServerCount}/${servers.length} Online Nodes`}
              </StatusBadge>
              <button type="button" onClick={openCreate} className={buttonClass("primary")}>
                {copy.tasksPage.newTask}
              </button>
            </div>
          }
        />

        {/* One-Click Ops Playbook Quick-Launch Studio Banner */}
        <BrutalCard className="mb-5 border-[var(--border-color)] bg-gradient-to-br from-[var(--bg-card)] to-[var(--bg-subtle)]">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div className="flex items-center gap-2">
                <span className="inline-block h-2 w-2 rounded-full bg-emerald-500" />
                <h2 className="text-sm font-semibold tracking-tight text-[var(--text-main)]">
                  {isZh
                    ? "一键运维剧本 / Fleet Ops Playbook Presets"
                    : "One-Click Ops Playbook / Command Presets"}
                </h2>
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                {isZh
                  ? "点击预设剧本即可自动填充常用排障指令、超时时间与当前在线节点。"
                  : "Click any preset chip to populate the command, timeout, and online target nodes with one click."}
              </p>
            </div>
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            {PLAYBOOK_PRESETS.map((preset) => (
              <button
                key={preset.id}
                type="button"
                onClick={() => openCreateWithPreset(preset)}
                className="group inline-flex items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-3 py-1.5 text-xs font-medium text-[var(--text-main)] shadow-[var(--shadow-xs)] transition-all hover:border-emerald-500/50 hover:bg-emerald-500/5 cursor-pointer"
              >
                <span className="font-semibold text-[var(--text-main)] group-hover:text-emerald-600 dark:group-hover:text-emerald-400">
                  {preset.label}
                </span>
                <code className="rounded bg-[var(--bg-muted)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                  {preset.command}
                </code>
                <span className="rounded-full bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[10px] text-emerald-600 dark:text-emerald-400">
                  {preset.timeoutSeconds}s
                </span>
              </button>
            ))}
          </div>
        </BrutalCard>

        {/* Status Filter Pills + Search Input */}
        <div className="mb-5 space-y-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Task status filters">
              {STATUS_FILTER_PILLS.map((pill) => {
                const active = statusFilter === pill.value;
                return (
                  <button
                    key={pill.value}
                    type="button"
                    onClick={() => setStatusFilter(pill.value)}
                    className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium transition-all cursor-pointer ${
                      active
                        ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-600 dark:text-emerald-300 shadow-[var(--shadow-xs)]"
                        : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-muted)] hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    <span>{pill.label}</span>
                    <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.2 font-mono text-[11px]">
                      {statusCounts[pill.value]}
                    </span>
                  </button>
                );
              })}
            </div>
            <div className="w-full sm:max-w-xs">
              <input
                className={inputClass}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={copy.tasksPage.searchPlaceholder}
              />
            </div>
          </div>
          <InlineError message={error ?? serverLoadError} />
        </div>

        {loading ? (
          <TableSkeleton columns={5} rows={5} />
        ) : filtered.length === 0 ? (
          <EmptyState title={copy.tasksPage.emptyTitle} detail={copy.tasksPage.emptyDetail} />
        ) : (
          <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
            <table className="w-full min-w-[52rem]">
              <thead>
                <tr>
                  <th className={thClass}>{copy.tasksPage.colName}</th>
                  <th className={thClass}>{copy.tasksPage.colType}</th>
                  <th className={thClass}>{copy.tasksPage.colSchedule}</th>
                  <th className={thClass}>{copy.tasksPage.colResult}</th>
                  <th className={thClass}>{copy.tasksPage.colActions}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-color)]">
                {filtered.map((task) => {
                  const isInspected = inspectedTask?.id === task.id;
                  const selectorInfo = parseSelector(task.server_selector_json);
                  return (
                    <tr
                      key={task.id}
                      onClick={() => void inspectTaskInline(task)}
                      className={`transition-colors hover:bg-[var(--bg-subtle)] cursor-pointer ${
                        isInspected ? "bg-emerald-500/5" : ""
                      }`}
                    >
                      <td className={tdClass}>
                        <div className="font-medium text-[var(--text-main)]">{task.name}</div>
                        {task.command ? (
                          <div className="mt-0.5 max-w-md truncate font-mono text-xs text-[var(--text-muted)]">
                            $ {task.command}
                          </div>
                        ) : null}
                      </td>
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <StatusBadge tone="blue">{task.task_type}</StatusBadge>
                          <span className="font-mono text-xs text-[var(--text-muted)]">
                            {task.cover_mode === "all"
                              ? isZh
                                ? "全部节点"
                                : "all nodes"
                              : `${selectorInfo.server_ids.length} ${isZh ? "节点" : "nodes"}`}
                          </span>
                        </div>
                      </td>
                      <td className={`${tdClass} font-mono text-xs text-[var(--text-muted)]`}>
                        {task.schedule || copy.tasksPage.scheduleManual}
                      </td>
                      <td className={tdClass}>
                        <StatusBadge
                          tone={
                            runningTaskId === task.id
                              ? "yellow"
                              : task.last_result === "success"
                                ? "green"
                                : task.last_result
                                  ? "red"
                                  : "gray"
                          }
                        >
                          {runningTaskId === task.id
                            ? "running"
                            : resultLabel(copy.tasksPage, task.last_result)}
                        </StatusBadge>
                      </td>
                      <td
                        className={tdClass}
                        onClick={(event) => {
                          event.stopPropagation();
                        }}
                      >
                        <div className="flex flex-wrap gap-2">
                          <button
                            className={buttonClass("good")}
                            disabled={rowBusy || runningTaskId === task.id}
                            onClick={() => void runRow(() => runTask(task))}
                          >
                            {copy.tasksPage.run}
                          </button>
                          <button className={buttonClass("secondary")} onClick={() => openEdit(task)}>
                            {copy.tasksPage.edit}
                          </button>
                          <button className={buttonClass("secondary")} onClick={() => void openRuns(task)}>
                            {copy.tasksPage.runsLink}
                          </button>
                          <button
                            className={buttonClass("danger")}
                            disabled={rowBusy}
                            onClick={() => void runRow(() => deleteTask(task))}
                          >
                            {copy.tasksPage.delete}
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

        {/* Inline Execution History & Terminal-Styled Output Inspector */}
        {inspectedTask ? (
          <div className="mt-6">
            <BrutalCard className="border-[var(--border-color)]">
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="h-2 w-2 rounded-full bg-emerald-500" />
                    <h3 className="text-sm font-semibold text-[var(--text-main)]">
                      {copy.tasksPage.runsTitle.replace("{name}", String(inspectedTask.name))}
                    </h3>
                  </div>
                  {inspectedTask.command ? (
                    <p className="mt-1 font-mono text-xs text-[var(--text-muted)]">
                      $ {inspectedTask.command}
                    </p>
                  ) : null}
                </div>
                <div className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void fetchRunsForTask(inspectedTask, false)}
                    className={buttonClass("secondary")}
                  >
                    {isZh ? "刷新记录" : "Refresh Runs"}
                  </button>
                  <button
                    type="button"
                    onClick={() => setInspectedTask(null)}
                    className={buttonClass("ghost")}
                  >
                    {isZh ? "收起" : "Hide"}
                  </button>
                </div>
              </div>
              <RunsOutputInspector
                runs={filteredRuns}
                totalRuns={runs.length}
                runsLoading={runsLoading}
                runsError={runsError}
                runStatusFilter={runStatusFilter}
                setRunStatusFilter={setRunStatusFilter}
                runSearchQuery={runSearchQuery}
                setRunSearchQuery={setRunSearchQuery}
                copiedRunId={copiedRunId}
                onCopyRun={(run) => void copyRunOutput(run)}
                serverMap={serverMap}
                copy={copy.tasksPage}
                isZh={isZh}
              />
            </BrutalCard>
          </div>
        ) : null}

        {modal === "create" || modal === "edit" ? (
          <Modal
            title={modal === "edit" ? copy.tasksPage.modalTitleEdit : copy.tasksPage.modalTitleCreate}
            onClose={() => setModal(null)}
          >
            {optionsLoading ? (
              <TableSkeleton columns={2} rows={4} />
            ) : (
              <>
                <InlineError message={optionsError} />
                <TaskFormView
                  form={form}
                  setForm={setForm}
                  servers={servers}
                  serverGroups={serverGroups}
                  notificationGroups={notificationGroups}
                  saving={saving || formBusy}
                  onSubmit={(event) => {
                    event.preventDefault();
                    void runForm(() => submitForm());
                  }}
                />
              </>
            )}
          </Modal>
        ) : null}

        {modal === "runs" ? (
          <Modal
            title={copy.tasksPage.runsTitle.replace("{name}", String(editing?.name || ""))}
            onClose={() => {
              runsRequestId.current += 1;
              runsAbortRef.current?.abort();
              setModal(null);
            }}
          >
            <RunsOutputInspector
              runs={filteredRuns}
              totalRuns={runs.length}
              runsLoading={runsLoading}
              runsError={runsError}
              runStatusFilter={runStatusFilter}
              setRunStatusFilter={setRunStatusFilter}
              runSearchQuery={runSearchQuery}
              setRunSearchQuery={setRunSearchQuery}
              copiedRunId={copiedRunId}
              onCopyRun={(run) => void copyRunOutput(run)}
              serverMap={serverMap}
              copy={copy.tasksPage}
              isZh={isZh}
            />
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function RunsOutputInspector({
  runs,
  totalRuns,
  runsLoading,
  runsError,
  runStatusFilter,
  setRunStatusFilter,
  runSearchQuery,
  setRunSearchQuery,
  copiedRunId,
  onCopyRun,
  serverMap,
  copy,
  isZh,
}: {
  runs: TaskRun[];
  totalRuns: number;
  runsLoading: boolean;
  runsError: string | null;
  runStatusFilter: StatusFilter;
  setRunStatusFilter: (filter: StatusFilter) => void;
  runSearchQuery: string;
  setRunSearchQuery: (query: string) => void;
  copiedRunId: string | null;
  onCopyRun: (run: TaskRun) => void;
  serverMap: Map<string, Server>;
  copy: typeof tasksPage;
  isZh: boolean;
}) {
  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-2.5 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex flex-wrap items-center gap-1.5">
          {STATUS_FILTER_PILLS.map((pill) => {
            const active = runStatusFilter === pill.value;
            return (
              <button
                key={pill.value}
                type="button"
                onClick={() => setRunStatusFilter(pill.value)}
                className={`rounded-full border px-2.5 py-1 text-xs font-medium transition-all cursor-pointer ${
                  active
                    ? "border-emerald-500/50 bg-emerald-500/15 text-emerald-600 dark:text-emerald-300"
                    : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                }`}
              >
                {pill.label}
              </button>
            );
          })}
        </div>
        <div className="w-full sm:w-64">
          <input
            className={inputClass}
            value={runSearchQuery}
            onChange={(e) => setRunSearchQuery(e.target.value)}
            placeholder={isZh ? "过滤节点或输出关键字..." : "Filter node or stdout/stderr..."}
          />
        </div>
      </div>

      {runsLoading ? (
        <TableSkeleton columns={2} rows={3} />
      ) : runsError ? (
        <InlineError message={runsError} />
      ) : totalRuns === 0 ? (
        <EmptyState title={copy.runsEmpty} />
      ) : runs.length === 0 ? (
        <EmptyState title={isZh ? "没有匹配的运行记录" : "No matching execution runs"} />
      ) : (
        <div className="grid gap-3">
          {runs.map((run) => {
            const server = serverMap.get(run.server_id);
            const exitCode = resolveExitCode(run);
            const consoleText = formatRunConsoleText(run);
            return (
              <div
                key={run.id}
                className="overflow-hidden rounded-xl border border-zinc-800 bg-[#0d1117] shadow-[var(--shadow-xs)]"
              >
                <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800/90 bg-zinc-900/90 px-4 py-2.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="inline-flex items-center gap-1.5 font-mono text-xs font-semibold text-zinc-200">
                      <span
                        className={`h-2 w-2 rounded-full ${
                          run.status === "success"
                            ? "bg-emerald-400"
                            : run.status === "pending" || run.status === "running"
                              ? "bg-amber-400 animate-pulse"
                              : "bg-rose-400"
                        }`}
                      />
                      {server
                        ? `${server.name} (${compactId(run.server_id)})`
                        : copy.runServerPrefix.replace("{id}", String(compactId(run.server_id)))}
                    </span>
                    <StatusBadge
                      tone={
                        run.status === "success"
                          ? "green"
                          : run.status === "pending" || run.status === "running"
                            ? "yellow"
                            : "red"
                      }
                    >
                      {resultLabel(copy, run.status)}
                    </StatusBadge>
                    <span
                      className={`rounded border px-2 py-0.5 font-mono text-[11px] font-semibold ${
                        exitCode === 0
                          ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                          : "border-rose-500/30 bg-rose-500/10 text-rose-300"
                      }`}
                    >
                      EXIT {exitCode}
                    </span>
                  </div>
                  <div className="flex items-center gap-2.5">
                    <span className="font-mono text-xs text-zinc-400">
                      {formatDate(run.created_at)} / {formatMs(run.delay_ms)}
                    </span>
                    <button
                      type="button"
                      onClick={() => onCopyRun(run)}
                      className="inline-flex items-center gap-1 rounded-md border border-zinc-700 bg-zinc-800/90 px-2.5 py-1 font-mono text-[11px] font-medium text-zinc-200 transition hover:border-emerald-500/50 hover:text-emerald-300 cursor-pointer"
                    >
                      {copiedRunId === run.id
                        ? isZh
                          ? "已复制 ✓"
                          : "Copied ✓"
                        : isZh
                          ? "复制输出 / Copy"
                          : "Copy Output"}
                    </button>
                  </div>
                </div>
                <pre className="max-h-56 overflow-auto bg-[#0d1117] p-4 font-mono text-xs leading-relaxed text-emerald-300">
                  {consoleText}
                </pre>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function TaskFormView({
  form,
  setForm,
  servers,
  serverGroups,
  notificationGroups,
  saving,
  onSubmit,
}: {
  form: TaskForm;
  setForm: React.Dispatch<React.SetStateAction<TaskForm>>;
  servers: Server[];
  serverGroups: ServerGroup[];
  notificationGroups: NotificationGroup[];
  saving: boolean;
  onSubmit: (event: FormEvent) => void;
}) {
  const { t: copy, locale } = useI18n();
  const isZh = locale === "zh-CN";

  function applyPreset(preset: PlaybookPreset) {
    setForm((current) => ({
      ...current,
      name: current.name.trim() ? current.name : preset.defaultName,
      task_type: "shell",
      command: preset.command,
      timeout_seconds: preset.timeoutSeconds,
      payload_json: updateTimeoutInPayload(current.payload_json, preset.timeoutSeconds),
    }));
  }

  function toggleServerSelection(serverId: string) {
    setForm((current) => {
      const exists = current.selected_server_ids.includes(serverId);
      const nextIds = exists
        ? current.selected_server_ids.filter((id) => id !== serverId)
        : [...current.selected_server_ids, serverId];
      return {
        ...current,
        cover_mode: "specific",
        selected_server_ids: nextIds,
      };
    });
  }

  function selectOnlineServers() {
    const onlineIds = servers.filter((s) => isServerOnline(s.status)).map((s) => s.id);
    setForm((current) => ({
      ...current,
      cover_mode: "specific",
      selected_server_ids: onlineIds,
    }));
  }

  function selectAllServers() {
    const allIds = servers.map((s) => s.id);
    setForm((current) => ({
      ...current,
      cover_mode: "specific",
      selected_server_ids: allIds,
    }));
  }

  function clearSelectedServers() {
    setForm((current) => ({
      ...current,
      selected_server_ids: [],
    }));
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={copy.tasksPage.fieldName}>
          <input
            className={inputClass}
            value={form.name}
            onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            required
          />
        </Field>
        <Field label={copy.tasksPage.fieldType}>
          <select
            className={selectClass}
            value={form.task_type}
            onChange={(e) => setForm((f) => ({ ...f, task_type: e.target.value as TaskType }))}
          >
            <option value="shell">shell</option>
            <option value="http_get">http_get</option>
            <option value="icmp_ping">icmp_ping</option>
            <option value="tcp_ping">tcp_ping</option>
          </select>
        </Field>
      </div>

      <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_160px]">
        <Field label={copy.tasksPage.fieldSchedule}>
          <input
            className={inputClass}
            value={form.schedule}
            onChange={(e) => setForm((f) => ({ ...f, schedule: e.target.value }))}
            placeholder={copy.tasksPage.schedulePlaceholder}
          />
        </Field>
        <Field label={isZh ? "超时 (秒) / Timeout" : "Timeout (s)"}>
          <input
            type="number"
            min={1}
            max={3600}
            className={inputClass}
            value={form.timeout_seconds}
            onChange={(e) => {
              const nextTimeout = Math.max(1, Number(e.target.value) || 30);
              setForm((f) => ({
                ...f,
                timeout_seconds: nextTimeout,
                payload_json: updateTimeoutInPayload(f.payload_json, nextTimeout),
              }));
            }}
          />
        </Field>
      </div>

      {/* One-Click Ops Playbook / Command Presets above the command editor */}
      <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3">
        <div className="mb-2 flex items-center justify-between gap-2">
          <span className="text-xs font-semibold text-[var(--text-main)]">
            {isZh ? "预设运维指令 / One-Click Command Presets" : "One-Click Command Presets"}
          </span>
          <span className="font-mono text-[11px] text-[var(--text-muted)]">
            {isZh ? "点击自动填充命令与超时" : "Click to populate command & timeout"}
          </span>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {PLAYBOOK_PRESETS.map((preset) => {
            const isSelected = form.command.trim() === preset.command;
            return (
              <button
                key={preset.id}
                type="button"
                onClick={() => applyPreset(preset)}
                className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium transition-all cursor-pointer ${
                  isSelected
                    ? "border-emerald-500/60 bg-emerald-500/15 text-emerald-600 dark:text-emerald-300"
                    : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-main)] hover:border-emerald-500/40"
                }`}
              >
                <span>{preset.label}</span>
                <span className="rounded bg-[var(--bg-muted)] px-1 py-0.2 font-mono text-[10px] text-[var(--text-muted)]">
                  {preset.timeoutSeconds}s
                </span>
              </button>
            );
          })}
        </div>
      </div>

      <Field label={copy.tasksPage.fieldCommand}>
        <textarea
          className={`${textareaClass} min-h-28 bg-[#0d1117] text-emerald-300`}
          value={form.command}
          onChange={(e) => setForm((f) => ({ ...f, command: e.target.value }))}
          placeholder="uname -a && uptime"
        />
      </Field>

      {/* Interactive Target Server Selector */}
      <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5 space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <span className="text-xs font-semibold text-[var(--text-main)]">
              {copy.tasksPage.fieldServers} ({form.selected_server_ids.length}/{servers.length})
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onClick={selectOnlineServers}
              className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-600 dark:text-emerald-300 transition hover:bg-emerald-500/20 cursor-pointer"
            >
              全选在线节点 / Select Online
            </button>
            <button
              type="button"
              onClick={selectAllServers}
              className="rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 text-xs font-medium text-[var(--text-main)] transition hover:border-[var(--border-strong)] cursor-pointer"
            >
              全选 / Select All
            </button>
            <button
              type="button"
              onClick={clearSelectedServers}
              className="rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 text-xs font-medium text-[var(--text-muted)] transition hover:text-[var(--text-main)] cursor-pointer"
            >
              清空 / Clear
            </button>
          </div>
        </div>

        {servers.length === 0 ? (
          <p className="text-xs text-[var(--text-muted)]">
            {isZh ? "暂无可用节点" : "No servers available"}
          </p>
        ) : (
          <div className="flex max-h-44 flex-wrap gap-2 overflow-y-auto p-0.5">
            {servers.map((server) => {
              const selected = form.selected_server_ids.includes(server.id);
              const online = isServerOnline(server.status);
              return (
                <button
                  key={server.id}
                  type="button"
                  onClick={() => toggleServerSelection(server.id)}
                  aria-pressed={selected}
                  className={`inline-flex items-center gap-2 rounded-lg border px-3 py-1.5 text-xs font-medium transition-all cursor-pointer ${
                    selected
                      ? "border-emerald-500/60 bg-emerald-500/15 text-[var(--text-main)] ring-1 ring-emerald-500/30"
                      : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-muted)] hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
                  }`}
                >
                  <span
                    className={`h-2 w-2 shrink-0 rounded-full ${
                      online ? "bg-emerald-500" : "bg-zinc-400"
                    }`}
                  />
                  <span className="truncate max-w-[10rem]">{server.name}</span>
                  <span className="font-mono text-[10px] opacity-75">({server.status})</span>
                </button>
              );
            })}
          </div>
        )}

        <div className="grid gap-4 pt-1 sm:grid-cols-2">
          <Field label={copy.tasksPage.fieldCoverMode}>
            <select
              className={selectClass}
              value={form.cover_mode}
              onChange={(e) => setForm((f) => ({ ...f, cover_mode: e.target.value as CoverMode }))}
            >
              <option value="specific">specific</option>
              <option value="all">all</option>
              <option value="any">any</option>
            </select>
          </Field>
          <Field label={copy.tasksPage.fieldPayloadJson}>
            <input
              className={`${inputClass} font-mono text-xs`}
              value={form.payload_json}
              onChange={(e) => setForm((f) => ({ ...f, payload_json: e.target.value }))}
            />
          </Field>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={copy.tasksPage.fieldServerGroups}>
          <select
            multiple
            className={`${selectClass} min-h-24`}
            value={form.selected_group_ids}
            onChange={(e) =>
              setForm((f) => ({
                ...f,
                selected_group_ids: Array.from(e.target.selectedOptions).map((option) => option.value),
              }))
            }
          >
            {serverGroups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name} ({group.server_ids.length})
              </option>
            ))}
          </select>
        </Field>
        <Field label={copy.tasksPage.fieldExcludeServers}>
          <select
            multiple
            className={`${selectClass} min-h-24`}
            value={form.excluded_server_ids}
            onChange={(e) =>
              setForm((f) => ({
                ...f,
                excluded_server_ids: Array.from(e.target.selectedOptions).map((option) => option.value),
              }))
            }
          >
            {servers.map((server) => (
              <option key={server.id} value={server.id}>
                {server.name} ({server.status})
              </option>
            ))}
          </select>
        </Field>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={copy.tasksPage.fieldTags}>
          <input
            className={inputClass}
            value={form.tag_names.join(", ")}
            onChange={(e) => setForm((f) => ({ ...f, tag_names: splitTags(e.target.value) }))}
            placeholder="prod, cn2, edge"
          />
        </Field>
        <Field label={copy.tasksPage.fieldNotificationGroup}>
          <select
            className={selectClass}
            value={form.notification_group_id}
            onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))}
          >
            <option value="">{copy.tasksPage.notificationNone}</option>
            {notificationGroups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name}
              </option>
            ))}
          </select>
        </Field>
      </div>
      <div className="flex flex-wrap gap-4 text-sm font-medium text-[var(--text-main)]">
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={form.source_server}
            onChange={(e) => setForm((f) => ({ ...f, source_server: e.target.checked }))}
          />
          {copy.tasksPage.checkboxSourceServer}
        </label>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={form.push_successful}
            onChange={(e) => setForm((f) => ({ ...f, push_successful: e.target.checked }))}
          />
          {copy.tasksPage.checkboxPushSuccessful}
        </label>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={form.enabled}
            onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
          />
          {copy.tasksPage.checkboxEnabled}
        </label>
      </div>
      <button disabled={saving} className={buttonClass("primary")}>
        {saving ? copy.tasksPage.saving : copy.tasksPage.saveTask}
      </button>
    </form>
  );
}

function isServerOnline(status?: string | null): boolean {
  return String(status || "").toLowerCase() === "online";
}

function classifyStatusFilter(status?: string | null, isCurrentlyRunning?: boolean): StatusFilter {
  if (isCurrentlyRunning) return "pending/running";
  const normalized = String(status || "").toLowerCase();
  if (!normalized || normalized === "pending" || normalized === "running" || normalized === "queued") {
    return "pending/running";
  }
  if (normalized === "success" || normalized === "completed" || normalized === "ok") {
    return "completed/success";
  }
  return "failed";
}

function resolveExitCode(run: TaskRun): number {
  if (typeof run.exit_code === "number" && Number.isFinite(run.exit_code)) {
    return run.exit_code;
  }
  const normalized = String(run.status || "").toLowerCase();
  if (normalized === "success" || normalized === "completed" || normalized === "ok") {
    return 0;
  }
  if (normalized === "timeout") {
    return 124;
  }
  return 1;
}

function formatRunConsoleText(run: TaskRun): string {
  const parts: string[] = [];
  if (run.stdout) parts.push(run.stdout);
  if (run.output && run.output !== run.stdout) parts.push(run.output);
  if (run.stderr) parts.push(`[stderr]\n${run.stderr}`);
  if (run.error && run.error !== run.stderr) parts.push(`[error]\n${run.error}`);
  return parts.join("\n").trim() || "(no stdout/stderr output)";
}

function parseTimeoutSeconds(payloadJson?: string | null): number {
  if (!payloadJson) return 30;
  try {
    const parsed = JSON.parse(payloadJson) as Record<string, unknown>;
    const raw = parsed["timeout_secs"] ?? parsed["timeout_seconds"] ?? parsed["timeout"];
    if (typeof raw === "number" && Number.isFinite(raw) && raw > 0) {
      return Math.floor(raw);
    }
  } catch {
    // ignore invalid json
  }
  return 30;
}

function updateTimeoutInPayload(payloadJson: string, timeoutSeconds: number): string {
  try {
    const trimmed = payloadJson.trim();
    const base = trimmed ? (JSON.parse(trimmed) as Record<string, unknown>) : {};
    return JSON.stringify({
      ...base,
      timeout_secs: timeoutSeconds,
    });
  } catch {
    return JSON.stringify({ timeout_secs: timeoutSeconds });
  }
}

function parseSelector(value: string): {
  server_ids: string[];
  exclude_server_ids: string[];
  group_ids: string[];
  tag_names: string[];
  source_server: boolean;
} {
  try {
    const parsed = JSON.parse(value) as {
      server_ids?: string[] | undefined;
      exclude_server_ids?: string[] | undefined;
      group_ids?: string[] | undefined;
      tag_names?: string[] | undefined;
      source_server?: boolean | undefined;
      tags?: Record<string, string> | undefined;
    };
    const mapTags = parsed.tags
      ? Object.entries(parsed.tags).map(([key, val]) => val || key)
      : [];
    return {
      server_ids: Array.isArray(parsed.server_ids) ? parsed.server_ids : [],
      exclude_server_ids: Array.isArray(parsed.exclude_server_ids) ? parsed.exclude_server_ids : [],
      group_ids: Array.isArray(parsed.group_ids) ? parsed.group_ids : [],
      tag_names: splitTags(
        [...(Array.isArray(parsed.tag_names) ? parsed.tag_names : []), ...mapTags].join(","),
      ),
      source_server: Boolean(parsed.source_server),
    };
  } catch {
    return {
      server_ids: [],
      exclude_server_ids: [],
      group_ids: [],
      tag_names: [],
      source_server: false,
    };
  }
}

function formToPayload(form: TaskForm): JsonObject {
  const tagNames = splitTags(form.tag_names.join(","));
  return {
    name: form.name.trim(),
    task_type: form.task_type,
    schedule: form.schedule.trim() || null,
    command: form.command,
    payload_json: form.payload_json.trim() || null,
    cover_mode: form.cover_mode,
    server_selector_json: JSON.stringify({
      server_ids: form.selected_server_ids,
      exclude_server_ids: form.excluded_server_ids,
      group_ids: form.selected_group_ids,
      tag_names: tagNames,
      tags: Object.fromEntries(tagNames.map((tag) => [tag, ""])),
      source_server: form.source_server,
    }),
    push_successful: form.push_successful,
    notification_group_id: form.notification_group_id || null,
    enabled: form.enabled,
  };
}

function splitTags(value: string): string[] {
  const seen = new Set<string>();
  return value
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
    .filter((item) => {
      const key = item.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

function resultLabel(copy: typeof tasksPage, value?: string | null): string {
  if (!value) return copy.resultNever;
  const labels: Record<string, string> = {
    success: copy.resultSuccess,
    failure: copy.resultFailure,
    error: copy.resultError,
    timeout: copy.resultTimeout,
  };
  return labels[value] ?? value;
}
