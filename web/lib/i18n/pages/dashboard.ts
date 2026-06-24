// Per-page i18n namespace for the admin dashboard overview page.
// `zh` is the source of truth; `en` must mirror its shape (enforced by the
// `typeof` annotation). Interpolated strings use {placeholder} tokens that the
// page fills in with String.prototype.replace.

export const dashboardPage = {
  // Page header
  eyebrow: "运维总览",
  title: "总览",
  detail: "服务器、服务和告警的实时工作台。",
  // KPIs
  kpiServers: "服务器",
  kpiServersOnline: "{count} 台在线",
  kpiServices: "服务",
  kpiServicesOnline: "{count} 个正常",
  kpiAlerts: "告警",
  kpiAlertsDetail: "活跃事件",
  kpiMode: "模式",
  kpiModeLive: "实时",
  kpiModeDetail: "API 已连接",
  // Servers card
  serversHeading: "服务器",
  serversEmptyTitle: "暂无服务器",
  serversEmptyDetail: "Agent 注册并上线后会显示在这里。",
  colName: "名称",
  colStatus: "状态",
  colLastSeen: "最后在线",
  // Alerts card
  recentAlertsHeading: "最近告警",
  alertsEmptyTitle: "暂无告警事件",
  alertsEmptyDetail: "规则触发或恢复后，告警历史会显示在这里。",
  alertFallbackName: "告警",
  alertFallbackEvent: "事件",
  // Server status labels (module-scope helper)
  statusOnline: "在线",
  statusOffline: "离线",
  statusRevoked: "已撤销",
  statusDown: "异常",
  statusDegraded: "降级",
};

export const dashboardPageEn: typeof dashboardPage = {
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
