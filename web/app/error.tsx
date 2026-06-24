"use client";

// Route-level error boundary (App Router convention). Before this existed, a
// render-time throw anywhere in a page — e.g. the old `load_1.toFixed` on a null
// metric — blanked the whole app. Now it collapses to this recoverable panel
// while the root layout stays mounted.

import { useEffect } from "react";
import { getTranslations } from "@/lib/i18n";

export default function RouteError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const copy = getTranslations().errorsPage;

  useEffect(() => {
    console.error("Route error:", error);
  }, [error]);

  const buttonBase =
    "inline-flex min-h-10 items-center justify-center border-2 border-black px-4 py-2 text-sm font-black uppercase tracking-wide shadow-[2px_2px_0_0_#000] transition hover:-translate-x-0.5 hover:-translate-y-0.5";

  return (
    <main className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-4 py-16">
      <div className="border-4 border-black bg-[var(--bg-card)] p-6 shadow-[10px_10px_0_0_#000]">
        <p className="inline-block border-2 border-black bg-[var(--accent-bg)] px-3 py-1 text-xs font-black uppercase tracking-wide shadow-[2px_2px_0_0_#000]">
          {copy.pageError}
        </p>
        <h1 className="mt-4 text-2xl font-black uppercase text-[var(--text-main)]">{copy.loadFailed}</h1>
        <p className="mt-3 text-sm font-bold text-[var(--text-muted)]">
          {error.message || copy.unexpected}
          {error.digest ? <span className="ml-2 opacity-60">({error.digest})</span> : null}
        </p>
        <div className="mt-6 flex flex-wrap gap-3">
          <button type="button" onClick={reset} className={`${buttonBase} bg-black text-white`}>
            {copy.retry}
          </button>
          <a href="/status" className={`${buttonBase} bg-[var(--bg-card)] text-[var(--text-main)]`}>
            {copy.backToStatus}
          </a>
        </div>
      </div>
    </main>
  );
}
