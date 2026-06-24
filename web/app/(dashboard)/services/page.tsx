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
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";

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

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return services;
    return services.filter((service) =>
      [
        service.name,
        service.target,
        serviceKind(service),
        service.last_status,
        ...serviceConfigWarnings(service),
        ...serviceServerIds(service),
        serviceServerLabel(servers, service),
      ]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    );
  }, [query, services, servers]);

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
      setNotice(modal === "edit" ? "服务已更新。" : "服务已创建。");
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
    const code = window.prompt("请输入 6 位 TOTP 验证码");
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError("请输入 6 位 TOTP 验证码。");
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
      const status = response.data.success ? "成功" : "失败";
      const latency = response.data.latency_ms ? `, ${response.data.latency_ms} ms` : "";
      const code = response.data.status_code ? `, HTTP ${response.data.status_code}` : "";
      const reason = response.data.error ? `, ${response.data.error}` : "";
      const cert = response.data.cert_not_after ? `, TLS 到期 ${formatDate(response.data.cert_not_after)}` : "";
      setProbeResult(`探测${status}${latency}${code}${cert}${reason}`);
    } else {
      setProbeResult(responseError(response));
    }
  }

  async function deleteService(service: Service) {
    if (!confirm(`确定删除服务「${service.name}」？`)) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteService(service.id, totpCode);
    if (response.success) {
      setNotice("服务已删除。");
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
          eyebrow="监控"
          title="服务"
          detail="HTTP、TCP、ICMP 服务监控与探测测试。"
          actions={<button type="button" onClick={openCreate} className={buttonClass("primary")}>新增服务</button>}
        />
        <div className="mb-5 space-y-3">
          <input value={query} onChange={(event) => setQuery(event.target.value)} className={inputClass} placeholder="搜索服务" />
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        {loading ? (
          <BrutalCard>正在加载服务...</BrutalCard>
        ) : filtered.length === 0 ? (
          <EmptyState title="暂无服务配置" detail="新增监控项或先执行一次探测测试。" />
        ) : (
          <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
            <table className="w-full">
              <thead>
                <tr>
                  <th className={thClass}>名称</th>
                  <th className={thClass}>目标</th>
                  <th className={thClass}>探测服务器</th>
                  <th className={thClass}>类型</th>
                  <th className={thClass}>状态</th>
                  <th className={thClass}>TLS</th>
                  <th className={thClass}>操作</th>
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
                            {serviceWarningText(service)}
                          </div>
                        ) : null}
                      </div>
                    </td>
                    <td className={tdClass}>{service.target}</td>
                    <td className={tdClass}>{serviceServerLabel(servers, service)}</td>
                    <td className={tdClass}>{serviceKind(service)}</td>
                    <td className={tdClass}><StatusBadge tone={serviceTone(service.last_status)}>{statusLabel(service.last_status)}</StatusBadge></td>
                    <td className={tdClass}>{service.cert_not_after ? formatDate(service.cert_not_after) : "N/A"}</td>
                    <td className={`${tdClass} flex flex-wrap gap-2`}>
                      <button className={buttonClass("secondary")} onClick={() => openEdit(service)}>编辑</button>
                      <button className={buttonClass("danger")} onClick={() => void deleteService(service)}>删除</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {modal ? (
          <Modal title={modal === "edit" ? "编辑服务" : "新增服务"} onClose={() => setModal(null)}>
            <form onSubmit={submitForm} className="space-y-4">
              {editing && serviceConfigWarnings(editing).length ? (
                <InlineNotice tone="yellow">{serviceWarningText(editing)}</InlineNotice>
              ) : null}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="名称">
                  <input className={inputClass} value={form.name} onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} required />
                </Field>
                <Field label="类型">
                  <select className={selectClass} value={form.service_type} onChange={(e) => setForm((f) => ({ ...f, service_type: e.target.value as ServiceKind }))}>
                    <option value="http">HTTP</option>
                    <option value="tcp">TCP</option>
                    <option value="icmp">ICMP</option>
                  </select>
                </Field>
              </div>
              <Field label="目标">
                <input className={inputClass} value={form.target} onChange={(e) => setForm((f) => ({ ...f, target: e.target.value }))} required />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="覆盖模式">
                  <select className={selectClass} value={form.cover_mode} onChange={(e) => setForm((f) => ({ ...f, cover_mode: e.target.value as ServiceCoverMode }))}>
                    <option value="local">主控探测</option>
                    <option value="all">全部在线服务器</option>
                    <option value="specific">指定服务器</option>
                    <option value="exclude">排除指定服务器</option>
                  </select>
                </Field>
              </div>
              {form.cover_mode === "specific" ? (
                <Field label="指定服务器">
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, server_ids: serverIds }))}
                  />
                </Field>
              ) : null}
              {form.cover_mode === "exclude" ? (
                <Field label="排除服务器">
                  <ServerMultiPicker
                    servers={servers}
                    selected={form.exclude_server_ids}
                    onChange={(serverIds) => setForm((f) => ({ ...f, exclude_server_ids: serverIds }))}
                  />
                </Field>
              ) : null}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="检查间隔（秒）">
                  <input className={inputClass} value={form.interval_seconds} onChange={(e) => setForm((f) => ({ ...f, interval_seconds: e.target.value }))} />
                </Field>
                <Field label="超时（秒）">
                  <input className={inputClass} value={form.timeout_seconds} onChange={(e) => setForm((f) => ({ ...f, timeout_seconds: e.target.value }))} />
                </Field>
              </div>
              <label className="flex items-center gap-2 text-sm font-black">
                <input type="checkbox" checked={form.enabled} onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))} />
                启用
              </label>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="失败任务 IDs">
                  <input className={inputClass} value={form.failure_task_ids} onChange={(e) => setForm((f) => ({ ...f, failure_task_ids: e.target.value }))} />
                </Field>
                <Field label="恢复任务 IDs">
                  <input className={inputClass} value={form.recovery_task_ids} onChange={(e) => setForm((f) => ({ ...f, recovery_task_ids: e.target.value }))} />
                </Field>
              </div>
              {probeResult ? <InlineNotice tone="pink">{probeResult}</InlineNotice> : null}
              <div className="flex flex-wrap gap-2">
                <button type="submit" disabled={saving} className={buttonClass("primary")}>{saving ? "保存中..." : "保存"}</button>
                <button type="button" disabled={testing} onClick={() => void testProbe()} className={buttonClass("secondary")}>{testing ? "测试中..." : "测试探测"}</button>
              </div>
            </form>
          </Modal>
        ) : null}
      </PageShell>
    </div>
  );
}

