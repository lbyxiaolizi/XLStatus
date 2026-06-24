"use client";

import { FormEvent, useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
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
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";
import type { servicesPage } from "@/lib/i18n/pages/services";

type ServiceKind = "http" | "tcp" | "icmp";
type ServiceCoverMode = "local" | "all" | "specific" | "exclude";

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
  server_id?: string | null;
  server_ids?: string[];
  cover_mode?: string;
  exclude_server_ids?: string[];
  failure_task_ids?: string[];
  recovery_task_ids?: string[];
  config_warnings?: string[];
  last_status?: string;
  last_check_at?: string;
  cert_fingerprint?: string;
  cert_not_after?: string;
}

interface ServiceForm {
  name: string;
  service_type: ServiceKind;
  target: string;
  cover_mode: ServiceCoverMode;
  server_ids: string[];
  exclude_server_ids: string[];
  interval_seconds: string;
  timeout_seconds: string;
  enabled: boolean;
  failure_task_ids: string;
  recovery_task_ids: string;
}

interface ServerOption {
  id: string;
  name: string;
}

const blankForm: ServiceForm = {
  name: "",
  service_type: "http",
  target: "",
  cover_mode: "local",
  server_ids: [],
  exclude_server_ids: [],
  interval_seconds: "60",
  timeout_seconds: "10",
  enabled: true,
  failure_task_ids: "",
  recovery_task_ids: "",
};

