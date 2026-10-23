import assert from "node:assert/strict";
import { test } from "node:test";
import {
  SECURITY_RESPONSE_HEADERS,
  encodePathSegment,
  escapeShellDoubleQuoted,
  hasControlCharacters,
  isValidCssVariableName,
  maskSensitiveToken,
  normalizeRemotePosixPath,
  redactSensitiveTokens,
  resolveLoginRedirectParam,
  resolveSafeApiUrl,
  sanitizeAssetUrl,
  sanitizeCssVariableValue,
  sanitizeDownloadFilename,
  sanitizeExternalHttpUrl,
  sanitizeRedirectPath,
  sanitizeThemeVariables,
} from "./security.ts";
import {
  MAX_LINE_BYTES,
  createBoundedLineReader,
  forwardJsonRpc,
  redactSensitiveTokens as redactBridgeTokens,
} from "../../../scripts/xlstatus-mcp-stdio.mjs";

test("sanitizeRedirectPath allows valid same-origin paths and blocks open redirects", () => {
  assert.equal(sanitizeRedirectPath("/dashboard"), "/dashboard");
  assert.equal(
    sanitizeRedirectPath("/servers/srv-01?tab=metrics#cpu"),
    "/servers/srv-01?tab=metrics#cpu",
  );
  assert.equal(sanitizeRedirectPath("/mcp"), "/mcp");

  // Open redirect vectors must fall back to /dashboard
  const maliciousRedirects = [
    null,
    undefined,
    "",
    "   ",
    " /dashboard",
    "/dashboard ",
    "//evil.example.com",
    "///evil.example.com",
    "/\\evil.example.com",
    "/servers\\..\\evil.example.com",
    "https://evil.example.com/phish",
    "http://evil.example.com",
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "/%2f/evil.example.com",
    "/%2F/evil.example.com",
    "/%5c/evil.example.com",
    "/%09/evil.example.com",
    "/%00/evil.example.com",
    "/\t/evil.example.com",
    "/\n/evil.example.com",
    "/\r/evil.example.com",
    "/login",
    "/login?return_to=/servers",
    "/login/sso",
    "/oauth/callback",
    "/oauth/callback?oauth=success",
  ];

  for (const vector of maliciousRedirects) {
    assert.equal(
      sanitizeRedirectPath(vector),
      "/dashboard",
      `expected ${String(vector)} to fall back to /dashboard`,
    );
  }
});

test("resolveLoginRedirectParam validates return_to, next, and redirect query parameters", () => {
  assert.equal(resolveLoginRedirectParam("?return_to=%2Fservers%2F1"), "/servers/1");
  assert.equal(resolveLoginRedirectParam("?next=%2Fterminal"), "/terminal");
  assert.equal(resolveLoginRedirectParam("?redirect=%2Fsettings"), "/settings");

  assert.equal(resolveLoginRedirectParam("?next=//evil.example.com"), "/dashboard");
  assert.equal(resolveLoginRedirectParam("?redirect=https://evil.example.com"), "/dashboard");
  assert.equal(resolveLoginRedirectParam("?return_to=/\\evil.example.com"), "/dashboard");
  assert.equal(resolveLoginRedirectParam("?next=javascript:alert(1)"), "/dashboard");
});

test("sanitizeAssetUrl allows safe HTTP(S) and relative URLs while rejecting XSS and CSS breakout payloads", () => {
  assert.equal(sanitizeAssetUrl("https://cdn.example.com/brand/logo.png?v=1"), "https://cdn.example.com/brand/logo.png?v=1");
  assert.equal(sanitizeAssetUrl("http://127.0.0.1:8080/assets/favicon.ico"), "http://127.0.0.1:8080/assets/favicon.ico");
  assert.equal(sanitizeAssetUrl("/public/bg.webp"), "/public/bg.webp");

  const unsafeUrls = [
    null,
    undefined,
    "",
    "javascript:alert(1)",
    "JAVASCRIPT:alert(document.cookie)",
    "java\nscript:alert(1)",
    "vbscript:msgbox(1)",
    "data:text/html,<script>alert(1)</script>",
    "data:image/svg+xml;base64,PHN2Zy8+",
    "file:///etc/passwd",
    "//evil.example.com/logo.png",
    "/\\evil.example.com/logo.png",
    "/%2f/evil.example.com/logo.png",
    "https://user:pass@example.com/logo.png",
    "https://example.com/bg.png);background:url(javascript:alert(1)",
    "https://example.com/bg.png\" onerror=\"alert(1)",
    "https://example.com/bg.png' onload='alert(1)",
    "https://example.com/bg.png\\29 ;",
    "https://example.com/<script>.png",
  ];

  for (const unsafe of unsafeUrls) {
    assert.equal(
      sanitizeAssetUrl(unsafe),
      null,
      `expected unsafe asset URL ${String(unsafe)} to be rejected`,
    );
  }
});

