"use client";

import { useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  InlineError,
  InlineNotice,
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
  net_rx_bps?: number;
  net_tx_bps?: number;
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
  const [updatedAt, setUpdatedAt] = useState<string>(new Date().toISOString());

  useEffect(() => {
    let cancelled = false;

    async function load() {
      setLoading(true);
      setError(null);
      const [serverResponse, serviceResponse] = await Promise.all([
        apiClient.listServers(100, 0, true),
        apiClient.listServices(100, 0, true),
      ]);

      if (cancelled) {
        return;
      }

      if (serverResponse.success && serverResponse.data) {
        setServers((serverResponse.data.servers as Server[]) ?? []);
      } else if (!isAuthDenied(serverResponse.status)) {
        setError(responseError(serverResponse));
      }

      if (serviceResponse.success && serviceResponse.data) {
        setServices((serviceResponse.data.services as Service[]) ?? []);
      } else if (!isAuthDenied(serviceResponse.status)) {
        setError((prev) => prev || responseError(serviceResponse));
      }

      setUpdatedAt(new Date().toISOString());
      setLoading(false);
    }

    void load();
    const timer = window.setInterval(() => {
      void load();
    }, 30000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const totalServers = servers.length;
  const onlineServers = servers.filter((server) => server.status === "online").length;
  const overall = useMemo(() => {
    if (servers.length === 0 && services.length === 0) {
      return { label: "No public data", tone: "gray" as const };
    }
    if (servers.some((server) => server.status === "down" || server.status === "offline")) {
      return { label: "Partial outage", tone: "yellow" as const };
    }
    return { label: "Operational", tone: "green" as const };
  }, [servers, services]);

  return (
    <div className="min-h-screen bg-gray-50">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">XLStatus</h1>
            <p className="mt-1 text-sm text-gray-500">Public status and availability overview.</p>
          </div>
          <StatusBadge tone={overall.tone}>{overall.label}</StatusBadge>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          <InlineNotice tone="blue">
            Updated {formatDate(updatedAt)}. Public data is limited to resources exposed by the backend.
          </InlineNotice>
        </div>

        <div className="mb-6 grid gap-3 sm:grid-cols-3">
          <div className="rounded-lg bg-white p-4 shadow">
            <div className="text-xs uppercase text-gray-500">Servers</div>
            <div className="mt-2 text-2xl font-bold text-gray-900">{totalServers}</div>
            <div className="mt-1 text-sm text-gray-500">{onlineServers} online</div>
          </div>
          <div className="rounded-lg bg-white p-4 shadow">
            <div className="text-xs uppercase text-gray-500">Services</div>
            <div className="mt-2 text-2xl font-bold text-gray-900">{services.length}</div>
            <div className="mt-1 text-sm text-gray-500">Public service monitors</div>
          </div>
          <div className="rounded-lg bg-white p-4 shadow">
            <div className="text-xs uppercase text-gray-500">Refresh</div>
            <div className="mt-2 text-2xl font-bold text-gray-900">30s</div>
            <div className="mt-1 text-sm text-gray-500">Auto refresh enabled</div>
          </div>
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <section>
            <h2 className="mb-3 text-base font-semibold text-gray-900">Servers</h2>
            {loading && servers.length === 0 ? (
              <div className="rounded-lg bg-white p-6 text-sm text-gray-600 shadow">Loading public servers...</div>
            ) : servers.length === 0 ? (
              <EmptyState title="No public servers available" detail="Hidden or unauthorized servers stay out of the public view." />
            ) : (
              <div className="grid gap-3">
                {servers.map((server) => (
                  <div key={server.id} className="rounded-lg bg-white p-4 shadow">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="font-semibold text-gray-900">{server.name}</h3>
                        <p className="mt-1 text-xs text-gray-500">{server.id}</p>
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
                  </div>
                ))}
              </div>
            )}
          </section>

          <section>
            <h2 className="mb-3 text-base font-semibold text-gray-900">Services</h2>
            {services.length === 0 ? (
              <EmptyState title="No public services available" detail="Service availability will appear once monitors are exposed publicly." />
            ) : (
              <div className="grid gap-3">
                {services.map((service) => (
                  <div key={service.id} className="rounded-lg bg-white p-4 shadow">
                    <div className="flex items-start justify-between gap-3">
                      <div>
                        <h3 className="font-semibold text-gray-900">{service.name}</h3>
                        <p className="mt-1 text-xs text-gray-500">{service.target}</p>
                      </div>
                      <StatusBadge tone={serviceTone(service.last_status)}>{service.last_status || "unknown"}</StatusBadge>
                    </div>
                    <div className="mt-4 flex items-center justify-between text-sm text-gray-500">
                      <span>{serviceKind(service)}</span>
                      <span>Checked {formatDate(service.last_check_at)}</span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>
        </div>
      </PageShell>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs uppercase text-gray-500">{label}</div>
      <div className="mt-1 font-medium text-gray-900">{value}</div>
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

function isAuthDenied(status?: number): boolean {
  return status === 401 || status === 403;
}
