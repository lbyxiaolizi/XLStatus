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
  formatDate,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

interface DdnsConfig {
  id: string;
  agent_id?: string | null;
  name?: string;
  provider?: string;
  domain?: string;
  webhook_url?: string | null;
  current_ip?: string | null;
  last_applied_ip?: string | null;
  enabled?: boolean;
  updated_at?: string;
}

interface DdnsHistory {
  id?: string;
  success?: boolean;
  message?: string;
  created_at?: string;
}

export default function DdnsPage() {
  const [configs, setConfigs] = useState<DdnsConfig[]>([]);
  const [history, setHistory] = useState<DdnsHistory[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState(false);
  const [form, setForm] = useState({
    agent_id: "",
    name: "",
    provider: "webhook",
    domain: "",
    record_id: "",
    zone_id: "",
    api_token: "",
    api_key: "",
    api_secret: "",
    webhook_url: "",
  });

  const load = useCallback(async () => {
    const response = await apiClient.listDdnsConfigs();
    if (response.success && response.data) {
      setConfigs((response.data.configs as DdnsConfig[]) ?? []);
    } else {
      setError(responseError(response));
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
      name: form.name,
      provider: form.provider,
      domain: form.domain,
      agent_id: form.agent_id.trim() || null,
      record_id: form.record_id.trim() || null,
      zone_id: form.zone_id.trim() || null,
      api_token: form.api_token.trim() || null,
      api_key: form.api_key.trim() || null,
      api_secret: form.api_secret.trim() || null,
      webhook_url: form.webhook_url.trim() || null,
      enabled: true,
    };
    const response = await apiClient.createDdnsConfig(payload);
    if (response.success) {
      setNotice("DDNS config created.");
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function reload() {
    const response = await apiClient.reloadDdnsProviders();
    if (response.success) setNotice("DDNS providers reloaded.");
    else setError(responseError(response));
  }

  async function showHistory(config: DdnsConfig) {
    const response = await apiClient.listDdnsHistory(config.id);
    if (response.success && response.data) {
      setHistory((response.data.history as DdnsHistory[]) ?? []);
      setNotice(`History loaded for ${config.name || config.domain || config.id}.`);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteConfig(config: DdnsConfig) {
    if (!confirm(`Delete DDNS config "${config.name || config.domain || config.id}"?`)) return;
    const response = await apiClient.deleteDdnsConfig(config.id);
    if (response.success) {
      setNotice("DDNS config deleted.");
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
          eyebrow="DNS"
          title="DDNS"
          detail="动态 DNS 配置、重载和更新历史。"
          actions={
            <>
              <button className={buttonClass("secondary")} onClick={() => void reload()}>Reload</button>
              <button className={buttonClass("primary")} onClick={() => setModal(true)}>Add Config</button>
            </>
          }
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {configs.length === 0 ? (
          <EmptyState title="No DDNS configs" />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead><tr><th className={thClass}>Name</th><th className={thClass}>Provider</th><th className={thClass}>Domain</th><th className={thClass}>IP</th><th className={thClass}>Status</th><th className={thClass}>Actions</th></tr></thead>
              <tbody>
                {configs.map((config) => (
                  <tr key={config.id}>
                    <td className={tdClass}>{config.name || config.id}</td>
                    <td className={tdClass}>{config.provider || "webhook"}</td>
                    <td className={tdClass}>{config.domain || "-"}</td>
                    <td className={tdClass}>{config.last_applied_ip || config.current_ip || "-"}</td>
                    <td className={tdClass}><StatusBadge tone={config.enabled === false ? "gray" : "green"}>{config.enabled === false ? "disabled" : "enabled"}</StatusBadge></td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("secondary")} onClick={() => void showHistory(config)}>History</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteConfig(config)}>Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {history.length > 0 ? (
          <section className="mt-6">
            <h2 className="mb-3 text-xl font-black uppercase">History</h2>
            <div className="grid gap-3">
              {history.map((item, index) => (
                <div key={item.id || index} className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
                  <StatusBadge tone={item.success ? "green" : "red"}>{item.success ? "success" : "failure"}</StatusBadge>
                  <span className="ml-3 text-sm font-black">{formatDate(item.created_at)}</span>
                  <p className="mt-2 text-sm font-bold text-[var(--text-muted)]">{item.message || ""}</p>
                </div>
              ))}
            </div>
          </section>
        ) : null}

        {modal ? (
          <Modal title="Add DDNS Config" onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Name"><input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} /></Field>
                <Field label="Provider"><select className={selectClass} value={form.provider} onChange={(e) => setForm((f) => ({ ...f, provider: e.target.value }))}><option value="webhook">webhook</option><option value="cloudflare">cloudflare</option><option value="aliyun">aliyun</option></select></Field>
              </div>
              <Field label="Agent ID"><input className={inputClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))} /></Field>
              <Field label="Domain"><input className={inputClass} value={form.domain} onChange={(e) => setForm((f) => ({ ...f, domain: e.target.value }))} /></Field>
              <Field label="Webhook URL"><textarea className={`${textareaClass} min-h-24`} value={form.webhook_url} onChange={(e) => setForm((f) => ({ ...f, webhook_url: e.target.value }))} placeholder="https://example.com/update?ip={{ip}}&hostname={{hostname}}" /></Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Record ID"><input className={inputClass} value={form.record_id} onChange={(e) => setForm((f) => ({ ...f, record_id: e.target.value }))} /></Field>
                <Field label="Zone ID"><input className={inputClass} value={form.zone_id} onChange={(e) => setForm((f) => ({ ...f, zone_id: e.target.value }))} /></Field>
                <Field label="API Token"><input className={inputClass} value={form.api_token} onChange={(e) => setForm((f) => ({ ...f, api_token: e.target.value }))} /></Field>
                <Field label="API Key"><input className={inputClass} value={form.api_key} onChange={(e) => setForm((f) => ({ ...f, api_key: e.target.value }))} /></Field>
              </div>
              <Field label="API Secret"><input className={inputClass} value={form.api_secret} onChange={(e) => setForm((f) => ({ ...f, api_secret: e.target.value }))} /></Field>
              <button className={buttonClass("primary")}>Save Config</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}
