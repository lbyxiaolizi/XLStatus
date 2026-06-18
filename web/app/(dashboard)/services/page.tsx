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
      setNotice(modal === "edit" ? "服务已更新。" : "服务已创建。");
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
    const response = await apiClient.deleteService(service.id);
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
                  <th className={thClass}>类型</th>
                  <th className={thClass}>状态</th>
                  <th className={thClass}>TLS</th>
                  <th className={thClass}>操作</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((service) => (
                  <tr key={service.id}>
                    <td className={tdClass}>{service.name}</td>
                    <td className={tdClass}>{service.target}</td>
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