export default function ServicesPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [services, setServices] = useState<Service[]>([]);
  const [servers, setServers] = useState<ServerOption[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"create" | "edit" | null>(null);
  const [editing, setEditing] = useState<Service | null>(null);
  const [form, setForm] = useState<ServiceForm>(blankForm);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
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

  const loadServers = useCallback(async () => {
    const response = await apiClient.listServers(200, 0);
    if (response.success && response.data) {
      setServers((response.data.servers as ServerOption[]) ?? []);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadServices();
    void loadServers();
  }, [loadServices, loadServers]);

  const deferredQuery = useDeferredValue(query);
  const filtered = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    if (!needle) return services;
    return services.filter((service) =>
      [
        service.name,
        service.target,
        serviceKind(copy, service),
        service.last_status,
        ...serviceConfigWarnings(service),
        ...serviceServerIds(service),
        serviceServerLabel(copy, servers, service),
      ]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [deferredQuery, services, servers, copy]);

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
      service_type: normalizeKind(serviceKind(copy, service)),
      target: service.target || "",
      cover_mode: normalizeCoverMode(service.cover_mode, serviceServerIds(service)),
      server_ids: serviceServerIds(service),
      exclude_server_ids: service.exclude_server_ids ?? [],
      interval_seconds: String(service.interval_seconds ?? 60),
      timeout_seconds: String(service.timeout_seconds ?? 10),
      enabled: service.enabled ?? true,
      failure_task_ids: (service.failure_task_ids ?? []).join(", "),
      recovery_task_ids: (service.recovery_task_ids ?? []).join(", "),
    });
    setProbeResult(null);
    setModal("edit");
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    const payload = formToPayload(form);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) {
      setSaving(false);
      return;
    }
    const response =
      modal === "edit" && editing
        ? await apiClient.updateService(editing.id, payload, totpCode)
        : await apiClient.createService(payload, totpCode);
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice(modal === "edit" ? copy.servicesPage.noticeUpdated : copy.servicesPage.noticeCreated);
      await loadServices();
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
      setError(copy.servicesPage.totpInvalid);
      return null;
    }
    return trimmed;
  }

  async function testProbe() {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setTesting(true);
    const response = await apiClient.testProbe({
      service_type: form.service_type,
      target: form.target.trim(),
      timeout_seconds: Number(form.timeout_seconds),
    }, totpCode);
    setTesting(false);
    if (response.success && response.data) {
      const status = response.data.success ? copy.servicesPage.probeSuccess : copy.servicesPage.probeFailure;
      const latency = response.data.latency_ms ? `, ${response.data.latency_ms} ms` : "";
      const code = response.data.status_code ? `, HTTP ${response.data.status_code}` : "";
      const reason = response.data.error ? `, ${response.data.error}` : "";
      const cert = response.data.cert_not_after
        ? copy.servicesPage.certExpiry.replace("{date}", String(formatDate(response.data.cert_not_after)))
        : "";
      setProbeResult(
        copy.servicesPage.probeResult
          .replace("{status}", status)
          .replace("{latency}", latency)
          .replace("{code}", code)
          .replace("{cert}", cert)
          .replace("{reason}", reason),
      );
    } else {
      setProbeResult(responseError(response));
    }
  }

  async function deleteService(service: Service) {
    if (!(await dialogs.confirm({ message: copy.servicesPage.deleteConfirm.replace("{name}", String(service.name)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteService(service.id, totpCode);
    if (response.success) {
      setNotice(copy.servicesPage.noticeDeleted);
      await loadServices();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.servicesPage.eyebrow}
          title={copy.servicesPage.title}
          detail={copy.servicesPage.detail}
          actions={<button type="button" onClick={openCreate} className={buttonClass("primary")}>{copy.servicesPage.newService}</button>}
        />
        <div className="mb-5 space-y-3">
          <input value={query} onChange={(event) => setQuery(event.target.value)} className={inputClass} placeholder={copy.servicesPage.searchPlaceholder} />
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <BrutalCard>{copy.servicesPage.loading}</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title={copy.servicesPage.emptyTitle} detail={copy.servicesPage.emptyDetail} />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>{copy.servicesPage.colName}</th>
                  <th className={thClass}>{copy.servicesPage.colTarget}</th>
                  <th className={thClass}>{copy.servicesPage.colProbeServers}</th>
                  <th className={thClass}>{copy.servicesPage.colType}</th>
                  <th className={thClass}>{copy.servicesPage.colStatus}</th>
                  <th className={thClass}>{copy.servicesPage.colTls}</th>
                  <th className={thClass}>{copy.servicesPage.colActions}</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((service) => (
                  <tr key={service.id}>
                    <td className={tdClass}>
                      <div className="space-y-2">
                        <div>{service.name}</div>
                        {serviceConfigWarnings(service).length ? (
                          <div className="text-xs font-bold text-[var(--text-muted)]">
                            {serviceWarningText(copy, service)}
                          </div>
                        ) : null}
                      </div>
                    </td>
                    <td className={tdClass}>{service.target}</td>
                    <td className={tdClass}>{serviceServerLabel(copy, servers, service)}</td>
                    <td className={tdClass}>{serviceKind(copy, service)}</td>
                    <td className={tdClass}><StatusBadge tone={serviceTone(service.last_status)}>{statusLabel(copy.servicesPage, service.last_status)}</StatusBadge></td>
                    <td className={tdClass}>{service.cert_not_after ? formatDate(service.cert_not_after) : "N/A"}</td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("secondary")} onClick={() => openEdit(service)}>{copy.servicesPage.edit}</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteService(service)}>{copy.servicesPage.delete}</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal ? (
          <Modal title={modal === "edit" ? copy.servicesPage.modalTitleEdit : copy.servicesPage.modalTitleCreate} onClose={() => setModal(null)}>
            <form onSubmit={submitForm} className="space-y-4">
              {editing && serviceConfigWarnings(editing).length ? (
                <InlineNotice tone="yellow">{serviceWarningText(copy, editing)}</InlineNotice>
              ) : null}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.servicesPage.fieldName}>
                  <input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required />
                </Field>
                <Field label={copy.servicesPage.fieldType}>
                  <select className={selectClass} value={form.service_type} onChange={(e) => setForm((f) => ({ ...f, service_type: e.target.value as ServiceKind }))}>
                    <option value="http">HTTP</option>
                    <option value="tcp">TCP</option>
                    <option value="icmp">ICMP</option>
                  </select>
                </Field>
              </div>
              <Field label={copy.servicesPage.fieldTarget}>
                <input className={inputClass} value={form.target} onChange={(e) => setForm((f) => ({ ...f, target: e.target.value }))} required />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.servicesPage.fieldCoverMode}>
                  <select className={selectClass} value={form.cover_mode} onChange={(e) => setForm((f) => ({ ...f, cover_mode: e.target.value as ServiceCoverMode }))}>
                    <option value="local">{copy.servicesPage.coverLocal}</option>
                    <option value="all">{copy.servicesPage.coverAll}</option>
                    <option value="specific">{copy.servicesPage.coverSpecific}</option>
                    <option value="exclude">{copy.servicesPage.coverExclude}</option>
                  </select>
                </Field>
              </div>
              {form.cover_mode === "specific" ? (
                <Field label={copy.servicesPage.fieldSpecificServers}>
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, server_ids: serverIds }))}
                  />
                </Field>
              ) : null}
              {form.cover_mode === "exclude" ? (
                <Field label={copy.servicesPage.fieldExcludeServers}>
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.exclude_server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, exclude_server_ids: serverIds }))}
                  />
                </Field>
              ) : null}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.servicesPage.fieldInterval}>
                  <input className={inputClass} value={form.interval_seconds} onChange={(e) => setForm((f) => ({ ...f, interval_seconds: e.target.value }))} />
                </Field>
                <Field label={copy.servicesPage.fieldTimeout}>
                  <input className={inputClass} value={form.timeout_seconds} onChange={(e) => setForm((f) => ({ ...f, timeout_seconds: e.target.value }))} />
                </Field>
              </div>
              <label className="flex items-center gap-2 text-sm font-black">
                <input type="checkbox" checked={form.enabled} onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))} />
                {copy.servicesPage.fieldEnabled}
              </label>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.servicesPage.fieldFailureTaskIds}>
                  <input className={inputClass} value={form.failure_task_ids} onChange={(e) => setForm((f) => ({ ...f, failure_task_ids: e.target.value }))} />
                </Field>
                <Field label={copy.servicesPage.fieldRecoveryTaskIds}>
                  <input className={inputClass} value={form.recovery_task_ids} onChange={(e) => setForm((f) => ({ ...f, recovery_task_ids: e.target.value }))} />
                </Field>
              </div>
              {probeResult ? <InlineNotice tone="pink">{probeResult}</InlineNotice> : null}
              <div className="flex flex-wrap gap-2">
                <button type="submit" disabled={saving} className={buttonClass("primary")}>{saving ? copy.servicesPage.saving : copy.servicesPage.save}</button>
                <button type="button" disabled={testing} onClick={() => void testProbe()} className={buttonClass("secondary")}>{testing ? copy.servicesPage.testing : copy.servicesPage.testProbe}</button>
              </div>
            </form>
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function serviceKind(copy: Translations, service: Service): string {
  return service.service_type || service.kind || service.type || copy.servicesPage.defaultKind;
}

