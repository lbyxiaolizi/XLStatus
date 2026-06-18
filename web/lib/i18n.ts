export const supportedLocales = ["zh-CN"] as const;

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
  },
  nav: {
    dashboard: "总览",
    servers: "服务器",
    services: "服务",
    tasks: "任务",
    terminal: "终端",
    alerts: "告警",
    nat: "NAT",
    ddns: "DDNS",
    settings: "设置",
    status: "状态页",
  },
};

export const t = zhCN;

export function formatLocaleDate(value: string | number | Date): string {
  return new Intl.DateTimeFormat(defaultLocale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(value instanceof Date ? value : new Date(value));
}