test("sanitizeExternalHttpUrl validates external redirect targets", () => {
  assert.equal(
    sanitizeExternalHttpUrl("https://github.com/login/oauth/authorize?client_id=123"),
    "https://github.com/login/oauth/authorize?client_id=123",
  );
  assert.equal(sanitizeExternalHttpUrl("/settings"), "/settings");
  assert.equal(sanitizeExternalHttpUrl("javascript:alert(1)"), null);
  assert.equal(sanitizeExternalHttpUrl("data:text/html,hi"), null);
  assert.equal(sanitizeExternalHttpUrl("//evil.example.com"), null);
});

test("sanitizeCssVariableValue and sanitizeThemeVariables block CSS and style-tag injection", () => {
  assert.equal(isValidCssVariableName("--bg-page"), true);
  assert.equal(isValidCssVariableName("--accent_color_1"), true);
  assert.equal(isValidCssVariableName("background-color"), false);
  assert.equal(isValidCssVariableName("--bad: red; } body {"), false);

  assert.equal(sanitizeCssVariableValue("#16a34a"), "#16a34a");
  assert.equal(sanitizeCssVariableValue("rgb(17, 24, 39)"), "rgb(17, 24, 39)");
  assert.equal(sanitizeCssVariableValue("rgba(255, 255, 255, 0.85)"), "rgba(255, 255, 255, 0.85)");
  assert.equal(sanitizeCssVariableValue("var(--accent-color)"), "var(--accent-color)");

  const unsafeCssValues = [
    null,
    undefined,
    "",
    "#fff; background: url(javascript:alert(1))",
    "#fff } body { display: none }",
    "</style><script>alert(1)</script>",
    "@import url(https://evil.example.com/steal.css)",
    "url(javascript:alert(1))",
    "url(https://evil.example.com/pixel.png)",
    "expression(alert(1))",
    "rgb(1, 2, 3))",
    "rgb((1, 2, 3)",
    "#fff /* comment */",
    "#fff\\3b color:red",
    "\"quoted\"",
    "'single-quoted'",
    "#fff\ncolor:red",
  ];

  for (const unsafe of unsafeCssValues) {
    assert.equal(
      sanitizeCssVariableValue(unsafe),
      null,
      `expected unsafe CSS value ${String(unsafe)} to be rejected`,
    );
  }

  const cleaned = sanitizeThemeVariables({
    "--bg-page": "#f8fafc",
    "--text-main": "rgb(17, 24, 39)",
    "--injected": "red; } </style><script>alert(1)</script>",
    "--tracking": "url(https://evil.example.com/pixel)",
    "not-a-var": "#000000",
  });
  assert.deepEqual(cleaned, {
    "--bg-page": "#f8fafc",
    "--text-main": "rgb(17, 24, 39)",
  });
});

test("redactSensitiveTokens and maskSensitiveToken protect PAT and enrollment tokens", () => {
  const rawError =
    "Request failed with Authorization: Bearer xlp_9f8e7d6c5b4a3210secret and enroll xle_1234567890abcdef";
  const redacted = redactSensitiveTokens(rawError);
  assert.equal(redacted.includes("xlp_9f8e7d6c5b4a3210secret"), false);
  assert.equal(redacted.includes("xle_1234567890abcdef"), false);
  assert.equal(
    redacted,
    "Request failed with Authorization: Bearer xlp_[REDACTED] and enroll xle_[REDACTED]",
  );

  assert.equal(maskSensitiveToken("xlp_1234567890abcdef"), "xlp_1234••••••••cdef");
  assert.equal(hasControlCharacters("normal-text"), false);
  assert.equal(hasControlCharacters("bad\u0000text"), true);
});

test("SECURITY_RESPONSE_HEADERS includes all required defensive HTTP headers", () => {
  const headerMap = new Map(SECURITY_RESPONSE_HEADERS.map((h) => [h.key, h.value]));
  assert.equal(headerMap.get("X-Content-Type-Options"), "nosniff");
  assert.equal(headerMap.get("X-Frame-Options"), "DENY");
  assert.equal(headerMap.get("Referrer-Policy"), "strict-origin-when-cross-origin");
  assert.equal(headerMap.get("Permissions-Policy"), "camera=(), microphone=(), geolocation=()");
});

test("MCP stdio bridge bounds line length and redacts xlp_ tokens in error output", async () => {
  const lines: string[] = [];
  let oversizedCount = 0;
  const reader = createBoundedLineReader(
    (line: string) => lines.push(line),
    () => {
      oversizedCount += 1;
    },
    64,
  );

  reader.push(`${"a".repeat(100)}\n{"jsonrpc":"2.0","id":1}\n`);
  reader.end();

  assert.equal(oversizedCount, 1);
  assert.deepEqual(lines, ['{"jsonrpc":"2.0","id":1}']);

  const stdoutChunks: string[] = [];
  await forwardJsonRpc(
    JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/list" }),
    {
      endpoint: "http://127.0.0.1:8080/mcp",
      token: "xlp_super_secret_pat_token_123",
      writeStdout: (chunk: string) => {
        stdoutChunks.push(chunk);
      },
      fetchImpl: async () => {
        throw new Error("Rejected token xlp_super_secret_pat_token_123 at upstream");
      },
    },
  );

  assert.equal(stdoutChunks.length, 1);
  const response = JSON.parse(stdoutChunks[0]!) as {
    id: number;
    error: { code: number; message: string };
  };
  assert.equal(response.id, 7);
  assert.equal(response.error.message.includes("xlp_super_secret_pat_token_123"), false);
  assert.equal(response.error.message.includes("xlp_[REDACTED]"), true);
  assert.equal(MAX_LINE_BYTES, 1024 * 1024);
  assert.equal(
    redactBridgeTokens("token=xlp_abc123xyz", "xlp_abc123xyz"),
    "token=xlp_[REDACTED]",
  );
});

