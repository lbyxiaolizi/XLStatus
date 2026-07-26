"use client";

import { useEffect, useSyncExternalStore } from "react";
import {
  getLocale,
  getTranslations,
  isEnLoaded,
  normalizeLocale,
  setLocale as persistLocale,
  supportedLocales,
  type Locale,
} from "@/lib/i18n";

export function useI18n() {
  // The snapshot folds in whether the en-US chunk has arrived: the locale alone
  // is unchanged by the async load, so subscribers would never re-render into
  // the real English copy.
  const snapshot = useSyncExternalStore(
    subscribeLocale,
    getLocaleSnapshot,
    getLocaleServerSnapshot,
  );
  const locale = normalizeLocale(snapshot.split(":")[0]);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  function setLocale(locale: Locale) {
    persistLocale(locale);
  }

  return {
    locale,
    locales: supportedLocales,
    setLocale,
    t: getTranslations(locale),
  };
}

function subscribeLocale(onLocaleChange: () => void): () => void {
  window.addEventListener("xlstatus:locale-change", onLocaleChange);
  window.addEventListener("storage", onLocaleChange);
  return () => {
    window.removeEventListener("xlstatus:locale-change", onLocaleChange);
    window.removeEventListener("storage", onLocaleChange);
  };
}

const getLocaleSnapshot = (): string => getLocale() + ":" + (isEnLoaded() ? "1" : "0");

const getLocaleServerSnapshot = (): string => supportedLocales[0] + ":0";
