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
      return { label: "No public data", tone: "gray" as const };
    }
    if (
      servers.some((server) => server.status === "down" || server.status === "offline") ||
      services.some((service) => service.last_status === "failure" || service.last_status === "down")
    ) {
      return { label: "Partial outage", tone: "yellow" as const };
    }
    return { label: "Operational", tone: "green" as const };
  }, [servers, services]);

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="Public Status"
          title="XLStatus"
          detail="服务器与服务可用性概览，数据来自后端公开 API。"
          actions={<StatusBadge tone={overall.tone}>{overall.label}</StatusBadge>}
        />

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          <InlineNotice tone="pink">
            {updatedAt ? `Updated ${formatDate(updatedAt)}.` : "Waiting for first refresh."} Auto refresh is enabled every 30 seconds.
          </InlineNotice>
        </div>

        <div className="mb-6 grid gap-4 sm:grid-cols-3">
          <Kpi title="Servers" value={String(servers.length)} detail={`${servers.filter((s) => s.status === "online").length} online`} />
          <Kpi title="Services" value={String(services.length)} detail="Public monitors" />
          <Kpi title="Refresh" value="30s" detail="Live polling" />
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <section>
            <h2 className="mb-3 text-xl font-black uppercase">Servers</h2>
            {loading && servers.length === 0 ? (
              <BrutalCard>Loading public servers...</BrutalCard>
            ) : servers.length === 0 ? (
              <EmptyState title="No public servers available" detail="Hidden or unauthorized servers stay out of the public view." />
            ) : (
              <div className="grid gap-4">
                {servers.map((server) => (
                  <BrutalCard key={server.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-xl font-black">{server.name}</h3>
                        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{server.id}</p>
                      </div>
                      <StatusBadge tone={serverTone(server.status)}>{server.status}</StatusBadge>
                    </div>
                    <div className="mt-4 grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
                      <Metric label="CPU" value={formatPercent(server.cpu_percent)} />
                      <Metric
                        label="Memory"
                        value={
                          server.memory_used !== undefined && server.memory_total !== undefined
                            ? `${(server.memory_used / 1e9).toFixed(1)} / ${(server.memory_total / 1e9).toFixed(1)} GB`
                            : "N/A"
                        }
                      />
                      <Metric label="Load" value={server.load_1 !== undefined ? server.load_1.toFixed(2) : "N/A"} />
                      <Metric label="Last seen" value={formatDate(server.last_seen_at)} />
                    </div>
                  </BrutalCard>
                ))}
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-3 text-xl font-black uppercase">Services</h2>
            {services.length === 0 ? (
              <EmptyState title="No public services available" detail="Service availability will appear once monitors are exposed publicly." />
            ) : (
              <div className="grid gap-4">
                {services.map((service) => (
                  <BrutalCard key={service.id}>
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="text-xl font-black">{service.name}</h3>
                        <p className="mt-1 text-xs font-bold text-[var(--text-muted)]">{service.target}</p>
                      </div>
                      <StatusBadge tone={serviceTone(service.last_status)}>{service.last_status || "unknown"}</StatusBadge>
                    </div>
                    <div className="mt-4 flex items-center justify-between text-sm font-bold text-[var(--text-muted)]">
                      <span>{serviceKind(service)}</span>
                      <span>Checked {formatDate(service.last_check_at)}</span>
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
  return service.service_type || service.kind || service.type || "service";
}
