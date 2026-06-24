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
  formatDate,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type DdnsConfig, type JsonObject } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useI18n } from "@/lib/use-i18n";

interface DdnsHistory {
  id?: string;
  success?: boolean;
  message?: string;
  created_at?: string;
}

export default function DdnsPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [configs, setConfigs] = useState<DdnsConfig[]>([]);
  const [history, setHistory] = useState<DdnsHistory[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<{ enabled?: boolean } | null>(null);
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
      setConfigs(response.data.configs ?? []);
    } else {
      setError(responseError(response));
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
  }, [load]);

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
      setError(copy.ddnsPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

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
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createDdnsConfig(payload, totpCode);
    if (response.success) {
      setNotice(copy.ddnsPage.configCreated);
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function reload() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.reloadDdnsProviders(totpCode);
    if (response.success) setNotice(copy.ddnsPage.providersReloaded);
    else setError(responseError(response));
  }

  async function checkNow() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.checkDdnsNow(totpCode);
    if (response.success) setNotice(copy.ddnsPage.checkTriggered);
    else setError(responseError(response));
  }

  async function showHistory(config: DdnsConfig) {
    const response = await apiClient.listDdnsHistory(config.id);
    if (response.success && response.data) {
      setHistory((response.data.history as DdnsHistory[]) ?? []);
      setNotice(copy.ddnsPage.historyLoaded.replace("{name}", String(config.name || config.domain || config.id)));
    } else {
      setError(responseError(response));
    }
  }

  async function deleteConfig(config: DdnsConfig) {
    if (!(await dialogs.confirm({ message: copy.ddnsPage.deleteConfigConfirm.replace("{name}", String(config.name || config.domain || config.id)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteDdnsConfig(config.id, totpCode);
    if (response.success) {
      setNotice(copy.ddnsPage.configDeleted);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.ddnsPage.eyebrow}
          title={copy.ddnsPage.title}
          detail={copy.ddnsPage.detail}
          actions={
            <>
              <button className={buttonClass("secondary")} onClick={() => void reload()}>{copy.ddnsPage.reload}</button>
              <button className={buttonClass("secondary")} onClick={() => void checkNow()}>{copy.ddnsPage.checkNow}</button>
              <button className={buttonClass("primary")} onClick={() => setModal(true)}>{copy.ddnsPage.addConfig}</button>
            </>
          }
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {configs.length === 0 ? (
          <EmptyState title={copy.ddnsPage.emptyTitle} />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead><tr><th className={thClass}>{copy.ddnsPage.colName}</th><th className={thClass}>{copy.ddnsPage.colProvider}</th><th className={thClass}>{copy.ddnsPage.colDomain}</th><th className={thClass}>{copy.ddnsPage.colIp}</th><th className={thClass}>{copy.ddnsPage.colStatus}</th><th className={thClass}>{copy.ddnsPage.colActions}</th></tr></thead>
              <tbody>
                {configs.map((config) => (
                  <tr key={config.id}>
                    <td className={tdClass}>{config.name || config.id}</td>
                    <td className={tdClass}>{config.provider || "webhook"}</td>
                    <td className={tdClass}>{config.domain || "-"}</td>
                    <td className={tdClass}>{config.last_applied_ip || config.current_ip || "-"}</td>
                    <td className={tdClass}><StatusBadge tone={config.enabled === false ? "gray" : "green"}>{config.enabled === false ? copy.ddnsPage.disabled : copy.ddnsPage.enabled}</StatusBadge></td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("secondary")} onClick={() => void showHistory(config)}>{copy.ddnsPage.history}</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteConfig(config)}>{copy.ddnsPage.delete}</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {history.length > 0 ? (
          <section className="mt-6">
            <h2 className="mb-3 text-xl font-black uppercase">{copy.ddnsPage.historyHeading}</h2>
            <div className="grid gap-3">
              {history.map((item, index) => (
                <div key={item.id || index} className="border-2 border-black bg-[var(--bg-card)] p-3 shadow-[var(--shadow-brutal-sm)]">
                  <StatusBadge tone={item.success ? "green" : "red"}>{item.success ? copy.ddnsPage.success : copy.ddnsPage.failed}</StatusBadge>
                  <span className="ml-3 text-sm font-black">{formatDate(item.created_at)}</span>
                  <p className="mt-2 text-sm font-bold text-[var(--text-muted)]">{item.message || ""}</p>
                </div>
              ))}
            </div>
          </section>
        ) : null}

        {modal ? (
          <Modal title={copy.ddnsPage.modalTitle} onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.ddnsPage.fieldName}><input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} /></Field>
                <Field label={copy.ddnsPage.fieldProvider}><select className={selectClass} value={form.provider} onChange={(e) => setForm((f) => ({ ...f, provider: e.target.value }))}><option value="webhook">webhook</option><option value="cloudflare">cloudflare</option><option value="aliyun">aliyun</option></select></Field>
              </div>
              <Field label={copy.ddnsPage.fieldAgentId}><input className={inputClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))} /></Field>
              <Field label={copy.ddnsPage.fieldDomain}><input className={inputClass} value={form.domain} onChange={(e) => setForm((f) => ({ ...f, domain: e.target.value }))} /></Field>
              <Field label={copy.ddnsPage.fieldWebhookUrl}><textarea className={`${textareaClass} min-h-24`} value={form.webhook_url} onChange={(e) => setForm((f) => ({ ...f, webhook_url: e.target.value }))} placeholder="https://example.com/update?ip={{ip}}&hostname={{hostname}}" /></Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.ddnsPage.fieldRecordId}><input className={inputClass} value={form.record_id} onChange={(e) => setForm((f) => ({ ...f, record_id: e.target.value }))} /></Field>
                <Field label={copy.ddnsPage.fieldZoneId}><input className={inputClass} value={form.zone_id} onChange={(e) => setForm((f) => ({ ...f, zone_id: e.target.value }))} /></Field>
                <Field label={copy.ddnsPage.fieldApiToken}><input className={inputClass} value={form.api_token} onChange={(e) => setForm((f) => ({ ...f, api_token: e.target.value }))} /></Field>
                <Field label={copy.ddnsPage.fieldApiKey}><input className={inputClass} value={form.api_key} onChange={(e) => setForm((f) => ({ ...f, api_key: e.target.value }))} /></Field>
              </div>
              <Field label={copy.ddnsPage.fieldApiSecret}><input className={inputClass} value={form.api_secret} onChange={(e) => setForm((f) => ({ ...f, api_secret: e.target.value }))} /></Field>
              <button className={buttonClass("primary")}>{copy.ddnsPage.saveConfig}</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}
