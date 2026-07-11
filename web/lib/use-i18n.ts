"use client";

import { useEffect, useSyncExternalStore } from "react";
import {
  getLocale,
  getTranslations,
  setLocale as persistLocale,
  supportedLocales,
  type Locale,
} from "@/lib/i18n";

export function useI18n() {
  const locale = useSyncExternalStore(
    subscribeLocale,
    getLocale,
    getLocaleServerSnapshot,
  );

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

const getLocaleServerSnapshot = (): Locale => supportedLocales[0];