function statusLabel(copy: typeof servicesPage, status?: string): string {
  if (!status) return copy.statusUnknown;
  const labels: Record<string, string> = {
    success: copy.statusSuccess,
    up: copy.statusUp,
    failure: copy.statusFailure,
    down: copy.statusDown,
    timeout: copy.statusTimeout,
    degraded: copy.statusDegraded,
  };
  return labels[status] || status;
}

function normalizeKind(value: string): ServiceKind {
  return value === "tcp" || value === "icmp" ? value : "http";
}

function normalizeCoverMode(value: string | undefined, serverIds: string[]): ServiceCoverMode {
  if (value === "all" || value === "specific" || value === "exclude" || value === "local") return value;
  return serverIds.length ? "specific" : "local";
}

function formToPayload(form: ServiceForm): JsonObject {
  const serverIds = form.cover_mode === "specific" ? form.server_ids : [];
  return {
    name: form.name.trim(),
    service_type: form.service_type,
    target: form.target.trim(),
    cover_mode: form.cover_mode,
    server_id: serverIds[0] || null,
    server_ids: serverIds,
    exclude_server_ids: form.cover_mode === "exclude" ? form.exclude_server_ids : [],
    interval_seconds: Number(form.interval_seconds),
    timeout_seconds: Number(form.timeout_seconds),
    enabled: form.enabled,
    failure_task_ids: splitIds(form.failure_task_ids),
    recovery_task_ids: splitIds(form.recovery_task_ids),
  };
}

