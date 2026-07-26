// English half of the `dashboard` i18n namespace, split out so the default zh-CN
// bundle never ships it. `lib/i18n/en.ts` pulls these in through a dynamic
// import that only runs when the user selects en-US, so no `typeof` annotation
// is used here (it would drag the zh object back into the default chunk);
// shape is enforced by the `Translations` annotation on `enUS`.

export const dashboardPageEn = {
  eyebrow: "Operations overview",
  title: "Dashboard",
  detail: "Real-time workspace for servers, services, and alerts.",
  kpiServers: "Servers",
  kpiServersOnline: "{count} online",
  kpiServices: "Services",
  kpiServicesOnline: "{count} healthy",
  kpiAlerts: "Alerts",
  kpiAlertsDetail: "Active events",
  kpiMode: "Mode",
  kpiModeLive: "Live",
  kpiModeDetail: "API connected",
  serversHeading: "Servers",
  serversEmptyTitle: "No servers",
  serversEmptyDetail: "Servers will appear here once an agent registers and comes online.",
  colName: "Name",
  colStatus: "Status",
  colLastSeen: "Last seen",
  recentAlertsHeading: "Recent alerts",
  alertsEmptyTitle: "No alert events",
  alertsEmptyDetail: "Alert history shows up here after rules fire or recover.",
  alertFallbackName: "Alert",
  alertFallbackEvent: "Event",
  statusOnline: "Online",
  statusOffline: "Offline",
  statusRevoked: "Revoked",
  statusDown: "Down",
  statusDegraded: "Degraded",
};
