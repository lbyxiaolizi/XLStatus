"use client";

import Link from "next/link";
import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  PageShell,
  StatusBadge,
  buttonClass,
  compactId,
  formatDate,
  formatPercent,
  inputClass as inputStyles,
  responseError,
  textareaClass as textareaStyles,
} from "@/app/components/M7Primitives";
import { apiClient, type FileEntry, type JsonObject } from "@/lib/api";

interface ServerDetail {
  id: string;
  name: string;
  status: string;
  last_seen_at?: string | null;
  last_state?: Record<string, unknown> | null;
  last_info?: Record<string, unknown> | null;
}

interface PageProps {
  params: Promise<{ id: string }> | { id: string };
}

const blankConfig = {
  report_interval_seconds: "3",
  ip_report_interval_seconds: "60",
  disable_auto_update: false,
  disable_force_update: false,
  disable_command_execute: false,
  disable_nat: false,
  disable_send_query: false,
};

export default function ServerDetailPage({ params }: PageProps) {
  const [serverId, setServerId] = useState("");
  const [server, setServer] = useState<ServerDetail | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [path, setPath] = useState("/");
  const [selectedPath, setSelectedPath] = useState("");
  const [fileContent, setFileContent] = useState("");
  const [writePath, setWritePath] = useState("");
  const [writeContent, setWriteContent] = useState("");
  const [configForm, setConfigForm] = useState(blankConfig);
  const [updateForm, setUpdateForm] = useState({
    version: "",
    download_url: "",
    checksum: "",
  });
  const [loading, setLoading] = useState(true);
  const [filesLoading, setFilesLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    Promise.resolve(params).then((value) => setServerId(value.id));
  }, [params]);

  const loadServer = useCallback(async () => {
    if (!serverId) return;
    setLoading(true);
    setError(null);
    const response = await apiClient.getServer(serverId);
    setLoading(false);
    if (response.success && response.data) {
      setServer(response.data as unknown as ServerDetail);
    } else {
      setError(responseError(response));
    }
  }, [serverId]);

  const loadFiles = useCallback(
    async (nextPath = path) => {
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
    [path, serverId],
  );

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServer();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServer]);

  useEffect(() => {
    if (!serverId) return;
    const timeoutId = window.setTimeout(() => {
      void loadFiles("/");
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [serverId, loadFiles]);

  const state = server?.last_state ?? {};
  const info = server?.last_info ?? {};
  const statusTone =
    server?.status === "online"
      ? "green"
      : server?.status === "revoked"
        ? "yellow"
        : "red";
  const currentDir = useMemo(() => parentPath(path), [path]);

  async function readSelectedFile(nextPath: string) {
    setSelectedPath(nextPath);
    setFileContent("");
    const response = await apiClient.readServerFile(serverId, nextPath, "utf8");
    if (response.success && response.data) {
      setFileContent(response.data.content);
      setWritePath(nextPath);
      setWriteContent(response.data.content);
      setNotice(`Read ${response.data.bytes} bytes from ${nextPath}.`);
    } else {
      setError(responseError(response));
    }
  }

  async function writeFile(event: FormEvent) {
    event.preventDefault();
    if (!writePath.trim()) {
      setError("Write path is required.");
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
      setNotice(`Wrote ${writePath.trim()}.`);
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteEntry(entryPath: string, recursive: boolean) {
    if (!confirm(`Delete ${entryPath}?`)) {
      return;
    }
    const response = await apiClient.deleteServerFile(serverId, {
      path: entryPath,
      recursive,
    });
    if (response.success) {
      setNotice(`Deleted ${entryPath}.`);
      await loadFiles(path);
    } else {
      setError(responseError(response));
    }
  }

  async function createTempUrl(kind: "download" | "upload") {
    const target = kind === "download" ? selectedPath || writePath : writePath;
    if (!target.trim()) {
      setError("Select or enter a file path first.");
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
    const response = await apiClient.applyServerConfig(serverId, config);
    setSaving(false);
    if (response.success) {
      setNotice("Config patch sent to agent.");
      await loadServer();
    } else {
      setError(responseError(response));
    }
  }

  async function forceUpdate(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    const response = await apiClient.forceUpdateServer(serverId, {
      version: updateForm.version.trim(),
      download_url: updateForm.download_url.trim(),
      checksum: updateForm.checksum.trim() || null,
    });
    setSaving(false);
    if (response.success) {
      setNotice("Force update request sent.");
      setUpdateForm({ version: "", download_url: "", checksum: "" });
    } else {
      setError(responseError(response));
    }
  }

  if (loading && !server) {
    return (
      <div className="min-h-screen bg-gray-100">
        <Navigation />
        <PageShell>
          <EmptyState title="Loading server" />
        </PageShell>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <Link href="/servers" className="text-sm text-blue-700 hover:underline">
              Back to servers
            </Link>
            <div className="mt-2 flex flex-wrap items-center gap-3">
              <h1 className="text-2xl font-bold text-gray-900">
                {server?.name || compactId(serverId)}
              </h1>
              {server ? <StatusBadge tone={statusTone}>{server.status}</StatusBadge> : null}
            </div>
            <p className="mt-1 text-sm text-gray-500">
              {serverId} · last seen {formatDate(server?.last_seen_at)}
            </p>
          </div>
          <button type="button" onClick={() => void loadServer()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
        </div>

        <div className="mb-6 grid gap-4 md:grid-cols-4">
          <Metric label="CPU" value={formatPercent(toNumber(state.cpu_percent))} />
          <Metric label="Memory" value={memoryValue(state)} />
          <Metric label="Load" value={formatNumber(state.load_1)} />
          <Metric label="Host" value={String(info.hostname ?? info.os ?? "N/A")} />
        </div>

        <div className="grid gap-6 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
          <section className="rounded-lg bg-white p-5 shadow">
            <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
              <Field label="Remote Path">
                <input
                  className={inputStyles}
                  value={path}
                  onChange={(event) => setPath(event.target.value)}
                />
              </Field>
              <div className="flex gap-2">
                <button type="button" onClick={() => void loadFiles(path)} className={buttonClass("primary")}>
                  Open
                </button>
                <button type="button" onClick={() => void loadFiles(currentDir)} className={buttonClass()}>
                  Up
                </button>
              </div>
            </div>

            {filesLoading ? (
              <div className="py-8 text-sm text-gray-500">Loading files...</div>
            ) : files.length === 0 ? (
              <EmptyState title="No files loaded" detail="Open an online agent directory to browse files." />
            ) : (
              <div className="overflow-x-auto">
                <table className="min-w-full divide-y divide-gray-200 text-sm">
                  <thead className="bg-gray-50 text-left text-xs uppercase text-gray-500">
                    <tr>
                      <th className="px-4 py-3">Name</th>
                      <th className="px-4 py-3">Type</th>
                      <th className="px-4 py-3">Size</th>
                      <th className="px-4 py-3">Mode</th>
                      <th className="px-4 py-3">Modified</th>
                      <th className="px-4 py-3">Actions</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100">
                    {files.map((entry) => {
                      const entryPath = joinPath(path, entry.name);
                      return (
                        <tr key={`${entry.file_type}:${entry.name}`} className="hover:bg-gray-50">
                          <td className="px-4 py-3 font-medium text-gray-900">{entry.name}</td>
                          <td className="px-4 py-3 text-gray-600">{entry.file_type}</td>
                          <td className="px-4 py-3 text-gray-600">{formatBytes(entry.size)}</td>
                          <td className="px-4 py-3 text-gray-600">{entry.mode.toString(8)}</td>
                          <td className="px-4 py-3 text-gray-600">{formatUnix(entry.modified_at)}</td>
                          <td className="px-4 py-3">
                            <div className="flex flex-wrap gap-2">
                              {entry.file_type === "dir" ? (
                                <button type="button" className="text-blue-700 hover:underline" onClick={() => void loadFiles(entryPath)}>
                                  Open
                                </button>
                              ) : (
                                <button type="button" className="text-blue-700 hover:underline" onClick={() => void readSelectedFile(entryPath)}>
                                  Read
                                </button>
                              )}
                              <button type="button" className="text-red-700 hover:underline" onClick={() => void deleteEntry(entryPath, entry.file_type === "dir")}>
                                Delete
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

            <form onSubmit={writeFile} className="mt-6 space-y-3 border-t border-gray-200 pt-5">
              <Field label="File Path">
                <input className={inputStyles} value={writePath} onChange={(event) => setWritePath(event.target.value)} placeholder="/tmp/xlstatus.txt" />
              </Field>
              <Field label="Content">
                <textarea className={textareaStyles} rows={8} value={writeContent} onChange={(event) => setWriteContent(event.target.value)} />
              </Field>
              <div className="flex flex-wrap gap-2">
                <button type="submit" disabled={saving} className={buttonClass("primary")}>
                  Write File
                </button>
                <button type="button" onClick={() => void createTempUrl("download")} className={buttonClass()}>
                  Download URL
                </button>
                <button type="button" onClick={() => void createTempUrl("upload")} className={buttonClass()}>
                  Upload URL
                </button>
              </div>
            </form>
          </section>

          <aside className="space-y-6">
            <section className="rounded-lg bg-white p-5 shadow">
              <h2 className="text-base font-semibold text-gray-900">File Preview</h2>
              <p className="mt-1 text-xs text-gray-500">{selectedPath || "No file selected"}</p>
              <pre className="mt-4 max-h-72 overflow-auto rounded bg-gray-950 p-3 text-xs text-gray-50">
                {fileContent || "Read a file to preview it here."}
              </pre>
            </section>

            <section className="rounded-lg bg-white p-5 shadow">
              <h2 className="text-base font-semibold text-gray-900">Agent Config</h2>
              <form onSubmit={applyConfig} className="mt-4 space-y-3">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Report Seconds">
                    <input className={inputStyles} value={configForm.report_interval_seconds} onChange={(event) => setConfigForm((prev) => ({ ...prev, report_interval_seconds: event.target.value }))} />
                  </Field>
                  <Field label="IP Report Seconds">
                    <input className={inputStyles} value={configForm.ip_report_interval_seconds} onChange={(event) => setConfigForm((prev) => ({ ...prev, ip_report_interval_seconds: event.target.value }))} />
                  </Field>
                </div>
                <div className="grid gap-2 text-sm text-gray-700">
                  {([
                    ["disable_command_execute", "Disable command/file/terminal"],
                    ["disable_force_update", "Disable force update"],
                    ["disable_nat", "Disable NAT"],
                    ["disable_send_query", "Disable probes"],
                    ["disable_auto_update", "Disable auto update"],
                  ] as const).map(([key, label]) => (
                    <label key={key} className="flex items-center gap-2">
                      <input
                        type="checkbox"
                        checked={Boolean(configForm[key])}
                        onChange={(event) =>
                          setConfigForm((prev) => ({ ...prev, [key]: event.target.checked }))
                        }
                      />
                      {label}
                    </label>
                  ))}
                </div>
                <button type="submit" disabled={saving} className={buttonClass("primary")}>
                  Apply Config
                </button>
              </form>
            </section>

            <section className="rounded-lg bg-white p-5 shadow">
              <h2 className="text-base font-semibold text-gray-900">Force Update</h2>
              <form onSubmit={forceUpdate} className="mt-4 space-y-3">
                <Field label="Version">
                  <input className={inputStyles} value={updateForm.version} onChange={(event) => setUpdateForm((prev) => ({ ...prev, version: event.target.value }))} />
                </Field>
                <Field label="Download URL">
                  <input className={inputStyles} value={updateForm.download_url} onChange={(event) => setUpdateForm((prev) => ({ ...prev, download_url: event.target.value }))} />
                </Field>
                <Field label="Checksum">
                  <input className={inputStyles} value={updateForm.checksum} onChange={(event) => setUpdateForm((prev) => ({ ...prev, checksum: event.target.value }))} />
                </Field>
                <button type="submit" disabled={saving} className={buttonClass("danger")}>
                  Send Update
                </button>
              </form>
            </section>
          </aside>
        </div>
      </PageShell>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg bg-white p-4 shadow">
      <div className="text-xs uppercase text-gray-500">{label}</div>
      <div className="mt-2 truncate text-lg font-semibold text-gray-900">{value}</div>
    </div>
  );
}

function joinPath(base: string, name: string): string {
  const root = base.endsWith("/") ? base.slice(0, -1) : base;
  return `${root || ""}/${name}`.replace(/\/+/g, "/");
}

function parentPath(value: string): string {
  const normalized = value.replace(/\/+$/g, "") || "/";
  if (normalized === "/") return "/";
  const idx = normalized.lastIndexOf("/");
  return idx <= 0 ? "/" : normalized.slice(0, idx);
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value)) return "N/A";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB`;
}

function formatUnix(value: number): string {
  if (!value) return "N/A";
  return new Date(value * 1000).toLocaleString();
}

function toNumber(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

function formatNumber(value: unknown): string {
  return typeof value === "number" ? value.toFixed(2) : "N/A";
}

function memoryValue(state: Record<string, unknown>): string {
  const used = toNumber(state.memory_used);
  const total = toNumber(state.memory_total);
  if (!used || !total) return "N/A";
  return `${((used / total) * 100).toFixed(1)}%`;
}
