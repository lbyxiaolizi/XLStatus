// API client for XLStatus backend
import {
  encodePathSegment,
  redactSensitiveTokens,
  sanitizeDownloadFilename,
  sanitizeExternalHttpUrl,
  sanitizeRedirectPath,
} from "@/app/lib/security";
import { getTranslations } from "@/lib/i18n";

const DEFAULT_API_PORT = "8080";

function trimTrailingSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

export function getApiBaseUrl(): string {
  const configured = process.env["NEXT_PUBLIC_API_URL"]?.trim();
  if (configured) {
    const safeConfigured = sanitizeExternalHttpUrl(configured);
    if (safeConfigured) return trimTrailingSlash(safeConfigured);
  }

  if (typeof window !== "undefined") {
    const currentOrigin = new URL(window.location.origin);
    currentOrigin.port = DEFAULT_API_PORT;
    return currentOrigin.origin;
  }

  return `http://localhost:${DEFAULT_API_PORT}`;
}

export function buildWebSocketUrl(path: string): string {
  const url = new URL(getApiBaseUrl());
  const protocol = url.protocol === "https:" ? "wss" : "ws";
  const safePath =
    path.startsWith("/") && !path.startsWith("//")
      ? path
      : `/${path.replace(/^\/+/, "")}`;
  return `${protocol}://${url.host}${safePath}`;
}

export const API_BASE_URL = getApiBaseUrl();

let authRedirectPending = false;

function redirectToLogin(): void {
  if (typeof window === "undefined") return;
  if (authRedirectPending) return;
  if (isAuthRedirectExemptPath(window.location.pathname)) return;

  authRedirectPending = true;
  // Only a previously stored identity justifies the "session expired" notice;
  // a never-logged-in visitor hitting a protected API just gets the bare form.
  const hadSession = window.localStorage.getItem("user") !== null;
  window.localStorage.removeItem("session_token");
  window.localStorage.removeItem("user");

  const returnTo = sanitizeRedirectPath(
    `${window.location.pathname}${window.location.search}${window.location.hash}`,
  );
  const reason = hadSession ? "&reason=expired" : "";
  window.location.assign(`/login?return_to=${encodeURIComponent(returnTo)}${reason}`);
}

function isAuthRedirectExemptPath(pathname: string): boolean {
  return pathname === "/login" || pathname.startsWith("/status") || pathname.startsWith("/oauth/");
}

export interface ApiResponse<T> {
  success: boolean;
  data?: T | undefined;
  error?: string | undefined;
  status?: number | undefined;
  request_id?: string | undefined;
}

export interface ServerListResponse {
  servers: unknown[];
  total?: number;
}

export interface ServerGroup {
  id: string;
  owner_user_id: string;
  name: string;
  color?: string | null;
  display_order?: number | null;
  server_ids: string[];
  created_at?: string;
  updated_at?: string;
}

export interface ServerGroupListResponse {
  groups: ServerGroup[];
  total?: number;
}

export interface ServerOwnerTransfer {
  id: string;
  server_id: string;
  from_user_id?: string | null;
  to_user_id: string;
  requested_by_user_id?: string | null;
  api_token_id?: string | null;
  status: string;
  attempts: number;
  error?: string | null;
  completed_at?: string | null;
  cancelled_at?: string | null;
  last_attempt_at: string;
  created_at: string;
  updated_at: string;
}

export interface ServerOwnerTransferListResponse {
  transfers: ServerOwnerTransfer[];
  total?: number;
}

export interface ServiceListResponse {
  services: unknown[];
  total?: number;
}

export interface PublicStatusResponse {
  servers: unknown[];
  services: unknown[];
  updated_at?: string;
  site?: PublicSiteBranding;
  theme?: ThemeDefinition | null;
}

export interface PublicSiteBranding {
  site_name: string;
  logo_url?: string | null;
  favicon_url?: string | null;
  theme_color?: string | null;
  background_url?: string | null;
  custom_head?: string | null;
  custom_body?: string | null;
}

