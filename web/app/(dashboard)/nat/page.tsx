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
  inputClass,
  responseError,
  selectClass,
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
  local_host: string;
  local_port: number;
  public_port: number;
  protocol: "tcp" | "udp" | string;
  enabled: boolean;
  description?: string | null;
}

interface NatForm {
  agent_id: string;
  local_host: string;
  local_port: string;
  public_port: string;
  protocol: "tcp" | "udp";
  description: string;
  enabled: boolean;
}

const blankForm: NatForm = {
  agent_id: "",
  local_host: "127.0.0.1",
  local_port: "8080",
  public_port: "18080",
  protocol: "tcp",
  description: "",
  enabled: true,
};

export default function NatPage() {
  const [mappings, setMappings] = useState<NatMapping[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [modal, setModal] = useState<"create" | "edit" | null>(null);
  const [editing, setEditing] = useState<NatMapping | null>(null);
  const [form, setForm] = useState<NatForm>(blankForm);
  const [formErrors, setFormErrors] = useState<Partial<Record<keyof NatForm, string>>>({});

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    const [mappingResponse, serverResponse] = await Promise.all([
      apiClient.listNatMappings(),
      apiClient.listServers(200, 0),
    ]);

    if (mappingResponse.success && mappingResponse.data) {
      setMappings((mappingResponse.data.mappings as NatMapping[]) ?? []);
    } else {
      setError(responseError(mappingResponse));
    }

    if (serverResponse.success && serverResponse.data) {
      const loadedServers = (serverResponse.data.servers as Server[]) ?? [];
      setServers(loadedServers);
      setForm((prev) => ({
        ...prev,
        agent_id: prev.agent_id || loadedServers[0]?.id || "",
      }));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadData();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadData]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return mappings;
    }
    return mappings.filter((mapping) =>
      [
        mapping.agent_id,
        serverName(mapping.agent_id, servers),
        mapping.local_host,
        mapping.local_port,
        mapping.public_port,
        mapping.protocol,
        mapping.description,
      ]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [mappings, query, servers]);

  function openCreate() {
    setEditing(null);
    setForm({ ...blankForm, agent_id: servers[0]?.id || "" });
    setFormErrors({});
    setModal("create");
  }

  function openEdit(mapping: NatMapping) {
    setEditing(mapping);
    setForm({
      agent_id: mapping.agent_id,
      local_host: mapping.local_host,
      local_port: String(mapping.local_port),
      public_port: String(mapping.public_port),
      protocol: mapping.protocol === "udp" ? "udp" : "tcp",
      description: mapping.description || "",
      enabled: Boolean(mapping.enabled),
    });
    setFormErrors({});
    setModal("edit");
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    const validation = validateNatForm(form, modal === "edit");
    setFormErrors(validation);
    if (Object.keys(validation).length > 0) {
      return;
    }

    setSaving(true);
    setError(null);
    const payload = formToPayload(form, modal === "edit");
    const response =
      modal === "edit" && editing
        ? await apiClient.updateNatMapping(editing.id, payload)
        : await apiClient.createNatMapping(payload);
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice(modal === "edit" ? "NAT mapping updated." : "NAT mapping created.");
      await loadData();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteMapping(mapping: NatMapping) {
    if (!confirm(`Delete NAT mapping :${mapping.public_port}?`)) {
      return;
    }
    const response = await apiClient.deleteNatMapping(mapping.id);
    if (response.success) {
      setNotice("NAT mapping deleted.");
      await loadData();
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
            <h1 className="text-2xl font-bold text-gray-900">NAT</h1>
            <p className="mt-1 text-sm text-gray-500">Port mappings for internal services.</p>
          </div>
          <button type="button" onClick={openCreate} className={buttonClass("primary")}>
            Add Mapping
          </button>
        </div>

        <div className="mb-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search NAT mappings"
            className={inputClass}
          />
          <button type="button" onClick={() => void loadData()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
          {servers.length === 0 ? (
            <InlineNotice tone="yellow">No accessible servers were returned. NAT mappings need an agent target.</InlineNotice>
          ) : null}
        </div>

        {loading ? (
          <div className="rounded bg-white p-6 text-sm text-gray-600 shadow">Loading NAT mappings...</div>
        ) : filtered.length === 0 ? (
          <EmptyState title="No NAT mappings configured" detail="Create a TCP mapping to expose an internal service." />
        ) : (
          <>
            <div className="hidden overflow-hidden rounded-lg bg-white shadow md:block">
              <table className="min-w-full divide-y divide-gray-200">
                <thead className="bg-gray-50">
                  <tr>
                    {["Public", "Target", "Agent", "Protocol", "State", "Description", "Actions"].map((heading) => (
                      <th key={heading} className="px-6 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500">
                        {heading}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 bg-white">
                  {filtered.map((mapping) => (
                    <tr key={mapping.id} className="hover:bg-gray-50">
                      <td className="px-6 py-4 text-sm font-medium text-gray-900">:{mapping.public_port}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{mapping.local_host}:{mapping.local_port}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">
                        <div>{serverName(mapping.agent_id, servers)}</div>
                        <div className="text-xs text-gray-400">{compactId(mapping.agent_id)}</div>
                      </td>
                      <td className="px-6 py-4 text-sm uppercase text-gray-500">{mapping.protocol}</td>
                      <td className="px-6 py-4">
                        {mapping.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}
                      </td>
                      <td className="px-6 py-4 text-sm text-gray-500">{mapping.description || "-"}</td>
                      <td className="px-6 py-4 text-sm">
                        <div className="flex gap-3">
                          <button type="button" onClick={() => openEdit(mapping)} className="text-blue-700 hover:underline">
                            Edit
                          </button>
                          <button type="button" onClick={() => void deleteMapping(mapping)} className="text-red-700 hover:underline">
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
              {filtered.map((mapping) => (
                <div key={mapping.id} className="rounded-lg bg-white p-4 shadow">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h2 className="font-semibold text-gray-900">:{mapping.public_port}</h2>
                      <p className="mt-1 text-sm text-gray-500">{mapping.local_host}:{mapping.local_port} · {mapping.protocol.toUpperCase()}</p>
                    </div>
                    {mapping.enabled ? <StatusBadge tone="green">Enabled</StatusBadge> : <StatusBadge tone="gray">Disabled</StatusBadge>}
                  </div>
                  <p className="mt-3 text-sm text-gray-600">{serverName(mapping.agent_id, servers)}</p>
                  {mapping.description ? <p className="mt-2 text-sm text-gray-500">{mapping.description}</p> : null}
                  <div className="mt-4 grid grid-cols-2 gap-2">
                    <button type="button" onClick={() => openEdit(mapping)} className={buttonClass()}>
                      Edit
                    </button>
                    <button type="button" onClick={() => void deleteMapping(mapping)} className={buttonClass("danger")}>
                      Delete
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {modal ? (
          <Modal title={modal === "edit" ? "Edit NAT Mapping" : "Add NAT Mapping"} onClose={() => setModal(null)}>
            <form onSubmit={(event) => void submitForm(event)} className="space-y-4">
              <Field label="Agent" error={formErrors.agent_id}>
                <select
                  value={form.agent_id}
                  onChange={(event) => setForm((prev) => ({ ...prev, agent_id: event.target.value }))}
                  className={selectClass}
                  disabled={modal === "edit"}
                >
                  <option value="">Select server</option>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name} ({server.status})
                    </option>
                  ))}
                </select>
              </Field>
              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Local Host" error={formErrors.local_host}>
                  <input value={form.local_host} onChange={(event) => setForm((prev) => ({ ...prev, local_host: event.target.value }))} className={inputClass} />
                </Field>
                <Field label="Protocol" error={formErrors.protocol}>
                  <select value={form.protocol} onChange={(event) => setForm((prev) => ({ ...prev, protocol: event.target.value as "tcp" | "udp" }))} className={selectClass}>
                    <option value="tcp">TCP</option>
                    <option value="udp">UDP</option>
                  </select>
                </Field>
                <Field label="Local Port" error={formErrors.local_port}>
                  <input type="number" min="1" max="65535" value={form.local_port} onChange={(event) => setForm((prev) => ({ ...prev, local_port: event.target.value }))} className={inputClass} />
                </Field>
                <Field label="Public Port" error={formErrors.public_port}>
                  <input type="number" min="1" max="65535" value={form.public_port} onChange={(event) => setForm((prev) => ({ ...prev, public_port: event.target.value }))} className={inputClass} />
                </Field>
              </div>
              <Field label="Description">
                <input value={form.description} onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))} className={inputClass} />
              </Field>
              {modal === "edit" ? (
                <label className="flex items-center gap-2 text-sm text-gray-700">
                  <input type="checkbox" checked={form.enabled} onChange={(event) => setForm((prev) => ({ ...prev, enabled: event.target.checked }))} />
                  Enabled
                </label>
              ) : null}
              {form.protocol === "udp" ? <InlineNotice tone="yellow">The current tunnel manager only starts TCP listeners. UDP mappings can be saved for future use.</InlineNotice> : null}
              <div className="flex justify-end gap-3 border-t border-gray-200 pt-4">
                <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                  Cancel
                </button>
                <button type="submit" disabled={saving} className={buttonClass("primary")}>
                  {saving ? "Saving..." : modal === "edit" ? "Save" : "Create"}
                </button>
              </div>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}

function serverName(agentId: string, servers: Server[]) {
  return servers.find((server) => server.id === agentId)?.name || compactId(agentId);
}

function validateNatForm(form: NatForm, editing: boolean): Partial<Record<keyof NatForm, string>> {
  const errors: Partial<Record<keyof NatForm, string>> = {};
  if (!editing && !form.agent_id) {
    errors.agent_id = "Select an agent.";
  }
  if (!form.local_host.trim()) {
    errors.local_host = "Local host is required.";
  }
  const localPort = Number(form.local_port);
  const publicPort = Number(form.public_port);
  if (!Number.isInteger(localPort) || localPort < 1 || localPort > 65535) {
    errors.local_port = "Use a port from 1 to 65535.";
  }
  if (!Number.isInteger(publicPort) || publicPort < 1 || publicPort > 65535) {
    errors.public_port = "Use a port from 1 to 65535.";
  }
  return errors;
}

function formToPayload(form: NatForm, editing: boolean): JsonObject {
  const payload: JsonObject = {
    local_host: form.local_host.trim(),
    local_port: Number(form.local_port),
    public_port: Number(form.public_port),
    protocol: form.protocol,
    description: form.description.trim() || null,
  };
  if (!editing) {
    payload.agent_id = form.agent_id;
  } else {
    payload.enabled = form.enabled;
  }
  return payload;
}