function serviceKind(service: Service): string {
  return service.service_type || service.kind || service.type || "服务";
}

function statusLabel(status?: string): string {
  if (!status) return "未知";
  const labels: Record<string, string> = {
    success: "成功",
    up: "正常",
    failure: "失败",
    down: "异常",
    timeout: "超时",
    degraded: "降级",
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
        主控/未指定
      </button>
      <div className="grid max-h-56 gap-2 overflow-auto border-2 border-black bg-[var(--bg-page)] p-3">
        {servers.length === 0 ? (
          <div className="text-sm font-black text-[var(--text-muted)]">暂无可选服务器</div>
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

function serviceWarningText(service: Service): string {
  const warnings = serviceConfigWarnings(service);
  if (!warnings.length) return "";
  return `历史配置异常：${warnings.join("；")}`;
}

function serviceServerLabel(servers: ServerOption[], service: Service): string {
  if (service.cover_mode === "all") return "全部在线服务器";
  if (service.cover_mode === "exclude") {
    const excluded = service.exclude_server_ids ?? [];
    if (!excluded.length) return "全部在线服务器";
    return `排除：${excluded.map((id) => serverName(servers, id)).join("、")}`;
  }
  const ids = serviceServerIds(service);
  if (!ids.length) return "主控探测";
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
