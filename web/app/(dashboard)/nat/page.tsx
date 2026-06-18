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
import { apiClient, type JsonObject } from "@/lib/api";

interface Server {
  id: string;
  name: string;
  status: string;
}

interface NatMapping {
  id: string;
  agent_id: string;
  description?: string;
  protocol?: string;
  local_host?: string;
  local_port?: number;
  public_port?: number;
  enabled?: boolean;
}

export default function NatPage() {
  const [mappings, setMappings] = useState<NatMapping[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState(false);
  const [form, setForm] = useState({ agent_id: "", description: "", protocol: "tcp", local_host: "127.0.0.1", local_port: "80", public_port: "10080" });

  const load = useCallback(async () => {
    const [mappingResponse, serverResponse] = await Promise.all([apiClient.listNatMappings(), apiClient.listServers(200, 0)]);
    if (mappingResponse.success && mappingResponse.data) {
      setMappings((mappingResponse.data.mappings as NatMapping[]) ?? []);
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
    };
    const response = await apiClient.createNatMapping(payload);
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
    const response = await apiClient.deleteNatMapping(mapping.id);
    if (response.success) {
      setNotice("NAT 映射已删除。");
      await load();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="网络"
          title="NAT"
          detail="Agent 端口映射与远程访问配置。"
          actions={<button className={buttonClass("primary")} onClick={() => setModal(true)}>新增映射</button>}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {mappings.length === 0 ? (
          <EmptyState title="暂无 NAT 映射" detail="创建映射后即可通过隧道暴露 Agent 侧目标。" />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr><th className={thClass}>描述</th><th className={thClass}>Agent</th><th className={thClass}>公网</th><th className={thClass}>本地</th><th className={thClass}>状态</th><th className={thClass}>操作</th></tr>
              </thead>
              <tbody>
                {mappings.map((mapping) => (
                  <tr key={mapping.id}>
                    <td className={tdClass}>{mapping.description || mapping.id}</td>
                    <td className={tdClass}>{mapping.agent_id}</td>
                    <td className={tdClass}>{mapping.protocol || "tcp"}://:{mapping.public_port}</td>
                    <td className={tdClass}>{mapping.local_host}:{mapping.local_port}</td>
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
                <Field label="本地主机"><input className={inputClass} value={form.local_host} onChange={(e) => setForm((f) => ({ ...f, local_host: e.target.value }))} /></Field>
              </div>
              <Field label="本地端口"><input className={inputClass} value={form.local_port} onChange={(e) => setForm((f) => ({ ...f, local_port: e.target.value }))} /></Field>
              <button className={buttonClass("primary")}>保存映射</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}
