"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
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
  const [probeResult, setProbeResult] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const loadServices = useCallback(async () => {
    setError(null);
    setLoading(true);
    const response = await apiClient.listServices(200, 0);
    setLoading(false);
    if (response.success && response.data) {
      setServices((response.data.services as Service[]) ?? []);
    } else {
      setError(responseError(response));
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
    if (!needle) return services;
    return services.filter((service) =>
      [service.name, service.target, serviceKind(service), service.last_status]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [query, services]);

  function openCreate() {
    setEditing(null);
    setForm(blankForm);
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
    setProbeResult(null);
    setModal("edit");
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
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
    }
  }

  async function testProbe() {
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
    if (!confirm(`Delete service "${service.name}"?`)) return;
    const response = await apiClient.deleteService(service.id);
    if (response.success) {
      setNotice("Service deleted.");
      await loadServices();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="Monitoring"
          title="Services"
          detail="HTTP、TCP、ICMP 服务监控与探测测试。"
          actions={<button type="button" onClick={openCreate} className={buttonClass("primary")}>Add Service</button>}
        />
        <div className="mb-5 space-y-3">
          <input value={query} onChange={(event) => setQuery(event.target.value)} className={inputClass} placeholder="Search services" />
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <BrutalCard>Loading services...</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title="No services configured" detail="Add a monitor or test a probe before saving it." />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>Name</th>
                  <th className={thClass}>Target</th>
                  <th className={thClass}>Kind</th>
                  <th className={thClass}>Status</th>
                  <th className={thClass}>TLS</th>
                  <th className={thClass}>Actions</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((service) => (
                  <tr key={service.id}>
                    <td className={tdClass}>{service.name}</td>
                    <td className={tdClass}>{service.target}</td>
                    <td className={tdClass}>{serviceKind(service)}</td>
                    <td className={tdClass}><StatusBadge tone={serviceTone(service.last_status)}>{service.last_status || "unknown"}</StatusBadge></td>
                    <td className={tdClass}>{service.cert_not_after ? formatDate(service.cert_not_after) : "N/A"}</td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("secondary")} onClick={() => openEdit(service)}>Edit</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteService(service)}>Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal ? (
          <Modal title={modal === "edit" ? "Edit Service" : "Add Service"} onClose={() => setModal(null)}>
            <form onSubmit={submitForm} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Name">
                  <input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required />
                </Field>
                <Field label="Type">
                  <select className={selectClass} value={form.service_type} onChange={(e) => setForm((f) => ({ ...f, service_type: e.target.value as ServiceKind }))}>
                    <option value="http">HTTP</option>
                    <option value="tcp">TCP</option>
                    <option value="icmp">ICMP</option>
                  </select>
                </Field>
              </div>
              <Field label="Target">
                <input className={inputClass} value={form.target} onChange={(e) => setForm((f) => ({ ...f, target: e.target.value }))} required />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Interval seconds">
                  <input className={inputClass} value={form.interval_seconds} onChange={(e) => setForm((f) => ({ ...f, interval_seconds: e.target.value }))} />
                </Field>
                <Field label="Timeout seconds">
                  <input className={inputClass} value={form.timeout_seconds} onChange={(e) => setForm((f) => ({ ...f, timeout_seconds: e.target.value }))} />
                </Field>
              </div>
              <label className="flex items-center gap-2 text-sm font-black">
                <input type="checkbox" checked={form.enabled} onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))} />
                Enabled
              </label>
              {probeResult ? <InlineNotice tone="pink">{probeResult}</InlineNotice> : null}
              <div className="flex flex-wrap gap-2">
                <button type="submit" disabled={saving} className={buttonClass("primary")}>{saving ? "Saving..." : "Save"}</button>
                <button type="button" disabled={testing} onClick={() => void testProbe()} className={buttonClass("secondary")}>{testing ? "Testing..." : "Test Probe"}</button>
              </div>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}

function serviceKind(service: Service): string {
  return service.service_type || service.kind || service.type || "service";
}

function normalizeKind(value: string): ServiceKind {
  return value === "tcp" || value === "icmp" ? value : "http";
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

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}
