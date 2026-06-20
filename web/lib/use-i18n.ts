"use client";

import { useEffect, useState } from "react";
import {
  getLocale,
  getTranslations,
  setLocale as persistLocale,
  supportedLocales,
  type Locale,
} from "@/lib/i18n";

export function useI18n() {
  const [locale, setLocaleState] = useState<Locale>(() => getLocale());

  useEffect(() => {
    function onLocaleChange() {
      setLocaleState(getLocale());
    }
    window.addEventListener("xlstatus:locale-change", onLocaleChange);
    window.addEventListener("storage", onLocaleChange);
    return () => {
      window.removeEventListener("xlstatus:locale-change", onLocaleChange);
      window.removeEventListener("storage", onLocaleChange);
    };
  }, []);

  function setLocale(locale: Locale) {
    persistLocale(locale);
    setLocaleState(locale);
  }

  return {
    locale,
    locales: supportedLocales,
    setLocale,
    t: getTranslations(locale),
  };
}