export interface ThemeDefinition {
  id: string;
  name: string;
  description?: string | null;
  target: "public" | "dashboard" | "both" | string;
  variables: Record<string, string>;
  light_variables?: Record<string, string>;
  dark_variables?: Record<string, string>;
  builtin: boolean;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface ThemeListResponse {
  themes: ThemeDefinition[];
  selected_public_theme_id?: string | null;
  selected_dashboard_theme_id?: string | null;
}

export interface ImportThemeRequest {
  theme: {
    id: string;
    name: string;
    description?: string | null;
    target?: "public" | "dashboard" | "both" | string;
    variables?: Record<string, string>;
    light_variables?: Record<string, string>;
    dark_variables?: Record<string, string>;
  };
}

export interface UserListResponse {
  users: unknown[];
  total?: number;
}

export interface SessionListResponse {
  sessions: unknown[];
  total?: number;
}

export interface WafBanListResponse {
  bans: unknown[];
  total?: number;
}

export interface CreateWafBansResponse {
  bans: unknown[];
}

export interface MaintenanceStatusResponse {
  database_backend: string;
  backup_supported: boolean;
  archive_supported: boolean;
  restore_supported: boolean;
  vacuum_supported: boolean;
  tsdb_compact_supported: boolean;
  tsdb_backend?: string;
  tsdb_status?: string;
  tsdb_samples?: number | null;
  tsdb_retention_days?: number | null;
  tsdb_retention_configurable?: boolean;
}

export interface MaintenanceRestoreResponse {
  dry_run: boolean;
  restored: boolean;
  compatible: boolean;
  database_backend: string;
  user_version: number;
  table_count: number;
  row_count: number;
  message: string;
}

export interface DownloadFileResponse {
  blob: Blob;
  filename: string;
}

export interface TsdbCompactResponse {
  action: string;
  success: boolean;
  backend: string;
  removed_samples: number;
  samples_before?: number | null;
  samples_after?: number | null;
  message: string;
}

export interface TsdbRetentionResponse {
  action: string;
  success: boolean;
  backend: string;
  retention_days: number;
  samples_before?: number | null;
  samples_after?: number | null;
  message: string;
}

export interface CloudflaredStatusResponse {
  token_configured: boolean;
  running: boolean;
  pid?: number | null;
  started_at?: string | null;
  last_error?: string | null;
  logs: string[];
}

export interface CloudflaredActionResponse {
  action: string;
  success: boolean;
  status: CloudflaredStatusResponse;
}

export interface TotpStatusResponse {
  enabled: boolean;
  setup_pending: boolean;
}

export interface TotpSetupResponse {
  secret: string;
  otpauth_uri: string;
  enabled: boolean;
}

export interface OAuthProvider {
  id: string;
  display_name: string;
  scopes: string[];
}

export interface OAuthProviderListResponse {
  providers: OAuthProvider[];
}

export interface OAuthAccount {
  provider: string;
  provider_display_name: string;
  subject: string;
  email?: string | null;
  display_name?: string | null;
  created_at: string;
  updated_at: string;
}

export interface OAuthAccountListResponse {
  accounts: OAuthAccount[];
}

export interface OAuthStartResponse {
  authorization_url: string;
}

export interface GeoIpLookupResponse {
  provider: string;
  ip: string;
  country?: string | null;
  region?: string | null;
  city?: string | null;
  latitude?: number | null;
  longitude?: number | null;
  isp?: string | null;
  organization?: string | null;
  timezone?: string | null;
  raw?: unknown;
}

export interface GeoIpMmdbStatus {
  configured: boolean;
  path: string;
  size_bytes?: number | null;
  modified_at?: string | null;
  database_type?: string | null;
  build_epoch?: number | null;
  build_at?: string | null;
  ip_version?: number | null;
  languages: string[];
  description: Record<string, string>;
  error?: string | null;
}

export interface GeoIpMaintenanceResponse {
  action: string;
  supported: boolean;
  message: string;
  status?: GeoIpMmdbStatus | null;
}

export interface SystemSettingsResponse {
  public_site_enabled: boolean;
  public_site_name: string;
  public_logo_url?: string | null;
  public_favicon_url?: string | null;
  public_theme_color?: string | null;
  public_background_url?: string | null;
  public_custom_head?: string | null;
  public_custom_body?: string | null;
  public_server_details_enabled: boolean;
  geoip_provider: string;
  geoip_ipinfo_token?: string;
  geoip_ipinfo_token_configured: boolean;
  geoip_ip_change_enabled: boolean;
  geoip_ip_change_notification_group_id?: string | null;
  geoip_ip_change_server_ids: string[];
  geoip_ip_change_severity: string;
  ddns_resolver_url?: string | null;
}

export interface TaskListResponse {
  tasks: unknown[];
  total?: number;
}

export interface TaskRunListResponse {
  runs: unknown[];
  total?: number;
}

export interface NatMapping {
  id: string;
  agent_id: string;
  description?: string | null;
  protocol?: string;
  local_host?: string;
  local_port?: number;
  public_port?: number;
  enabled?: boolean;
  allowed_sources?: string | null;
  max_active_tunnels?: number | null;
  idle_timeout_seconds?: number | null;
  max_bytes_per_tunnel?: number | null;
  max_bandwidth_bytes_per_second?: number | null;
  rate_limit_window_seconds?: number | null;
  max_connections_per_window?: number | null;
  max_bytes_per_window?: number | null;
}

export interface NatMappingListResponse {
  mappings: NatMapping[];
  total?: number;
}

export interface AlertRuleListResponse {
  rules: unknown[];
  total?: number;
}

export interface AlertEventListResponse {
  events: unknown[];
  total?: number;
}

export interface NotificationListResponse {
  notifications: unknown[];
  total?: number;
}

export interface NotificationGroupListResponse {
  groups: unknown[];
  total?: number;
}

export interface NotificationProviderListResponse {
  providers: unknown[];
}

export interface DdnsConfigListResponse {
  configs: DdnsConfig[];
  total?: number;
}

export interface DdnsConfig {
  id: string;
  owner_user_id: string;
  agent_id?: string | null;
  name: string;
  provider: string;
  domain: string;
  record_id?: string | null;
  zone_id?: string | null;
  api_token_configured: boolean;
  api_key_configured: boolean;
  api_secret_configured: boolean;
  webhook_url_configured: boolean;
  current_ip?: string | null;
  last_applied_ip?: string | null;
  last_applied_at?: string | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface DdnsHistoryListResponse {
  history: unknown[];
  total?: number;
}

export interface TerminalSessionResponse {
  session_id?: string;
  id?: string;
}

export interface FileEntry {
  name: string;
  file_type: "file" | "dir" | "symlink" | string;
  size: number;
  mode: number;
  modified_at: number;
  symlink_target?: string | null;
}

export interface FileListResponse {
  server_id: string;
  path: string;
  entries: FileEntry[];
}

export interface FileReadResponse {
  server_id: string;
  path: string;
  encoding: string;
  content: string;
  bytes: number;
}

export interface TempUrlResponse {
  server_id: string;
  path: string;
  url: string;
  method: string;
  expires_at: number;
}

export interface ProbeTestResponse {
  success: boolean;
  latency_ms?: number;
  status_code?: number;
  error?: string;
  cert_fingerprint?: string;
  cert_not_after?: string;
}

export interface CreatePatResponse {
  id: string;
  name: string;
  token: string;
  scopes: string[];
  expires_at: string;
  created_at: string;
}

export interface PatInfo {
  id: string;
  name?: string;
  scopes?: string[];
  server_ids?: string[] | null;
  expires_at?: string;
  last_used_at?: string | null;
  created_at?: string;
}

export interface CreateEnrollmentTokenResponse {
  token: string;
  expires_at: string;
}

export interface McpToolDefinition {
  name: string;
  description: string;
  category?: string | undefined;
  input_schema: Record<string, unknown>;
  annotations?: Record<string, unknown> | undefined;
}

export interface McpResourceDefinition {
  uri: string;
  name: string;
  description: string;
  mime_type: string;
}

export interface McpResourceTemplateDefinition {
  uri_template: string;
  name: string;
  description: string;
  mime_type: string;
}

export interface McpPromptArgument {
  name: string;
  description: string;
  required: boolean;
}

export interface McpPromptDefinition {
  name: string;
  description: string;
  arguments: McpPromptArgument[];
}

export interface McpCatalogResponse {
  protocol_version: string;
  server_info: Record<string, unknown>;
  tools: McpToolDefinition[];
  resources: McpResourceDefinition[];
  resource_templates: McpResourceTemplateDefinition[];
  prompts: McpPromptDefinition[];
}

export interface McpStudioResponse {
  kind: string;
  name: string;
  duration_ms: number;
  output: unknown;
}

export type JsonObject = Record<string, unknown>;

export type RequestOptions = ApiReadOptions;

type ApiRequestOptions = Omit<RequestInit, "signal"> & {
  signal?: AbortSignal | null | undefined;
  anonymous?: boolean | undefined;
  skipAuthRedirect?: boolean | undefined;
  clientCacheTtlMs?: number | undefined;
  bypassClientCache?: boolean | undefined;
};

type ApiReadOptions = {
  signal?: AbortSignal | undefined;
  bypassClientCache?: boolean | undefined;
};

// Newer servers can inline probe history into the service listing. Older ones
// simply ignore the extra query params and omit the `history` key, so callers
// must still be able to fall back to per-service history requests.
export type ListServicesOptions = ApiReadOptions & {
  includeHistory?: boolean | undefined;
  historyLimit?: number | undefined;
};

interface ClientCacheEntry {
  expiresAt: number;
  response: ApiResponse<unknown>;
}

const CLIENT_CACHE_MAX_ENTRIES = 64;
const CLIENT_CACHE_EPOCH_MAX_ENTRIES = 128;
const CACHE_TTL_FAST_MS = 5_000;
const CACHE_TTL_STANDARD_MS = 15_000;
const CACHE_TTL_STABLE_MS = 60_000;
const CACHE_TTL_STATIC_MS = 5 * 60_000;

class ApiClient {
  private baseUrl: string;
  private responseCache = new Map<string, ClientCacheEntry>();
  private inFlightRequests = new Map<string, Promise<ApiResponse<unknown>>>();
  private latestCacheRequestEpoch = new Map<string, number>();
  private cacheGeneration = 0;
  private cacheRequestSequence = 0;
  // Remembers which verb a legacy-compatible endpoint family accepted, so later
  // writes skip the probing round-trips (each of which also drops the read cache).
  private fallbackMethodPreference = new Map<string, string>();

