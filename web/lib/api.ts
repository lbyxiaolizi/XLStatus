// API client for XLStatus backend
const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';

export interface ApiResponse<T> {
  success: boolean;
  data?: T;
  error?: string;
}

class ApiClient {
  private baseUrl: string;

  constructor(baseUrl: string = API_BASE_URL) {
    this.baseUrl = baseUrl;
  }

  private async request<T>(
    path: string,
    options: RequestInit = {}
  ): Promise<ApiResponse<T>> {
    const url = `${this.baseUrl}${path}`;

    const defaultHeaders: HeadersInit = {
      'Content-Type': 'application/json',
    };

    const response = await fetch(url, {
      ...options,
      headers: {
        ...defaultHeaders,
        ...options.headers,
      },
      credentials: 'include', // Include cookies for session
    });

    const data = await response.json();
    return data;
  }

  // Auth
  async login(username: string, password: string) {
    return this.request('/api/v1/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    });
  }

  async logout() {
    return this.request('/api/v1/auth/logout', {
      method: 'POST',
    });
  }

  // Servers
  async listServers(limit = 50, offset = 0) {
    return this.request(`/api/v1/servers?limit=${limit}&offset=${offset}`);
  }

  async getServer(id: string) {
    return this.request(`/api/v1/servers/${id}`);
  }

  // Services
  async listServices(limit = 50, offset = 0) {
    return this.request(`/api/v1/services?limit=${limit}&offset=${offset}`);
  }

  async getService(id: string) {
    return this.request(`/api/v1/services/${id}`);
  }

  async createService(service: any) {
    return this.request('/api/v1/services', {
      method: 'POST',
      body: JSON.stringify(service),
    });
  }

  async updateService(id: string, service: any) {
    return this.request(`/api/v1/services/${id}`, {
      method: 'PUT',
      body: JSON.stringify(service),
    });
  }

  async deleteService(id: string) {
    return this.request(`/api/v1/services/${id}`, {
      method: 'DELETE',
    });
  }

  // Tasks
  async listTasks(limit = 50, offset = 0) {
    return this.request(`/api/v1/tasks?limit=${limit}&offset=${offset}`);
  }

  async getTask(id: string) {
    return this.request(`/api/v1/tasks/${id}`);
  }

  async createTask(task: any) {
    return this.request('/api/v1/tasks', {
      method: 'POST',
      body: JSON.stringify(task),
    });
  }

  async updateTask(id: string, task: any) {
    return this.request(`/api/v1/tasks/${id}`, {
      method: 'PUT',
      body: JSON.stringify(task),
    });
  }

  async deleteTask(id: string) {
    return this.request(`/api/v1/tasks/${id}`, {
      method: 'DELETE',
    });
  }

  // NAT Mappings
  async listNatMappings() {
    return this.request('/api/v1/nat/mappings');
  }

  async createNatMapping(mapping: any) {
    return this.request('/api/v1/nat/mappings', {
      method: 'POST',
      body: JSON.stringify(mapping),
    });
  }

  async updateNatMapping(id: string, mapping: any) {
    return this.request(`/api/v1/nat/mappings/${id}`, {
      method: 'PUT',
      body: JSON.stringify(mapping),
    });
  }

  async deleteNatMapping(id: string) {
    return this.request(`/api/v1/nat/mappings/${id}`, {
      method: 'DELETE',
    });
  }

  // MCP
  async listMcpTools() {
    return this.request('/api/v1/mcp/tools');
  }

  async executeMcpTool(tool: string, args: any) {
    return this.request('/api/v1/mcp/execute', {
      method: 'POST',
      body: JSON.stringify({ tool, arguments: args }),
    });
  }
}

export const apiClient = new ApiClient();
