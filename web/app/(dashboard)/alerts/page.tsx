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
  inputClass,
  responseError,
  selectClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

type AlertKind = "service_down" | "service_latency" | "server_offline" | "server_resource";
type ResourceType = "cpu" | "memory" | "disk" | "network" | "load";
type Operator = "gt" | "gte" | "lt" | "lte";

interface Service {
  id: string;
  name: string;
  target: string;
}

interface Server {
  id: string;
  name: string;
  status: string;
}

interface AlertRule {
  id: string;
  name: string;
  enabled: boolean;
  trigger: "once" | "always" | string;
  conditions: JsonObject[];
  notification_group_id?: string | null;
  created_at: string;
  updated_at: string;
}

interface AlertEvent {
  id: string;
  rule_id: string;
  agent_id?: string | null;
  service_id?: string | null;
  kind: string;
  payload?: JsonObject;
  fired_at: string;
}

interface AlertForm {
  name: string;
  trigger: "once" | "always";
  kind: AlertKind;
  service_id: string;
  agent_id: string;
  resource: ResourceType;
  operator: Operator;
  threshold: string;
  consecutive_failures: string;
  max_latency_ms: string;
  offline_seconds: string;
  duration_seconds: string;
}

const blankForm: AlertForm = {
  name: "",
  trigger: "once",
  kind: "server_offline",
  service_id: "",
  agent_id: "",
  resource: "cpu",
  operator: "gt",
  threshold: "90",
  consecutive_failures: "3",
  max_latency_ms: "1000",
  offline_seconds: "60",
  duration_seconds: "60",
};

