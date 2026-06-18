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
  isAdmin,
  responseError,
  selectClass,
  useStoredUser,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

type Provider = "cloudflare" | "tencent_cloud" | "he" | "webhook" | "dummy";

interface Server {
  id: string;
  name: string;
  status: string;
}

interface DdnsConfig {
  id: string;
  agent_id?: string | null;
  name: string;
  provider: Provider | string;
  domain: string;
  record_id?: string | null;
  zone_id?: string | null;
  current_ip?: string | null;
  last_applied_ip?: string | null;
  last_applied_at?: string | null;
  enabled: boolean;
  created_at?: string;
  updated_at?: string;
}

interface DdnsHistory {
  id: string;
  old_ip?: string | null;
  new_ip: string;
  success: boolean;
  error?: string | null;
  applied_at: string;
}

interface DdnsForm {
  name: string;
  provider: Provider;
  domain: string;
  agent_id: string;
  record_id: string;
  zone_id: string;
  api_token: string;
  api_key: string;
  api_secret: string;
  webhook_url: string;
  enabled: boolean;
}

const blankForm: DdnsForm = {
  name: "",
  provider: "dummy",
  domain: "",
  agent_id: "",
  record_id: "",
  zone_id: "",
  api_token: "",
  api_key: "",
  api_secret: "",
  webhook_url: "",
  enabled: true,
};

