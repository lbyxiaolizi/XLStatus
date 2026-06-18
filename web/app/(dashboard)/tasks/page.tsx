"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  Modal,
  PageShell,
  StatusBadge,
  buttonClass,
  compactId,
  formatDate,
  formatMs,
  inputClass,
  responseError,
  selectClass,
  textareaClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

type TaskType = "shell" | "http_get" | "icmp_ping" | "tcp_ping";
type CoverMode = "all" | "any" | "specific";

interface Server {
  id: string;
  name: string;
  status: string;
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
  created_at?: string;
}

interface TaskRun {
  id: string;
  task_id: string;
  server_id: string;
  status: string;
  delay_ms?: number | null;
  output?: string | null;
  output_truncated?: boolean;
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
  push_successful: boolean;
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
  push_successful: false,
  enabled: true,
};

export default function TasksPage() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [runs, setRuns] = useState<TaskRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "edit" | "runs" | null>(null);
  const [editing, setEditing] = useState<Task | null>(null);
  const [form, setForm] = useState<TaskForm>(blankForm);
  const [formErrors, setFormErrors] = useState<Partial<Record<keyof TaskForm, string>>>({});
  const [query, setQuery] = useState("");

  const loadTasks = useCallback(async () => {
    setError(null);
    try {
      setLoading(true);
      const [tasksResponse, serversResponse] = await Promise.all([
        apiClient.listTasks(),
        apiClient.listServers(200, 0),
      ]);

      if (tasksResponse.success && tasksResponse.data) {
        setTasks((tasksResponse.data.tasks as Task[]) ?? []);
      } else {
        setError(responseError(tasksResponse));
      }

      if (serversResponse.success && serversResponse.data) {
        setServers((serversResponse.data.servers as Server[]) ?? []);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadTasks();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadTasks]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return tasks;
    }
    return tasks.filter((task) =>
      [task.name, task.task_type, task.schedule, task.last_result]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [query, tasks]);

  function openCreate() {
    setEditing(null);
    setForm(blankForm);
    setFormErrors({});
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
      push_successful: Boolean(task.push_successful),
      enabled: task.enabled,
    });
    setFormErrors({});
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
    setNotice(null);
    const validation = validateTaskForm(form);
    setFormErrors(validation);
    if (Object.keys(validation).length > 0) {
      return;
    }

    setSaving(true);
    const payload = formToPayload(form, modal === "edit");
    const response =
      modal === "edit" && editing
        ? await apiClient.updateTask(editing.id, payload)
        : await apiClient.createTask(payload);
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice(modal === "edit" ? "Task updated." : "Task created.");
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  async function runTask(task: Task) {
    if (task.task_type !== "shell") {
      setError("The current backend can dispatch shell tasks only.");
      return;
    }
    if (!confirm(`Run task "${task.name}" now?`)) {
      return;
    }

    setRunningTaskId(task.id);
    setNotice(null);
    const response = await apiClient.runTask(task.id);
    setRunningTaskId(null);

    if (response.success && response.data) {
      const summary = response.data.summary as
        | { success?: number; failure?: number; offline?: number; timeout?: number; total?: number }
        | undefined;
      setNotice(
        summary
          ? `Run queued: ${summary.success ?? 0} success, ${summary.failure ?? 0} failure, ${summary.offline ?? 0} offline, ${summary.timeout ?? 0} timeout.`
          : "Task run completed.",
      );
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteTask(task: Task) {
    if (!confirm(`Delete task "${task.name}" and its saved run history?`)) {
      return;
    }

    const response = await apiClient.deleteTask(task.id);
    if (response.success) {
      setNotice("Task deleted.");
      await loadTasks();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">Tasks</h1>
            <p className="mt-1 text-sm text-gray-500">Scheduled and manual operations.</p>
          </div>
          <button type="button" onClick={openCreate} className={buttonClass("primary")}>
            Create Task
          </button>
        </div>

        <div className="mb-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search tasks"
            className={inputClass}
          />
          <button type="button" onClick={() => void loadTasks()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
          {servers.length === 0 ? (
            <InlineNotice tone="yellow">No accessible servers were returned. Specific task runs need at least one target server.</InlineNotice>
          ) : null}
        </div>

        {loading ? (
          <div className="rounded bg-white p-6 text-sm text-gray-600 shadow">Loading tasks...</div>
        ) : filtered.length === 0 ? (
          <EmptyState title="No tasks configured" detail="Create a shell task to run commands on selected servers." />
        ) : (
          <>
            <div className="hidden overflow-hidden rounded-lg bg-white shadow md:block">
              <table className="min-w-full divide-y divide-gray-200">
                <thead className="bg-gray-50">
                  <tr>
                    {["Name", "Type", "Schedule", "Targets", "State", "Last Result", "Actions"].map((heading) => (
                      <th key={heading} className="px-6 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                        {heading}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 bg-white">
                  {filtered.map((task) => (
                    <tr key={task.id} className="hover:bg-gray-50">
                      <td className="px-6 py-4">
                        <div className="text-sm font-medium text-gray-900">{task.name}</div>
                        <div className="text-xs text-gray-500">{task.id}</div>
                      </td>
                      <td className="px-6 py-4 text-sm text-gray-500">{taskTypeLabel(task.task_type)}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{task.schedule || "Manual"}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{targetLabel(task, servers)}</td>
                      <td className="px-6 py-4">{task.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}</td>
                      <td className="px-6 py-4">{resultBadge(task.last_result)}</td>
                      <td className="px-6 py-4 text-sm">
                        <div className="flex flex-wrap gap-2">
                          <button type="button" onClick={() => void runTask(task)} disabled={runningTaskId === task.id} className="text-green-700 hover:underline disabled:text-gray-400">
                            {runningTaskId === task.id ? "Running" : "Run"}
                          </button>
                          <button type="button" onClick={() => void openRuns(task)} className="text-gray-700 hover:underline">
                            Runs
                          </button>
                          <button type="button" onClick={() => openEdit(task)} className="text-blue-700 hover:underline">
                            Edit
                          </button>
                          <button type="button" onClick={() => void deleteTask(task)} className="text-red-700 hover:underline">
                            Delete
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <div className="grid gap-3 md:hidden">
              {filtered.map((task) => (
                <div key={task.id} className="rounded-lg bg-white p-4 shadow">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h2 className="font-semibold text-gray-900">{task.name}</h2>
                      <p className="mt-1 text-sm text-gray-500">{taskTypeLabel(task.task_type)} · {task.schedule || "Manual"}</p>
                    </div>
                    {resultBadge(task.last_result)}
                  </div>
                  <div className="mt-3 text-sm text-gray-600">{targetLabel(task, servers)}</div>
                  <div className="mt-4 grid grid-cols-2 gap-2">
                    <button type="button" onClick={() => void runTask(task)} disabled={runningTaskId === task.id} className={buttonClass()}>
                      {runningTaskId === task.id ? "Running" : "Run"}
                    </button>
                    <button type="button" onClick={() => void openRuns(task)} className={buttonClass()}>
                      Runs
                    </button>
                    <button type="button" onClick={() => openEdit(task)} className={buttonClass()}>
                      Edit
                    </button>
                    <button type="button" onClick={() => void deleteTask(task)} className={buttonClass("danger")}>
                      Delete
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </PageShell>

      {modal === "create" || modal === "edit" ? (
        <Modal title={modal === "edit" ? "Edit Task" : "Create Task"} onClose={() => setModal(null)}>
          <form onSubmit={submitForm} className="space-y-4">
            <Field label="Name" error={formErrors.name}>
              <input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} className={inputClass} />
            </Field>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Task Type" error={formErrors.task_type}>
                <select value={form.task_type} onChange={(event) => setForm((prev) => ({ ...prev, task_type: event.target.value as TaskType }))} className={selectClass}>
                  <option value="shell">Shell</option>
                  <option value="http_get">HTTP GET</option>
                  <option value="icmp_ping">ICMP Ping</option>
                  <option value="tcp_ping">TCP Ping</option>
                </select>
              </Field>
              <Field label="Schedule">
                <input
                  value={form.schedule}
                  onChange={(event) => setForm((prev) => ({ ...prev, schedule: event.target.value }))}
                  className={inputClass}
                  placeholder="Optional cron expression"
                />
              </Field>
            </div>

            <Field label={form.task_type === "shell" ? "Command" : "Payload JSON"} error={form.task_type === "shell" ? formErrors.command : formErrors.payload_json}>
              {form.task_type === "shell" ? (
                <textarea value={form.command} onChange={(event) => setForm((prev) => ({ ...prev, command: event.target.value }))} className={textareaClass} rows={4} />
              ) : (
                <textarea value={form.payload_json} onChange={(event) => setForm((prev) => ({ ...prev, payload_json: event.target.value }))} className={textareaClass} rows={4} />
              )}
            </Field>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Cover Mode" error={formErrors.cover_mode}>
                <select value={form.cover_mode} onChange={(event) => setForm((prev) => ({ ...prev, cover_mode: event.target.value as CoverMode }))} className={selectClass}>
                  <option value="specific">Specific servers</option>
                  <option value="all">All selector matches</option>
                  <option value="any">Any one match</option>
                </select>
              </Field>
              <Field label="Target Servers" error={formErrors.selected_server_ids}>
                <select
                  multiple
                  value={form.selected_server_ids}
                  onChange={(event) =>
                    setForm((prev) => ({
                      ...prev,
                      selected_server_ids: Array.from(event.target.selectedOptions).map((option) => option.value),
                    }))
                  }
                  className={`${selectClass} min-h-32`}
                >
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name} ({server.status})
                    </option>
                  ))}
                </select>
              </Field>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <label className="flex items-center gap-2 text-sm text-gray-700">
                <input
                  type="checkbox"
                  checked={form.enabled}
                  onChange={(event) => setForm((prev) => ({ ...prev, enabled: event.target.checked }))}
                  className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                Enabled
              </label>
              <label className="flex items-center gap-2 text-sm text-gray-700">
                <input
                  type="checkbox"
                  checked={form.push_successful}
                  onChange={(event) => setForm((prev) => ({ ...prev, push_successful: event.target.checked }))}
                  className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                Notify on success
              </label>
            </div>

            {form.task_type !== "shell" ? (
              <InlineNotice tone="yellow">This backend stores this task type, but manual dispatch currently supports shell tasks only.</InlineNotice>
            ) : null}

            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                Cancel
              </button>
              <button type="submit" disabled={saving} className={buttonClass("primary")}>
                {saving ? "Saving..." : modal === "edit" ? "Save Changes" : "Create Task"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {modal === "runs" && editing ? (
        <Modal title={`Runs: ${editing.name}`} onClose={() => setModal(null)}>
          {runs.length === 0 ? (
            <EmptyState title="No runs recorded" />
          ) : (
            <div className="space-y-3">
              {runs.map((run) => (
                <div key={run.id} className="rounded border border-gray-200 p-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="font-medium text-gray-900">{compactId(run.server_id)}</div>
                    {resultBadge(run.status)}
                  </div>
                  <div className="mt-2 grid gap-2 text-sm text-gray-600 sm:grid-cols-3">
                    <span>{formatDate(run.created_at)}</span>
                    <span>{formatMs(run.delay_ms)}</span>
                    <span>{run.output_truncated ? "Output truncated" : ""}</span>
                  </div>
                  {run.error ? <div className="mt-2 text-sm text-red-700">{run.error}</div> : null}
                  {run.output ? <pre className="mt-2 max-h-40 overflow-auto rounded bg-gray-50 p-2 text-xs text-gray-800">{run.output}</pre> : null}
                </div>
              ))}
            </div>
          )}
        </Modal>
      ) : null}
    </div>
  );
}

function validateTaskForm(form: TaskForm): Partial<Record<keyof TaskForm, string>> {
  const errors: Partial<Record<keyof TaskForm, string>> = {};
  if (!form.name.trim()) {
    errors.name = "Name is required";
  }
  if (form.task_type === "shell" && !form.command.trim()) {
    errors.command = "Command is required";
  }
  if (form.task_type !== "shell" && form.payload_json.trim()) {
    try {
      JSON.parse(form.payload_json);
    } catch {
      errors.payload_json = "Payload must be valid JSON";
    }
  }
  if (form.cover_mode === "specific" && form.selected_server_ids.length === 0) {
    errors.selected_server_ids = "Select at least one server";
  }
  return errors;
}

function formToPayload(form: TaskForm, includeEnabled: boolean): JsonObject {
  const selector = {
    server_ids: form.selected_server_ids,
    group_ids: [],
    tags: {},
  };
  const payload: JsonObject = {
    name: form.name.trim(),
    task_type: form.task_type,
    schedule: form.schedule.trim() || null,
    command: form.task_type === "shell" ? form.command.trim() : null,
    payload_json:
      form.task_type === "shell" ? null : form.payload_json.trim() || "{}",
    cover_mode: form.cover_mode,
    server_selector_json: JSON.stringify(selector),
    push_successful: form.push_successful,
    notification_group_id: null,
  };
  if (includeEnabled) {
    payload.enabled = form.enabled;
  }
  return payload;
}

function parseSelector(value: string): { server_ids: string[] } {
  try {
    const parsed = JSON.parse(value) as { server_ids?: unknown };
    return {
      server_ids: Array.isArray(parsed.server_ids)
        ? parsed.server_ids.filter((id): id is string => typeof id === "string")
        : [],
    };
  } catch {
    return { server_ids: [] };
  }
}

function targetLabel(task: Task, servers: Server[]): string {
  const selector = parseSelector(task.server_selector_json);
  if (selector.server_ids.length === 0) {
    return task.cover_mode === "specific" ? "No servers selected" : task.cover_mode;
  }
  const names = selector.server_ids.map((id) => servers.find((server) => server.id === id)?.name || compactId(id));
  return `${task.cover_mode}: ${names.join(", ")}`;
}

function resultBadge(result?: string | null) {
  if (!result) {
    return <StatusBadge tone="gray">Never Run</StatusBadge>;
  }
  if (result === "success") {
    return <StatusBadge tone="green">Success</StatusBadge>;
  }
  if (result === "failure") {
    return <StatusBadge tone="red">Failed</StatusBadge>;
  }
  if (result === "timeout" || result === "offline") {
    return <StatusBadge tone="yellow">{result}</StatusBadge>;
  }
  return <StatusBadge tone="blue">{result}</StatusBadge>;
}

function taskTypeLabel(type: string): string {
  return type.replace(/_/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}
