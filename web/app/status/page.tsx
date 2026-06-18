"use client";

import { useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  StatusBadge,
  formatDate,
  formatPercent,
  responseError,
} from "@/app/components/M7Primitives";
import { apiClient } from "@/lib/api";
import { t } from "@/lib/i18n";

interface Server {
  id: string;
  name: string;
  status: string;
  cpu_percent?: number;
  memory_used?: number;
  memory_total?: number;
  load_1?: number;
  last_seen_at?: string;
}

interface Service {
  id: string;
  name: string;
  target: string;
  last_status?: string;
  last_check_at?: string;
  kind?: string;
  type?: string;
  service_type?: string;
}

export default function StatusPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [services, setServices] = useState<Service[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState("");

  useEffect(() => {
    let cancelled = false;

    async function load() {
      setLoading(true);
      setError(null);
      const response = await apiClient.getPublicStatus();

      if (cancelled) return;

      if (response.success && response.data) {
        setServers((response.data.servers as Server[]) ?? []);
        setServices((response.data.services as Service[]) ?? []);
        setUpdatedAt(response.data.updated_at || new Date().toISOString());
      } else {
        setError(responseError(response));
        setUpdatedAt(new Date().toISOString());
      }

      setLoading(false);
    }

    void load();
    const timer = window.setInterval(() => void load(), 30000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const overall = useMemo(() => {
    if (servers.length === 0 && services.length === 0) {
      return { label: "暂无公开数据", tone: "gray" as const };
    }
    if (
      servers.some((server) => server.status === "down" || server.status === "offline") ||
      services.some((service) => service.last_status === "failure" || service.last_status === "down")
    ) {
      return { label: "部分异常", tone: "yellow" as const };
    }
    return { label: "运行正常", tone: "green" as const };
  }, [servers, services]);

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="公开状态"
          title="XLStatus"
          detail="服务器与服务可用性概览，数据来自后端公开 API。"
          actions={<StatusBadge tone={overall.tone}>{overall.label}</StatusBadge>}
        />

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          <InlineNotice tone="pink">
            {updatedAt ? `更新于 ${formatDate(updatedAt)}。` : "等待首次刷新。"} 每 30 秒自动刷新。
          </InlineNotice>
        </div>

        <div className="mb-6 grid gap-4 sm:grid-cols-3">
          <Kpi title="服务器" value={String(servers.length)} detail={`${servers.filter((s) => s.status === "online").length} 台在线`} />
          <Kpi title="服务" value={String(services.length)} detail="公开监控项" />
          <Kpi title="刷新" value="30s" detail="实时轮询" />
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <section>
            <h2 className="mb-3 text-xl font-black uppercase">服务器</h2>
            {loading && servers.length === 0 ? (
              <BrutalCard>正在加载公开服务器...</BrutalCard>
            ) : servers.length === 0 ? (
              <EmptyState title="暂无公开服务器" detail="隐藏或未授权的服务器不会出现在公开状态页。" />
            ) : (
              <div className="grid gap-4">
                {servers.map((server) => (
                  <BrutalCard key={server.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-xl font-black">{server.name}</h3>
                        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{server.id}</p>
                      </div>
                      <StatusBadge tone={serverTone(server.status)}>{statusLabel(server.status)}</StatusBadge>
                    </div>
                    <div className="mt-4 grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
                      <Metric label="CPU" value={formatPercent(server.cpu_percent)} />
                      <Metric
                        label="Memory"
                        value={
                          server.memory_used !== undefined && server.memory_total !== undefined
                            ? `${(server.memory_used / 1e9).toFixed(1)} / ${(server.memory_total / 1e9).toFixed(1)} GB`
                            : t.common.notAvailable
                        }
                      />
                      <Metric label="负载" value={server.load_1 !== undefined ? server.load_1.toFixed(2) : t.common.notAvailable} />
                      <Metric label="最后在线" value={formatDate(server.last_seen_at)} />
                    </div>
                  </BrutalCard>
                ))}
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-3 text-xl font-black uppercase">服务</h2>
            {services.length === 0 ? (
              <EmptyState title="暂无公开服务" detail="服务监控项公开后会显示在这里。" />
            ) : (
              <div className="grid gap-4">
                {services.map((service) => (
                  <BrutalCard key={service.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-xl font-black">{service.name}</h3>
                        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{service.target}</p>
                      </div>
                      <StatusBadge tone={serviceTone(service.last_status)}>{statusLabel(service.last_status)}</StatusBadge>
                    </div>
                    <div className="mt-4 flex items-center justify-between text-sm font-bold text-[var(--text-muted)]">
                      <span>{serviceKind(service)}</span>
                      <span>检查于 {formatDate(service.last_check_at)}</span>
                    </div>
                  </BrutalCard>
                ))}
              </div>
            )}
          </section>
        </div>
      </PageShell>
    </div>
  );
}

function Kpi({ title, value, detail }: { title: string; value: string; detail: string }) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{title}</div>
      <div className="mt-2 text-4xl font-black">{value}</div>
      <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-1 font-black">{value}</div>
    </div>
  );
}

function serverTone(status: string): "green" | "red" | "yellow" | "gray" {
  if (status === "online") return "green";
  if (status === "offline" || status === "down") return "red";
  if (status === "degraded" || status === "revoked") return "yellow";
  return "gray";
}

function serviceTone(status?: string): "green" | "red" | "yellow" | "gray" {
  if (status === "success" || status === "up") return "green";
  if (status === "failure" || status === "down") return "red";
  if (status === "timeout" || status === "degraded") return "yellow";
  return "gray";
}

function serviceKind(service: Service): string {
  return service.service_type || service.kind || service.type || "服务";
}

function statusLabel(status?: string): string {
  if (!status) return t.common.unknown;
  const labels: Record<string, string> = {
    online: "在线",
    offline: "离线",
    down: "异常",
    degraded: "降级",
    revoked: "已撤销",
    success: t.common.success,
    up: "正常",
    failure: t.common.failure,
    timeout: "超时",
  };
  return labels[status] || status;
}
