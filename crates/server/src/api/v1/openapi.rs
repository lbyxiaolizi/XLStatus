//! Lightweight OpenAPI contract for the current REST surface.

use axum::Json;
use serde_json::{json, Map, Value};

use crate::api::types::ApiResponse;

#[derive(Clone, Copy)]
struct RouteSpec {
    method: &'static str,
    path: &'static str,
    summary: &'static str,
    protected: bool,
}

const ROUTES: &[RouteSpec] = &[
    RouteSpec {
        method: "get",
        path: "/healthz",
        summary: "Health check",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/install-agent.sh",
        summary: "Agent install script",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/agents/install.sh",
        summary: "Parameterized Agent install script",
        protected: false,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/auth/login",
        summary: "Log in with username/password and optional TOTP",
        protected: false,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/auth/logout",
        summary: "Log out the current session",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/auth/totp/status",
        summary: "Read TOTP status",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/auth/totp/setup",
        summary: "Start TOTP setup",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/auth/totp/enable",
        summary: "Enable TOTP",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/auth/totp/disable",
        summary: "Disable TOTP",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/profile",
        summary: "Read current user profile",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/oauth2/providers",
        summary: "List OAuth2 providers",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/oauth2/{provider}",
        summary: "Start OAuth2 login",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/oauth2/{provider}/bind",
        summary: "Start OAuth2 account binding",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/oauth2/callback",
        summary: "OAuth2 callback",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/oauth2/bindings",
        summary: "List OAuth2 bindings",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/oauth2/{provider}/unbind",
        summary: "Unbind OAuth2 account",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/users",
        summary: "List users",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/users",
        summary: "Create user",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/users/{id}",
        summary: "Update user",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/users/{id}",
        summary: "Delete user",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/sessions",
        summary: "List active sessions",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/sessions/{id}",
        summary: "Revoke session",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/waf/bans",
        summary: "List WAF bans",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/waf/bans",
        summary: "Create WAF bans",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/waf/bans/{id}",
        summary: "Delete WAF ban",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/settings",
        summary: "Read system settings",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/settings",
        summary: "Update system settings",
        protected: true,
    },
    RouteSpec {
        method: "patch",
        path: "/api/v1/settings",
        summary: "Patch system settings",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/themes",
        summary: "List themes",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/themes/import",
        summary: "Import custom theme",
        protected: true,
    },
    RouteSpec {
        method: "put",
        path: "/api/v1/themes/import",
        summary: "Import custom theme",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/themes/{id}",
        summary: "Update custom theme",
        protected: true,
    },
    RouteSpec {
        method: "patch",
        path: "/api/v1/themes/{id}",
        summary: "Patch custom theme",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/themes/{id}",
        summary: "Delete custom theme",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/themes/{id}/select",
        summary: "Select active theme",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/maintenance/status",
        summary: "Read maintenance capabilities",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/maintenance/backup",
        summary: "Download SQLite backup",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/maintenance/archive",
        summary: "Download full maintenance archive",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/maintenance/restore",
        summary: "Restore SQLite backup",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/maintenance/sqlite-vacuum",
        summary: "Run SQLite VACUUM",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/maintenance/tsdb-compact",
        summary: "Run TSDB compact",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/maintenance/tsdb-retention",
        summary: "Update TSDB retention",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/cloudflared/status",
        summary: "Read cloudflared status",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/cloudflared/token",
        summary: "Save cloudflared token",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/cloudflared/start",
        summary: "Start cloudflared",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/cloudflared/stop",
        summary: "Stop cloudflared",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/geoip/status",
        summary: "Get GeoIP MMDB status",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/geoip/test",
        summary: "Test GeoIP lookup",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/geoip/update",
        summary: "Update GeoIP database",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/geoip/upload",
        summary: "Upload GeoIP MMDB database",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/tokens",
        summary: "List personal access tokens",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/tokens",
        summary: "Create personal access token",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/tokens/{id}",
        summary: "Revoke personal access token",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/enrollment-tokens",
        summary: "Create Agent enrollment token",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/agents/enroll",
        summary: "Enroll Agent",
        protected: false,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/agents/{id}/revoke",
        summary: "Revoke Agent",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/agents/jwt/challenge",
        summary: "Create Agent JWT challenge",
        protected: false,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/agents/jwt",
        summary: "Issue Agent JWT",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/servers",
        summary: "List servers",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/batch",
        summary: "Batch update servers",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/server-transfers",
        summary: "List server ownership transfers",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/server-transfers/{id}/retry",
        summary: "Retry server ownership transfer",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/server-transfers/{id}/cancel",
        summary: "Cancel server ownership transfer",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/transfers/temp/tokens",
        summary: "List temporary transfer tokens",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/transfers/temp/tokens/{id}/revoke",
        summary: "Revoke temporary transfer token",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/servers/{id}",
        summary: "Read server",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/{id}",
        summary: "Update server",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/servers/{id}/metrics",
        summary: "Read server metrics",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/servers/{id}/files",
        summary: "List server files",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/servers/{id}/files/read",
        summary: "Read server file",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/{id}/files/write",
        summary: "Write server file",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/{id}/files/delete",
        summary: "Delete server file",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/{id}/files/download-url",
        summary: "Create temporary server file download URL",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/servers/{id}/files/upload-url",
        summary: "Create temporary server file upload URL",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/server-groups",
        summary: "List server groups",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/server-groups",
        summary: "Create server group",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/server-groups/{id}",
        summary: "Update server group",
        protected: true,
    },
    RouteSpec {
        method: "patch",
        path: "/api/v1/server-groups/{id}",
        summary: "Patch server group",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/server-groups/{id}",
        summary: "Delete server group",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/server-groups/{id}/members",
        summary: "Add server group members",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/server-groups/{id}/members/{server_id}",
        summary: "Remove server group member",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/services",
        summary: "List services",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/services",
        summary: "Create service",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/services/test-probe",
        summary: "Test service probe",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/services/{id}",
        summary: "Read service",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/services/{id}",
        summary: "Update service",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/services/{id}",
        summary: "Delete service",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/services/{id}/history",
        summary: "Read service history",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/services/{id}/uptime",
        summary: "Read service uptime",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/alert-rules",
        summary: "List alert rules",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/alert-rules",
        summary: "Create alert rule",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/alert-rules/{id}",
        summary: "Delete alert rule",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/alert-events",
        summary: "List alert events",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/notifications",
        summary: "List notifications",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notifications",
        summary: "Create notification",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notifications/{id}",
        summary: "Update notification",
        protected: true,
    },
    RouteSpec {
        method: "patch",
        path: "/api/v1/notifications/{id}",
        summary: "Patch notification",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/notifications/{id}",
        summary: "Delete notification",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notifications/{id}/test",
        summary: "Test notification",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/notification-groups",
        summary: "List notification groups",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notification-groups",
        summary: "Create notification group",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notification-groups/{id}",
        summary: "Update notification group",
        protected: true,
    },
    RouteSpec {
        method: "patch",
        path: "/api/v1/notification-groups/{id}",
        summary: "Patch notification group",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/notification-groups/{id}",
        summary: "Delete notification group",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/notification-groups/{id}/members",
        summary: "Add notification group member",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/notification-groups/{id}/members/{notification_id}",
        summary: "Remove notification group member",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/notification-providers",
        summary: "List notification provider presets",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/tasks",
        summary: "List tasks",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/tasks",
        summary: "Create task",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/tasks/{id}",
        summary: "Read task",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/tasks/{id}",
        summary: "Update task",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/tasks/{id}",
        summary: "Delete task",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/tasks/{id}/run",
        summary: "Run task",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/tasks/{id}/runs",
        summary: "List task runs",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/terminal/sessions",
        summary: "Create terminal session",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/nat/mappings/all",
        summary: "List all NAT mappings",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/nat/mappings/agent/{agent_id}",
        summary: "List NAT mappings for an Agent",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/nat/mappings",
        summary: "Create NAT mapping",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/nat/mappings/{id}",
        summary: "Read NAT mapping",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/nat/mappings/{id}",
        summary: "Update NAT mapping",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/nat/mappings/{id}",
        summary: "Delete NAT mapping",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/ddns/configs",
        summary: "List DDNS configs",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/ddns/configs",
        summary: "Create DDNS config",
        protected: true,
    },
    RouteSpec {
        method: "delete",
        path: "/api/v1/ddns/configs/{id}",
        summary: "Delete DDNS config",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/ddns/configs/{id}/history",
        summary: "List DDNS history",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/ddns/reload",
        summary: "Reload DDNS providers",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/ddns/check-now",
        summary: "Run DDNS check now",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/mcp/tools",
        summary: "List MCP tools",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/api/v1/mcp/execute",
        summary: "Execute MCP tool",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/mcp/info",
        summary: "Read MCP info",
        protected: true,
    },
    RouteSpec {
        method: "post",
        path: "/mcp",
        summary: "MCP JSON-RPC endpoint",
        protected: true,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/public/status",
        summary: "Read public status",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/public/mjpeg",
        summary: "Read public MJPEG status stream",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/public/servers/{id}",
        summary: "Read public server detail",
        protected: false,
    },
    RouteSpec {
        method: "get",
        path: "/api/v1/transfers/temp/download",
        summary: "Temporary transfer download",
        protected: false,
    },
    RouteSpec {
        method: "put",
        path: "/api/v1/transfers/temp/upload",
        summary: "Temporary transfer upload",
        protected: false,
    },
];

