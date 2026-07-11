"use client";

import { Fragment, type ReactNode, useEffect } from "react";
import { useRouter } from "next/navigation";
import { readStoredUser, useStoredUser } from "@/app/lib/client-store";

export default function DashboardIdentityBoundary({ children }: { children: ReactNode }) {
  const router = useRouter();
  const user = useStoredUser();
  const identityKey = user?.id ?? readStoredUser()?.id ?? "anonymous";

  useEffect(() => {
    if (!user && !readStoredUser()) router.replace("/login");
  }, [router, user]);

  return <Fragment key={identityKey}>{children}</Fragment>;
}