  constructor(baseUrl: string = API_BASE_URL) {
    this.baseUrl = baseUrl;
  }

  invalidateCachedReads(): void {
    this.invalidateClientCache();
  }

  private async request<T>(
    path: string,
    options: ApiRequestOptions = {},
  ): Promise<ApiResponse<T>> {
    const url = `${this.baseUrl}${path}`;
    const {
      anonymous,
      headers,
      skipAuthRedirect,
      clientCacheTtlMs = 0,
      bypassClientCache = false,
      signal,
      ...fetchOptions
    } = options;
    const csrfToken = getCookie("xlstatus_csrf");
    const hasBody = fetchOptions.body !== undefined && fetchOptions.body !== null;
    const isFormData = typeof FormData !== "undefined" && fetchOptions.body instanceof FormData;
    const method = (fetchOptions.method ?? "GET").toUpperCase();
    const cacheable = method === "GET" && !hasBody && clientCacheTtlMs > 0;
    const cacheScope = anonymous ? "public" : `private:${csrfToken || "no-session"}`;
    const cacheKey = cacheable ? `${cacheScope}:${url}` : "";
    const defaultHeaders: HeadersInit = hasBody && !isFormData
      ? { "Content-Type": "application/json" }
      : {};

    if (cacheable && !bypassClientCache) {
      const cached = this.responseCache.get(cacheKey);
      if (cached && cached.expiresAt > Date.now()) {
        return this.waitForSharedRequest(
          Promise.resolve(cached.response as ApiResponse<T>),
          signal,
        );
      }
      if (cached) this.responseCache.delete(cacheKey);
    }

    if (method !== "GET") {
      this.invalidateClientCache();
    }

    const execute = async (): Promise<ApiResponse<T>> => {
      try {
        const response = await fetch(url, {
          ...fetchOptions,
          // Cached requests are shared. A component abort only stops its own
          // wait; it must not cancel the request another mounted view is using.
          ...(cacheable && !bypassClientCache
            ? { signal: null }
            : signal !== undefined
              ? { signal }
              : {}),
          headers: {
            ...defaultHeaders,
            ...(csrfToken && !anonymous ? { "x-csrf-token": csrfToken } : {}),
            ...headers,
          },
          credentials: anonymous ? "omit" : "include",
        });

        const text = await response.text();
        const payload = parseJson(text);
        const envelope = normalizeEnvelope<T>(payload, response.status);

        if (response.ok) {
          return envelope.success
            ? envelope
            : {
                success: true,
                data: payload as T,
                status: response.status,
              };
        }

        if (response.status === 401 && !anonymous && !skipAuthRedirect) {
          this.invalidateClientCache();
          redirectToLogin();
        }

        return {
          success: false,
          data: envelope.data,
          error:
            envelope.error ||
            response.statusText ||
            `Request failed with ${response.status}`,
          status: response.status,
          request_id: envelope.request_id,
        };
      } catch (error) {
        return {
          success: false,
          error: requestFailureMessage(error),
        };
      }
    };

    if (!cacheable) {
      const result = await execute();
      // A read started while a mutation was in flight may have captured the
      // pre-mutation state. Invalidate again after the write settles so that
      // the next read cannot reuse that snapshot.
      if (method !== "GET") this.invalidateClientCache();
      return result;
    }

    const requestGeneration = this.cacheGeneration;
    const executeAndCache = async (requestEpoch: number): Promise<ApiResponse<T>> => {
      const result = await execute();
      if (
        result.success
        && requestGeneration === this.cacheGeneration
        && this.latestCacheRequestEpoch.get(cacheKey) === requestEpoch
      ) {
        this.storeClientCache(cacheKey, result, clientCacheTtlMs);
      }
      return result;
    };

    if (bypassClientCache) {
      const requestEpoch = this.beginCacheRequest(cacheKey);
      return this.waitForSharedRequest(executeAndCache(requestEpoch), signal);
    }

    // Generation-scoped keys prevent a post-mutation reload from joining a
    // request that began before the mutation.
    const inFlightKey = `${requestGeneration}:${cacheKey}`;
    let shared = this.inFlightRequests.get(inFlightKey) as Promise<ApiResponse<T>> | undefined;
    if (!shared) {
      const requestEpoch = this.beginCacheRequest(cacheKey);
      shared = executeAndCache(requestEpoch);
      this.inFlightRequests.set(inFlightKey, shared as Promise<ApiResponse<unknown>>);
      void shared.finally(() => {
        if (this.inFlightRequests.get(inFlightKey) === shared) {
          this.inFlightRequests.delete(inFlightKey);
        }
      });
    }

    return this.waitForSharedRequest(shared, signal);
  }

  private invalidateClientCache(): void {
    this.cacheGeneration += 1;
    this.responseCache.clear();
    this.latestCacheRequestEpoch.clear();
  }

  private beginCacheRequest(key: string): number {
    this.cacheRequestSequence += 1;
    if (
      this.latestCacheRequestEpoch.size >= CLIENT_CACHE_EPOCH_MAX_ENTRIES
      && !this.latestCacheRequestEpoch.has(key)
    ) {
      const oldestKey = this.latestCacheRequestEpoch.keys().next().value as string | undefined;
      if (oldestKey) this.latestCacheRequestEpoch.delete(oldestKey);
    }
    this.latestCacheRequestEpoch.delete(key);
    this.latestCacheRequestEpoch.set(key, this.cacheRequestSequence);
    return this.cacheRequestSequence;
  }

  private storeClientCache<T>(key: string, response: ApiResponse<T>, ttlMs: number): void {
    const now = Date.now();
    for (const [cachedKey, entry] of this.responseCache) {
      if (entry.expiresAt <= now) this.responseCache.delete(cachedKey);
    }
    if (this.responseCache.size >= CLIENT_CACHE_MAX_ENTRIES) {
      const oldestKey = this.responseCache.keys().next().value as string | undefined;
      if (oldestKey) this.responseCache.delete(oldestKey);
    }
    this.responseCache.set(key, {
      expiresAt: now + ttlMs,
      response: response as ApiResponse<unknown>,
    });
  }

