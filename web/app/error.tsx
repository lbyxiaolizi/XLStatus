"use client";

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
    "inline-flex min-h-9 items-center justify-center rounded-lg border px-4 py-2 text-xs font-medium tracking-tight transition";

  return (
    <main className="mx-auto flex w-full max-w-2xl flex-col gap-5 px-4 py-16">
      <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-6 shadow-[var(--shadow-sm)]">
        <p className="inline-flex items-center gap-1.5 rounded-md border border-rose-500/30 bg-rose-500/10 px-2.5 py-1 text-[11px] font-semibold uppercase tracking-wider text-rose-600 dark:text-rose-400">
          <span className="h-1.5 w-1.5 rounded-full bg-rose-500" />
          {copy.pageError}
        </p>
        <h1 className="mt-4 text-xl font-semibold text-[var(--text-main)]">
          {copy.loadFailed}
        </h1>
        <p className="mt-2 text-sm text-[var(--text-muted)]">
          {error.message || copy.unexpected}
          {error.digest ? (
            <span className="ml-2 font-mono text-xs opacity-60">
              ({error.digest})
            </span>
          ) : null}
        </p>
        <div className="mt-6 flex flex-wrap gap-2.5">
          <button
            type="button"
            onClick={reset}
            className={`${buttonBase} border-transparent bg-[var(--btn-bg)] text-[var(--btn-text)] hover:opacity-90`}
          >
            {copy.retry}
          </button>
          <a
            href="/status"
            className={`${buttonBase} border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-main)] hover:bg-[var(--bg-hover)]`}
          >
            {copy.backToStatus}
          </a>
        </div>
      </div>
    </main>
  );
}