export default function AlertsPage() {
  const [rules, setRules] = useState<AlertRule[]>([]);
  const [events, setEvents] = useState<AlertEvent[]>([]);
  const [services, setServices] = useState<Service[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "details" | null>(null);
  const [selected, setSelected] = useState<AlertRule | null>(null);
  const [form, setForm] = useState<AlertForm>(blankForm);
  const [formErrors, setFormErrors] = useState<Partial<Record<keyof AlertForm, string>>>({});
  const [query, setQuery] = useState("");

  const loadAlerts = useCallback(async () => {
    setError(null);
    try {
      setLoading(true);
      const [rulesResponse, eventsResponse, serviceResponse, serverResponse] = await Promise.all([
        apiClient.listAlertRules(),
        apiClient.listAlertEvents(20),
        apiClient.listServices(),
        apiClient.listServers(200, 0),
      ]);

      if (rulesResponse.success && rulesResponse.data) {
        setRules((rulesResponse.data.rules as AlertRule[]) ?? []);
      } else {
        setError(responseError(rulesResponse));
      }

      if (eventsResponse.success && eventsResponse.data) {
        setEvents((eventsResponse.data.events as AlertEvent[]) ?? []);
      }
      if (serviceResponse.success && serviceResponse.data) {
        setServices((serviceResponse.data.services as Service[]) ?? []);
      }
      if (serverResponse.success && serverResponse.data) {
        setServers((serverResponse.data.servers as Server[]) ?? []);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadAlerts();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadAlerts]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return rules;
    }
    return rules.filter((rule) =>
      [rule.name, rule.trigger, describeConditions(rule, servers, services)]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [query, rules, servers, services]);

  function openCreate() {
    setForm({
      ...blankForm,
      agent_id: servers[0]?.id ?? "",
      service_id: services[0]?.id ?? "",
    });
    setFormErrors({});
    setModal("create");
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    setNotice(null);
    const validation = validateForm(form);
    setFormErrors(validation);
    if (Object.keys(validation).length > 0) {
      return;
    }

    setSaving(true);
    const response = await apiClient.createAlertRule(formToPayload(form));
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice("Alert rule created.");
      await loadAlerts();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteRule(rule: AlertRule) {
    if (!confirm(`Delete alert rule "${rule.name}"?`)) {
      return;
    }

    const response = await apiClient.deleteAlertRule(rule.id);
    if (response.success) {
      setNotice("Alert rule deleted.");
      await loadAlerts();
    } else {
      setError(responseError(response));
    }
  }

  function openDetails(rule: AlertRule) {
    setSelected(rule);
    setModal("details");
  }

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">Alert Rules</h1>
            <p className="mt-1 text-sm text-gray-500">Resource, offline, service status, and latency rules.</p>
          </div>
          <button type="button" onClick={openCreate} className={buttonClass("primary")}>
            Create Alert Rule
          </button>
        </div>

        <div className="mb-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search alert rules" className={inputClass} />
          <button type="button" onClick={() => void loadAlerts()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <div className="rounded bg-white p-6 text-sm text-gray-600 shadow">Loading alerts...</div>
        ) : (
          <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_360px]">
            <div>
              {filtered.length === 0 ? (
                <EmptyState title="No alert rules configured" detail="Create a rule to fire on service or server conditions." />
              ) : (
                <>
                  <div className="hidden overflow-hidden rounded-lg bg-white shadow md:block">
                    <table className="min-w-full divide-y divide-gray-200">
                      <thead className="bg-gray-50">
                        <tr>
                          {["Name", "Condition", "Trigger", "State", "Updated", "Actions"].map((heading) => (
                            <th key={heading} className="px-6 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                              {heading}
                            </th>
                          ))}
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-gray-200 bg-white">
                        {filtered.map((rule) => (
                          <tr key={rule.id} className="hover:bg-gray-50">
                            <td className="px-6 py-4">
                              <div className="text-sm font-medium text-gray-900">{rule.name}</div>
                              <div className="text-xs text-gray-500">{rule.id}</div>
                            </td>
                            <td className="px-6 py-4 text-sm text-gray-500">{describeConditions(rule, servers, services)}</td>
                            <td className="px-6 py-4 text-sm text-gray-500">{rule.trigger}</td>
                            <td className="px-6 py-4">{rule.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}</td>
                            <td className="px-6 py-4 text-sm text-gray-500">{formatDate(rule.updated_at)}</td>
                            <td className="px-6 py-4 text-sm">
                              <div className="flex gap-2">
                                <button type="button" onClick={() => openDetails(rule)} className="text-gray-700 hover:underline">
                                  Details
                                </button>
                                <button type="button" onClick={() => setNotice("This backend build exposes create/delete for alert rules, but not update.")} className="text-blue-700 hover:underline">
                                  Edit
                                </button>
                                <button type="button" onClick={() => void deleteRule(rule)} className="text-red-700 hover:underline">
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
                    {filtered.map((rule) => (
                      <div key={rule.id} className="rounded-lg bg-white p-4 shadow">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <h2 className="font-semibold text-gray-900">{rule.name}</h2>
                            <p className="mt-1 text-sm text-gray-500">{describeConditions(rule, servers, services)}</p>
                          </div>
                          {rule.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}
                        </div>
                        <div className="mt-4 grid grid-cols-2 gap-2">
                          <button type="button" onClick={() => openDetails(rule)} className={buttonClass()}>
                            Details
                          </button>
                          <button type="button" onClick={() => void deleteRule(rule)} className={buttonClass("danger")}>
                            Delete
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              )}
            </div>

            <div className="rounded-lg bg-white p-4 shadow">
              <h2 className="text-base font-semibold text-gray-900">Recent Events</h2>
              {events.length === 0 ? (
                <p className="mt-3 text-sm text-gray-500">No alert events recorded.</p>
              ) : (
                <div className="mt-3 space-y-3">
                  {events.map((event) => (
                    <div key={event.id} className="border-b border-gray-100 pb-3 last:border-b-0 last:pb-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm font-medium text-gray-900">{event.kind}</span>
                        <StatusBadge tone={event.kind === "recovered" ? "green" : "red"}>{event.kind}</StatusBadge>
                      </div>
                      <div className="mt-1 text-xs text-gray-500">{formatDate(event.fired_at)}</div>
                      <div className="mt-1 text-xs text-gray-500">{compactId(event.rule_id)}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
      </PageShell>

      {modal === "create" ? (
        <Modal title="Create Alert Rule" onClose={() => setModal(null)}>
          <form onSubmit={submitForm} className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Name" error={formErrors.name}>
                <input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="Trigger">
                <select value={form.trigger} onChange={(event) => setForm((prev) => ({ ...prev, trigger: event.target.value as "once" | "always" }))} className={selectClass}>
                  <option value="once">Once until recovery</option>
                  <option value="always">Every evaluation window</option>
                </select>
              </Field>
            </div>

            <Field label="Condition Type">
              <select value={form.kind} onChange={(event) => setForm((prev) => ({ ...prev, kind: event.target.value as AlertKind }))} className={selectClass}>
                <option value="server_offline">Server offline</option>
                <option value="server_resource">Server resource</option>
                <option value="service_down">Service down</option>
                <option value="service_latency">Service latency</option>
              </select>
            </Field>

            {form.kind === "service_down" || form.kind === "service_latency" ? (
              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Service" error={formErrors.service_id}>
                  <select value={form.service_id} onChange={(event) => setForm((prev) => ({ ...prev, service_id: event.target.value }))} className={selectClass}>
                    <option value="">Select a service</option>
                    {services.map((service) => (
                      <option key={service.id} value={service.id}>
                        {service.name}
                      </option>
                    ))}
                  </select>
                </Field>
                {form.kind === "service_down" ? (
                  <Field label="Consecutive Failures" error={formErrors.consecutive_failures}>
                    <input type="number" min={1} value={form.consecutive_failures} onChange={(event) => setForm((prev) => ({ ...prev, consecutive_failures: event.target.value }))} className={inputClass} />
                  </Field>
                ) : (
                  <Field label="Max Latency Ms" error={formErrors.max_latency_ms}>
                    <input type="number" min={1} value={form.max_latency_ms} onChange={(event) => setForm((prev) => ({ ...prev, max_latency_ms: event.target.value }))} className={inputClass} />
                  </Field>
                )}
              </div>
            ) : (
              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Server" error={formErrors.agent_id}>
                  <select value={form.agent_id} onChange={(event) => setForm((prev) => ({ ...prev, agent_id: event.target.value }))} className={selectClass}>
                    <option value="">Select a server</option>
                    {servers.map((server) => (
                      <option key={server.id} value={server.id}>
                        {server.name} ({server.status})
                      </option>
                    ))}
                  </select>
                </Field>
                {form.kind === "server_offline" ? (
                  <Field label="Offline Seconds" error={formErrors.offline_seconds}>
                    <input type="number" min={1} value={form.offline_seconds} onChange={(event) => setForm((prev) => ({ ...prev, offline_seconds: event.target.value }))} className={inputClass} />
                  </Field>
                ) : (
                  <Field label="Resource">
                    <select value={form.resource} onChange={(event) => setForm((prev) => ({ ...prev, resource: event.target.value as ResourceType }))} className={selectClass}>
                      <option value="cpu">CPU</option>
                      <option value="memory">Memory</option>
                      <option value="disk">Disk</option>
                      <option value="network">Network</option>
                      <option value="load">Load</option>
                    </select>
                  </Field>
                )}
              </div>
            )}

            {form.kind === "server_resource" ? (
              <div className="grid gap-4 md:grid-cols-3">
                <Field label="Operator">
                  <select value={form.operator} onChange={(event) => setForm((prev) => ({ ...prev, operator: event.target.value as Operator }))} className={selectClass}>
                    <option value="gt">&gt;</option>
                    <option value="gte">&gt;=</option>
                    <option value="lt">&lt;</option>
                    <option value="lte">&lt;=</option>
                  </select>
                </Field>
                <Field label="Threshold" error={formErrors.threshold}>
                  <input type="number" value={form.threshold} onChange={(event) => setForm((prev) => ({ ...prev, threshold: event.target.value }))} className={inputClass} />
                </Field>
                <Field label="Duration Seconds" error={formErrors.duration_seconds}>
                  <input type="number" min={0} value={form.duration_seconds} onChange={(event) => setForm((prev) => ({ ...prev, duration_seconds: event.target.value }))} className={inputClass} />
                </Field>
              </div>
            ) : null}

            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                Cancel
              </button>
              <button type="submit" disabled={saving} className={buttonClass("primary")}>
                {saving ? "Saving..." : "Create Rule"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {modal === "details" && selected ? (
        <Modal title={selected.name} onClose={() => setModal(null)}>
          <div className="space-y-4">
            <div className="grid gap-3 text-sm sm:grid-cols-2">
              <div>
                <div className="text-xs uppercase text-gray-500">Trigger</div>
                <div className="font-medium text-gray-900">{selected.trigger}</div>
              </div>
              <div>
                <div className="text-xs uppercase text-gray-500">Updated</div>
                <div className="font-medium text-gray-900">{formatDate(selected.updated_at)}</div>
              </div>
            </div>
            <pre className="max-h-80 overflow-auto rounded bg-gray-50 p-3 text-xs text-gray-800">
              {JSON.stringify(selected.conditions, null, 2)}
            </pre>
          </div>
        </Modal>
      ) : null}
    </div>
  );
}

function validateForm(form: AlertForm): Partial<Record<keyof AlertForm, string>> {
  const errors: Partial<Record<keyof AlertForm, string>> = {};
  if (!form.name.trim()) {
    errors.name = "Name is required";
  }
  if ((form.kind === "service_down" || form.kind === "service_latency") && !form.service_id) {
    errors.service_id = "Service is required";
  }
  if ((form.kind === "server_offline" || form.kind === "server_resource") && !form.agent_id) {
    errors.agent_id = "Server is required";
  }
  if (form.kind === "service_down" && !positiveNumber(form.consecutive_failures)) {
    errors.consecutive_failures = "Use a positive number";
  }
  if (form.kind === "service_latency" && !positiveNumber(form.max_latency_ms)) {
    errors.max_latency_ms = "Use a positive number";
  }
  if (form.kind === "server_offline" && !positiveNumber(form.offline_seconds)) {
    errors.offline_seconds = "Use a positive number";
  }
  if (form.kind === "server_resource" && !Number.isFinite(Number(form.threshold))) {
    errors.threshold = "Use a number";
  }
  return errors;
}

function formToPayload(form: AlertForm): JsonObject {
  return {
    name: form.name.trim(),
    trigger: form.trigger,
    conditions: [conditionFromForm(form)],
    notification_group_id: null,
  };
}

function conditionFromForm(form: AlertForm): JsonObject {
  if (form.kind === "service_down") {
    return {
      type: "service_down",
      service_id: form.service_id,
      consecutive_failures: Number(form.consecutive_failures),
    };
  }
  if (form.kind === "service_latency") {
    return {
      type: "service_latency",
      service_id: form.service_id,
      max_latency_ms: Number(form.max_latency_ms),
    };
  }
  if (form.kind === "server_offline") {
    return {
      type: "server_offline",
      agent_id: form.agent_id,
      offline_seconds: Number(form.offline_seconds),
    };
  }
  return {
    type: "server_resource",
    agent_id: form.agent_id,
    resource: form.resource,
    operator: form.operator,
    threshold: Number(form.threshold),
    duration_seconds: Number(form.duration_seconds) || 0,
  };
}

function positiveNumber(value: string): boolean {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0;
}

function describeConditions(rule: AlertRule, servers: Server[], services: Service[]): string {
  if (!rule.conditions.length) {
    return "No conditions";
  }
  return rule.conditions.map((condition) => describeCondition(condition, servers, services)).join("; ");
}

function describeCondition(condition: JsonObject, servers: Server[], services: Service[]): string {
  const type = String(condition.type || "");
  if (type === "service_down") {
    return `${serviceName(String(condition.service_id || ""), services)} down x${condition.consecutive_failures}`;
  }
  if (type === "service_latency") {
    return `${serviceName(String(condition.service_id || ""), services)} latency > ${condition.max_latency_ms}ms`;
  }
  if (type === "server_offline") {
    return `${serverName(String(condition.agent_id || ""), servers)} offline ${condition.offline_seconds}s`;
  }
  if (type === "server_resource") {
    return `${serverName(String(condition.agent_id || ""), servers)} ${condition.resource} ${condition.operator} ${condition.threshold}`;
  }
  return JSON.stringify(condition);
}

function serverName(id: string, servers: Server[]): string {
  const server = servers.find((item) => item.id === id);
  return server ? server.name : compactId(id);
}

function serviceName(id: string, services: Service[]): string {
  const service = services.find((item) => item.id === id);
  return service ? service.name : compactId(id);
}
