// Only the zh-CN namespaces are imported statically. The en-US half lives in
// `./i18n/en` (which re-exports every `pages/*.en.ts`) and is loaded through a
// dynamic import, so the English copy is code-split out of the initial chunk.
import { loginPage } from "./i18n/pages/login";
import { statusPage } from "./i18n/pages/status";
import { statusDetailPage } from "./i18n/pages/statusDetail";
import { dashboardPage } from "./i18n/pages/dashboard";
import { serversPage } from "./i18n/pages/servers";
import { serverDetailPage } from "./i18n/pages/serverDetail";
import { settingsPage } from "./i18n/pages/settings";
import { servicesPage } from "./i18n/pages/services";
import { tasksPage } from "./i18n/pages/tasks";
import { alertsPage } from "./i18n/pages/alerts";
import { natPage } from "./i18n/pages/nat";
import { ddnsPage } from "./i18n/pages/ddns";
import { notificationsPage } from "./i18n/pages/notifications";
import { terminalPage } from "./i18n/pages/terminal";
import { dialogs, errorsPage } from "./i18n/pages/shared";
import { worldMap } from "./i18n/pages/worldMap";

export const supportedLocales = ["zh-CN", "en-US"] as const;

export type Locale = (typeof supportedLocales)[number];

export const defaultLocale: Locale = "zh-CN";

export const i18nConfig = {
  locales: [...supportedLocales],
  defaultLocale,
  localeDetection: false,
};

export const zhCN = {
  appDescription: "自托管服务器监控与运维系统",
  common: {
    close: "关闭",
    login: "登录",
    logout: "退出登录",
    menu: "菜单",
    light: "浅色",
    dark: "深色",
    enabled: "启用",
    disabled: "停用",
    success: "成功",
    failure: "失败",
    cancel: "取消",
    confirm: "确认",
    submit: "确定",
    unknown: "未知",
    never: "从未",
    manual: "手动",
    loading: "加载中...",
    notAvailable: "N/A",
    requestFailed: "请求失败",
    networkError: "网络错误",
    noRequestAttempted: "没有发起请求",
    authRequired: "需要登录",
    permissionDenied: "权限不足",
    backendNotFound: "后端路由或资源不存在",
    search: "搜索",
    noMatches: "没有匹配结果",
    localPreference: "本地偏好",
    language: "语言",
    zhCN: "中文",
    enUS: "EN",
  },
  nav: {
    dashboard: "总览",
    servers: "服务器",
    services: "服务",
    tasks: "任务",
    terminal: "终端",
    mcp: "AI 运维",
    alerts: "告警",
    notifications: "通知",
    nat: "NAT",
    ddns: "DDNS",
    settings: "设置",
    status: "状态页",
  },
  command: {
    pages: "页面",
    servers: "服务器",
    commands: "命令",
    searchPlaceholder: "搜索页面或服务器",
  },
  loginPage,
  statusPage,
  statusDetailPage,
  dashboardPage,
  serversPage,
  serverDetailPage,
  settingsPage,
  servicesPage,
  tasksPage,
  alertsPage,
  natPage,
  ddnsPage,
  notificationsPage,
  terminalPage,
  dialogs,
  errorsPage,
  worldMap,
};

const enUSNavPatch = {
  mcp: "AI & MCP",
};

// en-US is deliberately absent: it is not resolvable synchronously.
export const translations = {
  "zh-CN": zhCN,
} satisfies Partial<Record<Locale, typeof zhCN>>;

export type Translations = Omit<typeof zhCN, "nav"> & {
  nav: Omit<typeof zhCN.nav, "mcp"> & { mcp?: string };
};

export const t = zhCN;

let loadedEn: typeof zhCN | null = null;
let enPromise: Promise<unknown> | null = null;

export function isEnLoaded(): boolean {
  return loadedEn !== null;
}

// Fire the en-US chunk fetch at most once. Nothing awaits it: callers keep
// rendering zh-CN, and the locale-change event is what brings subscribers back
// for a re-render once the real English copy is in memory. A failed fetch
// clears the latch so a later locale switch can retry.
function ensureEnLoaded(): void {
  if (typeof window === "undefined" || loadedEn !== null || enPromise !== null) return;
  enPromise = import("./i18n/en")
    .then((enModule) => {
      loadedEn = {
        ...enModule.enUS,
        nav: {
          ...enModule.enUS.nav,
          ...enUSNavPatch,
        },
      };
      window.dispatchEvent(new Event("xlstatus:locale-change"));
    })
    .catch(() => {
      enPromise = null;
    });
}

export function normalizeLocale(value?: string | null): Locale {
  return supportedLocales.find((locale) => locale === value) ?? defaultLocale;
}

export function getLocale(): Locale {
  if (typeof window === "undefined") return defaultLocale;
  return normalizeLocale(window.localStorage.getItem("xlstatus_locale"));
}

export function setLocale(locale: Locale): void {
  if (typeof window === "undefined") return;
  const normalized = normalizeLocale(locale);
  window.localStorage.setItem("xlstatus_locale", normalized);
  document.documentElement.lang = normalized;
  if (normalized === "en-US") ensureEnLoaded();
  window.dispatchEvent(new CustomEvent("xlstatus:locale-change", { detail: { locale: normalized } }));
}

export function getTranslations(locale: Locale = getLocale()): typeof zhCN {
  const normalized = normalizeLocale(locale);
  if (normalized !== "en-US") return translations["zh-CN"];
  if (loadedEn !== null) return loadedEn;
  // First en-US render (and every SSR pass) falls back to zh-CN; ensureEnLoaded
  // schedules the swap on the client and no-ops on the server.
  ensureEnLoaded();
  return translations["zh-CN"];
}

export function formatLocaleDate(value: string | number | Date, locale: Locale = getLocale()): string {
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(value instanceof Date ? value : new Date(value));
}