  private async waitForSharedRequest<T>(
    request: Promise<ApiResponse<T>>,
    signal?: AbortSignal | null,
  ): Promise<ApiResponse<T>> {
    if (!signal) return request;
    if (signal.aborted) return { success: false, error: "Request aborted" };

    return new Promise((resolve) => {
      let settled = false;
      const finish = (response: ApiResponse<T>) => {
        if (settled) return;
        settled = true;
        signal.removeEventListener("abort", onAbort);
        resolve(response);
      };
      const onAbort = () => finish({ success: false, error: "Request aborted" });
      signal.addEventListener("abort", onAbort, { once: true });
      void request.then(finish);
    });
  }

  // Groups sibling resources (e.g. /api/v1/servers/<id>) under one key so the
  // learned verb is reused across ids instead of per-instance.
  private fallbackPreferenceKey(path: string, methods: string[]): string {
    const pathname = path.split(/[?#]/, 1)[0] ?? path;
    const family = pathname.split("/").slice(0, 4).join("/");
    return `${methods.join("|")}:${family}`;
  }

  private async requestWithFallback<T>(
    path: string,
    body: JsonObject,
    methods: string[],
    options: Omit<ApiRequestOptions, "method" | "body"> = {},
  ): Promise<ApiResponse<T>> {
    let last: ApiResponse<T> | null = null;

    const preferenceKey = this.fallbackPreferenceKey(path, methods);
    const preferred = this.fallbackMethodPreference.get(preferenceKey);
    const orderedMethods = preferred && methods.includes(preferred)
      ? [preferred, ...methods.filter((method) => method !== preferred)]
      : methods;

    for (const method of orderedMethods) {
      const response = await this.request<T>(path, {
        ...options,
        method,
        body: JSON.stringify(body),
      });
      last = response;

      if (
        response.success ||
        ![404, 405, 501].includes(response.status ?? 0)
      ) {
        this.fallbackMethodPreference.set(preferenceKey, method);
        return response;
      }
    }

    return last ?? { success: false, error: getTranslations().common.noRequestAttempted };
  }

  private async downloadFile(
    path: string,
    options: ApiRequestOptions = {},
  ): Promise<ApiResponse<DownloadFileResponse>> {
    const url = `${this.baseUrl}${path}`;
    const { anonymous, headers, skipAuthRedirect, signal, ...fetchOptions } = options;
    const csrfToken = getCookie("xlstatus_csrf");

    try {
      const response = await fetch(url, {
        ...fetchOptions,
        ...(signal !== undefined ? { signal } : {}),
        headers: {
          ...(csrfToken && !anonymous ? { "x-csrf-token": csrfToken } : {}),
          ...headers,
        },
        credentials: anonymous ? "omit" : "include",
      });

      if (!response.ok) {
        const text = await response.text();
        const payload = parseJson(text);
        const envelope = normalizeEnvelope<DownloadFileResponse>(payload, response.status);
        if (response.status === 401 && !anonymous && !skipAuthRedirect) {
          redirectToLogin();
        }
        return {
          success: false,
          data: envelope.data,
          error:
            envelope.error ||
            response.statusText ||
            `Request failed with ${response.status}`,
          status: response.status,
          request_id: envelope.request_id,
        };
      }

      const blob = await response.blob();
      return {
        success: true,
        status: response.status,
        data: {
          blob,
          filename: filenameFromContentDisposition(response.headers.get("Content-Disposition")),
        },
      };
    } catch (error) {
      return {
        success: false,
        error: requestFailureMessage(error),
      };
    }
  }

  private sensitiveHeaders(totpCode?: string): HeadersInit {
    const code = totpCode?.replace(/[\u0000-\u001f\u007f]/g, "").trim();
    return code ? { "x-totp-code": code } : {};
  }

  // Auth
  async login(
    username: string,
    password: string,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password, totp_code: totpCode }),
      skipAuthRedirect: true,
    });
  }

