"use client";

import { useEffect, useRef, useState } from "react";
import { apiClient, type ThemeDefinition } from "@/lib/api";
import { readStoredUser, type StoredUser } from "./client-store";
import {
  applyDashboardTheme,
  cacheDashboardTheme,
  clearDashboardThemeCache,
  DASHBOARD_THEME_CACHE_KEY,
  DASHBOARD_THEME_EVENT,
  readCachedDashboardTheme,
} from "./dashboard-theme";

interface OwnedDashboardTheme {
  userId: string;
  theme: ThemeDefinition | null;
}

// Owns every side effect that binds the per-user dashboard theme to the
// document: initial fetch, cross-tab/custom-event sync, dark-mode reapply and
// teardown when leaving dashboard routes. `theme` is only a reapply trigger.
export function useDashboardTheme({
  user,
  dashboardPath,
  theme,
}: {
  user: StoredUser | null;
  dashboardPath: boolean;
  theme: "light" | "dark";
}): void {
  const [dashboardTheme, setDashboardTheme] = useState<OwnedDashboardTheme>(() => {
    const storedUser = readStoredUser();
    return {
      userId: storedUser?.id ?? "",
      theme: storedUser ? readCachedDashboardTheme(storedUser.id) : null,
    };
  });
  const dashboardThemeRevisionRef = useRef(0);

  useEffect(() => {
    if (!dashboardPath) {
      applyDashboardTheme(null);
      return;
    }
    if (!user) {
      // useStoredUser hydrates from a null server snapshot. Keep the inline
      // cached theme intact until the client identity snapshot arrives.
      if (readStoredUser()) return;
      applyDashboardTheme(null);
      clearDashboardThemeCache();
      return;
    }
    const cachedTheme = readCachedDashboardTheme(user.id);
    applyDashboardTheme(cachedTheme);
    let cancelled = false;
    const requestRevision = dashboardThemeRevisionRef.current;
    apiClient.listThemes({ bypassClientCache: true }).then((response) => {
      if (cancelled || requestRevision !== dashboardThemeRevisionRef.current) return;
      if (response.success && response.data) {
        const selected = response.data.themes.find(
          (item) => item.id === response.data?.selected_dashboard_theme_id,
        );
        const nextTheme = selected ?? null;
        cacheDashboardTheme(user.id, nextTheme);
        setDashboardTheme({ userId: user.id, theme: nextTheme });
        applyDashboardTheme(nextTheme);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [dashboardPath, user]);

  useEffect(() => {
    if (!dashboardPath) return;
    return () => applyDashboardTheme(null);
  }, [dashboardPath]);

  useEffect(() => {
    if (!user) return;
    const handleThemeChange = (event: Event) => {
      if (event.type === "storage") {
        const storageKey = (event as StorageEvent).key;
        if (storageKey === "user") {
          apiClient.invalidateCachedReads();
          return;
        }
        if (storageKey !== DASHBOARD_THEME_CACHE_KEY) return;
        apiClient.invalidateCachedReads();
      }
      const detail = (event as CustomEvent<{
        userId?: string;
        theme?: ThemeDefinition | null;
      }>).detail;
      if (event.type !== "storage" && detail?.userId && detail.userId !== user.id) return;
      dashboardThemeRevisionRef.current += 1;
      const nextTheme = detail && "theme" in detail
        ? detail.theme ?? null
        : readCachedDashboardTheme(user.id);
      setDashboardTheme({ userId: user.id, theme: nextTheme });
      if (dashboardPath) applyDashboardTheme(nextTheme);
    };
    window.addEventListener(DASHBOARD_THEME_EVENT, handleThemeChange);
    window.addEventListener("storage", handleThemeChange);
    return () => {
      window.removeEventListener(DASHBOARD_THEME_EVENT, handleThemeChange);
      window.removeEventListener("storage", handleThemeChange);
    };
  }, [dashboardPath, user]);

  useEffect(() => {
    if (!dashboardPath) {
      applyDashboardTheme(null);
      return;
    }
    if (!user) {
      if (!readStoredUser()) applyDashboardTheme(null);
      return;
    }
    const nextTheme = dashboardTheme.userId === user.id
      ? dashboardTheme.theme
      : readCachedDashboardTheme(user.id);
    applyDashboardTheme(nextTheme);
  }, [dashboardPath, dashboardTheme, theme, user]);
}