export default function DdnsPage() {
  const user = useStoredUser();
  const [configs, setConfigs] = useState<DdnsConfig[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [history, setHistory] = useState<DdnsHistory[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "history" | null>(null);
  const [selected, setSelected] = useState<DdnsConfig | null>(null);
  const [form, setForm] = useState<DdnsForm>(blankForm);
  const [formErrors, setFormErrors] = useState<Partial<Record<keyof DdnsForm, string>>>({});
  const [query, setQuery] = useState("");

  const loadConfigs = useCallback(async () => {
    setError(null);
    try {
      setLoading(true);
      const [ddnsResponse, serversResponse] = await Promise.all([
        apiClient.listDdnsConfigs(),
        apiClient.listServers(200, 0),
      ]);

      if (ddnsResponse.success && ddnsResponse.data) {
        setConfigs((ddnsResponse.data.configs as DdnsConfig[]) ?? []);
      } else {
        setError(responseError(ddnsResponse));
      }

      if (serversResponse.success && serversResponse.data) {
        setServers((serversResponse.data.servers as Server[]) ?? []);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadConfigs();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadConfigs]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return configs;
    }
    return configs.filter((config) =>
      [config.name, config.provider, config.domain, config.last_applied_ip, config.current_ip]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [configs, query]);

  function openCreate() {
    setForm({ ...blankForm, agent_id: servers[0]?.id ?? "" });
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
    const response = await apiClient.createDdnsConfig(formToPayload(form));
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice("DDNS profile created.");
      await loadConfigs();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteConfig(config: DdnsConfig) {
    if (!confirm(`Delete DDNS profile "${config.name}"? Saved provider credentials will be removed.`)) {
      return;
    }
    const response = await apiClient.deleteDdnsConfig(config.id);
    if (response.success) {
      setNotice("DDNS profile deleted.");
      await loadConfigs();
    } else {
      setError(responseError(response));
    }
  }

  async function openHistory(config: DdnsConfig) {
    setSelected(config);
    setHistory([]);
    setModal("history");
    const response = await apiClient.listDdnsHistory(config.id);
    if (response.success && response.data) {
      setHistory((response.data.history as DdnsHistory[]) ?? []);
    } else {
      setError(responseError(response));
    }
  }

  async function reloadProviders() {
    const response = await apiClient.reloadDdnsProviders();
    if (response.success) {
      setNotice("DDNS providers reloaded.");
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">DDNS</h1>
            <p className="mt-1 text-sm text-gray-500">Provider profiles for agent IP updates.</p>
          </div>
          <div className="flex gap-2">
            <button type="button" onClick={() => void reloadProviders()} className={buttonClass()}>
              Reload
            </button>
            <button type="button" onClick={openCreate} className={buttonClass("primary")}>
              Add Profile
            </button>
          </div>
        </div>

        {!isAdmin(user) ? (
          <div className="mb-4">
            <InlineNotice tone="yellow">The current DDNS API requires an admin session.</InlineNotice>
          </div>
        ) : null}

        <div className="mb-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search DDNS profiles" className={inputClass} />
          <button type="button" onClick={() => void loadConfigs()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <div className="rounded bg-white p-6 text-sm text-gray-600 shadow">Loading DDNS profiles...</div>
        ) : filtered.length === 0 ? (
          <EmptyState title="No DDNS profiles configured" detail="Create a provider profile and reload providers to apply it without restarting." />
        ) : (
          <>
            <div className="hidden overflow-hidden rounded-lg bg-white shadow md:block">
              <table className="min-w-full divide-y divide-gray-200">
                <thead className="bg-gray-50">
                  <tr>
                    {["Name", "Provider", "Domain", "Agent", "Last IP", "State", "Actions"].map((heading) => (
                      <th key={heading} className="px-6 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                        {heading}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 bg-white">
                  {filtered.map((config) => (
                    <tr key={config.id} className="hover:bg-gray-50">
                      <td className="px-6 py-4">
                        <div className="text-sm font-medium text-gray-900">{config.name}</div>
                        <div className="text-xs text-gray-500">{config.id}</div>
                      </td>
                      <td className="px-6 py-4 text-sm text-gray-500">{providerLabel(config.provider)}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{config.domain}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{config.agent_id ? serverName(config.agent_id, servers) : "Any agent"}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">
                        {config.last_applied_ip || config.current_ip || "-"}
                        <div className="text-xs">{formatDate(config.last_applied_at)}</div>
                      </td>
                      <td className="px-6 py-4">{config.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}</td>
                      <td className="px-6 py-4 text-sm">
                        <div className="flex gap-2">
                          <button type="button" onClick={() => void openHistory(config)} className="text-gray-700 hover:underline">
                            History
                          </button>
                          <button type="button" onClick={() => setNotice("This backend build exposes create/delete/reload for DDNS, but not update.")} className="text-blue-700 hover:underline">
                            Edit
                          </button>
                          <button type="button" onClick={() => void deleteConfig(config)} className="text-red-700 hover:underline">
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
              {filtered.map((config) => (
                <div key={config.id} className="rounded-lg bg-white p-4 shadow">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h2 className="font-semibold text-gray-900">{config.name}</h2>
                      <p className="mt-1 text-sm text-gray-500">{config.domain}</p>
                    </div>
                    {config.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}
                  </div>
                  <div className="mt-3 text-sm text-gray-600">{providerLabel(config.provider)} · {config.last_applied_ip || config.current_ip || "No IP yet"}</div>
                  <div className="mt-4 grid grid-cols-2 gap-2">
                    <button type="button" onClick={() => void openHistory(config)} className={buttonClass()}>
                      History
                    </button>
                    <button type="button" onClick={() => void deleteConfig(config)} className={buttonClass("danger")}>
                      Delete
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </PageShell>

      {modal === "create" ? (
        <Modal title="Add DDNS Profile" onClose={() => setModal(null)}>
          <form onSubmit={submitForm} className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Name" error={formErrors.name}>
                <input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="Provider">
                <select value={form.provider} onChange={(event) => setForm((prev) => ({ ...prev, provider: event.target.value as Provider }))} className={selectClass}>
                  <option value="dummy">Dummy</option>
                  <option value="cloudflare">Cloudflare</option>
                  <option value="tencent_cloud">Tencent Cloud</option>
                  <option value="he">Hurricane Electric</option>
                  <option value="webhook">Webhook</option>
                </select>
              </Field>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Domain" error={formErrors.domain}>
                <input value={form.domain} onChange={(event) => setForm((prev) => ({ ...prev, domain: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="Agent">
                <select value={form.agent_id} onChange={(event) => setForm((prev) => ({ ...prev, agent_id: event.target.value }))} className={selectClass}>
                  <option value="">Any agent</option>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name} ({server.status})
                    </option>
                  ))}
                </select>
              </Field>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Record ID">
                <input value={form.record_id} onChange={(event) => setForm((prev) => ({ ...prev, record_id: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="Zone ID">
                <input value={form.zone_id} onChange={(event) => setForm((prev) => ({ ...prev, zone_id: event.target.value }))} className={inputClass} />
              </Field>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="API Token">
                <input type="password" value={form.api_token} onChange={(event) => setForm((prev) => ({ ...prev, api_token: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="Webhook URL" error={formErrors.webhook_url}>
                <input value={form.webhook_url} onChange={(event) => setForm((prev) => ({ ...prev, webhook_url: event.target.value }))} className={inputClass} />
              </Field>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="API Key">
                <input type="password" value={form.api_key} onChange={(event) => setForm((prev) => ({ ...prev, api_key: event.target.value }))} className={inputClass} />
              </Field>
              <Field label="API Secret">
                <input type="password" value={form.api_secret} onChange={(event) => setForm((prev) => ({ ...prev, api_secret: event.target.value }))} className={inputClass} />
              </Field>
            </div>

            <label className="flex items-center gap-2 text-sm text-gray-700">
              <input type="checkbox" checked={form.enabled} onChange={(event) => setForm((prev) => ({ ...prev, enabled: event.target.checked }))} className="rounded border-gray-300 text-blue-600 focus:ring-blue-500" />
              Enabled
            </label>

            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                Cancel
              </button>
              <button type="submit" disabled={saving} className={buttonClass("primary")}>
                {saving ? "Saving..." : "Create Profile"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {modal === "history" && selected ? (
        <Modal title={`DDNS History: ${selected.name}`} onClose={() => setModal(null)}>
          {history.length === 0 ? (
            <EmptyState title="No history recorded" />
          ) : (
            <div className="space-y-3">
              {history.map((item) => (
                <div key={item.id} className="rounded border border-gray-200 p-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="text-sm font-medium text-gray-900">
                      <span>{item.old_ip || "-"}</span>
                      <span className="px-1 text-gray-500">to</span>
                      <span>{item.new_ip}</span>
                    </div>
                    {item.success ? <StatusBadge tone="green">Success</StatusBadge> : <StatusBadge tone="red">Failed</StatusBadge>}
                  </div>
                  <div className="mt-2 text-sm text-gray-500">{formatDate(item.applied_at)}</div>
                  {item.error ? <div className="mt-2 text-sm text-red-700">{item.error}</div> : null}
                </div>
              ))}
            </div>
          )}
        </Modal>
      ) : null}
    </div>
  );
}

function validateForm(form: DdnsForm): Partial<Record<keyof DdnsForm, string>> {
  const errors: Partial<Record<keyof DdnsForm, string>> = {};
  if (!form.name.trim()) {
    errors.name = "Name is required";
  }
  if (!form.domain.trim()) {
    errors.domain = "Domain is required";
  }
  if (form.provider === "webhook" && !/^https?:\/\//i.test(form.webhook_url.trim())) {
    errors.webhook_url = "Webhook URL must start with http:// or https://";
  }
  return errors;
}

function formToPayload(form: DdnsForm): JsonObject {
  return {
    name: form.name.trim(),
    provider: form.provider,
    domain: form.domain.trim(),
    agent_id: form.agent_id || null,
    record_id: form.record_id.trim() || null,
    zone_id: form.zone_id.trim() || null,
    api_token: form.api_token || null,
    api_key: form.api_key || null,
    api_secret: form.api_secret || null,
    webhook_url: form.webhook_url.trim() || null,
    enabled: form.enabled,
  };
}

function providerLabel(value: string): string {
  return value.replace(/_/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function serverName(id: string, servers: Server[]): string {
  const server = servers.find((item) => item.id === id);
  return server ? `${server.name} (${server.status})` : compactId(id);
}
