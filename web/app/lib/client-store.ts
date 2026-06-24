"use client";

// Synchronous client stores for auth identity and theme, backed by
// localStorage and exposed via useSyncExternalStore.
//
// Previously useStoredUser / useBoldTheme lived in M7Primitives and read their
// value inside a `setTimeout(0)` effect, so the first render always returned the
// logged-out / light default and then swapped — a visible flicker (e.g. the nav
// rendering the public menu for a logged-in user on every page load). Reading
// synchronously here removes that flash. getServerSnapshot returns the default
// so SSR markup matches the very first client paint before hydration.

import { useSyncExternalStore } from "react";

export interface StoredUser {
  id: string;
  username: string;
  role: string;
}

declare global {
  interface Window {
    applyBoldTheme?: () => void;
  }
}

const USER_EVENT = "xlstatus:user-change";
const THEME_EVENT = "xlstatus:theme-change";

// --- shared subscription helper -------------------------------------------

function makeSubscribe(eventName: string) {
  return (callback: () => void) => {
    if (typeof window === "undefined") return () => {};
    window.addEventListener("storage", callback);
    window.addEventListener(eventName, callback);
    return () => {
      window.removeEventListener("storage", callback);
      window.removeEventListener(eventName, callback);
    };
  };
}

// --- user store ------------------------------------------------------------

// Cache the parsed snapshot keyed on the raw string so getSnapshot returns a
// stable reference between renders (required by useSyncExternalStore — a fresh
// object every call would loop).
let cachedUserRaw: string | null = null;
let cachedUser: StoredUser | null = null;

export function readStoredUser(): StoredUser | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem("user");
  if (raw === cachedUserRaw) return cachedUser;
  cachedUserRaw = raw;
  if (!raw) {
    cachedUser = null;
    return null;
  }
  try {
    cachedUser = JSON.parse(raw) as StoredUser;
  } catch {
    cachedUser = null;
  }
  return cachedUser;
}

const subscribeUser = makeSubscribe(USER_EVENT);
const getUserServerSnapshot = (): StoredUser | null => null;

export function useStoredUser(): StoredUser | null {
  return useSyncExternalStore(subscribeUser, readStoredUser, getUserServerSnapshot);
}

export function isAdmin(user: StoredUser | null): boolean {
  return user?.role?.toLowerCase() === "admin";
}

// Persist the logged-in user and notify same-tab subscribers (the native
// `storage` event only fires in other tabs).
export function setStoredUser(user: StoredUser): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem("user", JSON.stringify(user));
  window.localStorage.removeItem("session_token");
  window.dispatchEvent(new Event(USER_EVENT));
}

export function clearStoredUser(): void {
  if (typeof window === "undefined") return;
  window.localStorage.removeItem("user");
  window.localStorage.removeItem("session_token");
  window.dispatchEvent(new Event(USER_EVENT));
}

// --- theme store -----------------------------------------------------------

const subscribeTheme = makeSubscribe(THEME_EVENT);
const getThemeSnapshot = (): "light" | "dark" => {
  if (typeof window === "undefined") return "light";
  return window.localStorage.getItem("darkMode") === "true" ? "dark" : "light";
};
const getThemeServerSnapshot = (): "light" | "dark" => "light";

export function useBoldTheme(): ["light" | "dark", (mode: "light" | "dark") => void] {
  const mode = useSyncExternalStore(subscribeTheme, getThemeSnapshot, getThemeServerSnapshot);

  function setMode(nextMode: "light" | "dark") {
    if (typeof window === "undefined") return;
    window.localStorage.setItem("darkMode", nextMode === "dark" ? "true" : "false");
    window.applyBoldTheme?.();
    window.dispatchEvent(new Event(THEME_EVENT));
  }

  return [mode, setMode];
}
