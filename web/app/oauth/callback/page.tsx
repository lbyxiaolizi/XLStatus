"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { apiClient } from "@/lib/api";
import {
  BrutalCard,
  InlineError,
  InlineNotice,
  buttonClass,
  responseError,
  setStoredUser,
  type StoredUser,
} from "@/app/components/M7Primitives";
import { sanitizeReturnTo } from "@/app/lib/format";
import { useI18n } from "@/lib/use-i18n";

export default function OAuthCallbackPage() {
  const router = useRouter();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState(copy.loginPage.oauthCompleting);

  useEffect(() => {
    let cancelled = false;
    async function completeOAuth() {
      const params = new URLSearchParams(window.location.search);
      const status = params.get("oauth");
      const returnTo = sanitizeReturnTo(params.get("return_to"));
      if (status !== "success") {
        if (!cancelled) {
          setNotice("");
          setError(params.get("message") || copy.loginPage.oauthFailed);
        }
        return;
      }

      const response = await apiClient.getProfile();
      if (cancelled) return;
      if (response.success && response.data) {
        setStoredUser(response.data as unknown as StoredUser);
        router.replace(returnTo);
        return;
      }

      setNotice("");
      setError(responseError(response));
    }
    void completeOAuth();
    return () => {
      cancelled = true;
    };
  }, [router, copy]);

  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-10">
      <div className="w-full max-w-md">
        <BrutalCard accent className="p-6 sm:p-8">
          <h1 className="mb-4 text-3xl font-black uppercase">OAuth</h1>
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
          <InlineError message={error} />
          {error ? (
            <Link href="/login" className={`${buttonClass("primary")} mt-5 w-full text-center`}>
              {copy.loginPage.backToLogin}
            </Link>
          ) : null}
        </BrutalCard>
      </div>
    </main>
  );
}
