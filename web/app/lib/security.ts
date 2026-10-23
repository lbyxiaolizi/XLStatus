// Frontend security sanitization & validation helpers.
//
// Defends against:
// - Open redirects in login / OAuth return_to, next, and redirect parameters
// - DOM XSS & CSS url(...) breakout via untrusted branding asset URLs
// - CSS rule / <style> tag injection via theme custom properties and color tokens
// - Sensitive PAT / enrollment token leakage in error messages and previews

const INTERNAL_ORIGIN = "https://xlstatus.internal";
const MAX_REDIRECT_PATH_LENGTH = 2048;
const MAX_ASSET_URL_LENGTH = 2048;
const MAX_CSS_VARIABLE_NAME_LENGTH = 128;
const MAX_CSS_VARIABLE_VALUE_LENGTH = 256;

export const SECURITY_RESPONSE_HEADERS = [
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
] as const;

// True if the string contains any ASCII control character (0x00-0x1F, 0x7F)
// or Unicode C1 control character (0x80-0x9F).
export function hasControlCharacters(value: string): boolean {
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) {
      return true;
    }
  }
  return false;
}

// Validate a post-login or OAuth redirect target path.
// Only same-origin relative paths starting with a single `/` are permitted.
// Rejects protocol-relative URLs (`//evil.com`), backslash tricks (`/\evil.com`),
// percent-encoded double-slashes (`/%2f/evil.com`), control characters,
// whitespace padding, and auth loop paths (`/login`, `/oauth/callback`).
export function sanitizeRedirectPath(
  value: string | null | undefined,
  fallback = "/dashboard",
): string {
  if (!value || typeof value !== "string") return fallback;
  if (
    value.length > MAX_REDIRECT_PATH_LENGTH ||
    value !== value.trim() ||
    !value.startsWith("/") ||
    value.startsWith("//") ||
    value.includes("\\") ||
    hasControlCharacters(value)
  ) {
    return fallback;
  }

  const rawPathname = value.split(/[?#]/, 1)[0] ?? "";
  let decodedPathname: string;
  try {
    decodedPathname = decodeURIComponent(rawPathname);
  } catch {
    return fallback;
  }

  if (
    !decodedPathname.startsWith("/") ||
    decodedPathname.startsWith("//") ||
    decodedPathname.includes("\\") ||
    hasControlCharacters(decodedPathname)
  ) {
    return fallback;
  }

  try {
    const parsed = new URL(value, INTERNAL_ORIGIN);
    if (
      parsed.origin !== INTERNAL_ORIGIN ||
      !parsed.pathname.startsWith("/") ||
      parsed.pathname.startsWith("//") ||
      parsed.pathname.includes("\\")
    ) {
      return fallback;
    }
  } catch {
    return fallback;
  }

  const lowerPath = decodedPathname.toLowerCase();
  if (
    lowerPath === "/login" ||
    lowerPath.startsWith("/login/") ||
    lowerPath === "/oauth/callback" ||
    lowerPath.startsWith("/oauth/callback/")
  ) {
    return fallback;
  }

  return value;
}

// Resolve a redirect target from URL query parameters (`return_to`, `next`, or `redirect`).
export function resolveLoginRedirectParam(
  search: string | URLSearchParams | null | undefined,
  fallback = "/dashboard",
): string {
  if (!search) return fallback;
  const params = typeof search === "string" ? new URLSearchParams(search) : search;
  const candidate =
    params.get("return_to") ??
    params.get("next") ??
    params.get("redirect");
  return sanitizeRedirectPath(candidate, fallback);
}

// Sanitize a URL used for public branding assets (`logo_url`, `favicon_url`, `background_url`).
// Strictly allows only `http://`, `https://`, or safe same-origin `/...` relative paths,
// and rejects `javascript:`, `data:`, `vbscript:`, `file:`, credentials, whitespace,
// and CSS `url(...)` / HTML attribute breakout characters (`"`, `'`, `(`, `)`, `\`, `;`, `<`, `>`, `` ` ``, `{`, `}`).
export function sanitizeAssetUrl(value: string | null | undefined): string | null {
  if (!value || typeof value !== "string") return null;
  const trimmed = value.trim();
  if (
    !trimmed ||
    trimmed.length > MAX_ASSET_URL_LENGTH ||
    hasControlCharacters(trimmed) ||
    /[\s"'()\\;<>`{}]/.test(trimmed)
  ) {
    return null;
  }

  if (trimmed.startsWith("/")) {
    if (trimmed.startsWith("//")) return null;
    const rawPathname = trimmed.split(/[?#]/, 1)[0] ?? "";
    let decodedPathname: string;
    try {
      decodedPathname = decodeURIComponent(rawPathname);
    } catch {
      return null;
    }
    if (
      !decodedPathname.startsWith("/") ||
      decodedPathname.startsWith("//") ||
      decodedPathname.includes("\\") ||
      hasControlCharacters(decodedPathname)
    ) {
      return null;
    }
    try {
      const parsed = new URL(trimmed, INTERNAL_ORIGIN);
      if (parsed.origin !== INTERNAL_ORIGIN) return null;
    } catch {
      return null;
    }
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return null;
    }
    if (!parsed.hostname || parsed.username || parsed.password) {
      return null;
    }
    return trimmed;
  } catch {
    return null;
  }
}

// Validate an external HTTP(S) or same-origin redirect URL (e.g., OAuth authorization_url).
export function sanitizeExternalHttpUrl(value: string | null | undefined): string | null {
  if (!value || typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > MAX_ASSET_URL_LENGTH || hasControlCharacters(trimmed)) {
    return null;
  }

  if (trimmed.startsWith("/")) {
    if (trimmed.startsWith("//") || trimmed.includes("\\")) return null;
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return null;
    }
    if (!parsed.hostname || parsed.username || parsed.password) {
      return null;
    }
    return trimmed;
  } catch {
    return null;
  }
}

// Validate a CSS custom property name (`--foo-bar`).
export function isValidCssVariableName(name: string): boolean {
  if (!name || typeof name !== "string") return false;
  if (name.length > MAX_CSS_VARIABLE_NAME_LENGTH) return false;
  return /^--[a-zA-Z0-9_-]+$/.test(name);
}

// Sanitize a CSS variable or color value before applying via `style.setProperty` or `<style>`.
// Rejects `;`, `{`, `}`, `\`, `<`, `>`, quotes, CSS comments, `@import` / `@`-rules,
// `url(...)`, `expression(...)`, `javascript:`, `vbscript:`, `data:`, and unbalanced parentheses.
export function sanitizeCssVariableValue(value: string | null | undefined): string | null {
  if (!value || typeof value !== "string") return null;
  const trimmed = value.trim();
  if (
    !trimmed ||
    trimmed.length > MAX_CSS_VARIABLE_VALUE_LENGTH ||
    hasControlCharacters(trimmed)
  ) {
    return null;
  }

  if (/[;{}<>\\'"`@]/.test(trimmed)) {
    return null;
  }

  if (trimmed.includes("/*") || trimmed.includes("*/")) {
    return null;
  }

  if (
    /(?:url\s*\(|expression\s*\(|javascript\s*:|vbscript\s*:|data\s*:|-moz-binding|behavior\s*:)/i.test(
      trimmed,
    )
  ) {
    return null;
  }

  let parenDepth = 0;
  for (let i = 0; i < trimmed.length; i += 1) {
    const ch = trimmed[i];
    if (ch === "(") {
      parenDepth += 1;
    } else if (ch === ")") {
      parenDepth -= 1;
      if (parenDepth < 0) return null;
    }
  }
  if (parenDepth !== 0) return null;

  return trimmed;
}

// Filter and sanitize a map of CSS custom properties.
export function sanitizeThemeVariables(
  variables: Record<string, string> | null | undefined,
): Record<string, string> {
  const sanitized: Record<string, string> = {};
  if (!variables || typeof variables !== "object") return sanitized;
  for (const [rawKey, rawValue] of Object.entries(variables)) {
    const key = rawKey.trim();
    if (!isValidCssVariableName(key)) continue;
    const safeValue = sanitizeCssVariableValue(rawValue);
    if (!safeValue) continue;
    sanitized[key] = safeValue;
  }
  return sanitized;
}

// Redact XLStatus PAT (`xlp_...`) and enrollment (`xle_...`) tokens from error or log strings.
export function redactSensitiveTokens(
  input: string,
  extraSecrets: readonly string[] = [],
): string {
  if (!input || typeof input !== "string") return "";
  let output = input
    .replace(/\bxlp_[A-Za-z0-9_-]+/g, "xlp_[REDACTED]")
    .replace(/\bxle_[A-Za-z0-9_-]+/g, "xle_[REDACTED]");

  for (const secret of extraSecrets) {
    const trimmed = secret.trim();
    if (trimmed.length >= 4 && output.includes(trimmed)) {
      output = output.split(trimmed).join("[REDACTED]");
    }
  }
  return output;
}

// Mask a token for safe UI display while preserving its recognizable prefix.
export function maskSensitiveToken(token: string | null | undefined): string {
  if (!token || typeof token !== "string") return "";
  const trimmed = token.trim();
  if (!trimmed) return "";
  if (trimmed.length <= 12) return "••••••••";
  return `${trimmed.slice(0, 8)}••••••••${trimmed.slice(-4)}`;
}

const DOT_SEGMENT_RE = /^(?:\.{1,2}|%2e|%2e\.|\.%2e|%2e%2e)$/i;
const PATH_DOT_TRAVERSAL_RE = /(?:^|\/)(?:\.{1,2}|%2e|%2e\.|\.%2e|%2e%2e)(?:\/|$|\?|#)/i;
const BIDI_AND_ZERO_WIDTH_RE = /[\u200B-\u200F\u202A-\u202E\u2066-\u2069\uFEFF]/g;

// Escape a string for safe interpolation inside double-quoted POSIX shell arguments (`"..."`).
// Strips ASCII/C1 control characters and bidi/zero-width overrides, and escapes `\`, `"`, `$`, and `` ` ``.
export function escapeShellDoubleQuoted(value: string | null | undefined): string {
  if (!value || typeof value !== "string") return "";
  let cleaned = "";
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) continue;
    cleaned += value[i];
  }
  return cleaned
    .replace(BIDI_AND_ZERO_WIDTH_RE, "")
    .replace(/[\\"$`]/g, "\\$&");
}

// Normalize a remote POSIX filesystem path for server file operations.
// Resolves `.` and `..` segments against `/`, converts backslashes to `/`,
// and rejects empty strings, NUL/control characters, or overlong paths.
export function normalizeRemotePosixPath(rawPath: string | null | undefined): string | null {
  if (typeof rawPath !== "string") return null;
  const trimmed = rawPath.trim();
  if (!trimmed || trimmed.length > 4096 || hasControlCharacters(trimmed)) {
    return null;
  }
  const unified = trimmed.replace(/\\+/g, "/");
  const stack: string[] = [];
  for (const segment of unified.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      stack.pop();
      continue;
    }
    stack.push(segment);
  }
  return `/${stack.join("/")}`;
}

// Encode a single URL path segment (e.g. server ID, user ID, group ID) while
// preventing WHATWG URL dot-segment normalization (`..`, `.`, `%2e%2e`),
// empty segments, or control characters from traversing parent API routes.
export function encodePathSegment(value: string | null | undefined): string {
  if (typeof value !== "string") return "_invalid_";
  const trimmed = value.trim();
  if (
    !trimmed ||
    hasControlCharacters(trimmed) ||
    BIDI_AND_ZERO_WIDTH_RE.test(trimmed) ||
    DOT_SEGMENT_RE.test(trimmed)
  ) {
    return "_invalid_";
  }
  const encoded = encodeURIComponent(trimmed);
  if (!encoded || DOT_SEGMENT_RE.test(encoded)) {
    return "_invalid_";
  }
  return encoded;
}

// Resolve a relative `/...` or absolute `http(s)://` API transfer URL against
// the configured API base URL, rejecting cross-origin hosts, credentials,
// protocol-relative URLs, dangerous schemes (`javascript:`, `data:`), and
// path-traversal dot segments.
export function resolveSafeApiUrl(
  rawUrl: string | null | undefined,
  apiBaseUrl?: string | null,
  runtimeOrigin?: string | null,
): string | null {
  if (typeof rawUrl !== "string") return null;
  const trimmed = rawUrl.trim();
  if (
    !trimmed ||
    trimmed.length > 2048 ||
    hasControlCharacters(trimmed) ||
    BIDI_AND_ZERO_WIDTH_RE.test(trimmed) ||
    /[\s\\]/.test(trimmed) ||
    trimmed.startsWith("//") ||
    PATH_DOT_TRAVERSAL_RE.test(trimmed)
  ) {
    return null;
  }

  const normalizedBase =
    typeof apiBaseUrl === "string" && apiBaseUrl.trim()
      ? sanitizeExternalHttpUrl(apiBaseUrl)
      : null;
  if (typeof apiBaseUrl === "string" && apiBaseUrl.trim() && !normalizedBase) {
    return null;
  }

  const effectiveOrigin = (() => {
    if (normalizedBase) {
      try {
        return new URL(normalizedBase).origin;
      } catch {
        return null;
      }
    }
    if (typeof runtimeOrigin === "string" && runtimeOrigin.trim()) {
      try {
        const parsedOrigin = new URL(runtimeOrigin.trim());
        if (parsedOrigin.protocol === "http:" || parsedOrigin.protocol === "https:") {
          return parsedOrigin.origin;
        }
      } catch {
        return null;
      }
    }
    if (typeof window !== "undefined" && window.location?.origin) {
      return window.location.origin;
    }
    return null;
  })();

  if (trimmed.startsWith("/")) {
    try {
      const dummyParsed = new URL(trimmed, "https://xlstatus.invalid");
      if (dummyParsed.origin !== "https://xlstatus.invalid") {
        return null;
      }
      const relative = `${dummyParsed.pathname}${dummyParsed.search}${dummyParsed.hash}`;
      return normalizedBase ? `${normalizedBase.replace(/\/+$/, "")}${relative}` : relative;
    } catch {
      return null;
    }
  }

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return null;
    }
    if (parsed.username || parsed.password) {
      return null;
    }
    if (effectiveOrigin && parsed.origin !== effectiveOrigin) {
      return null;
    }
    return parsed.toString();
  } catch {
    return null;
  }
}

// Sanitize a downloaded filename from Content-Disposition or user input so it
// cannot contain path separators, parent directory traversal (`..`), control
// characters, or leading hidden-file dots.
export function sanitizeDownloadFilename(
  rawFilename: string | null | undefined,
  fallback = "download",
): string {
  const safeFallback =
    typeof fallback === "string" && fallback.trim() ? fallback.trim() : "download";
  if (typeof rawFilename !== "string") return safeFallback;
  let cleaned = "";
  for (let i = 0; i < rawFilename.length; i += 1) {
    const code = rawFilename.charCodeAt(i);
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) continue;
    cleaned += rawFilename[i];
  }
  cleaned = cleaned.replace(BIDI_AND_ZERO_WIDTH_RE, "").trim();
  if (!cleaned) return safeFallback;

  const segments = cleaned.split(/[/\\]+/).filter(Boolean);
  const basename = segments[segments.length - 1] ?? "";
  if (!basename || DOT_SEGMENT_RE.test(basename)) {
    return safeFallback;
  }

  const sanitized = basename
    .replace(/[<>:"/\\|?*]+/g, "_")
    .replace(/^\.+/, "")
    .replace(/\.+$/, "")
    .trim()
    .slice(0, 200);

  if (!sanitized || DOT_SEGMENT_RE.test(sanitized)) {
    return safeFallback;
  }
  return sanitized;
}

