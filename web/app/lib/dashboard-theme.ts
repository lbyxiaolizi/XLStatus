import type { ThemeDefinition } from "@/lib/api";

export const DASHBOARD_THEME_CACHE_KEY = "xlstatus:dashboard-theme:v1";
export const DASHBOARD_THEME_EVENT = "xlstatus:dashboard-theme-change";

interface CachedDashboardTheme {
  userId: string;
  theme: ThemeDefinition | null;
  revision?: string;
}

export function readCachedDashboardTheme(userId: string): ThemeDefinition | null {
  if (typeof window === "undefined" || !userId) return null;
  try {
    const raw = window.localStorage.getItem(DASHBOARD_THEME_CACHE_KEY);
    if (!raw) return null;
    const cached = JSON.parse(raw) as Partial<CachedDashboardTheme>;
    if (cached.userId !== userId || !cached.theme || typeof cached.theme.id !== "string") {
      return null;
    }
    return cached.theme;
  } catch {
    return null;
  }
}

export function cacheDashboardTheme(
  userId: string,
  theme: ThemeDefinition | null,
  forceBroadcast = false,
): void {
  if (typeof window === "undefined" || !userId) return;
  let cached: Partial<CachedDashboardTheme> | null = null;
  try {
    const raw = window.localStorage.getItem(DASHBOARD_THEME_CACHE_KEY);
    cached = raw ? JSON.parse(raw) as Partial<CachedDashboardTheme> : null;
  } catch {
    // A valid response below repairs malformed local cache data.
  }
  if (
    !forceBroadcast
    && cached?.userId === userId
    && themeSignature(cached.theme ?? null) === themeSignature(theme)
  ) {
    return;
  }
  try {
    const revision = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    window.localStorage.setItem(
      DASHBOARD_THEME_CACHE_KEY,
      JSON.stringify({ userId, theme, revision } satisfies CachedDashboardTheme),
    );
  } catch {
    // Applying the selected theme should still work when storage is blocked.
  }
  window.dispatchEvent(new CustomEvent(DASHBOARD_THEME_EVENT, { detail: { userId, theme } }));
}

export function clearDashboardThemeCache(): void {
  if (typeof window === "undefined") return;
  let userId: string | undefined;
  try {
    const raw = window.localStorage.getItem(DASHBOARD_THEME_CACHE_KEY);
    if (raw) {
      const cached = JSON.parse(raw) as Partial<CachedDashboardTheme>;
      if (typeof cached.userId === "string") userId = cached.userId;
    }
  } catch {
    // Malformed cache data is still removed below.
  }
  try {
    window.localStorage.removeItem(DASHBOARD_THEME_CACHE_KEY);
  } catch {
    // Storage may be unavailable in restricted browser contexts.
  }
  window.dispatchEvent(new CustomEvent(DASHBOARD_THEME_EVENT, { detail: { userId, theme: null } }));
}

export function applyDashboardTheme(theme: ThemeDefinition | null): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const body = document.body;
  root.dataset.xlstatusDashboardThemeId = theme?.id ?? "";
  const previousKeys = (root.dataset.xlstatusDashboardThemeVars || "")
    .split(",")
    .map((key) => key.trim())
    .filter(Boolean);
  for (const key of previousKeys) {
    root.style.removeProperty(key);
    body?.style.removeProperty(key);
  }
  root.dataset.xlstatusDashboardThemeVars = "";

  document.getElementById("xlstatus-dashboard-theme-style")?.remove();
  if (!theme) return;

  const activeVariables = themeVariablesForMode(theme, root.classList.contains("dark-mode"));
  const nextKeys: string[] = [];
  for (const [key, value] of Object.entries(activeVariables)) {
    if (!key.startsWith("--") || !value) continue;
    root.style.setProperty(key, value);
    body?.style.setProperty(key, value);
    nextKeys.push(key);
  }
  root.dataset.xlstatusDashboardThemeVars = nextKeys.join(",");
}

function themeVariablesForMode(theme: ThemeDefinition, isDark: boolean): Record<string, string> {
  const base = theme.variables ?? {};
  const light = Object.keys(theme.light_variables ?? {}).length ? theme.light_variables ?? {} : base;
  const dark = Object.keys(theme.dark_variables ?? {}).length ? theme.dark_variables ?? {} : light;
  return isDark ? { ...base, ...dark } : { ...base, ...light };
}

function themeSignature(theme: ThemeDefinition | null): string {
  if (!theme) return "null";
  return JSON.stringify({
    id: theme.id,
    name: theme.name,
    description: theme.description ?? null,
    target: theme.target,
    variables: sortedVariableEntries(theme.variables),
    lightVariables: sortedVariableEntries(theme.light_variables),
    darkVariables: sortedVariableEntries(theme.dark_variables),
    builtin: theme.builtin,
    createdAt: theme.created_at ?? null,
    updatedAt: theme.updated_at ?? null,
  });
}

function sortedVariableEntries(variables?: Record<string, string>): Array<[string, string]> {
  return Object.entries(variables ?? {}).sort(([left], [right]) => (
    left < right ? -1 : left > right ? 1 : 0
  ));
}
