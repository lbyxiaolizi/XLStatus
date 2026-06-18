// API client for XLStatus backend
import { t } from "@/lib/i18n";

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";

export interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
  status?: number;
  request_id?: string;
}

export interface ServerListResponse {
  servers: unknown[];
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
}

export interface TaskListResponse {
  tasks: unknown[];
  total?: number;
}

export interface TaskRunListResponse {
  runs: unknown[];
  total?: number;
}

export interface NatMappingListResponse {
  mappings: unknown[];
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

export interface DdnsConfigListResponse {
  configs: unknown[];
  total?: number;
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
  created_at: string;
}

export type JsonObject = Record<string, unknown>;

type ApiRequestOptions = RequestInit & {
  anonymous?: boolean;
};

class ApiClient {
  private baseUrl: string;

  constructor(baseUrl: string = API_BASE_URL) {
    this.baseUrl = baseUrl;
  }

  private async request<T>(
    path: string,
    options: ApiRequestOptions = {},
  ): Promise<ApiResponse<T>> {
    const url = `${this.baseUrl}${path}`;
    const { anonymous, headers, ...fetchOptions } = options;
    const csrfToken = getCookie("xlstatus_csrf");
    const hasBody = fetchOptions.body !== undefined && fetchOptions.body !== null;
    const defaultHeaders: HeadersInit = hasBody
      ? { "Content-Type": "application/json" }
      : {};

    try {
      const response = await fetch(url, {
        ...fetchOptions,
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
        error: error instanceof Error ? error.message : t.common.networkError,
      };
    }
  }

  private async requestWithFallback<T>(
    path: string,
    body: JsonObject,
    methods: string[],
  ): Promise<ApiResponse<T>> {
    let last: ApiResponse<T> | null = null;

    for (const method of methods) {
      const response = await this.request<T>(path, {
        method,
        body: JSON.stringify(body),
      });
      last = response;

      if (
        response.success ||
        ![404, 405, 501].includes(response.status ?? 0)
      ) {
        return response;
      }
    }

    return last ?? { success: false, error: t.common.noRequestAttempted };
  }

  // Auth
  async login(
    username: string,
    password: string,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    });
  }

