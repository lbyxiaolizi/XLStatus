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
import { sanitizeReturnTo } from "@/app/lib/format";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

// The generic responseError maps 401 to "authentication required" and 403 to
// "permission denied" — both are wrong on the login screen itself, where 401
// means bad credentials and 403 means this request was refused (WAF ban, or a
// stale-cookie CSRF rejection from the auth middleware, which arrives with no
// body). Every 403 gets login-context copy; the ban copy avoids asserting a
// cause or duration because manual WAF bans share the same backend message.
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
  // Read query params synchronously on first client render (no setTimeout(0)
  // deferral, which left a tick where returnTo was the stale /dashboard).
  const [returnTo] = useState(() =>
    typeof window === "undefined"
      ? "/dashboard"
      : sanitizeReturnTo(new URLSearchParams(window.location.search).get("return_to")),
  );
  // Set after mount rather than in the initializer: the notice changes the DOM,
  // and this page is prerendered, so reading the query during the first render
  // would make hydration mismatch the static HTML.
  const [sessionExpired, setSessionExpired] = useState(false);
  useEffect(() => {
    if (new URLSearchParams(window.location.search).get("reason") === "expired") {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- post-hydration reveal of query-dependent UI
      setSessionExpired(true);
    }
  }, []);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // A user with a live identity has no business on the login form; bounce them
  // to their destination. A stale identity (expired session) is fine: the first
  // authenticated request 401s, clears the stored user, and lands back here.
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
    // The expired-session context is stale the moment the user acts on the
    // form; without this it would flash back while the request is in flight.
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
        // No router.push here: the storedUser effect above is the single
        // redirect owner, and it fires as soon as setStoredUser lands.
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
                  // Symmetric with the password field: switching identity
                  // leaves the MFA step, otherwise the submit button stays
                  // disabled with no way to re-trigger it.
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