pub async fn openapi_json() -> Json<ApiResponse<Value>> {
    Json(ApiResponse::success(openapi_document()))
}

pub fn openapi_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "XLStatus API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "REST API contract for XLStatus. Responses use the success/data/error envelope unless an endpoint explicitly streams or downloads a file."
        },
        "servers": [
            { "url": "/" }
        ],
        "paths": build_paths(),
        "components": {
            "responses": {
                "BadRequest": { "description": "Bad request", "content": error_content() },
                "Unauthorized": { "description": "Unauthorized", "content": error_content() },
                "Forbidden": { "description": "Forbidden", "content": error_content() },
                "NotFound": { "description": "Not found", "content": error_content() },
                "InternalError": { "description": "Internal server error", "content": error_content() }
            },
            "securitySchemes": {
                "sessionCookie": {
                    "type": "apiKey",
                    "in": "cookie",
                    "name": "xlstatus_session"
                },
                "csrfHeader": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "x-csrf-token"
                },
                "sensitiveTotpCode": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "x-totp-code",
                    "description": "Required for sensitive write operations when the current account has TOTP enabled."
                },
                "personalAccessToken": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "xlp_*"
                }
            },
            "schemas": {
                "ApiEnvelope": {
                    "type": "object",
                    "required": ["success"],
                    "properties": {
                        "success": { "type": "boolean" },
                        "data": { "description": "Endpoint-specific payload", "nullable": true },
                        "error": { "type": "string", "nullable": true }
                    }
                },
                "ErrorEnvelope": {
                    "type": "object",
                    "required": ["success", "error"],
                    "properties": {
                        "success": { "const": false },
                        "data": { "nullable": true },
                        "error": { "type": "string" }
                    }
                }
            }
        },
        "x-xlstatus": {
            "apiEnvelope": "success/data/error",
            "frontendTypes": "web/lib/api.ts",
            "typecheck": "cd web && pnpm typecheck",
            "sensitiveTotpHeader": "x-totp-code"
        }
    })
}

