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
  formatDate,
  inputClass,
  responseError,
  selectClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

type ServiceKind = "http" | "tcp" | "icmp";

interface Service {
  id: string;
  name: string;
  kind?: string;
  type?: string;
  service_type?: string;
  target: string;
  interval_seconds?: number;
  timeout_seconds?: number;
  enabled?: boolean;
  last_status?: string;
  last_check_at?: string;
  cert_fingerprint?: string;
  cert_not_after?: string;
  updated_at?: string;
}

interface ServiceForm {
  name: string;
  service_type: ServiceKind;
  target: string;
  interval_seconds: string;
  timeout_seconds: string;
  enabled: boolean;
}

const blankForm: ServiceForm = {
  name: "",
  service_type: "http",
  target: "",
  interval_seconds: "60",
  timeout_seconds: "10",
  enabled: true,
};

export default function ServicesPage() {
  const [services, setServices] = useState<Service[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "edit" | null>(null);
  const [editing, setEditing] = useState<Service | null>(null);
  const [form, setForm] = useState<ServiceForm>(blankForm);
  const [formErrors, setFormErrors] = useState<Partial<Record<keyof ServiceForm, string>>>({});
  const [probeResult, setProbeResult] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const loadServices = useCallback(async () => {
    setError(null);
    setNotice(null);
    try {
      setLoading(true);
      const response = await apiClient.listServices();
      if (response.success && response.data) {
        setServices((response.data.services as Service[]) ?? []);
      } else {
        setError(responseError(response));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServices();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServices]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return services;
    }
    return services.filter((service) =>
      [service.name, service.target, serviceKind(service), service.last_status]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [query, services]);

  function openCreate() {
    setEditing(null);
    setForm(blankForm);
    setFormErrors({});
    setProbeResult(null);
    setModal("create");
  }

  function openEdit(service: Service) {
    setEditing(service);
    setForm({
      name: service.name || "",
      service_type: normalizeKind(serviceKind(service)),
      target: service.target || "",
      interval_seconds: String(service.interval_seconds ?? 60),
      timeout_seconds: String(service.timeout_seconds ?? 10),
      enabled: service.enabled ?? true,
    });
    setFormErrors({});
    setProbeResult(null);
    setModal("edit");
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    setProbeResult(null);
    setNotice(null);
    const validation = validateForm(form);
    setFormErrors(validation);
    if (Object.keys(validation).length > 0) {
      return;
    }

    setSaving(true);
    const payload = formToPayload(form);
    const response =
      modal === "edit" && editing
        ? await apiClient.updateService(editing.id, payload)
        : await apiClient.createService(payload);
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice(modal === "edit" ? "Service updated." : "Service created.");
      await loadServices();
    } else {
      setError(responseError(response));
      if (response.status === 404 || response.status === 405) {
        setNotice("This server build does not expose service CRUD routes yet. Probe testing is available.");
      }
    }
  }

  async function testProbe() {
    const validation = validateForm(form, true);
    setFormErrors(validation);
    setProbeResult(null);
    if (Object.keys(validation).length > 0) {
      return;
    }

    setTesting(true);
    const response = await apiClient.testProbe({
      service_type: form.service_type,
      target: form.target.trim(),
      timeout_seconds: Number(form.timeout_seconds),
    });
    setTesting(false);

    if (response.success && response.data) {
      const status = response.data.success ? "success" : "failure";
      const latency = response.data.latency_ms ? `, ${response.data.latency_ms} ms` : "";
      const code = response.data.status_code ? `, HTTP ${response.data.status_code}` : "";
      const reason = response.data.error ? `, ${response.data.error}` : "";
      const cert = response.data.cert_not_after ? `, TLS expires ${formatDate(response.data.cert_not_after)}` : "";
      setProbeResult(`Probe ${status}${latency}${code}${cert}${reason}`);
    } else {
      setProbeResult(responseError(response));
    }
  }

  async function deleteService(service: Service) {
    if (!confirm(`Delete service "${service.name}"? This removes its monitor configuration.`)) {
      return;
    }

    const response = await apiClient.deleteService(service.id);
    if (response.success) {
      setNotice("Service deleted.");
      await loadServices();
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
            <h1 className="text-2xl font-bold text-gray-900">Services</h1>
            <p className="mt-1 text-sm text-gray-500">
              Availability monitors and probe checks.
            </p>
          </div>
          <button type="button" onClick={openCreate} className={buttonClass("primary")}>
            Add Service
          </button>
        </div>

        <div className="mb-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-center">
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search services"
            className={inputClass}
          />
          <button type="button" onClick={() => void loadServices()} className={buttonClass()}>
            Refresh
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <div className="rounded bg-white p-6 text-sm text-gray-600 shadow">Loading services...</div>
        ) : filtered.length === 0 ? (
          <EmptyState
            title="No services configured"
            detail="Add a monitor or test a probe before saving it."
          />
        ) : (
          <>
            <div className="hidden overflow-hidden rounded-lg bg-white shadow md:block">
              <table className="min-w-full divide-y divide-gray-200">
                <thead className="bg-gray-50">
                  <tr>
                    {["Name", "Type", "Target", "Interval", "Status", "Last Check", "Actions"].map((heading) => (
                      <th
                        key={heading}
                        className="px-6 py-3 text-left text-xs font-medium uppercase tracking-wider text-gray-500"
                      >
                        {heading}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-200 bg-white">
                  {filtered.map((service) => (
                    <tr key={service.id} className="hover:bg-gray-50">
                      <td className="px-6 py-4">
                        <div className="text-sm font-medium text-gray-900">{service.name}</div>
                        <div className="text-xs text-gray-500">{service.id}</div>
                      </td>
                      <td className="px-6 py-4 text-sm text-gray-500 uppercase">{serviceKind(service)}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">
                        <div className="break-all">{service.target}</div>
                        {service.cert_not_after ? (
                          <div className="mt-1 text-xs text-gray-500">TLS expires {formatDate(service.cert_not_after)}</div>
                        ) : null}
                      </td>
                      <td className="px-6 py-4 text-sm text-gray-500">{service.interval_seconds ?? 60}s</td>
                      <td className="px-6 py-4">{serviceBadge(service)}</td>
                      <td className="px-6 py-4 text-sm text-gray-500">{formatDate(service.last_check_at || service.updated_at)}</td>
                      <td className="px-6 py-4 text-sm">
                        <div className="flex gap-2">
                          <button type="button" onClick={() => openEdit(service)} className="text-blue-700 hover:underline">
                            Edit
                          </button>
                          <button type="button" onClick={() => void deleteService(service)} className="text-red-700 hover:underline">
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
              {filtered.map((service) => (
                <div key={service.id} className="rounded-lg bg-white p-4 shadow">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <h2 className="font-semibold text-gray-900">{service.name}</h2>
                      <p className="mt-1 break-all text-sm text-gray-500">{service.target}</p>
                      {service.cert_not_after ? (
                        <p className="mt-1 text-xs text-gray-500">TLS expires {formatDate(service.cert_not_after)}</p>
                      ) : null}
                    </div>
                    {serviceBadge(service)}
                  </div>
                  <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
                    <div>
                      <div className="text-xs uppercase text-gray-500">Type</div>
                      <div className="font-medium uppercase text-gray-800">{serviceKind(service)}</div>
                    </div>
                    <div>
                      <div className="text-xs uppercase text-gray-500">Interval</div>
                      <div className="font-medium text-gray-800">{service.interval_seconds ?? 60}s</div>
                    </div>
                  </div>
                  <div className="mt-4 flex gap-2">
                    <button type="button" onClick={() => openEdit(service)} className={buttonClass()}>
                      Edit
                    </button>
                    <button type="button" onClick={() => void deleteService(service)} className={buttonClass("danger")}>
                      Delete
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </PageShell>

      {modal ? (
        <Modal title={modal === "edit" ? "Edit Service" : "Add Service"} onClose={() => setModal(null)}>
          <form onSubmit={submitForm} className="space-y-4">
            <Field label="Name" error={formErrors.name}>
              <input
                value={form.name}
                onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
                className={inputClass}
              />
            </Field>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Probe Type" error={formErrors.service_type}>
                <select
                  value={form.service_type}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, service_type: event.target.value as ServiceKind }))
                  }
                  className={selectClass}
                >
                  <option value="http">HTTP</option>
                  <option value="tcp">TCP</option>
                  <option value="icmp">ICMP</option>
                </select>
              </Field>
              <Field label="Target" error={formErrors.target}>
                <input
                  value={form.target}
                  onChange={(event) => setForm((prev) => ({ ...prev, target: event.target.value }))}
                  className={inputClass}
                  placeholder={form.service_type === "tcp" ? "host:port" : "https://example.com"}
                />
              </Field>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <Field label="Interval Seconds" error={formErrors.interval_seconds}>
                <input
                  type="number"
                  min={10}
                  value={form.interval_seconds}
                  onChange={(event) => setForm((prev) => ({ ...prev, interval_seconds: event.target.value }))}
                  className={inputClass}
                />
              </Field>
              <Field label="Timeout Seconds" error={formErrors.timeout_seconds}>
                <input
                  type="number"
                  min={1}
                  value={form.timeout_seconds}
                  onChange={(event) => setForm((prev) => ({ ...prev, timeout_seconds: event.target.value }))}
                  className={inputClass}
                />
              </Field>
            </div>

            <label className="flex items-center gap-2 text-sm text-gray-700">
              <input
                type="checkbox"
                checked={form.enabled}
                onChange={(event) => setForm((prev) => ({ ...prev, enabled: event.target.checked }))}
                className="rounded border-gray-300 text-blue-600 focus:ring-blue-500"
              />
              Enabled
            </label>

            {probeResult ? <InlineNotice tone={probeResult.includes("success") ? "green" : "yellow"}>{probeResult}</InlineNotice> : null}

            <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <button type="button" onClick={testProbe} disabled={testing || saving} className={buttonClass()}>
                {testing ? "Testing..." : "Test Probe"}
              </button>
              <button type="submit" disabled={saving || testing} className={buttonClass("primary")}>
                {saving ? "Saving..." : modal === "edit" ? "Save Changes" : "Create Service"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}
    </div>
  );
}

function serviceKind(service: Service): string {
  return service.service_type || service.kind || service.type || "http";
}

function normalizeKind(value: string): ServiceKind {
  return value === "tcp" || value === "icmp" ? value : "http";
}

function serviceBadge(service: Service) {
  if (service.enabled === false) {
    return <StatusBadge tone="gray">Disabled</StatusBadge>;
  }
  if (service.last_status === "success" || service.last_status === "up") {
    return <StatusBadge tone="green">Up</StatusBadge>;
  }
  if (service.last_status === "failure" || service.last_status === "down") {
    return <StatusBadge tone="red">Down</StatusBadge>;
  }
  return <StatusBadge tone="yellow">Unknown</StatusBadge>;
}

function validateForm(form: ServiceForm, probeOnly = false): Partial<Record<keyof ServiceForm, string>> {
  const errors: Partial<Record<keyof ServiceForm, string>> = {};
  if (!probeOnly && !form.name.trim()) {
    errors.name = "Name is required";
  }
  if (!form.target.trim()) {
    errors.target = "Target is required";
  } else if (form.service_type === "http" && !/^https?:\/\//i.test(form.target.trim())) {
    errors.target = "HTTP targets must start with http:// or https://";
  } else if (form.service_type === "tcp" && !/^[^:]+:\d+$/.test(form.target.trim())) {
    errors.target = "TCP targets must be host:port";
  }

  const interval = Number(form.interval_seconds);
  if (!Number.isFinite(interval) || interval < 10) {
    errors.interval_seconds = "Use 10 seconds or more";
  }

  const timeout = Number(form.timeout_seconds);
  if (!Number.isFinite(timeout) || timeout < 1) {
    errors.timeout_seconds = "Use at least 1 second";
  }

  return errors;
}

function formToPayload(form: ServiceForm): JsonObject {
  return {
    name: form.name.trim(),
    service_type: form.service_type,
    target: form.target.trim(),
    interval_seconds: Number(form.interval_seconds),
    timeout_seconds: Number(form.timeout_seconds),
    enabled: form.enabled,
  };
}
