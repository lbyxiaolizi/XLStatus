"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
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
import { useDialogs } from "@/app/components/Dialogs";
import { useI18n } from "@/lib/use-i18n";

interface Server {
  id: string;
  name: string;
  status: string;
}

export default function NatPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
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
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
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
      setNotice(copy.natPage.mappingCreated);
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteMapping(mapping: NatMapping) {
    if (!(await dialogs.confirm({ message: copy.natPage.deleteMappingConfirm.replace("{name}", String(mapping.description || mapping.id)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNatMapping(mapping.id, totpCode);
    if (response.success) {
      setNotice(copy.natPage.mappingDeleted);
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
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.natPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.natPage.eyebrow}
          title={copy.natPage.title}
          detail={copy.natPage.detail}
          actions={<button className={buttonClass("primary")} onClick={() => setModal(true)}>{copy.natPage.addMapping}</button>}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {mappings.length === 0 ? (
          <EmptyState title={copy.natPage.emptyTitle} detail={copy.natPage.emptyDetail} />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr><th className={thClass}>{copy.natPage.colDescription}</th><th className={thClass}>{copy.natPage.colAgent}</th><th className={thClass}>{copy.natPage.colPublic}</th><th className={thClass}>{copy.natPage.colLocal}</th><th className={thClass}>{copy.natPage.colPolicy}</th><th className={thClass}>{copy.natPage.colStatus}</th><th className={thClass}>{copy.natPage.colActions}</th></tr>
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
                        <div className="break-all">{copy.natPage.policySource}{mapping.allowed_sources || copy.natPage.policyGlobalPolicy}</div>
                        <div>{copy.natPage.policyConcurrency}{mapping.max_active_tunnels ?? copy.natPage.policyGlobalLimit}</div>
                        <div>{copy.natPage.policyIdle}{mapping.idle_timeout_seconds ? `${mapping.idle_timeout_seconds}s` : copy.natPage.policyUnlimited}</div>
                        <div>{copy.natPage.policyTraffic}{mapping.max_bytes_per_tunnel ? `${mapping.max_bytes_per_tunnel} bytes` : copy.natPage.policyUnlimited}</div>
                        <div>{copy.natPage.policyBandwidth}{mapping.max_bandwidth_bytes_per_second ? `${mapping.max_bandwidth_bytes_per_second} B/s` : copy.natPage.policyUnlimited}</div>
                        <div>{copy.natPage.policyWindow}{mapping.rate_limit_window_seconds ? `${mapping.rate_limit_window_seconds}s` : copy.natPage.policyDefaultDisabled}</div>
                        <div>{copy.natPage.policyWindowConnections}{mapping.max_connections_per_window ?? copy.natPage.policyUnlimited}</div>
                        <div>{copy.natPage.policyWindowTraffic}{mapping.max_bytes_per_window ? `${mapping.max_bytes_per_window} bytes` : copy.natPage.policyUnlimited}</div>
                      </div>
                    </td>
                    <td className={tdClass}><StatusBadge tone={mapping.enabled === false ? "gray" : "green"}>{mapping.enabled === false ? copy.natPage.disabled : copy.natPage.enabled}</StatusBadge></td>
                    <td className={tdClass}><button className={buttonClass("danger")} onClick={() => void deleteMapping(mapping)}>{copy.natPage.delete}</button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal ? (
          <Modal title={copy.natPage.modalTitle} onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.natPage.fieldAgent}><select className={selectClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}>{servers.map((server) => <option key={server.id} value={server.id}>{server.name}</option>)}</select></Field>
                <Field label={copy.natPage.fieldDescription}><input className={inputClass} value={form.description} onChange={(e) => setForm((f) => ({ ...f, description: e.target.value }))} /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldProtocol}><select className={selectClass} value={form.protocol} onChange={(e) => setForm((f) => ({ ...f, protocol: e.target.value }))}><option value="tcp">tcp</option><option value="udp">udp</option></select></Field>
                <Field label={copy.natPage.fieldPublicPort}><input className={inputClass} value={form.public_port} onChange={(e) => setForm((f) => ({ ...f, public_port: e.target.value }))} /></Field>
                <Field label={copy.natPage.fieldLocalHost}><input className={inputClass} value={form.local_host} onChange={(e) => setForm((f) => ({ ...f, local_host: e.target.value }))} placeholder="127.0.0.1" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldLocalPort}><input className={inputClass} value={form.local_port} onChange={(e) => setForm((f) => ({ ...f, local_port: e.target.value }))} /></Field>
                <Field label={copy.natPage.fieldSourceCidr}><input className={inputClass} value={form.allowed_sources} onChange={(e) => setForm((f) => ({ ...f, allowed_sources: e.target.value }))} placeholder="203.0.113.0/24" /></Field>
                <Field label={copy.natPage.fieldMaxConcurrent}><input className={inputClass} value={form.max_active_tunnels} onChange={(e) => setForm((f) => ({ ...f, max_active_tunnels: e.target.value }))} placeholder="2" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldIdleTimeoutSeconds}><input className={inputClass} value={form.idle_timeout_seconds} onChange={(e) => setForm((f) => ({ ...f, idle_timeout_seconds: e.target.value }))} placeholder="300" /></Field>
                <Field label={copy.natPage.fieldMaxBytesPerTunnel}><input className={inputClass} value={form.max_bytes_per_tunnel} onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_tunnel: e.target.value }))} placeholder="104857600" /></Field>
                <Field label={copy.natPage.fieldBandwidth}><input className={inputClass} value={form.max_bandwidth_bytes_per_second} onChange={(e) => setForm((f) => ({ ...f, max_bandwidth_bytes_per_second: e.target.value }))} placeholder="1048576" /></Field>
              </div>
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.natPage.fieldWindowSeconds}><input className={inputClass} value={form.rate_limit_window_seconds} onChange={(e) => setForm((f) => ({ ...f, rate_limit_window_seconds: e.target.value }))} placeholder="60" /></Field>
                <Field label={copy.natPage.fieldMaxConnectionsPerWindow}><input className={inputClass} value={form.max_connections_per_window} onChange={(e) => setForm((f) => ({ ...f, max_connections_per_window: e.target.value }))} placeholder="30" /></Field>
                <Field label={copy.natPage.fieldMaxBytesPerWindow}><input className={inputClass} value={form.max_bytes_per_window} onChange={(e) => setForm((f) => ({ ...f, max_bytes_per_window: e.target.value }))} placeholder="104857600" /></Field>
              </div>
              <button className={buttonClass("primary")}>{copy.natPage.saveMapping}</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}
