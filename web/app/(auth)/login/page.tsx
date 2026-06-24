"use client";

import Link from "next/link";
import { FormEvent, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { apiClient, type OAuthProvider } from "@/lib/api";
import {
  BrutalCard,
  Field,
  InlineError,
  buttonClass,
  inputClass,
  responseError,
  setStoredUser,
  type StoredUser,
} from "@/app/components/M7Primitives";
import { sanitizeReturnTo } from "@/app/lib/format";
import { useI18n } from "@/lib/use-i18n";

export default function LoginPage() {
  const router = useRouter();
  const { t: copy } = useI18n();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [mfaRequired, setMfaRequired] = useState(false);
  const [oauthProviders, setOauthProviders] = useState<OAuthProvider[]>([]);
  // Read return_to synchronously on first client render (no setTimeout(0)
  // deferral, which left a tick where returnTo was the stale /dashboard).
  const [returnTo] = useState(() =>
    typeof window === "undefined"
      ? "/dashboard"
      : sanitizeReturnTo(new URLSearchParams(window.location.search).get("return_to")),
  );
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    async function loadProviders() {
      const response = await apiClient.listOAuthProviders();
      if (!cancelled && response.success && response.data) {
        setOauthProviders(response.data.providers ?? []);
      }
    }
    void loadProviders();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    setLoading(true);

    const response = await apiClient.login(username, password, mfaRequired ? totpCode : undefined);
    setLoading(false);

    if (response.success && response.data) {
      const data = response.data as { user?: unknown; mfa_required?: boolean };
      if (data.mfa_required) {
        setMfaRequired(true);
        setTotpCode("");
        return;
      }
      if (data.user) {
        setStoredUser(data.user as StoredUser);
        router.push(returnTo);
        return;
      }
    }

    setError(responseError(response));
  }

  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-10">
      <div className="w-full max-w-md">
        <Link href="/status" className="mb-6 inline-block border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
          {copy.loginPage.backToStatus}
        </Link>
        <BrutalCard className="p-6 sm:p-8" accent>
          <div className="mb-8">
            <p className="mb-2 inline-block border-2 border-black bg-white px-3 py-1 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
              {copy.loginPage.adminEntry}
            </p>
            <h1 className="text-5xl font-black uppercase tracking-tight">XLStatus</h1>
            <p className="mt-2 text-sm font-bold text-[var(--text-muted)]">
              {copy.loginPage.tagline}
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-5">
            <InlineError message={error} />
            <Field label={copy.loginPage.username}>
              <input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                required
                autoComplete="username"
                className={inputClass}
                placeholder="admin"
              />
            </Field>
            <Field label={copy.loginPage.password}>
              <input
                type="password"
                value={password}
                onChange={(event) => {
                  setPassword(event.target.value);
                  setMfaRequired(false);
                  setTotpCode("");
                }}
                required
                autoComplete="current-password"
                className={inputClass}
                placeholder={copy.loginPage.passwordPlaceholder}
              />
            </Field>
            {mfaRequired ? (
              <Field label={copy.loginPage.totpLabel}>
                <input
                  value={totpCode}
                  onChange={(event) => setTotpCode(event.target.value.replace(/\D/g, "").slice(0, 6))}
                  required
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  className={inputClass}
                  placeholder="123456"
                />
              </Field>
            ) : null}
            <button type="submit" disabled={loading} className={`${buttonClass("primary")} w-full`}>
              {loading ? copy.loginPage.loggingIn : mfaRequired ? copy.loginPage.verifyAndLogin : copy.common.login}
            </button>
          </form>
          {oauthProviders.length > 0 ? (
            <div className="mt-5 grid gap-2 border-t-2 border-black pt-5">
              {oauthProviders.map((provider) => (
                <a
                  key={provider.id}
                  className={`${buttonClass("secondary")} text-center`}
                  href={apiClient.getOAuthLoginUrl(provider.id, returnTo)}
                >
                  {copy.loginPage.loginWith.replace("{provider}", provider.display_name)}
                </a>
              ))}
            </div>
          ) : null}
        </BrutalCard>
      </div>
    </main>
  );
}
