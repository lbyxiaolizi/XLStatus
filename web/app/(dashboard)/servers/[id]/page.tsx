"use client";

import Link from "next/link";
import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
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
  const [updateForm, setUpdateForm] = useState({ version: "", download_url: "", checksum: "" });
  const [loading, setLoading] = useState(true);
  const [filesLoading, setFilesLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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
      setServer(response.data as unknown as ServerDetail);
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

  const statusTone = server?.status === "online" ? "green" : server?.status === "revoked" ? "yellow" : "red";
  const currentDir = useMemo(() => parentPath(path), [path]);

  async function readSelectedFile(nextPath: string) {
    setSelectedPath(nextPath);
    setFileContent("");
    const response = await apiClient.readServerFile(serverId, nextPath, "utf8");
    if (response.success && response.data) {
      setFileContent(response.data.content);
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
      setNotice("配置补丁已发送到 Agent。");
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
      setNotice("强制更新请求已发送。");
      setUpdateForm({ version: "", download_url: "", checksum: "" });
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

        <div className="grid gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
          <BrutalCard>
            <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
              <div>
                <h2 className="text-xl font-black uppercase">文件</h2>
                <p className="text-sm font-bold text-[var(--text-muted)]">{path}</p>
              </div>
              <div className="flex flex-wrap gap-2">
                <button className={buttonClass("secondary")} onClick={() => void loadFiles(currentDir)}>上级</button>
                <button className={buttonClass("secondary")} onClick={() => void loadFiles(path)}>刷新</button>
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
                              <button className={buttonClass("secondary")} onClick={() => void loadFiles(nextPath)}>打开</button>
                            ) : (
                              <button className={buttonClass("primary")} onClick={() => void readSelectedFile(nextPath)}>读取</button>
                            )}
                            <button className={buttonClass("danger")} onClick={() => void deleteEntry(nextPath, entry.file_type === "dir")}>删除</button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </BrutalCard>

          <div className="grid gap-6">
            <BrutalCard accent>
              <h2 className="mb-4 text-xl font-black uppercase">写入文件</h2>
              <form onSubmit={writeFile} className="space-y-4">
                <Field label="路径">
                  <input className={inputClass} value={writePath} onChange={(event) => setWritePath(event.target.value)} placeholder="/tmp/xlstatus.txt" />
                </Field>
                <Field label="内容">
                  <textarea className={`${textareaClass} min-h-40`} value={writeContent || fileContent} onChange={(event) => setWriteContent(event.target.value)} />
                </Field>
                <div className="flex flex-wrap gap-2">
                  <button disabled={saving} className={buttonClass("primary")}>写入文件</button>
                  <button type="button" className={buttonClass("secondary")} onClick={() => void createTempUrl("download")}>下载 URL</button>
                  <button type="button" className={buttonClass("secondary")} onClick={() => void createTempUrl("upload")}>上传 URL</button>
                </div>
              </form>
            </BrutalCard>

            <BrutalCard>
              <h2 className="mb-4 text-xl font-black uppercase">应用配置</h2>
              <form onSubmit={applyConfig} className="space-y-4">
                <div className="grid gap-3 sm:grid-cols-2">
                  <Field label="上报间隔">
                    <input className={inputClass} value={configForm.report_interval_seconds} onChange={(e) => setConfigForm((f) => ({ ...f, report_interval_seconds: e.target.value }))} />
                  </Field>
                  <Field label="IP 上报间隔">
                    <input className={inputClass} value={configForm.ip_report_interval_seconds} onChange={(e) => setConfigForm((f) => ({ ...f, ip_report_interval_seconds: e.target.value }))} />
                  </Field>
                </div>
                <div className="grid gap-2">
                  {(["disable_auto_update", "disable_force_update", "disable_command_execute", "disable_nat", "disable_send_query"] as const).map((key) => (
                    <label key={key} className="flex items-center gap-2 text-sm font-black">
                      <input type="checkbox" checked={configForm[key]} onChange={(e) => setConfigForm((f) => ({ ...f, [key]: e.target.checked }))} />
                      {configFlagLabel(key)}
                    </label>
                  ))}
                </div>
                <button disabled={saving} className={buttonClass("primary")}>应用配置</button>
              </form>
            </BrutalCard>

            <BrutalCard>
              <h2 className="mb-4 text-xl font-black uppercase">发送更新</h2>
              <form onSubmit={forceUpdate} className="space-y-4">
                <Field label="版本">
                  <input className={inputClass} value={updateForm.version} onChange={(e) => setUpdateForm((f) => ({ ...f, version: e.target.value }))} />
                </Field>
                <Field label="下载 URL">
                  <input className={inputClass} value={updateForm.download_url} onChange={(e) => setUpdateForm((f) => ({ ...f, download_url: e.target.value }))} />
                </Field>
                <Field label="校验和">
                  <input className={inputClass} value={updateForm.checksum} onChange={(e) => setUpdateForm((f) => ({ ...f, checksum: e.target.value }))} />
                </Field>
                <button disabled={saving} className={buttonClass("danger")}>发送更新</button>
              </form>
            </BrutalCard>
          </div>
        </div>
      </PageShell>
    </div>
  );
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