  async logout(): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/auth/logout", {
      method: "POST",
    });
  }

  async createUser(user: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/users", {
      method: "POST",
      body: JSON.stringify(user),
    });
  }

  // Personal access tokens
  async listPats(): Promise<ApiResponse<unknown[]>> {
    return this.request<unknown[]>("/api/v1/tokens");
  }

  async createPat(token: JsonObject): Promise<ApiResponse<CreatePatResponse>> {
    return this.request<CreatePatResponse>("/api/v1/tokens", {
      method: "POST",
      body: JSON.stringify(token),
    });
  }

  async revokePat(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tokens/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  }

  // Servers
  async listServers(
    limit = 50,
    offset = 0,
    anonymous = false,
  ): Promise<ApiResponse<ServerListResponse>> {
    return this.request<ServerListResponse>(
      `/api/v1/servers?limit=${limit}&offset=${offset}`,
      { anonymous },
    );
  }

  async getServer(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/servers/${encodeURIComponent(id)}`);
  }

  async getServerMetrics(
    id: string,
    range = "1d",
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/metrics?range=${encodeURIComponent(range)}`,
    );
  }

  async listServerFiles(
    id: string,
    path: string,
  ): Promise<ApiResponse<FileListResponse>> {
    return this.request<FileListResponse>(
      `/api/v1/servers/${encodeURIComponent(id)}/files?path=${encodeURIComponent(path)}`,
    );
  }

  async readServerFile(
    id: string,
    path: string,
    encoding = "utf8",
  ): Promise<ApiResponse<FileReadResponse>> {
    return this.request<FileReadResponse>(
      `/api/v1/servers/${encodeURIComponent(id)}/files/read?path=${encodeURIComponent(path)}&encoding=${encodeURIComponent(encoding)}`,
    );
  }

  async writeServerFile(
    id: string,
    payload: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/files/write`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  }

  async deleteServerFile(
    id: string,
    payload: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/files/delete`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  }

  async getServerDownloadUrl(
    id: string,
    path: string,
  ): Promise<ApiResponse<TempUrlResponse>> {
    return this.request<TempUrlResponse>(
      `/api/v1/servers/${encodeURIComponent(id)}/files/download-url?path=${encodeURIComponent(path)}`,
    );
  }

  async getServerUploadUrl(
    id: string,
    path: string,
  ): Promise<ApiResponse<TempUrlResponse>> {
    return this.request<TempUrlResponse>(
      `/api/v1/servers/${encodeURIComponent(id)}/files/upload-url?path=${encodeURIComponent(path)}`,
    );
  }

  async getServerConfig(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/config`,
    );
  }

  async applyServerConfig(
    id: string,
    config: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/config`,
      {
        method: "POST",
        body: JSON.stringify({ config }),
      },
    );
  }

  async forceUpdateServer(
    id: string,
    payload: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/servers/${encodeURIComponent(id)}/force-update`,
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
  }

  // Services
  async listServices(
    limit = 50,
    offset = 0,
    anonymous = false,
  ): Promise<ApiResponse<ServiceListResponse>> {
    return this.request<ServiceListResponse>(
      `/api/v1/services?limit=${limit}&offset=${offset}`,
      { anonymous },
    );
  }

  async getPublicStatus(): Promise<ApiResponse<PublicStatusResponse>> {
    return this.request<PublicStatusResponse>("/api/v1/public/status", {
      anonymous: true,
    });
  }

  async getService(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/services/${encodeURIComponent(id)}`);
  }

  async createService(service: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/services", {
      method: "POST",
      body: JSON.stringify(service),
    });
  }

  async updateService(
    id: string,
    service: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/services/${encodeURIComponent(id)}`,
      service,
      ["PATCH", "POST", "PUT"],
    );
  }

  async deleteService(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/services/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  }

  async testProbe(probe: JsonObject): Promise<ApiResponse<ProbeTestResponse>> {
    return this.request<ProbeTestResponse>("/api/v1/services/test-probe", {
      method: "POST",
      body: JSON.stringify(probe),
    });
  }

  async getServiceHistory(
    id: string,
    limit = 30,
  ): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/services/${encodeURIComponent(id)}/history?limit=${limit}`,
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

  async createAlertRule(rule: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/alert-rules", {
      method: "POST",
      body: JSON.stringify(rule),
    });
  }

  async updateAlertRule(
    id: string,
    rule: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/alert-rules/${encodeURIComponent(id)}`,
      rule,
      ["PATCH", "POST", "PUT"],
    );
  }

  async deleteAlertRule(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/alert-rules/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  }

  async listAlertEvents(limit = 20): Promise<ApiResponse<AlertEventListResponse>> {
    return this.request<AlertEventListResponse>(
      `/api/v1/alert-events?limit=${limit}`,
    );
  }

  // Tasks
  async listTasks(
    limit = 50,
    offset = 0,
  ): Promise<ApiResponse<TaskListResponse>> {
    return this.request<TaskListResponse>(
      `/api/v1/tasks?limit=${limit}&offset=${offset}`,
    );
  }

  async getTask(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodeURIComponent(id)}`);
  }

  async createTask(task: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/tasks", {
      method: "POST",
      body: JSON.stringify(task),
    });
  }

  async updateTask(id: string, task: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/tasks/${encodeURIComponent(id)}`,
      task,
      ["POST", "PATCH", "PUT"],
    );
  }

  async deleteTask(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  }

  async runTask(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(`/api/v1/tasks/${encodeURIComponent(id)}/run`, {
      method: "POST",
    });
  }

  async createTerminalSession(
    agentId: string,
    cols: number,
    rows: number,
  ): Promise<ApiResponse<TerminalSessionResponse>> {
    return this.requestWithFallback<TerminalSessionResponse>(
      "/api/v1/terminal/sessions",
      {
        agent_id: agentId,
        cols,
        rows,
      },
      ["POST"],
    );
  }

  async listTaskRuns(
    id: string,
    limit = 20,
  ): Promise<ApiResponse<TaskRunListResponse>> {
    return this.request<TaskRunListResponse>(
      `/api/v1/tasks/${encodeURIComponent(id)}/runs?limit=${limit}`,
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
      `/api/v1/nat/mappings/agent/${encodeURIComponent(agentId)}`,
    );
  }

  async createNatMapping(mapping: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/nat/mappings", {
      method: "POST",
      body: JSON.stringify(mapping),
    });
  }

  async updateNatMapping(
    id: string,
    mapping: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/nat/mappings/${encodeURIComponent(id)}`,
      mapping,
      ["POST", "PATCH", "PUT"],
    );
  }

  async deleteNatMapping(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/nat/mappings/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  }

  // DDNS
  async listDdnsConfigs(): Promise<ApiResponse<DdnsConfigListResponse>> {
    return this.request<DdnsConfigListResponse>("/api/v1/ddns/configs");
  }

  async createDdnsConfig(config: JsonObject): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/ddns/configs", {
      method: "POST",
      body: JSON.stringify(config),
    });
  }

  async updateDdnsConfig(
    id: string,
    config: JsonObject,
  ): Promise<ApiResponse<JsonObject>> {
    return this.requestWithFallback<JsonObject>(
      `/api/v1/ddns/configs/${encodeURIComponent(id)}`,
      config,
      ["PATCH", "POST", "PUT"],
    );
  }

  async deleteDdnsConfig(id: string): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>(
      `/api/v1/ddns/configs/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
  }

  async listDdnsHistory(
    id: string,
  ): Promise<ApiResponse<DdnsHistoryListResponse>> {
    return this.request<DdnsHistoryListResponse>(
      `/api/v1/ddns/configs/${encodeURIComponent(id)}/history`,
    );
  }

  async reloadDdnsProviders(): Promise<ApiResponse<JsonObject>> {
    return this.request<JsonObject>("/api/v1/ddns/reload", {
      method: "POST",
    });
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
}

export const apiClient = new ApiClient();

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
  if (!payload || typeof payload !== "object") {
    return {
      success: status >= 200 && status < 300,
      data: payload as T,
      status,
    };
  }

  const obj = payload as Record<string, unknown>;
  const success =
    typeof obj.success === "boolean"
      ? obj.success
      : typeof obj.ok === "boolean"
        ? obj.ok
        : status >= 200 && status < 300;
  const errorValue = obj.error;
  const error =
    typeof errorValue === "string"
      ? errorValue
      : errorValue && typeof errorValue === "object"
        ? String(
            (errorValue as Record<string, unknown>).message ??
              (errorValue as Record<string, unknown>).code ??
              t.common.requestFailed,
          )
        : undefined;

  return {
    success,
    data: obj.data as T,
    error,
    status,
    request_id:
      typeof obj.request_id === "string" ? obj.request_id : undefined,
  };
}

function getCookie(name: string): string | null {
  if (typeof document === "undefined") {
    return null;
  }

  return (
    document.cookie
      .split("; ")
      .find((row) => row.startsWith(`${name}=`))
      ?.split("=")[1] ?? null
  );
}
