"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  Modal,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type NatMapping, type TotpStatusResponse } from "@/lib/api";

interface Server {
  id: string;
  name: string;
  status: string;
}

export default function NatPage() {
  const [mappings, setMappings] = useState<NatMapping[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [modal, setModal] = useState(false);
  const [form, setForm] = useState({ agent_id: "", description: "", protocol: "tcp", local_host: "127.0.0.1", local_port: "80", public_port: "10080", allowed_sources: "", max_active_tunnels: "", idle_timeout_seconds: "", max_bytes_per_tunnel: "", max_bandwidth_bytes_per_second: "", rate_limit_window_seconds: "", max_connections_per_window: "", max_bytes_per_window: "" });

  const load = useCallback(async () => {
    const [mappingResponse, serverResponse] = await Promise.all([apiClient.listNatMappings(), apiClient.listServers(200, 0)]);
    if (mappingResponse.success && mappingResponse.data) {
      setMappings(mappingResponse.data.mappings ?? []);
    } else {
      setError(responseError(mappingResponse));
    }
    if (serverResponse.success && serverResponse.data) {
      const loaded = (serverResponse.data.servers as Server[]) ?? [];
      setServers(loaded);
      setForm((current) => ({ ...current, agent_id: current.agent_id || loaded[0]?.id || "" }));
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void load();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [load]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const payload: JsonObject = {
      agent_id: form.agent_id,
      protocol: form.protocol,
      local_host: form.local_host,
      local_port: Number(form.local_port),
      public_port: Number(form.public_port),
      description: form.description.trim() || null,
      allowed_sources: form.allowed_sources.trim() || null,
      max_active_tunnels: form.max_active_tunnels.trim() ? Number(form.max_active_tunnels) : null,
      idle_timeout_seconds: form.idle_timeout_seconds.trim() ? Number(form.idle_timeout_seconds) : null,
      max_bytes_per_tunnel: form.max_bytes_per_tunnel.trim() ? Number(form.max_bytes_per_tunnel) : null,
      max_bandwidth_bytes_per_second: form.max_bandwidth_bytes_per_second.trim() ? Number(form.max_bandwidth_bytes_per_second) : null,
      rate_limit_window_seconds: form.rate_limit_window_seconds.trim() ? Number(form.rate_limit_window_seconds) : null,
      max_connections_per_window: form.max_connections_per_window.trim() ? Number(form.max_connections_per_window) : null,
      max_bytes_per_window: form.max_bytes_per_window.trim() ? Number(form.max_bytes_per_window) : null,
    };
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createNatMapping(payload, totpCode);
    if (response.success) {
      setNotice("NAT 映射已创建。");
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteMapping(mapping: NatMapping) {
    if (!confirm(`确定删除 NAT 映射「${mapping.description || mapping.id}」？`)) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNatMapping(mapping.id, totpCode);
    if (response.success) {
      setNotice("NAT 映射已删除。");
      await load();
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
    const code = window.prompt("请输入 6 位 TOTP 验证码");
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError("请输入 6 位 TOTP 验证码。");
      return null;
    }
    return trimmed;
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="网络"
          title="NAT"
          detail="Agent loopback 端口映射与远程访问配置。"
          actions={<button className={buttonClass("primary")} onClick={() => setModal(true)}>新增映射</button>}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {mappings.length === 0 ? (
          <EmptyState title="暂无 NAT 映射" detail="创建映射后即可通过隧道暴露 Agent 本机 loopback 目标。" />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr><th className={thClass}>描述</th><th className={thClass}>Agent</th><th className={thClass}>公网</th><th className={thClass}>本地</th><th className={thClass}>策略</th><th className={thClass}>状态</th><th className={thClass}>操作</th></tr>
              </thead>
              <tbody>
                {mappings.map((mapping) => (
                  <tr key={mapping.id}>
                    <td className={tdClass}>{mapping.description || mapping.id}</td>
                    <td className={tdClass}>{mapping.agent_id}</td>
                    <td className={tdClass}>{mapping.protocol || "tcp"}://:{mapping.public_port}</td>
                    <td className={tdClass}>{mapping.local_host}:{mapping.local_port}</td>
                    <td className={tdClass}>
                      <div className="space-y-1 text-xs font-bold">
                        <div className="break-all">来源：{mapping.allowed_sources || "全局策略"}</div>
                        <div>并发：{mapping.max_active_tunnels ?? "全局上限"}</div>
                        <div>空闲：{mapping.idle_timeout_seconds ? `${mapping.idle_timeout_seconds}s` : "未限制"}</div>
                        <div>流量：{mapping.max_bytes_per_tunnel ? `${mapping.max_bytes_per_tunnel} bytes` : "未限制"}</div>
                        <div>带宽：{mapping.max_bandwidth_bytes_per_second ? `${mapping.max_bandwidth_bytes_per_second} B/s` : "未限制"}</div>
                        <div>窗口：{mapping.rate_limit_window_seconds ? `${mapping.rate_limit_window_seconds}s` : "默认/未启用"}</div>
                        <div>窗口连接：{mapping.max_connections_per_window ?? "未限制"}</div>
                        <div>窗口流量：{mapping.max_bytes_per_window ? `${mapping.max_bytes_per_window} bytes` : "未限制"}</div>
                      </div>
                    </td>
                    <td className={tdClass}><StatusBadge tone={mapping.enabled === false ? "gray" : "green"}>{mapping.enabled === false ? "停用" : "启用"}</StatusBadge></td>
                    <td className={tdClass}><button className={buttonClass("danger")} onClick={() => void deleteMapping(mapping)}>删除</button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal ? (
          <Modal title="新增 NAT 映射" onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Agent"><select className={selectClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}>{servers.map((server) => <option key={server.id} value={server.id}>{server.name}</option>)}</select></Field>
                <Field label="描述"><input className={inputClass} value={form.description} onChange={(e) => setForm((f) => ({ ...f, description: e.target.value }))} /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label="协议"><select className={selectClass} value={form.protocol} onChange={(e) => setForm((f) => ({ ...f, protocol: e.target.value }))}><option value="tcp">tcp</option><option value="udp">udp</option></select></Field>
                <Field label="公网端口"><input className={inputClass} value={form.public_port} onChange={(e) => setForm((f) => ({ ...f, public_port: e.target.value }))} /></Field>
                <Field label="本地主机"><input className={inputClass} value={form.local_host} onChange={(e) => setForm((f) => ({ ...f, local_host: e.target.value }))} placeholder="127.0.0.1" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label="本地端口"><input className={inputClass} value={form.local_port} onChange={(e) => setForm((f) => ({ ...f, local_port: e.target.value }))} /></Field>
                <Field label="来源 CIDR"><input className={inputClass} value={form.allowed_sources} onChange={(e) => setForm((f) => ({ ...f, allowed_sources: e.target.value }))} placeholder="203.0.113.0/24" /></Field>
                <Field label="最大并发"><input className={inputClass} value={form.max_active_tunnels} onChange={(e) => setForm((f) => ({ ...f, max_active_tunnels: e.target.value }))} placeholder="2" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label="空闲超时秒"><input className={inputClass} value={form.idle_timeout_seconds} onChange={(e) => setForm((f) => ({ ...f, idle_timeout_seconds: e.target.value }))} placeholder="300" /></Field>
                <Field label="每隧道字节上限"><input className={inputClass} value={form.max_bytes_per_tunnel} onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_tunnel: e.target.value }))} placeholder="104857600" /></Field>
                <Field label="带宽 B/s"><input className={inputClass} value={form.max_bandwidth_bytes_per_second} onChange={(e) => setForm((f) => ({ ...f, max_bandwidth_bytes_per_second: e.target.value }))} placeholder="1048576" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label="窗口秒"><input className={inputClass} value={form.rate_limit_window_seconds} onChange={(e) => setForm((f) => ({ ...f, rate_limit_window_seconds: e.target.value }))} placeholder="60" /></Field>
                <Field label="窗口最大连接"><input className={inputClass} value={form.max_connections_per_window} onChange={(e) => setForm((f) => ({ ...f, max_connections_per_window: e.target.value }))} placeholder="30" /></Field>
                <Field label="窗口最大字节"><input className={inputClass} value={form.max_bytes_per_window} onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_window: e.target.value }))} placeholder="104857600" /></Field>
              </div>
              <button className={buttonClass("primary")}>保存映射</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}
