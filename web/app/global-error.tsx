"use client";

import { useEffect } from "react";
import { getTranslations } from "@/lib/i18n";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const copy = getTranslations().errorsPage;

  useEffect(() => {
    console.error("Global error:", error);
  }, [error]);

  return (
    <html lang="zh-CN">
      <body
        style={{
          margin: 0,
          minHeight: "100vh",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontFamily: "system-ui, -apple-system, sans-serif",
          background: "#09090b",
          color: "#f4f4f5",
        }}
      >
        <div
          style={{
            maxWidth: 460,
            margin: "0 16px",
            padding: 24,
            borderRadius: 12,
            border: "1px solid rgba(255, 255, 255, 0.12)",
            background: "#121215",
            boxShadow: "0 20px 25px -5px rgba(0, 0, 0, 0.5)",
          }}
        >
          <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>
            {copy.appError}
          </h1>
          <p style={{ marginTop: 10, fontSize: 14, color: "#a1a1aa", lineHeight: 1.5 }}>
            {error.message || copy.unexpectedShort}
          </p>
          <button
            type="button"
            onClick={reset}
            style={{
              marginTop: 20,
              padding: "8px 16px",
              borderRadius: 8,
              border: "none",
              background: "#10b981",
              color: "#09090b",
              fontSize: 13,
              fontWeight: 600,
              cursor: "pointer",
            }}
          >
            {copy.reload}
          </button>
        </div>
      </body>
    </html>
  );
}