test("encodePathSegment prevents WHATWG URL dot-segment traversal and encodes slashes", () => {
  assert.equal(encodePathSegment("srv-01"), "srv-01");
  assert.equal(encodePathSegment("a/b"), "a%2Fb");
  assert.equal(encodePathSegment("."), "_invalid_");
  assert.equal(encodePathSegment(".."), "_invalid_");
  assert.equal(encodePathSegment("%2e"), "_invalid_");
  assert.equal(encodePathSegment("%2E%2E"), "_invalid_");
  assert.equal(encodePathSegment(".%2e"), "_invalid_");
  assert.equal(encodePathSegment("%2e."), "_invalid_");
  assert.equal(encodePathSegment(""), "_invalid_");
  assert.equal(encodePathSegment("   "), "_invalid_");
  assert.equal(encodePathSegment("bad\u0000id"), "_invalid_");

  const resolvedUrl = new URL(
    `/api/v1/servers/${encodePathSegment("..")}/files`,
    "https://status.example.com",
  );
  assert.equal(resolvedUrl.pathname, "/api/v1/servers/_invalid_/files");
});

test("resolveSafeApiUrl validates relative and same-origin transfer URLs", () => {
  assert.equal(
    resolveSafeApiUrl("/api/v1/files/temp/abc?token=1", "https://api.example.com:8443"),
    "https://api.example.com:8443/api/v1/files/temp/abc?token=1",
  );
  assert.equal(
    resolveSafeApiUrl("/api/v1/files/temp/abc", ""),
    "/api/v1/files/temp/abc",
  );
  assert.equal(
    resolveSafeApiUrl(
      "https://api.example.com:8443/api/v1/files/temp/abc",
      "https://api.example.com:8443",
    ),
    "https://api.example.com:8443/api/v1/files/temp/abc",
  );

  // Reject cross-origin, protocol-relative, credentials, dangerous schemes, and dot traversal
  assert.equal(
    resolveSafeApiUrl("https://evil.example.com/api/v1/files/temp/abc", "https://api.example.com"),
    null,
  );
  assert.equal(resolveSafeApiUrl("//evil.example.com/file", "https://api.example.com"), null);
  assert.equal(resolveSafeApiUrl("javascript:alert(1)", "https://api.example.com"), null);
  assert.equal(resolveSafeApiUrl("data:text/html,evil", "https://api.example.com"), null);
  assert.equal(
    resolveSafeApiUrl("https://user:pass@api.example.com/file", "https://api.example.com"),
    null,
  );
  assert.equal(
    resolveSafeApiUrl("/api/v1/files/../../admin", "https://api.example.com"),
    null,
  );
  assert.equal(
    resolveSafeApiUrl("https://api.example.com/api/v1/files/%2e%2e/admin", "https://api.example.com"),
    null,
  );
});

test("sanitizeDownloadFilename and normalizeRemotePosixPath block path traversal", () => {
  assert.equal(sanitizeDownloadFilename("xlstatus-backup-2026.sqlite"), "xlstatus-backup-2026.sqlite");
  assert.equal(sanitizeDownloadFilename("../../etc/passwd", "backup.db"), "passwd");
  assert.equal(sanitizeDownloadFilename("..\\..\\evil.exe", "backup.db"), "evil.exe");
  assert.equal(sanitizeDownloadFilename(".bashrc", "backup.db"), "bashrc");
  assert.equal(sanitizeDownloadFilename("..", "backup.db"), "backup.db");
  assert.equal(sanitizeDownloadFilename(".", "backup.db"), "backup.db");
  assert.equal(sanitizeDownloadFilename("bad\u0000name?.db", "backup.db"), "badname_.db");
  assert.equal(sanitizeDownloadFilename("", "backup.db"), "backup.db");

  assert.equal(normalizeRemotePosixPath("/var/log/../log/syslog"), "/var/log/syslog");
  assert.equal(normalizeRemotePosixPath("../../../etc/shadow"), "/etc/shadow");
  assert.equal(normalizeRemotePosixPath("bad\u0000path"), null);

  assert.equal(
    escapeShellDoubleQuoted('node"$(rm -rf /)`\\$HOME\n'),
    'node\\"\\$(rm -rf /)\\`\\\\\\$HOME',
  );
});

