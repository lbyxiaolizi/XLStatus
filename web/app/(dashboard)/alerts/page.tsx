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
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

interface AlertRule {
  id: string;
  name: string;
  trigger?: string;
  conditions?: JsonObject[];
  enabled?: boolean;
  created_at?: string;
}

interface AlertEvent {
  id?: string;
  kind?: string;
  payload?: unknown;
  fired_at?: string;
}

export default function AlertsPage() {
  const [rules, setRules] = useState<AlertRule[]>([]);
  const [events, setEvents] = useState<AlertEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState(false);
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
    notification_group_id: "",
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
    const timeoutId = window.setTimeout(() => {
      void load();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [load]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const payload: JsonObject = {
      name: form.name,
      trigger: form.trigger,
      conditions: [buildCondition(form)],
      notification_group_id: form.notification_group_id.trim() || null,
    };
    const response = await apiClient.createAlertRule(payload);
    if (response.success) {
      setNotice("Alert rule created.");
      setModal(false);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteRule(rule: AlertRule) {
    if (!confirm(`Delete alert rule "${rule.name}"?`)) return;
    const response = await apiClient.deleteAlertRule(rule.id);
    if (response.success) {
      setNotice("Alert rule deleted.");
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
          eyebrow="Incident Rules"
          title="Alerts"
          detail="资源、服务和恢复通知的规则管理。"
          actions={<button className={buttonClass("primary")} onClick={() => setModal(true)}>Add Rule</button>}
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <section>
            <h2 className="mb-3 text-xl font-black uppercase">Rules</h2>
            {rules.length === 0 ? (
              <EmptyState title="No alert rules" />
            ) : (
              <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
                <table className="w-full">
                  <thead>
                    <tr><th className={thClass}>Name</th><th className={thClass}>Condition</th><th className={thClass}>Status</th><th className={thClass}>Action</th></tr>
                  </thead>
                  <tbody>
                    {rules.map((rule) => (
                      <tr key={rule.id}>
                        <td className={tdClass}>{rule.name}</td>
                        <td className={tdClass}>{formatConditions(rule.conditions)}</td>
                        <td className={tdClass}><StatusBadge tone={rule.enabled === false ? "gray" : "green"}>{rule.enabled === false ? "disabled" : "enabled"}</StatusBadge></td>
                        <td className={tdClass}><button className={buttonClass("danger")} onClick={() => void deleteRule(rule)}>Delete</button></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-3 text-xl font-black uppercase">Events</h2>
            <div className="grid gap-3">
              {events.length === 0 ? <EmptyState title="No alert events" /> : events.map((event, index) => (
                <div key={event.id || index} className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal-sm)]">
                  <div className="font-black">{event.kind || "Alert"}</div>
                  <p className="text-sm font-bold text-[var(--text-muted)]">{formatPayload(event.payload)}</p>
                  <p className="mt-2 text-xs font-black uppercase">{formatDate(event.fired_at)}</p>
                </div>
              ))}
            </div>
          </section>
        </div>

        {modal ? (
          <Modal title="Add Alert Rule" onClose={() => setModal(false)}>
            <form onSubmit={submit} className="space-y-4">
              <Field label="Name"><input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required /></Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Trigger"><select className={selectClass} value={form.trigger} onChange={(e) => setForm((f) => ({ ...f, trigger: e.target.value }))}><option value="once">once</option><option value="always">always</option></select></Field>
                <Field label="Condition type"><select className={selectClass} value={form.condition_type} onChange={(e) => setForm((f) => ({ ...f, condition_type: e.target.value }))}><option value="server_resource">server_resource</option><option value="server_offline">server_offline</option><option value="service_down">service_down</option><option value="service_latency">service_latency</option></select></Field>
              </div>
              {form.condition_type.startsWith("server") ? (
                <Field label="Agent ID"><input className={inputClass} value={form.agent_id} onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))} required /></Field>
              ) : (
                <Field label="Service ID"><input className={inputClass} value={form.service_id} onChange={(e) => setForm((f) => ({ ...f, service_id: e.target.value }))} required /></Field>
              )}
              {form.condition_type === "server_resource" ? (
                <div className="grid gap-4 sm:grid-cols-4">
                  <Field label="Resource"><select className={selectClass} value={form.resource} onChange={(e) => setForm((f) => ({ ...f, resource: e.target.value }))}><option value="cpu">cpu</option><option value="memory">memory</option><option value="disk">disk</option><option value="network">network</option><option value="load">load</option></select></Field>
                  <Field label="Operator"><select className={selectClass} value={form.operator} onChange={(e) => setForm((f) => ({ ...f, operator: e.target.value }))}><option value="gt">gt</option><option value="gte">gte</option><option value="lt">lt</option><option value="lte">lte</option></select></Field>
                  <Field label="Threshold"><input className={inputClass} value={form.threshold} onChange={(e) => setForm((f) => ({ ...f, threshold: e.target.value }))} /></Field>
                  <Field label="Duration seconds"><input className={inputClass} value={form.duration_seconds} onChange={(e) => setForm((f) => ({ ...f, duration_seconds: e.target.value }))} /></Field>
                </div>
              ) : null}
              {form.condition_type === "server_offline" ? (
                <Field label="Offline seconds"><input className={inputClass} value={form.offline_seconds} onChange={(e) => setForm((f) => ({ ...f, offline_seconds: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "service_down" ? (
                <Field label="Consecutive failures"><input className={inputClass} value={form.consecutive_failures} onChange={(e) => setForm((f) => ({ ...f, consecutive_failures: e.target.value }))} /></Field>
              ) : null}
              {form.condition_type === "service_latency" ? (
                <Field label="Max latency ms"><input className={inputClass} value={form.max_latency_ms} onChange={(e) => setForm((f) => ({ ...f, max_latency_ms: e.target.value }))} /></Field>
              ) : null}
              <Field label="Notification group ID">
                <input className={inputClass} value={form.notification_group_id} onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))} />
              </Field>
              <button className={buttonClass("primary")}>Save Rule</button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
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
  if (form.condition_type === "service_latency") {
    return {
      type: "service_latency",
      service_id: form.service_id.trim(),
      max_latency_ms: Number(form.max_latency_ms),
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

function formatConditions(conditions?: JsonObject[]): string {
  if (!conditions || conditions.length === 0) return "-";
  return conditions
    .map((condition) => {
      const type = String(condition.type || "condition");
      if (type === "server_resource") {
        return `${condition.agent_id}:${condition.resource} ${condition.operator} ${condition.threshold}`;
      }
      if (type === "server_offline") return `${condition.agent_id} offline ${condition.offline_seconds}s`;
      if (type === "service_down") return `${condition.service_id} down x${condition.consecutive_failures}`;
      if (type === "service_latency") return `${condition.service_id} latency > ${condition.max_latency_ms}ms`;
      return type;
    })
    .join(", ");
}

function formatPayload(payload: unknown): string {
  if (!payload) return "Event";
  if (typeof payload === "string") return payload;
  try {
    return JSON.stringify(payload);
  } catch {
    return "Event";
  }
}
