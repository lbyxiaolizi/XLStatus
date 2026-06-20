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
};

export const enUS: typeof zhCN = {
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
};

export const translations = {
  "zh-CN": zhCN,
  "en-US": enUS,
} satisfies Record<Locale, typeof zhCN>;

export type Translations = typeof zhCN;

export const t = zhCN;

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
  window.dispatchEvent(new CustomEvent("xlstatus:locale-change", { detail: { locale: normalized } }));
}

export function getTranslations(locale: Locale = getLocale()): Translations {
  return translations[normalizeLocale(locale)];
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
