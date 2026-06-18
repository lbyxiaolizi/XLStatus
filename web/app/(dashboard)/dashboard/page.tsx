"use client";

import { useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  PageHeader,
  PageShell,
  StatusBadge,
  formatDate,
  responseError,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient } from "@/lib/api";

interface Server {
  id: string;
  name: string;
  status: string;
  last_seen_at?: string;
}

interface Service {
  id: string;
  name: string;
  target: string;
  last_status?: string;
}

interface AlertEvent {
  id?: string;
  rule_name?: string;
  message?: string;
  status?: string;
  created_at?: string;
}

export default function DashboardPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [services, setServices] = useState<Service[]>([]);
  const [alerts, setAlerts] = useState<AlertEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function load() {
      setLoading(true);
      const [serverResponse, serviceResponse, alertResponse] = await Promise.all([
        apiClient.listServers(100, 0),
        apiClient.listServices(100, 0),
        apiClient.listAlertEvents(10),
      ]);

      if (serverResponse.success && serverResponse.data) {
        setServers((serverResponse.data.servers as Server[]) ?? []);
      } else {
        setError(responseError(serverResponse));
      }

      if (serviceResponse.success && serviceResponse.data) {
        setServices((serviceResponse.data.services as Service[]) ?? []);
      }

      if (alertResponse.success && alertResponse.data) {
        setAlerts((alertResponse.data.events as AlertEvent[]) ?? []);
      }
      setLoading(false);
    }

    void load();
  }, []);

  const summary = useMemo(
    () => ({
      onlineServers: servers.filter((server) => server.status === "online").length,
      activeAlerts: alerts.filter((alert) => alert.status !== "resolved").length,
      onlineServices: services.filter((service) => ["success", "up"].includes(service.last_status || "")).length,
    }),
    [alerts, servers, services],
  );

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="Operations"
          title="Dashboard"
          detail="服务器、服务和告警的实时工作台。"
        />
        <InlineError message={error} />

        <div className="mt-5 grid gap-4 md:grid-cols-4">
          <Kpi label="Servers" value={String(servers.length)} detail={`${summary.onlineServers} online`} />
          <Kpi label="Services" value={String(services.length)} detail={`${summary.onlineServices} healthy`} />
          <Kpi label="Alerts" value={String(summary.activeAlerts)} detail="active events" />
          <Kpi label="Mode" value={loading ? "..." : "Live"} detail="API connected" />
        </div>

        <div className="mt-6 grid gap-6 lg:grid-cols-2">
          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">Servers</h2>
            {servers.length === 0 ? (
              <EmptyState title="No servers found" detail="Agents will appear here after enrollment." />
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead>
                    <tr>
                      <th className={thClass}>Name</th>
                      <th className={thClass}>Status</th>
                      <th className={thClass}>Last Seen</th>
                    </tr>
                  </thead>
                  <tbody>
                    {servers.slice(0, 8).map((server) => (
                      <tr key={server.id}>
                        <td className={tdClass}>{server.name}</td>
                        <td className={tdClass}><StatusBadge tone={server.status === "online" ? "green" : "red"}>{server.status}</StatusBadge></td>
                        <td className={tdClass}>{formatDate(server.last_seen_at)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </BrutalCard>

          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">Recent Alerts</h2>
            {alerts.length === 0 ? (
              <EmptyState title="No alert events" detail="Alert history will appear here once rules fire or recover." />
            ) : (
              <div className="grid gap-3">
                {alerts.map((alert, index) => (
                  <div key={alert.id || index} className="border-2 border-black bg-[var(--accent-bg)] p-3">
                    <div className="font-black">{alert.rule_name || "Alert"}</div>
                    <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{alert.message || alert.status || "Event"}</div>
                    <div className="mt-2 text-xs font-black uppercase">{formatDate(alert.created_at)}</div>
                  </div>
                ))}
              </div>
            )}
          </BrutalCard>
        </div>
      </PageShell>
    </div>
  );
}

function Kpi({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      <div className="mt-2 text-4xl font-black">{value}</div>
      <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{detail}</div>
    </BrutalCard>
  );
}
