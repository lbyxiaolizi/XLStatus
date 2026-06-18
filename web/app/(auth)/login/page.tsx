"use client";

import Link from "next/link";
import { FormEvent, useState } from "react";
import { useRouter } from "next/navigation";
import { apiClient } from "@/lib/api";
import { BrutalCard, Field, InlineError, buttonClass, inputClass, responseError } from "@/app/components/M7Primitives";

export default function LoginPage() {
  const router = useRouter();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    setLoading(true);

    const response = await apiClient.login(username, password);
    setLoading(false);

    if (response.success && response.data) {
      const data = response.data as { session_token?: string; user?: unknown };
      localStorage.setItem("session_token", data.session_token || "");
      localStorage.setItem("user", JSON.stringify(data.user));
      router.push("/dashboard");
      return;
    }

    setError(responseError(response));
  }

  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-10">
      <div className="w-full max-w-md">
        <Link href="/status" className="mb-6 inline-block border-2 border-black bg-[var(--bg-card)] px-3 py-2 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
          返回状态页
        </Link>
        <BrutalCard className="p-6 sm:p-8" accent>
          <div className="mb-8">
            <p className="mb-2 inline-block border-2 border-black bg-white px-3 py-1 text-xs font-black uppercase shadow-[var(--shadow-brutal-sm)]">
              管理员入口
            </p>
            <h1 className="text-5xl font-black uppercase tracking-tight">XLStatus</h1>
            <p className="mt-2 text-sm font-bold text-[var(--text-muted)]">
              登录后管理服务器、服务、任务和远程运维能力。
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-5">
            <InlineError message={error} />
            <Field label="用户名">
              <input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                required
                autoComplete="username"
                className={inputClass}
                placeholder="admin"
              />
            </Field>
            <Field label="密码">
              <input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                required
                autoComplete="current-password"
                className={inputClass}
                placeholder="admin123"
              />
            </Field>
            <button type="submit" disabled={loading} className={`${buttonClass("primary")} w-full`}>
              {loading ? "正在登录..." : "登录"}
            </button>
          </form>
        </BrutalCard>
      </div>
    </main>
  );
}
