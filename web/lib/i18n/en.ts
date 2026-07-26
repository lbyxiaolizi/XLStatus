// English translation bundle. This module is the sole entry point for every
// `*.en.ts` namespace and is only reachable through the dynamic import in
// `lib/i18n.ts`, so the ~112K of English copy stays out of the default zh-CN
// chunk. The `Translations` annotation below is what keeps this bundle aligned
// with the zh source of truth: a missing or misspelled key fails the build.
// The import is type-only, so the cycle back to `lib/i18n.ts` is erased.
import type { Translations } from "@/lib/i18n";

import { loginPageEn } from "./pages/login.en";
import { statusPageEn } from "./pages/status.en";
import { statusDetailPageEn } from "./pages/statusDetail.en";
import { dashboardPageEn } from "./pages/dashboard.en";
import { serversPageEn } from "./pages/servers.en";
import { serverDetailPageEn } from "./pages/serverDetail.en";
import { settingsPageEn } from "./pages/settings.en";
import { servicesPageEn } from "./pages/services.en";
import { tasksPageEn } from "./pages/tasks.en";
import { alertsPageEn } from "./pages/alerts.en";
import { natPageEn } from "./pages/nat.en";
import { ddnsPageEn } from "./pages/ddns.en";
import { notificationsPageEn } from "./pages/notifications.en";
import { terminalPageEn } from "./pages/terminal.en";
import { dialogsEn, errorsPageEn } from "./pages/shared.en";
import { worldMapEn } from "./pages/worldMap.en";

export const enUS: Translations = {
  appDescription: "Self-hosted server monitoring and operations system",
  common: {
    close: "Close",
    login: "Log in",
    logout: "Log out",
    menu: "Menu",
    light: "Light",
    dark: "Dark",
    enabled: "Enabled",
    disabled: "Disabled",
    success: "Success",
    failure: "Failure",
    cancel: "Cancel",
    confirm: "Confirm",
    submit: "Submit",
    unknown: "Unknown",
    never: "Never",
    manual: "Manual",
    loading: "Loading...",
    notAvailable: "N/A",
    requestFailed: "Request failed",
    networkError: "Network error",
    noRequestAttempted: "No request attempted",
    authRequired: "Authentication required",
    permissionDenied: "Permission denied",
    backendNotFound: "Backend route or resource not found",
    search: "Search",
    noMatches: "No matches",
    localPreference: "Local preference",
    language: "Language",
    zhCN: "中文",
    enUS: "EN",
  },
  nav: {
    dashboard: "Dashboard",
    servers: "Servers",
    services: "Services",
    tasks: "Tasks",
    terminal: "Terminal",
    alerts: "Alerts",
    notifications: "Notifications",
    nat: "NAT",
    ddns: "DDNS",
    settings: "Settings",
    status: "Status",
  },
  command: {
    pages: "Pages",
    servers: "Servers",
    commands: "Commands",
    searchPlaceholder: "Search pages or servers",
  },
  loginPage: loginPageEn,
  statusPage: statusPageEn,
  statusDetailPage: statusDetailPageEn,
  dashboardPage: dashboardPageEn,
  serversPage: serversPageEn,
  serverDetailPage: serverDetailPageEn,
  settingsPage: settingsPageEn,
  servicesPage: servicesPageEn,
  tasksPage: tasksPageEn,
  alertsPage: alertsPageEn,
  natPage: natPageEn,
  ddnsPage: ddnsPageEn,
  notificationsPage: notificationsPageEn,
  terminalPage: terminalPageEn,
  dialogs: dialogsEn,
  errorsPage: errorsPageEn,
  worldMap: worldMapEn,
};
