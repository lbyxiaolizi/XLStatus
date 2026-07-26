"use client";

import { useCallback, useEffect, useRef } from "react";
import type { Dialogs } from "@/app/components/Dialogs";
import { responseError } from "@/app/components/M7Primitives";
import { apiClient } from "@/lib/api";
import { getTranslations } from "@/lib/i18n";

export type SensitiveTotpCode = () => Promise<string | undefined | null>;

// Sensitive admin calls only carry a TOTP code when the account has TOTP
// enabled. Resolves to `undefined` when no code is required, and to `null` when
// the status lookup failed or the user cancelled / typed an invalid code —
// callers must abort on `null`. The status read is served from the API client's
// short-lived cache, and any mutation invalidates that cache, so a freshly
// enabled or disabled TOTP is picked up on the next call.
export function useSensitiveTotp(
  dialogs: Dialogs,
  setError: (message: string | null) => void,
): SensitiveTotpCode {
  const dialogsRef = useRef(dialogs);
  const setErrorRef = useRef(setError);

  useEffect(() => {
    dialogsRef.current = dialogs;
    setErrorRef.current = setError;
  }, [dialogs, setError]);

  return useCallback(async () => {
    const response = await apiClient.getTotpStatus();
    if (!response.success || !response.data) {
      setErrorRef.current(responseError(response));
      return null;
    }
    if (!response.data.enabled) return undefined;
    const code = await dialogsRef.current.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setErrorRef.current(getTranslations().settingsPage.totpEnterSixDigits);
      return null;
    }
    return trimmed;
  }, []);
}
