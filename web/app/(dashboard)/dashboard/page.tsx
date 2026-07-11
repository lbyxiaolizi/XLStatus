"use client";

import { useEffect, useMemo, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  InlineError,
  PageHeader,
  PageShell,
  Skeleton,
  StatusBadge,
  formatDate,
  responseError,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { useServerData } from "@/app/components/ServerDataProvider";
import { apiClient } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";

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
  const { t: copy } = useI18n();
  const {
    servers,
    loading: serversLoading,
    error: serversError,
  } = useServerData();
  const [services, setServices] = useState<Service[]>([]);
  const [alerts, setAlerts] = useState<AlertEvent[]>([]);
  const [errors, setErrors] = useState({ services: "", alerts: "" });
  const [loading, setLoading] = useState({ services: true, alerts: true });

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    const { signal } = controller;

    void apiClient.listServices(100, 0, false, { signal }).then((response) => {
      if (cancelled || signal.aborted) return;
      if (response.success && response.data) {
        setServices((response.data.services as Service[]) ?? []);
      } else {
        setErrors((current) => ({ ...current, services: responseError(response) }));
      }
      setLoading((current) => ({ ...current, services: false }));
    });

    void apiClient.listAlertEvents(10, { signal }).then((response) => {
      if (cancelled || signal.aborted) return;
      if (response.success && response.data) {
        setAlerts((response.data.events as AlertEvent[]) ?? []);
      } else {
        setErrors((current) => ({ ...current, alerts: responseError(response) }));
      }
      setLoading((current) => ({ ...current, alerts: false }));
    });

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, []);

  const anyLoading = serversLoading || loading.services || loading.alerts;

  const summary = useMemo(
    () => ({
      onlineServers: servers.filter((server) => server.status === "online").length,
      activeAlerts: alerts.filter((alert) => alert.status !== "resolved").length,
      onlineServices: services.filter((service) => ["success", "up"].includes(service.last_status || "")).length,
    }),
    [alerts, servers, services],
  );

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.dashboardPage.eyebrow}
          title={copy.dashboardPage.title}
          detail={copy.dashboardPage.detail}
        />
        <InlineError message={serversError ? `${copy.dashboardPage.kpiServers}: ${serversError}` : null} />
        <InlineError message={errors.services ? `${copy.dashboardPage.kpiServices}: ${errors.services}` : null} />
        <InlineError message={errors.alerts ? `${copy.dashboardPage.kpiAlerts}: ${errors.alerts}` : null} />

        <div className="mt-5 grid gap-4 md:grid-cols-4">
          <Kpi loading={serversLoading} label={copy.dashboardPage.kpiServers} value={String(servers.length)} detail={copy.dashboardPage.kpiServersOnline.replace("{count}", String(summary.onlineServers))} />
          <Kpi loading={loading.services} label={copy.dashboardPage.kpiServices} value={String(services.length)} detail={copy.dashboardPage.kpiServicesOnline.replace("{count}", String(summary.onlineServices))} />
          <Kpi loading={loading.alerts} label={copy.dashboardPage.kpiAlerts} value={String(summary.activeAlerts)} detail={copy.dashboardPage.kpiAlertsDetail} />
          <Kpi loading={anyLoading} label={copy.dashboardPage.kpiMode} value={copy.dashboardPage.kpiModeLive} detail={copy.dashboardPage.kpiModeDetail} />
        </div>

        <div className="mt-6 grid gap-6 lg:grid-cols-2">
          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">{copy.dashboardPage.serversHeading}</h2>
            {serversLoading ? (
              <TableRowsSkeleton columns={3} rows={5} />
            ) : servers.length === 0 ? (
              <EmptyState title={copy.dashboardPage.serversEmptyTitle} detail={copy.dashboardPage.serversEmptyDetail} />
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead>
                    <tr>
                      <th className={thClass}>{copy.dashboardPage.colName}</th>
                      <th className={thClass}>{copy.dashboardPage.colStatus}</th>
                      <th className={thClass}>{copy.dashboardPage.colLastSeen}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {servers.slice(0, 8).map((server) => (
                      <tr key={server.id}>
                        <td className={tdClass}>{server.name}</td>
                        <td className={tdClass}><StatusBadge tone={server.status === "online" ? "green" : "red"}>{serverStatusLabel(server.status, copy)}</StatusBadge></td>
                        <td className={tdClass}>{formatDate(server.last_seen_at)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </BrutalCard>

          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">{copy.dashboardPage.recentAlertsHeading}</h2>
            {loading.alerts ? (
              <div className="grid gap-3">
                {Array.from({ length: 4 }).map((_, index) => (
                  <div key={index} className="border-2 border-black bg-[var(--accent-bg)] p-3">
                    <Skeleton className="h-5 w-40" />
                    <Skeleton className="mt-2 h-4 w-3/4" />
                    <Skeleton className="mt-3 h-3 w-24" />
                  </div>
                ))}
              </div>
            ) : alerts.length === 0 ? (
              <EmptyState title={copy.dashboardPage.alertsEmptyTitle} detail={copy.dashboardPage.alertsEmptyDetail} />
            ) : (
              <div className="grid gap-3">
                {alerts.map((alert, index) => (
                  <div key={alert.id || index} className="border-2 border-black bg-[var(--accent-bg)] p-3">
                    <div className="font-black">{alert.rule_name || copy.dashboardPage.alertFallbackName}</div>
                    <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{alert.message || alert.status || copy.dashboardPage.alertFallbackEvent}</div>
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

function Kpi({
  label,
  value,
  detail,
  loading = false,
}: {
  label: string;
  value: string;
  detail: string;
  loading?: boolean;
}) {
  return (
    <BrutalCard accent>
      <div className="text-xs font-black uppercase text-[var(--text-muted)]">{label}</div>
      {loading ? (
        <>
          <Skeleton className="mt-3 h-10 w-20" />
          <Skeleton className="mt-2 h-4 w-36" />
        </>
      ) : (
        <>
          <div className="mt-2 text-4xl font-black">{value}</div>
          <div className="mt-1 text-sm font-bold text-[var(--text-muted)]">{detail}</div>
        </>
      )}
    </BrutalCard>
  );
}

function TableRowsSkeleton({ columns, rows }: { columns: number; rows: number }) {
  return (
    <div className="grid gap-2" aria-busy="true">
      {Array.from({ length: rows }).map((_, row) => (
        <div key={row} className="grid gap-3 border-b border-black/20 py-2 last:border-b-0" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
          {Array.from({ length: columns }).map((_, column) => (
            <Skeleton key={column} className={`h-6 ${column === 0 ? "w-28" : "w-20"}`} />
          ))}
        </div>
      ))}
    </div>
  );
}

function serverStatusLabel(status: string, copy: import("@/lib/i18n").Translations): string {
  const labels: Record<string, string> = {
    online: copy.dashboardPage.statusOnline,
    offline: copy.dashboardPage.statusOffline,
    revoked: copy.dashboardPage.statusRevoked,
    down: copy.dashboardPage.statusDown,
    degraded: copy.dashboardPage.statusDegraded,
  };
  return labels[status] || status;
}
