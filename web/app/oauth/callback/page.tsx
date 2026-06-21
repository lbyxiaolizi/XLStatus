"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { apiClient } from "@/lib/api";
import { BrutalCard, InlineError, InlineNotice, buttonClass, responseError } from "@/app/components/M7Primitives";

export default function OAuthCallbackPage() {
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("正在完成 OAuth 登录...");

  useEffect(() => {
    let cancelled = false;
    async function completeOAuth() {
      const params = new URLSearchParams(window.location.search);
      const status = params.get("oauth");
      const returnTo = sanitizeReturnTo(params.get("return_to"));
      if (status !== "success") {
        if (!cancelled) {
          setNotice("");
          setError(params.get("message") || "OAuth 登录失败。");
        }
        return;
      }

      const response = await apiClient.getProfile();
      if (cancelled) return;
      if (response.success && response.data) {
        localStorage.removeItem("session_token");
        localStorage.setItem("user", JSON.stringify(response.data));
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
  }, [router]);

  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-10">
      <div className="w-full max-w-md">
        <BrutalCard accent className="p-6 sm:p-8">
          <h1 className="mb-4 text-3xl font-black uppercase">OAuth</h1>
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
          <InlineError message={error} />
          {error ? (
            <Link href="/login" className={`${buttonClass("primary")} mt-5 w-full text-center`}>
              返回登录
            </Link>
          ) : null}
        </BrutalCard>
      </div>
    </main>
  );
}

function sanitizeReturnTo(value: string | null): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) return "/dashboard";
  return value;
}
