"use client";

// Re-entrancy guard for mutation handlers, generalized from the servers
// page's runExclusive: the ref gives a synchronous double-click guard (state
// alone lags a render behind), the state drives the disabled/busy UI.

import { useCallback, useRef, useState } from "react";

export function useBusy(): [boolean, (fn: () => Promise<void>) => Promise<void>] {
  const busyRef = useRef(false);
  const [busy, setBusy] = useState(false);

  const run = useCallback(async (fn: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      await fn();
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }, []);

  return [busy, run];
}
