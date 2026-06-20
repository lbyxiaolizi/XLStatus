"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  Modal,
  PageHeader,
  PageShell,
  StatusBadge,
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
import { apiClient, type JsonObject, type ServerGroup } from "@/lib/api";

type TaskType = "shell" | "http_get" | "icmp_ping" | "tcp_ping";
type CoverMode = "all" | "any" | "specific";

interface Server {
  id: string;
  name: string;
  status: string;
  tags?: string[];
}

interface NotificationGroup {
  id: string;
  name: string;
}

interface Task {
  id: string;
  name: string;
  task_type: TaskType;
  schedule?: string | null;
  command?: string | null;
  payload_json?: string | null;
  cover_mode: CoverMode;
  server_selector_json: string;
  push_successful: boolean;
  notification_group_id?: string | null;
  enabled: boolean;
  last_executed_at?: string | null;
  last_result?: string | null;
}

interface TaskRun {
  id: string;
  server_id: string;
  status: string;
  delay_ms?: number | null;
  output?: string | null;
  error?: string | null;
  created_at: string;
}

interface TaskForm {
  name: string;
  task_type: TaskType;
  schedule: string;
  command: string;
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

const blankForm: TaskForm = {
  name: "",
  task_type: "shell",
  schedule: "",
  command: "",
  payload_json: "",
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
  const [tasks, setTasks] = useState<Task[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [serverGroups, setServerGroups] = useState<ServerGroup[]>([]);
  const [notificationGroups, setNotificationGroups] = useState<NotificationGroup[]>([]);
  const [runs, setRuns] = useState<TaskRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "edit" | "runs" | null>(null);
  const [editing, setEditing] = useState<Task | null>(null);
  const [form, setForm] = useState<TaskForm>(blankForm);
  const [query, setQuery] = useState("");

  const loadTasks = useCallback(async () => {
    setLoading(true);
    setError(null);
    const [tasksResponse, serversResponse, groupsResponse, notificationGroupsResponse] = await Promise.all([
      apiClient.listTasks(200, 0),
      apiClient.listServers(200, 0),
      apiClient.listServerGroups(),
      apiClient.listNotificationGroups(200, 0),
    ]);
    if (tasksResponse.success && tasksResponse.data) {
      setTasks((tasksResponse.data.tasks as Task[]) ?? []);
    } else {
      setError(responseError(tasksResponse));
    }
    if (serversResponse.success && serversResponse.data) {
      setServers((serversResponse.data.servers as Server[]) ?? []);
    }
    if (groupsResponse.success && groupsResponse.data) {
      setServerGroups(groupsResponse.data.groups ?? []);
    }
    if (notificationGroupsResponse.success && notificationGroupsResponse.data) {
      setNotificationGroups((notificationGroupsResponse.data.groups as NotificationGroup[]) ?? []);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadTasks();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadTasks]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return !needle
      ? tasks
      : tasks.filter((task) => [task.name, task.task_type, task.schedule, task.last_result].filter(Boolean).some((value) => String(value).toLowerCase().includes(needle)));
  }, [query, tasks]);

  function openCreate() {
    setEditing(null);
    setForm(blankForm);
    setModal("create");
  }

  function openEdit(task: Task) {
    const selector = parseSelector(task.server_selector_json);
    setEditing(task);
    setForm({
      name: task.name || "",
      task_type: task.task_type || "shell",
      schedule: task.schedule || "",
      command: task.command || "",
      payload_json: task.payload_json || "",
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
  }

  async function openRuns(task: Task) {
    setEditing(task);
    setRuns([]);
    setModal("runs");
    const response = await apiClient.listTaskRuns(task.id, 30);
    if (response.success && response.data) {
      setRuns((response.data.runs as TaskRun[]) ?? []);
    } else {
      setError(responseError(response));
    }
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    const payload = formToPayload(form);
    const response =
      modal === "edit" && editing
        ? await apiClient.updateTask(editing.id, payload)
        : await apiClient.createTask(payload);
    setSaving(false);
    if (response.success) {
      setModal(null);
      setNotice(modal === "edit" ? "任务已更新。" : "任务已创建。");
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  async function runTask(task: Task) {
    if (!confirm(`现在运行任务「${task.name}」？`)) return;
    setRunningTaskId(task.id);
    const response = await apiClient.runTask(task.id);
    setRunningTaskId(null);
    if (response.success) {
      setNotice("任务运行请求已发送。");
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteTask(task: Task) {
    if (!confirm(`确定删除任务「${task.name}」？`)) return;
    const response = await apiClient.deleteTask(task.id);
    if (response.success) {
      setNotice("任务已删除。");
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="自动化"
          title="任务"
          detail="下发命令、调度任务并查看执行记录。"
          actions={<button type="button" onClick={openCreate} className={buttonClass("primary")}>新增任务</button>}
        />
        <div className="mb-5 space-y-3">
          <input className={inputClass} value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索任务" />
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <BrutalCard>正在加载任务...</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title="暂无任务配置" detail="创建任务后即可向选定 Agent 下发命令。" />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>名称</th>
                  <th className={thClass}>类型</th>
                  <th className={thClass}>调度</th>
                  <th className={thClass}>结果</th>
                  <th className={thClass}>操作</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((task) => (
                  <tr key={task.id}>
                    <td className={tdClass}>{task.name}</td>
                    <td className={tdClass}>{task.task_type}</td>
                    <td className={tdClass}>{task.schedule || "手动"}</td>
                    <td className={tdClass}><StatusBadge tone={task.last_result === "success" ? "green" : task.last_result ? "red" : "gray"}>{resultLabel(task.last_result)}</StatusBadge></td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("good")} disabled={runningTaskId === task.id} onClick={() => void runTask(task)}>运行</button>
                      <button className={buttonClass("secondary")} onClick={() => openEdit(task)}>编辑</button>
                      <button className={buttonClass("secondary")} onClick={() => void openRuns(task)}>记录</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteTask(task)}>删除</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal === "create" || modal === "edit" ? (
          <Modal title={modal === "edit" ? "编辑任务" : "新增任务"} onClose={() => setModal(null)}>
            <TaskFormView
              form={form}
              setForm={setForm}
              servers={servers}
              serverGroups={serverGroups}
              notificationGroups={notificationGroups}
              saving={saving}
              onSubmit={submitForm}
            />
          </Modal>
        ) : null}

        {modal === "runs" ? (
          <Modal title={`运行记录：${editing?.name || ""}`} onClose={() => setModal(null)}>
            {runs.length === 0 ? (
              <EmptyState title="暂无运行记录" />
            ) : (
              <div className="grid gap-3">
                {runs.map((run) => (
                  <BrutalCard key={run.id}>
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <StatusBadge tone={run.status === "success" ? "green" : "red"}>{resultLabel(run.status)}</StatusBadge>
                      <span className="text-sm font-black">{formatDate(run.created_at)} / {formatMs(run.delay_ms)}</span>
                    </div>
                    <p className="mt-2 text-xs font-bold text-[var(--text-muted)]">服务器 {compactId(run.server_id)}</p>
                    <pre className="mt-3 max-h-40 overflow-auto border-2 border-black bg-black p-3 text-xs text-green-300">{run.output || run.error || ""}</pre>
                  </BrutalCard>
                ))}
              </div>
            )}
          </Modal>
        ) : null}
      </PageShell>
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
  return (
    <form onSubmit={onSubmit} className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="名称">
          <input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required />
        </Field>
        <Field label="类型">
          <select className={selectClass} value={form.task_type} onChange={(e) => setForm((f) => ({ ...f, task_type: e.target.value as TaskType }))}>
            <option value="shell">shell</option>
            <option value="http_get">http_get</option>
            <option value="icmp_ping">icmp_ping</option>
            <option value="tcp_ping">tcp_ping</option>
          </select>
        </Field>
      </div>
      <Field label="调度">
        <input className={inputClass} value={form.schedule} onChange={(e) => setForm((f) => ({ ...f, schedule: e.target.value }))} placeholder="填写 cron，留空则手动运行" />
      </Field>
      <Field label="命令">
        <textarea className={`${textareaClass} min-h-28`} value={form.command} onChange={(e) => setForm((f) => ({ ...f, command: e.target.value }))} />
      </Field>
      <Field label="载荷 JSON">
        <textarea className={`${textareaClass} min-h-24`} value={form.payload_json} onChange={(e) => setForm((f) => ({ ...f, payload_json: e.target.value }))} />
      </Field>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="覆盖模式">
          <select className={selectClass} value={form.cover_mode} onChange={(e) => setForm((f) => ({ ...f, cover_mode: e.target.value as CoverMode }))}>
            <option value="specific">specific</option>
            <option value="all">all</option>
            <option value="any">any</option>
          </select>
        </Field>
        <Field label="服务器">
          <select
            multiple
            className={`${selectClass} min-h-32`}
            value={form.selected_server_ids}
            onChange={(e) => setForm((f) => ({ ...f, selected_server_ids: Array.from(e.target.selectedOptions).map((option) => option.value) }))}
          >
            {servers.map((server) => (
              <option key={server.id} value={server.id}>{server.name} ({server.status})</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="服务器分组">
          <select
            multiple
            className={`${selectClass} min-h-28`}
            value={form.selected_group_ids}
            onChange={(e) => setForm((f) => ({ ...f, selected_group_ids: Array.from(e.target.selectedOptions).map((option) => option.value) }))}
          >
            {serverGroups.map((group) => (
              <option key={group.id} value={group.id}>{group.name} ({group.server_ids.length})</option>
            ))}
          </select>
        </Field>
        <Field label="排除服务器">
          <select
            multiple
            className={`${selectClass} min-h-28`}
            value={form.excluded_server_ids}
            onChange={(e) => setForm((f) => ({ ...f, excluded_server_ids: Array.from(e.target.selectedOptions).map((option) => option.value) }))}
          >
            {servers.map((server) => (
              <option key={server.id} value={server.id}>{server.name} ({server.status})</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="标签条件">
          <input
            className={inputClass}
            value={form.tag_names.join(", ")}
            onChange={(e) => setForm((f) => ({ ...f, tag_names: splitTags(e.target.value) }))}
            placeholder="prod, cn2, edge"
          />
        </Field>
        <Field label="通知组">
          <select className={selectClass} value={form.notification_group_id} onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))}>
            <option value="">不通知</option>
            {notificationGroups.map((group) => (
              <option key={group.id} value={group.id}>{group.name}</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="flex flex-wrap gap-4 text-sm font-black">
        <label><input type="checkbox" checked={form.source_server} onChange={(e) => setForm((f) => ({ ...f, source_server: e.target.checked }))} /> 触发来源服务器</label>
        <label><input type="checkbox" checked={form.push_successful} onChange={(e) => setForm((f) => ({ ...f, push_successful: e.target.checked }))} /> 推送成功结果</label>
        <label><input type="checkbox" checked={form.enabled} onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))} /> 启用</label>
      </div>
      <button disabled={saving} className={buttonClass("primary")}>{saving ? "保存中..." : "保存任务"}</button>
    </form>
  );
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
      server_ids?: string[];
      exclude_server_ids?: string[];
      group_ids?: string[];
      tag_names?: string[];
      source_server?: boolean;
      tags?: Record<string, string>;
    };
    const mapTags = parsed.tags
      ? Object.entries(parsed.tags).map(([key, value]) => value || key)
      : [];
    return {
      server_ids: Array.isArray(parsed.server_ids) ? parsed.server_ids : [],
      exclude_server_ids: Array.isArray(parsed.exclude_server_ids) ? parsed.exclude_server_ids : [],
      group_ids: Array.isArray(parsed.group_ids) ? parsed.group_ids : [],
      tag_names: splitTags([...(Array.isArray(parsed.tag_names) ? parsed.tag_names : []), ...mapTags].join(",")),
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

function resultLabel(value?: string | null): string {
  if (!value) return "从未";
  const labels: Record<string, string> = {
    success: "成功",
    failure: "失败",
    error: "错误",
    timeout: "超时",
  };
  return labels[value] || value;
}