fn error_content() -> Value {
    json!({
        "application/json": {
            "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }
        }
    })
}

fn build_paths() -> Value {
    let mut paths = Map::new();
    for route in ROUTES {
        let path = paths
            .entry(route.path.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let methods = path.as_object_mut().expect("path entry must be object");
        methods.insert(route.method.to_string(), operation(route));
    }
    Value::Object(paths)
}

fn operation(route: &RouteSpec) -> Value {
    let mut op = json!({
        "summary": route.summary,
        "operationId": operation_id(route),
        "responses": {
            "200": {
                "description": "Success",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/ApiEnvelope" }
                    }
                }
            },
            "400": { "$ref": "#/components/responses/BadRequest" },
            "401": { "$ref": "#/components/responses/Unauthorized" },
            "403": { "$ref": "#/components/responses/Forbidden" },
            "404": { "$ref": "#/components/responses/NotFound" },
            "500": { "$ref": "#/components/responses/InternalError" }
        }
    });

    if let Some(parameters) = path_parameters(route.path) {
        op.as_object_mut()
            .expect("operation must be object")
            .insert("parameters".into(), parameters);
    }
    if route.protected {
        op.as_object_mut()
            .expect("operation must be object")
            .insert(
                "security".into(),
                json!([
                    { "sessionCookie": [], "csrfHeader": [] },
                    { "personalAccessToken": [] }
                ]),
            );
    }
    op
}

fn operation_id(route: &RouteSpec) -> String {
    let slug = route
        .path
        .trim_matches('/')
        .replace("/api/v1/", "")
        .replace(['/', '-', '{', '}'], "_")
        .trim_matches('_')
        .to_string();
    format!("{}_{}", route.method, slug)
}

fn path_parameters(path: &str) -> Option<Value> {
    let mut params = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('}') else {
            break;
        };
        let name = &after_start[..end];
        params.push(json!({
            "name": name,
            "in": "path",
            "required": true,
            "schema": { "type": "string" }
        }));
        rest = &after_start[end + 1..];
    }
    (!params.is_empty()).then(|| Value::Array(params))
}

#[cfg(test)]
mod tests {
    use super::openapi_document;

    #[test]
    fn openapi_document_includes_current_core_paths() {
        let doc = openapi_document();
        let paths = doc["paths"].as_object().unwrap();
        for path in [
            "/api/v1/auth/login",
            "/api/v1/servers",
            "/api/v1/services",
            "/api/v1/alert-rules",
            "/api/v1/notifications",
            "/api/v1/tasks/{id}/run",
            "/api/v1/public/status",
            "/api/v1/maintenance/status",
            "/api/v1/waf/bans",
            "/api/v1/settings",
            "/api/v1/themes",
            "/api/v1/servers/{id}/files/download-url",
            "/api/v1/servers/{id}/files/upload-url",
        ] {
            assert!(paths.contains_key(path), "missing OpenAPI path {path}");
        }
    }

    #[test]
    fn openapi_document_marks_protected_paths_with_security() {
        let doc = openapi_document();
        assert!(doc["paths"]["/api/v1/servers"]["get"]
            .get("security")
            .is_some());
        assert!(doc["paths"]["/api/v1/public/status"]["get"]
            .get("security")
            .is_none());
    }

    #[test]
    fn openapi_documents_temp_url_creation_as_post_only() {
        let doc = openapi_document();
        let download = &doc["paths"]["/api/v1/servers/{id}/files/download-url"];
        let upload = &doc["paths"]["/api/v1/servers/{id}/files/upload-url"];

        assert!(download["post"].get("security").is_some());
        assert!(upload["post"].get("security").is_some());
        assert!(download["get"].is_null());
        assert!(upload["get"].is_null());
    }
}