  async logout(): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/auth/logout", {
      method: "POST",
    });
  }

  async getProfile(options?: ApiReadOptions): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/profile", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async listOAuthProviders(): Promise<ApiResponse<OAuthProviderListResponse>> {
    return this.request<OAuthProviderListResponse>("/api/v1/oauth2/providers", {
      anonymous: true,
      clientCacheTtlMs: CACHE_TTL_STATIC_MS,
    });
  }

  async listOAuthBindings(): Promise<ApiResponse<OAuthAccountListResponse>> {
    return this.request<OAuthAccountListResponse>("/api/v1/oauth2/bindings");
  }

  getOAuthLoginUrl(providerId: string, returnTo = "/dashboard"): string {
    const query = new URLSearchParams({ return_to: sanitizeRedirectPath(returnTo) });
    return `${this.baseUrl}/api/v1/oauth2/${encodePathSegment(providerId)}?${query.toString()}`;
  }

  async startOAuthBind(
    providerId: string,
    returnTo = "/settings",
    totpCode?: string,
  ): Promise<ApiResponse<OAuthStartResponse>> {
    const query = new URLSearchParams({ return_to: sanitizeRedirectPath(returnTo, "/settings") });
    return this.request<OAuthStartResponse>(
      `/api/v1/oauth2/${encodePathSegment(providerId)}/bind?${query.toString()}`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  async unbindOAuthProvider(
    providerId: string,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/oauth2/${encodePathSegment(providerId)}/unbind`, {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async createUser(user: JsonObject, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/users", {
      method: "POST",
      body: JSON.stringify(user),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async listUsers(limit = 100, offset = 0): Promise<ApiResponse<UserListResponse>> {
    return this.request<UserListResponse>(
      `/api/v1/users?limit=${limit}&offset=${offset}`,
    );
  }

  async updateUser(
    id: string,
    user: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/users/${encodePathSegment(id)}`, {
      method: "POST",
      body: JSON.stringify(user),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async deleteUser(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/users/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async listSessions(
    limit = 100,
    offset = 0,
  ): Promise<ApiResponse<SessionListResponse>> {
    return this.request<SessionListResponse>(
      `/api/v1/sessions?limit=${limit}&offset=${offset}`,
    );
  }

  async deleteSession(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/sessions/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async listWafBans(
    limit = 100,
    offset = 0,
  ): Promise<ApiResponse<WafBanListResponse>> {
    return this.request<WafBanListResponse>(
      `/api/v1/waf/bans?limit=${limit}&offset=${offset}`,
    );
  }

  async createWafBans(
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<CreateWafBansResponse>> {
    return this.request<CreateWafBansResponse>("/api/v1/waf/bans", {
      method: "POST",
      body: JSON.stringify(payload),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async deleteWafBan(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/waf/bans/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async getTotpStatus(options?: ApiReadOptions): Promise<ApiResponse<TotpStatusResponse>> {
    return this.request<TotpStatusResponse>("/api/v1/auth/totp/status", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async setupTotp(code?: string): Promise<ApiResponse<TotpSetupResponse>> {
    return this.request<TotpSetupResponse>("/api/v1/auth/totp/setup", {
      method: "POST",
      ...(code ? { body: JSON.stringify({ code }) } : {}),
    });
  }

  async enableTotp(code: string): Promise<ApiResponse<TotpStatusResponse>> {
    return this.request<TotpStatusResponse>("/api/v1/auth/totp/enable", {
      method: "POST",
      body: JSON.stringify({ code }),
    });
  }

  async disableTotp(code: string): Promise<ApiResponse<TotpStatusResponse>> {
    return this.request<TotpStatusResponse>("/api/v1/auth/totp/disable", {
      method: "POST",
      body: JSON.stringify({ code }),
    });
  }

  // Personal access tokens
  async listPats(): Promise<ApiResponse<PatInfo[]>> {
    return this.request<PatInfo[]>("/api/v1/tokens");
  }

  async createPat(
    token: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<CreatePatResponse>> {
    return this.request<CreatePatResponse>("/api/v1/tokens", {
      method: "POST",
      body: JSON.stringify(token),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async revokePat(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tokens/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  getAgentInstallScriptUrl(params: Record<string, string>, baseUrl = this.baseUrl): string {
    const safeParams: Record<string, string> = {};
    for (const [key, value] of Object.entries(params)) {
      if (key === "server_url" || key === "download_url") {
        const safeUrl = sanitizeExternalHttpUrl(value);
        if (safeUrl) safeParams[key] = safeUrl;
      } else {
        safeParams[key] = value;
      }
    }
    const query = new URLSearchParams(safeParams);
    const trimmedBase = baseUrl.trim();
    const safeBase = trimmedBase
      ? (sanitizeExternalHttpUrl(trimmedBase) ?? this.baseUrl)
      : this.baseUrl;
    return `${safeBase.replace(/\/+$/, "")}/api/v1/agents/install.sh?${query.toString()}`;
  }

  async createEnrollmentToken(
    expiresInHours = 1,
    totpCode?: string,
  ): Promise<ApiResponse<CreateEnrollmentTokenResponse>> {
    return this.request<CreateEnrollmentTokenResponse>("/api/v1/enrollment-tokens", {
      method: "POST",
      body: JSON.stringify({ expires_in_hours: expiresInHours }),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  // Servers
  async listServers(
    limit = 50,
    offset = 0,
    anonymous = false,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<ServerListResponse>> {
    return this.request<ServerListResponse>(
      `/api/v1/servers?limit=${limit}&offset=${offset}`,
      {
        anonymous,
        ...options,
        clientCacheTtlMs: CACHE_TTL_FAST_MS,
      },
    );
  }

  async getServer(id: string, options?: { signal?: AbortSignal }): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/servers/${encodePathSegment(id)}`, options);
  }

  async updateServer(
    id: string,
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}`,
      payload,
      ["POST", "PATCH", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async batchUpdateServers(
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/servers/batch", {
      method: "POST",
      body: JSON.stringify(payload),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async listServerOwnerTransfers(
    limit = 20,
    offset = 0,
    serverId?: string,
  ): Promise<ApiResponse<ServerOwnerTransferListResponse>> {
    const query = new URLSearchParams({
      limit: String(limit),
      offset: String(offset),
    });
    if (serverId) query.set("server_id", serverId);
    return this.request<ServerOwnerTransferListResponse>(
      `/api/v1/server-transfers?${query.toString()}`,
    );
  }

  async retryServerOwnerTransfer(
    id: string,
    totpCode?: string,
  ): Promise<ApiResponse<ServerOwnerTransfer>> {
    return this.request<ServerOwnerTransfer>(
      `/api/v1/server-transfers/${encodePathSegment(id)}/retry`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  async cancelServerOwnerTransfer(
    id: string,
    totpCode?: string,
  ): Promise<ApiResponse<ServerOwnerTransfer>> {
    return this.request<ServerOwnerTransfer>(
      `/api/v1/server-transfers/${encodePathSegment(id)}/cancel`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  async listServerGroups(options?: ApiReadOptions): Promise<ApiResponse<ServerGroupListResponse>> {
    return this.request<ServerGroupListResponse>("/api/v1/server-groups", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async createServerGroup(group: JsonObject, totpCode?: string): Promise<ApiResponse<ServerGroup>> {
    return this.request<ServerGroup>("/api/v1/server-groups", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(group),
    });
  }

  async updateServerGroup(
    id: string,
    group: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<ServerGroup>> {
    return this.requestWithFallback<ServerGroup>(
      `/api/v1/server-groups/${encodePathSegment(id)}`,
      group,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteServerGroup(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/server-groups/${encodePathSegment(id)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async addServerGroupMembers(
    id: string,
    serverIds: string[],
    totpCode?: string,
  ): Promise<ApiResponse<ServerGroup>> {
    return this.request<ServerGroup>(
      `/api/v1/server-groups/${encodePathSegment(id)}/members`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify({ server_ids: serverIds }),
      },
    );
  }

  async deleteServerGroupMember(
    id: string,
    serverId: string,
    totpCode?: string,
  ): Promise<ApiResponse<ServerGroup>> {
    return this.request<ServerGroup>(
      `/api/v1/server-groups/${encodePathSegment(id)}/members/${encodePathSegment(serverId)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async getServerMetrics(
    id: string,
    range = "1d",
    options?: { signal?: AbortSignal },
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/metrics?range=${encodeURIComponent(range)}`,
      options,
    );
  }

  async listServerFiles(
    id: string,
    path: string,
  ): Promise<ApiResponse<FileListResponse>> {
    return this.request<FileListResponse>(
      `/api/v1/servers/${encodePathSegment(id)}/files`,
      {
        method: "POST",
        body: JSON.stringify({ path }),
      },
    );
  }

  async readServerFile(
    id: string,
    path: string,
    encoding = "utf8",
  ): Promise<ApiResponse<FileReadResponse>> {
    return this.request<FileReadResponse>(
      `/api/v1/servers/${encodePathSegment(id)}/files/read`,
      {
        method: "POST",
        body: JSON.stringify({ path, encoding }),
      },
    );
  }

  async writeServerFile(
    id: string,
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/files/write`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify(payload),
      },
    );
  }

  async deleteServerFile(
    id: string,
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/files/delete`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify(payload),
      },
    );
  }

  async getServerDownloadUrl(
    id: string,
    path: string,
  ): Promise<ApiResponse<TempUrlResponse>> {
    return this.request<TempUrlResponse>(
      `/api/v1/servers/${encodePathSegment(id)}/files/download-url`,
      {
        method: "POST",
        body: JSON.stringify({ path }),
      },
    );
  }

  async getServerUploadUrl(
    id: string,
    path: string,
    totpCode?: string,
  ): Promise<ApiResponse<TempUrlResponse>> {
    return this.request<TempUrlResponse>(
      `/api/v1/servers/${encodePathSegment(id)}/files/upload-url`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify({ path }),
      },
    );
  }

  async getServerConfig(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/config`,
    );
  }

  async applyServerConfig(
    id: string,
    config: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/config`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify({ config }),
      },
    );
  }

  async forceUpdateServer(
    id: string,
    payload: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodePathSegment(id)}/force-update`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify(payload),
      },
    );
  }

  // Services
  async listServices(
    limit = 50,
    offset = 0,
    anonymous = false,
    options?: ListServicesOptions,
  ): Promise<ApiResponse<ServiceListResponse>> {
    const { includeHistory, historyLimit, ...readOptions }: ListServicesOptions = options ?? {};
    // The history params live in the URL, so history-bearing responses land in
    // their own client-cache slot instead of poisoning plain listings.
    let query = `/api/v1/services?limit=${limit}&offset=${offset}`;
    if (includeHistory) {
      query += "&include_history=1";
      if (typeof historyLimit === "number" && Number.isFinite(historyLimit)) {
        query += `&history_limit=${Math.max(1, Math.trunc(historyLimit))}`;
      }
    }
    return this.request<ServiceListResponse>(query, {
      anonymous,
      ...readOptions,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async getPublicStatus(options?: ApiReadOptions): Promise<ApiResponse<PublicStatusResponse>> {
    return this.request<PublicStatusResponse>("/api/v1/public/status", {
      anonymous: true,
      ...options,
      clientCacheTtlMs: CACHE_TTL_FAST_MS,
    });
  }

  async getPublicServer(
    id: string,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/public/servers/${encodePathSegment(id)}`,
      {
        anonymous: true,
        ...options,
        clientCacheTtlMs: CACHE_TTL_FAST_MS,
      },
    );
  }

  async getService(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/services/${encodePathSegment(id)}`);
  }

  async createService(service: JsonObject, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/services", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(service),
    });
  }

  async updateService(
    id: string,
    service: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/services/${encodePathSegment(id)}`,
      service,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteService(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/services/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async testProbe(
    probe: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<ProbeTestResponse>> {
    return this.request<ProbeTestResponse>("/api/v1/services/test-probe", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(probe),
    });
  }

  async getServiceHistory(
    id: string,
    limit = 30,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/services/${encodePathSegment(id)}/history?limit=${limit}`,
      { ...options, clientCacheTtlMs: CACHE_TTL_FAST_MS },
    );
  }

  // Alert rules
  async listAlertRules(
    limit = 50,
    offset = 0,
  ): Promise<ApiResponse<AlertRuleListResponse>> {
    return this.request<AlertRuleListResponse>(
      `/api/v1/alert-rules?limit=${limit}&offset=${offset}`,
    );
  }

  async createAlertRule(rule: JsonObject, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/alert-rules", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(rule),
    });
  }

  async updateAlertRule(
    id: string,
    rule: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/alert-rules/${encodePathSegment(id)}`,
      rule,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteAlertRule(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/alert-rules/${encodePathSegment(id)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async listAlertEvents(
    limit = 20,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<AlertEventListResponse>> {
    return this.request<AlertEventListResponse>(
      `/api/v1/alert-events?limit=${limit}`,
      { ...options, clientCacheTtlMs: CACHE_TTL_FAST_MS },
    );
  }

  // Notifications
  async listNotifications(
    limit = 100,
    offset = 0,
  ): Promise<ApiResponse<NotificationListResponse>> {
    return this.request<NotificationListResponse>(
      `/api/v1/notifications?limit=${limit}&offset=${offset}`,
    );
  }

  async createNotification(
    notification: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/notifications", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(notification),
    });
  }

  async updateNotification(
    id: string,
    notification: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/notifications/${encodePathSegment(id)}`,
      notification,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteNotification(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/notifications/${encodePathSegment(id)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async testNotification(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/notifications/${encodePathSegment(id)}/test`,
      { method: "POST", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async listNotificationGroups(
    limit = 100,
    offset = 0,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<NotificationGroupListResponse>> {
    return this.request<NotificationGroupListResponse>(
      `/api/v1/notification-groups?limit=${limit}&offset=${offset}`,
      { ...options, clientCacheTtlMs: CACHE_TTL_STANDARD_MS },
    );
  }

  async createNotificationGroup(
    group: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/notification-groups", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(group),
    });
  }

  async updateNotificationGroup(
    id: string,
    group: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/notification-groups/${encodePathSegment(id)}`,
      group,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteNotificationGroup(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/notification-groups/${encodePathSegment(id)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async addNotificationGroupMember(
    id: string,
    notificationId: string,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/notification-groups/${encodePathSegment(id)}/members`,
      {
        method: "POST",
        headers: this.sensitiveHeaders(totpCode),
        body: JSON.stringify({ notification_id: notificationId }),
      },
    );
  }

  async deleteNotificationGroupMember(
    id: string,
    notificationId: string,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/notification-groups/${encodePathSegment(id)}/members/${encodePathSegment(notificationId)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async listNotificationProviders(): Promise<ApiResponse<NotificationProviderListResponse>> {
    return this.request<NotificationProviderListResponse>(
      "/api/v1/notification-providers",
    );
  }

  // Tasks
  async listTasks(
    limit = 50,
    offset = 0,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<TaskListResponse>> {
    return this.request<TaskListResponse>(
      `/api/v1/tasks?limit=${limit}&offset=${offset}`,
      options,
    );
  }

  async getTask(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodePathSegment(id)}`);
  }

  async createTask(task: JsonObject, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/tasks", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(task),
    });
  }

  async updateTask(
    id: string,
    task: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/tasks/${encodePathSegment(id)}`,
      task,
      ["POST", "PATCH", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteTask(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodePathSegment(id)}`, {
      method: "DELETE",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async runTask(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodePathSegment(id)}/run`, {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async createTerminalSession(
    agentId: string,
    cols: number,
    rows: number,
    totpCode?: string,
  ): Promise<ApiResponse<TerminalSessionResponse>> {
    return this.requestWithFallback<TerminalSessionResponse>(
      "/api/v1/terminal/sessions",
      {
        agent_id: agentId,
        cols,
        rows,
      },
      ["POST"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async listTaskRuns(
    id: string,
    limit = 20,
    options?: ApiReadOptions,
  ): Promise<ApiResponse<TaskRunListResponse>> {
    return this.request<TaskRunListResponse>(
      `/api/v1/tasks/${encodePathSegment(id)}/runs?limit=${limit}`,
      options,
    );
  }

  // NAT mappings
  async listNatMappings(): Promise<ApiResponse<NatMappingListResponse>> {
    return this.request<NatMappingListResponse>("/api/v1/nat/mappings/all");
  }

  async listNatMappingsForAgent(
    agentId: string,
  ): Promise<ApiResponse<NatMappingListResponse>> {
    return this.request<NatMappingListResponse>(
      `/api/v1/nat/mappings/agent/${encodePathSegment(agentId)}`,
    );
  }

  async createNatMapping(mapping: JsonObject, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/nat/mappings", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify(mapping),
    });
  }

  async updateNatMapping(
    id: string,
    mapping: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/nat/mappings/${encodePathSegment(id)}`,
      mapping,
      ["POST", "PATCH", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteNatMapping(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/nat/mappings/${encodePathSegment(id)}`,
      { method: "DELETE", headers: this.sensitiveHeaders(totpCode) },
    );
  }

  // DDNS
  async listDdnsConfigs(): Promise<ApiResponse<DdnsConfigListResponse>> {
    return this.request<DdnsConfigListResponse>("/api/v1/ddns/configs");
  }

  async createDdnsConfig(
    config: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/ddns/configs", {
      method: "POST",
      body: JSON.stringify(config),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async updateDdnsConfig(
    id: string,
    config: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/ddns/configs/${encodePathSegment(id)}`,
      config,
      ["PATCH", "POST", "PUT"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async deleteDdnsConfig(
    id: string,
    totpCode?: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/ddns/configs/${encodePathSegment(id)}`,
      {
        method: "DELETE",
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  async listDdnsHistory(
    id: string,
  ): Promise<ApiResponse<DdnsHistoryListResponse>> {
    return this.request<DdnsHistoryListResponse>(
      `/api/v1/ddns/configs/${encodePathSegment(id)}/history`,
    );
  }

  async reloadDdnsProviders(totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/ddns/reload", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async checkDdnsNow(totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/ddns/check-now", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  // Maintenance
  async getMaintenanceStatus(): Promise<ApiResponse<MaintenanceStatusResponse>> {
    return this.request<MaintenanceStatusResponse>("/api/v1/maintenance/status");
  }

  async downloadMaintenanceBackup(
    totpCode?: string,
  ): Promise<ApiResponse<DownloadFileResponse>> {
    return this.downloadFile("/api/v1/maintenance/backup", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async downloadMaintenanceArchive(
    totpCode?: string,
  ): Promise<ApiResponse<DownloadFileResponse>> {
    return this.downloadFile("/api/v1/maintenance/archive", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async vacuumSqlite(totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/maintenance/sqlite-vacuum", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async compactTsdb(totpCode?: string): Promise<ApiResponse<TsdbCompactResponse>> {
    return this.request<TsdbCompactResponse>("/api/v1/maintenance/tsdb-compact", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async updateTsdbRetention(
    retentionDays: number,
    totpCode?: string,
  ): Promise<ApiResponse<TsdbRetentionResponse>> {
    return this.request<TsdbRetentionResponse>("/api/v1/maintenance/tsdb-retention", {
      method: "POST",
      body: JSON.stringify({ retention_days: retentionDays }),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async getCloudflaredStatus(): Promise<ApiResponse<CloudflaredStatusResponse>> {
    return this.request<CloudflaredStatusResponse>("/api/v1/cloudflared/status");
  }

  async saveCloudflaredToken(
    token: string | null,
    totpCode?: string,
  ): Promise<ApiResponse<CloudflaredActionResponse>> {
    return this.request<CloudflaredActionResponse>("/api/v1/cloudflared/token", {
      method: "POST",
      body: JSON.stringify({ token }),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async startCloudflared(totpCode?: string): Promise<ApiResponse<CloudflaredActionResponse>> {
    return this.request<CloudflaredActionResponse>("/api/v1/cloudflared/start", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async stopCloudflared(totpCode?: string): Promise<ApiResponse<CloudflaredActionResponse>> {
    return this.request<CloudflaredActionResponse>("/api/v1/cloudflared/stop", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async restoreBackup(
    file: File,
    dryRun: boolean,
    totpCode?: string,
  ): Promise<ApiResponse<MaintenanceRestoreResponse>> {
    return this.request<MaintenanceRestoreResponse>(
      `/api/v1/maintenance/restore?dry_run=${dryRun ? "true" : "false"}`,
      {
        method: "POST",
        body: file,
        headers: {
          "Content-Type": "application/vnd.sqlite3",
          ...this.sensitiveHeaders(totpCode),
        },
      },
    );
  }

  async testGeoIp(
    ip: string,
    provider = "empty",
    token = "",
    totpCode?: string,
  ): Promise<ApiResponse<GeoIpLookupResponse>> {
    return this.request<GeoIpLookupResponse>("/api/v1/geoip/test", {
      method: "POST",
      headers: this.sensitiveHeaders(totpCode),
      body: JSON.stringify({
        ip,
        provider,
        ...(token.trim() ? { token: token.trim() } : {}),
      }),
    });
  }

  async getGeoIpStatus(): Promise<ApiResponse<GeoIpMmdbStatus>> {
    return this.request<GeoIpMmdbStatus>("/api/v1/geoip/status");
  }

  async updateGeoIpDatabase(
    input: JsonObject = {},
    totpCode?: string,
  ): Promise<ApiResponse<GeoIpMaintenanceResponse>> {
    const body = Object.keys(input).length ? JSON.stringify(input) : null;
    return this.request<GeoIpMaintenanceResponse>("/api/v1/geoip/update", {
      method: "POST",
      ...(body !== null ? { body } : {}),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async uploadGeoIpDatabase(
    file: File,
    totpCode?: string,
  ): Promise<ApiResponse<GeoIpMaintenanceResponse>> {
    const form = new FormData();
    form.set("file", file);
    return this.request<GeoIpMaintenanceResponse>("/api/v1/geoip/upload", {
      method: "POST",
      body: form,
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async getSettings(options?: ApiReadOptions): Promise<ApiResponse<SystemSettingsResponse>> {
    return this.request<SystemSettingsResponse>("/api/v1/settings", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async updateSettings(
    settings: Partial<SystemSettingsResponse>,
    totpCode?: string,
  ): Promise<ApiResponse<SystemSettingsResponse>> {
    return this.requestWithFallback<SystemSettingsResponse>(
      "/api/v1/settings",
      settings,
      ["PATCH", "POST"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async listThemes(options?: ApiReadOptions): Promise<ApiResponse<ThemeListResponse>> {
    return this.request<ThemeListResponse>("/api/v1/themes", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STABLE_MS,
    });
  }

  async importTheme(
    theme: ImportThemeRequest["theme"],
    totpCode?: string,
  ): Promise<ApiResponse<ThemeDefinition>> {
    return this.request<ThemeDefinition>("/api/v1/themes/import", {
      method: "POST",
      body: JSON.stringify({ theme }),
      headers: this.sensitiveHeaders(totpCode),
    });
  }

  async updateTheme(
    id: string,
    theme: JsonObject,
    totpCode?: string,
  ): Promise<ApiResponse<ThemeDefinition>> {
    return this.requestWithFallback<ThemeDefinition>(
      `/api/v1/themes/${encodePathSegment(id)}`,
      theme,
      ["PATCH", "POST"],
      { headers: this.sensitiveHeaders(totpCode) },
    );
  }

  async selectTheme(
    id: string,
    target: "public" | "dashboard" | "both",
    totpCode?: string,
  ): Promise<ApiResponse<ThemeListResponse>> {
    return this.request<ThemeListResponse>(
      `/api/v1/themes/${encodePathSegment(id)}/select`,
      {
        method: "POST",
        body: JSON.stringify({ target }),
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  async deleteTheme(id: string, totpCode?: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/themes/${encodePathSegment(id)}`,
      {
        method: "DELETE",
        headers: this.sensitiveHeaders(totpCode),
      },
    );
  }

  // MCP
  async listMcpTools(): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/mcp/tools");
  }

  async executeMcpTool(
    tool: string,
    args: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/mcp/execute", {
      method: "POST",
      body: JSON.stringify({ tool, arguments: args }),
    });
  }

  async getMcpCatalog(options?: RequestOptions): Promise<ApiResponse<McpCatalogResponse>> {
    return this.request<McpCatalogResponse>("/api/v1/mcp/catalog", {
      ...options,
      clientCacheTtlMs: CACHE_TTL_STANDARD_MS,
    });
  }

  async executeMcpStudio(
    body: {
      kind: "tool" | "resource" | "prompt";
      name: string;
      arguments?: Record<string, unknown> | undefined;
    },
    options?: RequestOptions,
  ): Promise<ApiResponse<McpStudioResponse>> {
    return this.request<McpStudioResponse>("/api/v1/mcp/studio", {
      ...options,
      method: "POST",
      body: JSON.stringify({
        kind: body.kind,
        name: body.name,
        arguments: body.arguments ?? {},
      }),
    });
  }

  async callMcpJsonRpc(
    patToken: string,
    payload: Record<string, unknown>,
    options?: { signal?: AbortSignal; timeoutMs?: number },
  ): Promise<ApiResponse<JsonObject>> {
    const token = patToken.trim();
    if (/[\u0000-\u001f\u007f\s]/.test(token)) {
      return {
        success: false,
        error: "Invalid PAT token format",
      };
    }
    const timeoutMs = options?.timeoutMs ?? 30_000;
    const controller = new AbortController();
    const externalSignal = options?.signal;
    const onExternalAbort = () => controller.abort();
    if (externalSignal) {
      if (externalSignal.aborted) {
        controller.abort();
      } else {
        externalSignal.addEventListener("abort", onExternalAbort, { once: true });
      }
    }
    const timeoutId = setTimeout(() => controller.abort(), timeoutMs);

    try {
      const response = await fetch(`${getApiBaseUrl()}/mcp`, {
        method: "POST",
        credentials: "omit",
        signal: controller.signal,
        headers: {
          "Content-Type": "application/json",
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify(payload),
      });
      const text = await response.text();
      const parsed = parseJson(text);
      const dataObj =
        parsed && typeof parsed === "object" && !Array.isArray(parsed)
          ? (parsed as JsonObject)
          : { raw: parsed };
      if (!response.ok) {
        const envelope = normalizeEnvelope<JsonObject>(parsed, response.status);
        const rawErr =
          envelope.error ||
          response.statusText ||
          `Request failed with ${response.status}`;
        return {
          success: false,
          data: dataObj,
          error: redactSensitiveTokens(rawErr, [token]),
          status: response.status,
        };
      }
      const rpcError =
        dataObj["error"] && typeof dataObj["error"] === "object"
          ? (dataObj["error"] as Record<string, unknown>)
          : null;
      if (rpcError) {
        const rawRpcErr = String(rpcError["message"] ?? rpcError["code"] ?? "JSON-RPC error");
        return {
          success: false,
          data: dataObj,
          error: redactSensitiveTokens(rawRpcErr, [token]),
          status: response.status,
        };
      }
      return {
        success: true,
        data: dataObj,
        status: response.status,
      };
    } catch (error) {
      return {
        success: false,
        error: redactSensitiveTokens(requestFailureMessage(error), [token]),
      };
    } finally {
      clearTimeout(timeoutId);
      if (externalSignal) {
        externalSignal.removeEventListener("abort", onExternalAbort);
      }
    }
  }
}

export const apiClient = new ApiClient();

function requestFailureMessage(error: unknown): string {
  if (error instanceof TypeError) return getTranslations().common.networkError;
  return error instanceof Error ? error.message : getTranslations().common.networkError;
}

function parseJson(text: string): unknown {
  if (!text.trim()) {
    return undefined;
  }

  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

function normalizeEnvelope<T>(payload: unknown, status: number): ApiResponse<T> {
  // Bare values (null/primitive) and bare arrays are the data themselves — they
  // have no { success, data } envelope. Without the Array branch, list
  // endpoints that return a top-level array (e.g. listPats → PatInfo[]) fell
  // through to `obj.data`, which is undefined for an array, silently producing
  // empty lists.
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return {
      success: status >= 200 && status < 300,
      data: payload as T,
      status,
    };
  }

  const obj = payload as Record<string, unknown>;
  const success =
    typeof obj["success"] === "boolean"
      ? obj["success"]
      : typeof obj["ok"] === "boolean"
        ? obj["ok"]
        : status >= 200 && status < 300;
  const errorValue = obj["error"];
  const error =
    typeof errorValue === "string"
      ? errorValue
      : errorValue && typeof errorValue === "object"
        ? String(
              (errorValue as Record<string, unknown>)["message"] ??
              (errorValue as Record<string, unknown>)["code"] ??
              getTranslations().common.requestFailed,
          )
        : undefined;

  return {
    success,
    data: obj["data"] as T,
    error,
    status,
    request_id:
      typeof obj["request_id"] === "string" ? obj["request_id"] : undefined,
  };
}

function getCookie(name: string): string | null {
  if (typeof document === "undefined" || !name) {
    return null;
  }

  const prefix = `${name}=`;
  for (const rawEntry of document.cookie.split(";")) {
    const entry = rawEntry.trim();
    if (entry.startsWith(prefix)) {
      const rawValue = entry.slice(prefix.length).trim();
      if (!rawValue || /[\u0000-\u001f\u007f]/.test(rawValue)) {
        return null;
      }
      return rawValue;
    }
  }
  return null;
}

function filenameFromContentDisposition(value: string | null): string {
  if (!value) return "download";
  const utf8 = value.match(/filename\*=UTF-8''([^;]+)/i);
  if (utf8?.[1]) {
    try {
      return sanitizeDownloadFilename(
        decodeURIComponent(utf8[1].replace(/^"|"$/g, "")),
        "download",
      );
    } catch {
      return sanitizeDownloadFilename(utf8[1].replace(/^"|"$/g, ""), "download");
    }
  }
  const plain = value.match(/filename="?([^";]+)"?/i);
  return sanitizeDownloadFilename(plain?.[1]?.trim(), "download");
}
