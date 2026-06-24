"use client";

// Last-resort boundary that catches errors thrown in the root layout itself.
// It replaces the entire document, so it must render its own <html>/<body>.

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
          fontFamily: "system-ui, sans-serif",
          background: "#f5f5f5",
          color: "#111",
        }}
      >
        <div
          style={{
            maxWidth: 480,
            margin: "0 16px",
            padding: 24,
            border: "4px solid #000",
            background: "#fff",
            boxShadow: "10px 10px 0 0 #000",
          }}
        >
          <h1 style={{ fontSize: 24, fontWeight: 900, textTransform: "uppercase", margin: 0 }}>
            {copy.appError}
          </h1>
          <p style={{ marginTop: 12, fontWeight: 700, color: "#555" }}>
            {error.message || copy.unexpectedShort}
          </p>
          <button
            type="button"
            onClick={reset}
            style={{
              marginTop: 20,
              padding: "8px 16px",
              border: "2px solid #000",
              background: "#000",
              color: "#fff",
              fontWeight: 900,
              textTransform: "uppercase",
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
