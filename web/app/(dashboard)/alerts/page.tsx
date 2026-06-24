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
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

interface AlertRule {
  id: string;
  name: string;
  trigger?: string;
  conditions?: JsonObject[];
  enabled?: boolean;
  failure_task_ids?: string[];
  recovery_task_ids?: string[];
  created_at?: string;
}

interface AlertEvent {
  id?: string;
  kind?: string;
  payload?: unknown;
  fired_at?: string;
}

export default function AlertsPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [rules, setRules] = useState<AlertRule[]>([]);
  const [events, setEvents] = useState<AlertEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState(false);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [form, setForm] = useState({
    name: "",
    trigger: "once",
    condition_type: "server_resource",
    agent_id: "",
    service_id: "",
    resource: "cpu",
    operator: "gt",
    threshold: "90",
    duration_seconds: "60",
    offline_seconds: "120",
    consecutive_failures: "3",
    max_latency_ms: "1000",
    cert_days_before: "7",
    server_expiry_days_before: "7",
    traffic_quota_percent: "80",
    traffic_quota_direction: "total",
    notification_group_id: "",
    failure_task_ids: "",
    recovery_task_ids: "",
  });

  const load = useCallback(async () => {
    const [rulesResponse, eventsResponse] = await Promise.all([apiClient.listAlertRules(100, 0), apiClient.listAlertEvents(50)]);
    if (rulesResponse.success && rulesResponse.data) {
      setRules((rulesResponse.data.rules as AlertRule[]) ?? []);
    } else {
      setError(responseError(rulesResponse));
    }
    if (eventsResponse.success && eventsResponse.data) {
      setEvents((eventsResponse.data.events as AlertEvent[]) ?? []);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
  }, [load]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const payload: JsonObject = {
      name: form.name,
      trigger: form.trigger,
      conditions: [buildCondition(form)],
      notification_group_id: form.notification_group_id.trim() || null,
      failure_task_ids: splitIds(form.failure_task_ids),
      recovery_task_ids: splitIds(form.recovery_task_ids),
    };
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createAlertRule(payload, totpCode);
    if (response.success) {
      setNotice(copy.alertsPage.ruleCreated);
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteRule(rule: AlertRule) {
    if (!(await dialogs.confirm({ message: copy.alertsPage.deleteRuleConfirm.replace("{name}", String(rule.name)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteAlertRule(rule.id, totpCode);
    if (response.success) {
      setNotice(copy.alertsPage.ruleDeleted);
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
      setError(copy.alertsPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.alertsPage.eyebrow}
          title={copy.alertsPage.title}
          detail={copy.alertsPage.detail}
          actions={<button className={buttonClass("primary")} onClick={() => setModal(true)}>{copy.alertsPage.addRule}</button>}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <section>
            <h2 className="mb-3 text-xl font-black uppercase">{copy.alertsPage.rulesHeading}</h2>
            {rules.length === 0 ? (
              <EmptyState title={copy.alertsPage.noRules} />
            ) : (
              <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
                <table className="w-full">
                  <thead>
                    <tr><th className={thClass}>{copy.alertsPage.colName}</th><th className={thClass}>{copy.alertsPage.colCondition}</th><th className={thClass}>{copy.alertsPage.colStatus}</th><th className={thClass}>{copy.alertsPage.colActions}</th></tr>
                  </thead>
                  <tbody>
                    {rules.map((rule) => (
                      <tr key={rule.id}>
                        <td className={tdClass}>{rule.name}</td>
                        <td className={tdClass}>{formatConditions(copy, rule.conditions)}</td>
                        <td className={tdClass}><StatusBadge tone={rule.enabled === false ? "gray" : "green"}>{rule.enabled === false ? copy.alertsPage.disabled : copy.alertsPage.enabled}</StatusBadge></td>
                        <td className={tdClass}><button className={buttonClass("danger")} onClick={() => void deleteRule(rule)}>{copy.alertsPage.delete}</button></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-3 text-xl font-black uppercase">{copy.alertsPage.eventsHeading}</h2>
            <div className="grid gap-3">
              {events.length === 0 ? <EmptyState title={copy.alertsPage.noEvents} /> : events.map((event, index) => (
                <div key={event.id || index} className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal-sm)]">
                  <div className="font-black">{event.kind || copy.alertsPage.title}</div>
                  <p className="text-sm font-bold text-[var(--text-muted)]">{formatPayload(copy, event.payload)}</p>
                  <p className="mt-2 text-xs font-black uppercase">{formatDate(event.fired_at)}</p>
                </div>
              ))}
            </div>
          </section>
        </div>

        {modal ? (
          <Modal title={copy.alertsPage.modalTitle} onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <Field label={copy.alertsPage.fieldName}><input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required /></Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.alertsPage.fieldTrigger}><select className={selectClass} value={form.trigger} onChange={(e) => setForm((f) => ({ ...f, trigger: e.target.value }))}><option value="once">once</option><option value="always">always</option></select></Field>
                <Field label={copy.alertsPage.fieldConditionType}><select className={selectClass} value={form.condition_type} onChange={(e) => setForm((f) => ({ ...f, condition_type: e.target.value }))}><option value="server_resource">server_resource</option><option value="server_offline">server_offline</option><option value="server_expiry">server_expiry</option><option value="server_traffic_quota">server_traffic_quota</option><option value="service_down">service_down</option><option value="service_latency">service_latency</option><option value="certificate_expiry">certificate_expiry</option></select></Field>
              </div>
              {form.condition_type.startsWith("server") ? (
                <Field label={copy.alertsPage.fieldAgentId}><input className={inputClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))} required /></Field>
              ) : (
                <Field label={copy.alertsPage.fieldServiceId}><input className={inputClass} value={form.service_id} onChange={(e) => setForm((f) => ({ ...f, service_id: e.target.value }))} required /></Field>
              )}
              {form.condition_type === "server_resource" ? (
                <div className="grid gap-4 sm:grid-cols-4">
                  <Field label={copy.alertsPage.fieldResource}><select className={selectClass} value={form.resource} onChange={(e) => setForm((f) => ({ ...f, resource: e.target.value }))}><option value="cpu">cpu</option><option value="memory">memory</option><option value="disk">disk</option><option value="swap">swap</option><option value="network">network_delta</option><option value="network_in">network_in</option><option value="network_out">network_out</option><option value="network_total">network_total</option><option value="traffic_in_total">traffic_in_total</option><option value="traffic_out_total">traffic_out_total</option><option value="load">load1</option><option value="load5">load5</option><option value="load15">load15</option><option value="tcp">tcp</option><option value="udp">udp</option><option value="process">process</option><option value="temperature">temperature</option><option value="gpu">gpu</option></select></Field>
                  <Field label={copy.alertsPage.fieldOperator}><select className={selectClass} value={form.operator} onChange={(e) => setForm((f) => ({ ...f, operator: e.target.value }))}><option value="gt">gt</option><option value="gte">gte</option><option value="lt">lt</option><option value="lte">lte</option></select></Field>
                  <Field label={copy.alertsPage.fieldThreshold}><input className={inputClass} value={form.threshold} onChange={(e) => setForm((f) => ({ ...f, threshold: e.target.value }))} /></Field>
                  <Field label={copy.alertsPage.fieldDurationSeconds}><input className={inputClass} value={form.duration_seconds} onChange={(e) => setForm((f) => ({ ...f, duration_seconds: e.target.value }))} /></Field>
                </div>
              ) : null}
              {form.condition_type === "server_offline" ? (
                <Field label={copy.alertsPage.fieldOfflineSeconds}><input className={inputClass} value={form.offline_seconds} onChange={(e) => setForm((f) => ({ ...f, offline_seconds: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "server_expiry" ? (
                <Field label={copy.alertsPage.fieldDaysBefore}><input className={inputClass} value={form.server_expiry_days_before} onChange={(e) => setForm((f) => ({ ...f, server_expiry_days_before: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "server_traffic_quota" ? (
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field label={copy.alertsPage.fieldUsagePercent}>
                    <input className={inputClass} value={form.traffic_quota_percent} onChange={(e) => setForm((f) => ({ ...f, traffic_quota_percent: e.target.value }))} />
                  </Field>
                  <Field label={copy.alertsPage.fieldDirection}>
                    <select className={selectClass} value={form.traffic_quota_direction} onChange={(e) => setForm((f) => ({ ...f, traffic_quota_direction: e.target.value }))}>
                      <option value="total">total</option>
                      <option value="in">in</option>
                      <option value="out">out</option>
                    </select>
                  </Field>
                </div>
              ) : null}
              {form.condition_type === "service_down" ? (
                <Field label={copy.alertsPage.fieldConsecutiveFailures}><input className={inputClass} value={form.consecutive_failures} onChange={(e) => setForm((f) => ({ ...f, consecutive_failures: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "service_latency" ? (
                <Field label={copy.alertsPage.fieldMaxLatencyMs}><input className={inputClass} value={form.max_latency_ms} onChange={(e) => setForm((f) => ({ ...f, max_latency_ms: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "certificate_expiry" ? (
                <Field label={copy.alertsPage.fieldDaysBefore}><input className={inputClass} value={form.cert_days_before} onChange={(e) => setForm((f) => ({ ...f, cert_days_before: e.target.value }))} /></Field>
              ) : null}
              <Field label={copy.alertsPage.fieldNotificationGroupId}>
                <input className={inputClass} value={form.notification_group_id} onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))} />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.alertsPage.fieldFailureTaskIds}>
                  <input className={inputClass} value={form.failure_task_ids} onChange={(e) => setForm((f) => ({ ...f, failure_task_ids: e.target.value }))} />
                </Field>
                <Field label={copy.alertsPage.fieldRecoveryTaskIds}>
                  <input className={inputClass} value={form.recovery_task_ids} onChange={(e) => setForm((f) => ({ ...f, recovery_task_ids: e.target.value }))} />
                </Field>
              </div>
              <button className={buttonClass("primary")}>{copy.alertsPage.saveRule}</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function buildCondition(form: {
  condition_type: string;
  agent_id: string;
  service_id: string;
  resource: string;
  operator: string;
  threshold: string;
  duration_seconds: string;
  offline_seconds: string;
  consecutive_failures: string;
  max_latency_ms: string;
  cert_days_before: string;
  server_expiry_days_before: string;
  traffic_quota_percent: string;
  traffic_quota_direction: string;
}): JsonObject {
  if (form.condition_type === "server_offline") {
    return {
      type: "server_offline",
      agent_id: form.agent_id.trim(),
      offline_seconds: Number(form.offline_seconds),
    };
  }
  if (form.condition_type === "service_down") {
    return {
      type: "service_down",
      service_id: form.service_id.trim(),
      consecutive_failures: Number(form.consecutive_failures),
    };
  }
  if (form.condition_type === "server_expiry") {
    return {
      type: "server_expiry",
      agent_id: form.agent_id.trim(),
      days_before: Number(form.server_expiry_days_before),
    };
  }
  if (form.condition_type === "server_traffic_quota") {
    return {
      type: "server_traffic_quota",
      agent_id: form.agent_id.trim(),
      percent: Number(form.traffic_quota_percent),
      direction: form.traffic_quota_direction,
    };
  }
  if (form.condition_type === "service_latency") {
    return {
      type: "service_latency",
      service_id: form.service_id.trim(),
      max_latency_ms: Number(form.max_latency_ms),
    };
  }
  if (form.condition_type === "certificate_expiry") {
    return {
      type: "certificate_expiry",
      service_id: form.service_id.trim(),
      days_before: Number(form.cert_days_before),
    };
  }
  return {
    type: "server_resource",
    agent_id: form.agent_id.trim(),
    resource: form.resource,
    operator: form.operator,
    threshold: Number(form.threshold),
    duration_seconds: Number(form.duration_seconds),
  };
}

function splitIds(value: string): string[] {
  return value
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function formatConditions(copy: Translations, conditions?: JsonObject[]): string {
  if (!conditions || conditions.length === 0) return "-";
  const t = copy.alertsPage;
  return conditions
    .map((condition) => {
      const type = String(condition.type || "condition");
      if (type === "server_resource") {
        return `${condition.agent_id}:${condition.resource} ${condition.operator} ${condition.threshold}`;
      }
      if (type === "server_offline") return `${condition.agent_id} ${t.conditionOffline} ${condition.offline_seconds}s`;
      if (type === "server_expiry") return `${condition.agent_id} ${condition.days_before} ${t.conditionExpiryDays}`;
      if (type === "server_traffic_quota") return `${condition.agent_id} ${t.conditionTraffic} ${condition.direction || "total"} >= ${condition.percent}%`;
      if (type === "service_down") return `${condition.service_id} ${t.conditionServiceDown} x${condition.consecutive_failures}`;
      if (type === "service_latency") return `${condition.service_id} ${t.conditionLatency} > ${condition.max_latency_ms}ms`;
      if (type === "certificate_expiry") return `${condition.service_id} ${t.conditionCert} ${condition.days_before} ${t.conditionExpiryDays}`;
      return type;
    })
    .join(", ");
}

function formatPayload(copy: Translations, payload: unknown): string {
  if (!payload) return copy.alertsPage.payloadEvent;
  if (typeof payload === "string") return payload;
  try {
    return JSON.stringify(payload);
  } catch {
    return copy.alertsPage.payloadEvent;
  }
}