function splitIds(value: string): string[] {
  return value
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function ServerMultiPicker({
  servers,
  selected,
  onChange,
}: {
  servers: ServerOption[];
  selected: string[];
  onChange: (serverIds: string[]) => void;
}) {
  const { t: copy } = useI18n();
  return (
    <div className="space-y-3">
      <button
        type="button"
        onClick={() => onChange([])}
        className={`border-2 border-black px-3 py-2 text-left text-sm font-black shadow-[var(--shadow-brutal-sm)] ${
          selected.length === 0
            ? "bg-[var(--btn-bg)] text-[var(--btn-text)]"
            : "bg-[var(--accent-bg)] text-[var(--text-main)]"
        }`}
      >
        {copy.servicesPage.pickerMasterUnassigned}
      </button>
      <div className="grid max-h-56 gap-2 overflow-auto border-2 border-black bg-[var(--bg-page)] p-3">
        {servers.length === 0 ? (
          <div className="text-sm font-black text-[var(--text-muted)]">{copy.servicesPage.pickerNoServers}</div>
        ) : (
          servers.map((server) => {
            const checked = selected.includes(server.id);
            return (
              <label
                key={server.id}
                className={`flex cursor-pointer items-center gap-3 border-2 border-black px-3 py-2 text-sm font-black shadow-[var(--shadow-brutal-sm)] ${
                  checked
                    ? "bg-[var(--accent-color)] text-[var(--btn-text)]"
                    : "bg-[var(--bg-card)] text-[var(--text-main)]"
                }`}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => onChange(toggleServerId(selected, server.id))}
                  className="h-4 w-4 accent-black"
                />
                <span className="min-w-0 truncate">{server.name || server.id}</span>
              </label>
            );
          })
        )}
      </div>
    </div>
  );
}

function toggleServerId(selected: string[], serverId: string): string[] {
  return selected.includes(serverId)
    ? selected.filter((id) => id !== serverId)
    : [...selected, serverId];
}

function serviceServerIds(service: Service): string[] {
  const ids = Array.isArray(service.server_ids) ? service.server_ids : [];
  const all = [...ids, ...(service.server_id ? [service.server_id] : [])];
  return all.filter((id, index) => id && all.indexOf(id) === index);
}

function serviceConfigWarnings(service: Service): string[] {
  return Array.isArray(service.config_warnings)
    ? service.config_warnings.filter((warning) => typeof warning === "string" && warning.trim())
    : [];
}

function serviceWarningText(copy: Translations, service: Service): string {
  const warnings = serviceConfigWarnings(service);
  if (!warnings.length) return "";
  return `${copy.servicesPage.warningPrefix}${warnings.join("；")}`;
}

function serviceServerLabel(copy: Translations, servers: ServerOption[], service: Service): string {
  if (service.cover_mode === "all") return copy.servicesPage.serverLabelAllOnline;
  if (service.cover_mode === "exclude") {
    const excluded = service.exclude_server_ids ?? [];
    if (!excluded.length) return copy.servicesPage.serverLabelAllOnline;
    return `${copy.servicesPage.serverLabelExcludePrefix}${excluded.map((id) => serverName(servers, id)).join("、")}`;
  }
  const ids = serviceServerIds(service);
  if (!ids.length) return copy.servicesPage.serverLabelLocal;
  return ids.map((id) => serverName(servers, id)).join("、");
}

function serverName(servers: ServerOption[], id?: string | null): string {
  if (!id) return "";
  return servers.find((server) => server.id === id)?.name || id;
}

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}
