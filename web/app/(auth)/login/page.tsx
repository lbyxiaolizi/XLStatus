"use client";

import Link from "next/link";
import { FormEvent, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { apiClient, type ApiResponse, type OAuthProvider } from "@/lib/api";
import {
  BrutalCard,
  Field,
  InlineError,
  InlineNotice,
  buttonClass,
  inputClass,
  responseError,
  setStoredUser,
  useStoredUser,
  type StoredUser,
} from "@/app/components/M7Primitives";
import { resolveLoginRedirectParam } from "@/app/lib/security";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

function loginErrorMessage(response: ApiResponse<unknown>, copy: Translations): string {
  const backendError = response.error ?? "";
  if (response.status === 401) {
    return /totp/i.test(backendError)
      ? copy.loginPage.errInvalidTotp
      : copy.loginPage.errInvalidCredentials;
  }
  if (response.status === 403) {
    return /waf|blocked/i.test(backendError)
      ? copy.loginPage.errIpBanned
      : copy.loginPage.errLoginRejected;
  }
  if ((response.status ?? 0) >= 500) {
    return copy.loginPage.errServerUnavailable;
  }
  return responseError(response);
}

export default function LoginPage() {
  const router = useRouter();
  const { t: copy } = useI18n();
  const storedUser = useStoredUser();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [mfaRequired, setMfaRequired] = useState(false);
  const [oauthProviders, setOauthProviders] = useState<OAuthProvider[]>([]);
  const [returnTo] = useState(() =>
    typeof window === "undefined"
      ? "/dashboard"
      : resolveLoginRedirectParam(window.location.search),
  );
  const [sessionExpired, setSessionExpired] = useState(false);
  useEffect(() => {
    if (new URLSearchParams(window.location.search).get("reason") === "expired") {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- post-hydration reveal of query-dependent UI
      setSessionExpired(true);
    }
  }, []);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (storedUser) router.replace(returnTo);
  }, [returnTo, router, storedUser]);

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
    setSessionExpired(false);
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
        return;
      }
    }

    setError(loginErrorMessage(response, copy));
  }

  if (storedUser) return null;

  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-10">
      <div className="w-full max-w-md">
        <Link href="/status" className={`${buttonClass("secondary")} mb-6`}>
          {copy.loginPage.backToStatus}
        </Link>
        <BrutalCard className="p-6 sm:p-8">
          <div className="mb-6">
            <p className="mb-2 inline-flex items-center gap-1.5 rounded-full border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2.5 py-0.5 text-[11px] font-medium tracking-wide text-[var(--text-muted)] uppercase">
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent-color)]" aria-hidden="true" />
              {copy.loginPage.adminEntry}
            </p>
            <h1 className="text-3xl font-semibold tracking-tight text-[var(--text-main)]">XLStatus</h1>
            <p className="mt-1.5 text-sm text-[var(--text-muted)]">
              {copy.loginPage.tagline}
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            {sessionExpired && !error ? (
              <InlineNotice tone="yellow">{copy.loginPage.sessionExpired}</InlineNotice>
            ) : null}
            <InlineError message={error} />
            <Field label={copy.loginPage.username}>
              <input
                name="username"
                value={username}
                onChange={(event) => {
                  setUsername(event.target.value);
                  setMfaRequired(false);
                  setTotpCode("");
                }}
                required
                autoComplete="username"
                className={inputClass}
                placeholder="admin"
              />
            </Field>
            <Field label={copy.loginPage.password}>
              <input
                name="password"
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
                  name="totp_code"
                  value={totpCode}
                  onChange={(event) => setTotpCode(event.target.value.replace(/\D/g, "").slice(0, 6))}
                  required
                  autoFocus
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  className={inputClass}
                  placeholder="123456"
                />
              </Field>
            ) : null}
            <button
              type="submit"
              disabled={loading || (mfaRequired && totpCode.length !== 6)}
              className={`${buttonClass("primary")} w-full`}
            >
              {loading ? copy.loginPage.loggingIn : mfaRequired ? copy.loginPage.verifyAndLogin : copy.common.login}
            </button>
          </form>
          {oauthProviders.length > 0 ? (
            <div className="mt-5 grid gap-2 border-t border-[var(--border-color)] pt-5">
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
