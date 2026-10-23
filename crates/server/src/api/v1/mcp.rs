use std::time::Instant;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::auth::middleware::AuthUser;
use crate::mcp::executor::McpExecutor;
use crate::mcp::tools::{
    get_available_prompts, get_available_resource_templates, get_available_resources,
    get_available_tools, McpPrompt, McpResource, McpResourceTemplate, McpTool, McpToolRequest,
};

const MCP_MAX_BODY_BYTES: usize = 1024 * 1024;
const MCP_MAX_BATCH_ITEMS: usize = 16;

#[derive(Debug, Serialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Serialize)]
pub struct McpCatalogResponse {
    pub protocol_version: String,
    pub server_info: Value,
    pub server: Value,
    pub tools: Vec<McpTool>,
    pub resources: Vec<McpResource>,
    pub resource_templates: Vec<McpResourceTemplate>,
    pub prompts: Vec<McpPrompt>,
}

#[derive(Debug, Deserialize)]
pub struct McpExecuteRequest {
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct McpStudioRequest {
    pub kind: Option<String>,
    pub action: Option<String>,
    pub tool: Option<String>,
    pub name: Option<String>,
    pub uri: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

fn build_mcp_server_summary() -> Value {
    let tools = get_available_tools();
    let resources = get_available_resources();
    let resource_templates = get_available_resource_templates();
    let prompts = get_available_prompts();

    json!({
        "name": "XLStatus MCP Server",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": "2024-11-05",
        "capabilities": {
            "tools": true,
            "resources": true,
            "prompts": true,
        },
        "tools_count": tools.len(),
        "resources_count": resources.len(),
        "resource_templates_count": resource_templates.len(),
        "prompts_count": prompts.len(),
    })
}

fn build_executor(state: &AppState) -> McpExecutor {
    McpExecutor::new(
        state.db.clone(),
        state.session_registry.clone(),
        state.config.security.session_secret.clone(),
    )
    .with_metrics(state.metrics.clone())
}

/// List available MCP tools (PAT-authenticated)
pub async fn list_mcp_tools(
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    auth_user.require_pat().map_err(forbidden)?;
    let tools = get_available_tools();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(McpToolsResponse { tools }),
        error: None,
    }))
}

/// Get complete MCP catalog for the Web AI Ops Studio (accepts cookie session or PAT)
pub async fn get_mcp_catalog(
    _auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let tools = get_available_tools();
    let resources = get_available_resources();
    let resource_templates = get_available_resource_templates();
    let prompts = get_available_prompts();

    let server_summary = build_mcp_server_summary();
    Ok(Json(ApiResponse {
        success: true,
        data: Some(McpCatalogResponse {
            protocol_version: "2024-11-05".to_string(),
            server_info: server_summary.clone(),
            server: server_summary,
            tools,
            resources,
            resource_templates,
            prompts,
        }),
        error: None,
    }))
}

/// Interactive MCP Studio execution endpoint for Web UI (accepts cookie session or PAT)
pub async fn execute_mcp_studio(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(req): Json<McpStudioRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let started = Instant::now();
    let action = req
        .kind
        .as_deref()
        .or(req.action.as_deref())
        .unwrap_or_else(|| {
            if req.uri.is_some() {
                "resource"
            } else if req.prompt.is_some() {
                "prompt"
            } else {
                "tool"
            }
        });

    match action {
        "resource" => {
            let Some(uri) = req.uri.or(req.name) else {
                return Err(bad_request("Missing resource uri"));
            };
            check_mcp_resource_scope(&auth_user, &uri).map_err(forbidden)?;
            let executor = build_executor(&state);
            match executor
                .read_resource(&auth_user.auth_session(), &uri)
                .await
            {
                Ok(data) => Ok(Json(ApiResponse {
                    success: true,
                    data: Some(json!({
                        "kind": "resource",
                        "action": "resource",
                        "name": uri,
                        "uri": uri,
                        "success": true,
                        "output": data,
                        "result": data,
                        "error": null,
                        "duration_ms": started.elapsed().as_millis() as u64,
                    })),
                    error: None,
                })),
                Err(err) => Err(bad_request(&err.to_string())),
            }
        }
        "prompt" => {
            let Some(prompt_name) = req.prompt.or(req.name) else {
                return Err(bad_request("Missing prompt name"));
            };
            auth_user.require_scope("server:read").map_err(forbidden)?;
            let executor = build_executor(&state);
            match executor
                .get_prompt(&auth_user.auth_session(), &prompt_name, &req.arguments)
                .await
            {
                Ok(prompt_result) => Ok(Json(ApiResponse {
                    success: true,
                    data: Some(json!({
                        "kind": "prompt",
                        "action": "prompt",
                        "name": prompt_name,
                        "success": true,
                        "output": prompt_result,
                        "result": prompt_result,
                        "error": null,
                        "duration_ms": started.elapsed().as_millis() as u64,
                    })),
                    error: None,
                })),
                Err(err) => Err(bad_request(&err.to_string())),
            }
        }
        _ => {
            let Some(tool_name) = req.tool.or(req.name) else {
                return Err(bad_request("Missing tool name"));
            };
            check_mcp_studio_sensitive_totp(&state.db, &auth_user, &tool_name, &headers).await?;
            let response = execute_tool(&state, &auth_user, tool_name.clone(), req.arguments)
                .await
                .map_err(forbidden)?;

            if response.success {
                Ok(Json(ApiResponse {
                    success: true,
                    data: Some(json!({
                        "kind": "tool",
                        "action": "tool",
                        "name": tool_name,
                        "tool": tool_name,
                        "success": true,
                        "output": response.result,
                        "result": response.result,
                        "error": null,
                        "duration_ms": started.elapsed().as_millis() as u64,
                    })),
                    error: None,
                }))
            } else {
                Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        data: None,
                        error: response.error,
                    }),
                ))
            }
        }
    }
}

/// Execute an MCP tool (PAT-authenticated)
pub async fn execute_mcp_tool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<McpExecuteRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    auth_user.require_pat().map_err(forbidden)?;
    let response = execute_tool(&state, &auth_user, req.tool, req.arguments)
        .await
        .map_err(forbidden)?;

    if response.success {
        Ok(Json(ApiResponse {
            success: true,
            data: Some(response),
            error: None,
        }))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                data: None,
                error: response.error,
            }),
        ))
    }
}

/// MCP JSON-RPC endpoint.
///
/// The older REST-style `/api/v1/mcp/execute` route remains available for
/// local scripts, while `/mcp` exposes the transport expected by MCP clients.
pub async fn handle_mcp_jsonrpc(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<ApiResponse<()>>)> {
    auth_user.require_pat().map_err(forbidden)?;

    if let Some(batch) = payload.as_array() {
        if let Err(error) = validate_jsonrpc_batch(batch) {
            return Ok(Json(error));
        }
        let mut responses = Vec::with_capacity(batch.len());
        for item in batch {
            responses.push(handle_jsonrpc_request(&state, &auth_user, item.clone()).await);
        }
        return Ok(Json(Value::Array(responses)));
    }

    Ok(Json(
        handle_jsonrpc_request(&state, &auth_user, payload).await,
    ))
}

pub fn mcp_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MCP_MAX_BODY_BYTES)
}

fn validate_jsonrpc_batch(batch: &[Value]) -> Result<(), Value> {
    if batch.is_empty() {
        return Err(jsonrpc_error(Value::Null, -32600, "Invalid Request"));
    }
    if batch.len() > MCP_MAX_BATCH_ITEMS {
        return Err(jsonrpc_error(Value::Null, -32600, "Batch too large"));
    }
    Ok(())
}

/// Get MCP server information
pub async fn get_mcp_info(
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    auth_user.require_pat().map_err(forbidden)?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(build_mcp_server_summary()),
        error: None,
    }))
}

async fn handle_jsonrpc_request(state: &AppState, auth_user: &AuthUser, value: Value) -> Value {
    let parsed = match serde_json::from_value::<JsonRpcRequest>(value) {
        Ok(req) => req,
        Err(_) => return jsonrpc_error(Value::Null, -32600, "Invalid Request"),
    };
    let id = parsed.id.unwrap_or(Value::Null);
    if parsed.jsonrpc.as_deref().unwrap_or("2.0") != "2.0" {
        return jsonrpc_error(id, -32600, "Invalid Request");
    }

    let Some(method) = parsed.method else {
        return jsonrpc_error(id, -32600, "Invalid Request");
    };

    match method.as_str() {
        "initialize" => jsonrpc_success(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "XLStatus MCP Server",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    },
                    "resources": {
                        "subscribe": false,
                        "listChanged": false
                    },
                    "prompts": {
                        "listChanged": false
                    }
                },
                "instructions": "XLStatus AI-Native Cluster Operations MCP Server. Start with cluster.overview or cluster.diagnose, resolve nodes by fuzzy name with server.resolve, inspect processes/containers/services with server.inspect, and execute batch commands with server.batch_exec."
            }),
        ),
        "notifications/initialized" | "ping" => jsonrpc_success(id, json!({})),
        "tools/list" => jsonrpc_success(
            id,
            json!({
                "tools": get_available_tools()
                    .into_iter()
                    .map(|tool| json!({
                        "name": tool.name,
                        "description": tool.description,
                        "category": tool.category,
                        "inputSchema": tool.input_schema,
                        "annotations": tool.annotations,
                    }))
                    .collect::<Vec<_>>()
            }),
        ),
        "tools/call" => {
            let params = parsed.params.unwrap_or_else(|| json!({}));
            let tool = params
                .get("name")
                .or_else(|| params.get("tool"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let Some(tool) = tool else {
                return jsonrpc_error(id, -32602, "Missing tool name");
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            execute_jsonrpc_tool(state, auth_user, id, tool, arguments).await
        }
        "resources/list" => jsonrpc_success(
            id,
            json!({
                "resources": get_available_resources()
                    .into_iter()
                    .map(|res| json!({
                        "uri": res.uri,
                        "name": res.name,
                        "description": res.description,
                        "mimeType": res.mime_type,
                        "mime_type": res.mime_type,
                    }))
                    .collect::<Vec<_>>()
            }),
        ),
        "resources/templates/list" => jsonrpc_success(
            id,
            json!({
                "resourceTemplates": get_available_resource_templates()
                    .into_iter()
                    .map(|tpl| json!({
                        "uriTemplate": tpl.uri_template,
                        "uri_template": tpl.uri_template,
                        "name": tpl.name,
                        "description": tpl.description,
                        "mimeType": tpl.mime_type,
                        "mime_type": tpl.mime_type,
                    }))
                    .collect::<Vec<_>>()
            }),
        ),
        "resources/read" => {
            let params = parsed.params.unwrap_or_else(|| json!({}));
            let Some(uri) = params.get("uri").and_then(Value::as_str) else {
                return jsonrpc_error(id, -32602, "Missing resource uri");
            };
            if check_mcp_resource_scope(auth_user, uri).is_err() {
                return jsonrpc_error(id, -32001, "Permission denied");
            }
            let executor = build_executor(state);
            match executor.read_resource(&auth_user.auth_session(), uri).await {
                Ok(structured) => {
                    let text = serde_json::to_string_pretty(&structured)
                        .unwrap_or_else(|_| structured.to_string());
                    jsonrpc_success(
                        id,
                        json!({
                            "contents": [
                                {
                                    "uri": uri,
                                    "mimeType": "application/json",
                                    "text": text,
                                }
                            ],
                            "structuredContent": structured,
                        }),
                    )
                }
                Err(err) => jsonrpc_error(id, -32002, &err.to_string()),
            }
        }
        "prompts/list" => jsonrpc_success(
            id,
            json!({
                "prompts": get_available_prompts(),
            }),
        ),
        "prompts/get" => {
            let params = parsed.params.unwrap_or_else(|| json!({}));
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return jsonrpc_error(id, -32602, "Missing prompt name");
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if auth_user.require_scope("server:read").is_err() {
                return jsonrpc_error(id, -32001, "Permission denied");
            }
            let executor = build_executor(state);
            match executor
                .get_prompt(&auth_user.auth_session(), name, &arguments)
                .await
            {
                Ok(prompt) => jsonrpc_success(id, prompt),
                Err(err) => jsonrpc_error(id, -32602, &err.to_string()),
            }
        }
        direct_tool
            if get_available_tools()
                .iter()
                .any(|tool| tool.name.as_str() == direct_tool) =>
        {
            execute_jsonrpc_tool(
                state,
                auth_user,
                id,
                direct_tool.to_string(),
                parsed.params.unwrap_or_else(|| json!({})),
            )
            .await
        }
        _ => jsonrpc_error(id, -32601, "Method not found"),
    }
}

async fn execute_jsonrpc_tool(
    state: &AppState,
    auth_user: &AuthUser,
    id: Value,
    tool: String,
    arguments: Value,
) -> Value {
    let response = match execute_tool(state, auth_user, tool, arguments).await {
        Ok(response) => response,
        Err(StatusCode::FORBIDDEN) => {
            return jsonrpc_error(id, -32001, "Permission denied");
        }
        Err(_) => return jsonrpc_error(id, -32000, "MCP tool execution failed"),
    };

    if response.success {
        let structured = response.result.unwrap_or(Value::Null);
        let text =
            serde_json::to_string_pretty(&structured).unwrap_or_else(|_| structured.to_string());
        jsonrpc_success(
            id,
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": text,
                    }
                ],
                "structuredContent": structured,
                "isError": false,
            }),
        )
    } else {
        jsonrpc_error(
            id,
            -32000,
            &response
                .error
                .unwrap_or_else(|| "MCP tool execution failed".to_string()),
        )
    }
}

async fn execute_tool(
    state: &AppState,
    auth_user: &AuthUser,
    tool: String,
    arguments: Value,
) -> Result<crate::mcp::tools::McpToolResponse, StatusCode> {
    check_mcp_tool_scope(auth_user, &tool)?;

    let executor = build_executor(state);

    Ok(executor
        .execute(
            &auth_user.auth_session(),
            McpToolRequest { tool, arguments },
        )
        .await)
}

fn jsonrpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

pub(crate) fn required_mcp_scope(tool: &str) -> &'static str {
    match tool {
        "meta.whoami" => "",
        "cluster.overview" | "cluster.diagnose" | "server.list" | "server.get"
        | "server.resolve" | "server.metrics" => "server:read",
        "service.list" | "service.history" => "service:read",
        "alert.events" | "alert.rules" => "alert:read",
        "task.list" => "task:read",
        "network.topology" => "nat:read",
        "server.exec" | "server.batch_exec" | "server.inspect" => "server:exec",
        "fs.list" | "fs.read" | "fs.download_url" => "transfer:read",
        "fs.write" | "fs.delete" | "fs.upload_url" => "transfer:write",
        _ => "admin:*",
    }
}

fn check_mcp_tool_scope(auth_user: &AuthUser, tool: &str) -> Result<(), StatusCode> {
    match tool {
        "meta.whoami" => Ok(()),
        "cluster.overview" | "cluster.diagnose" | "server.list" | "server.get"
        | "server.resolve" | "server.metrics" => auth_user.require_scope("server:read"),
        "service.list" | "service.history" => auth_user.require_scope("service:read"),
        "alert.events" | "alert.rules" => auth_user
            .require_scope("alert:read")
            .or_else(|_| auth_user.require_scope("server:read")),
        "task.list" => auth_user.require_scope("task:read"),
        "network.topology" => auth_user
            .require_scope("nat:read")
            .or_else(|_| auth_user.require_scope("ddns:read"))
            .or_else(|_| auth_user.require_scope("server:read")),
        "server.exec" | "server.batch_exec" | "server.inspect" => {
            auth_user.require_scope("server:exec")
        }
        "fs.list" | "fs.read" | "fs.download_url" => auth_user.require_scope("transfer:read"),
        "fs.write" | "fs.delete" | "fs.upload_url" => auth_user.require_scope("transfer:write"),
        _ => auth_user.require_scope(required_mcp_scope(tool)),
    }
}

fn check_mcp_resource_scope(auth_user: &AuthUser, uri: &str) -> Result<(), StatusCode> {
    match uri.trim() {
        "xlstatus://services/health" => check_mcp_tool_scope(auth_user, "service.list"),
        "xlstatus://alerts/active" => check_mcp_tool_scope(auth_user, "alert.events"),
        "xlstatus://network/topology" => check_mcp_tool_scope(auth_user, "network.topology"),
        _ => auth_user.require_scope("server:read"),
    }
}

async fn check_mcp_studio_sensitive_totp(
    db: &crate::db::Db,
    auth_user: &AuthUser,
    tool: &str,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if !auth_user.is_pat() && matches!(tool.trim(), "server.exec" | "server.batch_exec") {
        crate::api::v1::auth::require_sensitive_totp(db, auth_user.user.id, headers)
            .await
            .map_err(|err| {
                let message = match err {
                    crate::api::v1::auth::AppError::Forbidden(msg) => msg,
                    other => format!("{other:?}"),
                };
                (
                    StatusCode::FORBIDDEN,
                    Json(ApiResponse {
                        success: false,
                        data: None,
                        error: Some(message),
                    }),
                )
            })?;
    }
    Ok(())
}

fn forbidden(_: StatusCode) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::FORBIDDEN,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some("permission denied".to_string()),
        }),
    )
}

fn bad_request(message: &str) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(message.to_string()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_batch_rejects_empty_batch() {
        let err = validate_jsonrpc_batch(&[]).unwrap_err();
        assert_eq!(err["error"]["code"], -32600);
        assert_eq!(err["error"]["message"], "Invalid Request");
    }

    #[test]
    fn jsonrpc_batch_rejects_oversized_batch() {
        let batch = vec![
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
            MCP_MAX_BATCH_ITEMS + 1
        ];
        let err = validate_jsonrpc_batch(&batch).unwrap_err();
        assert_eq!(err["error"]["code"], -32600);
        assert_eq!(err["error"]["message"], "Batch too large");
    }

    #[test]
    fn jsonrpc_batch_allows_configured_maximum() {
        let batch =
            vec![json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}); MCP_MAX_BATCH_ITEMS];
        assert!(validate_jsonrpc_batch(&batch).is_ok());
    }

    #[test]
    fn mcp_server_summary_advertises_full_capabilities() {
        let summary = build_mcp_server_summary();
        assert_eq!(summary["protocol_version"], "2024-11-05");
        assert_eq!(summary["capabilities"]["tools"], true);
        assert_eq!(summary["capabilities"]["resources"], true);
        assert_eq!(summary["capabilities"]["prompts"], true);
        assert!(summary["tools_count"].as_u64().unwrap() >= 19);
        assert!(summary["resources_count"].as_u64().unwrap() >= 5);
        assert!(summary["prompts_count"].as_u64().unwrap() >= 4);
    }

    fn test_auth_user(scopes: Vec<&str>, is_pat: bool) -> AuthUser {
        use crate::auth::middleware::AuthKind;
        use crate::db::User;
        use chrono::Utc;
        use uuid::Uuid;
        use xlstatus_shared::{UserId, UserRole};

        let now = Utc::now();
        AuthUser {
            user: User {
                id: UserId(Uuid::now_v7()),
                username: "tester".to_string(),
                password_hash: "x".to_string(),
                role: UserRole::Member,
                token_version: 0,
                created_at: now,
                updated_at: now,
            },
            session_id: "sess-1".to_string(),
            csrf_token: "csrf-1".to_string(),
            auth_kind: if is_pat {
                AuthKind::PersonalAccessToken
            } else {
                AuthKind::Session
            },
            scopes: scopes.into_iter().map(str::to_string).collect(),
            server_ids: None,
            pat_id: is_pat.then(|| "pat-1".to_string()),
        }
    }

    #[test]
    fn mcp_tool_and_resource_scopes_enforce_least_privilege() {
        let server_read_only = test_auth_user(vec!["server:read"], true);
        assert!(check_mcp_tool_scope(&server_read_only, "cluster.overview").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "cluster.diagnose").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "server.resolve").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "server.metrics").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "alert.events").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "alert.rules").is_ok());
        assert!(check_mcp_tool_scope(&server_read_only, "network.topology").is_ok());
        assert_eq!(
            check_mcp_tool_scope(&server_read_only, "service.list"),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            check_mcp_tool_scope(&server_read_only, "service.history"),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            check_mcp_tool_scope(&server_read_only, "task.list"),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            check_mcp_resource_scope(&server_read_only, "xlstatus://services/health"),
            Err(StatusCode::FORBIDDEN)
        );

        let service_reader = test_auth_user(vec!["service:read"], true);
        assert!(check_mcp_tool_scope(&service_reader, "service.list").is_ok());
        assert!(check_mcp_tool_scope(&service_reader, "service.history").is_ok());
        assert!(check_mcp_resource_scope(&service_reader, "xlstatus://services/health").is_ok());

        let task_reader = test_auth_user(vec!["task:read"], true);
        assert!(check_mcp_tool_scope(&task_reader, "task.list").is_ok());
        assert_eq!(
            check_mcp_tool_scope(&task_reader, "network.topology"),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            check_mcp_tool_scope(&task_reader, "alert.events"),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[tokio::test]
    async fn mcp_studio_enforces_totp_for_cookie_session_exec_tools() {
        let db = crate::db::DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();

        let session_user = test_auth_user(vec!["server:read", "server:exec"], false);
        let mut pat_user = test_auth_user(vec!["server:read", "server:exec"], true);
        pat_user.user.id = session_user.user.id;

        let crate::db::DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, totp_enabled, totp_secret, created_at, updated_at) VALUES (?, 'tester', 'x', 'member', 1, 'JBSWY3DPEHPK3PXP', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(session_user.user.id.0.to_string())
        .execute(pool)
        .await
        .unwrap();

        let headers = HeaderMap::new();

        // Cookie session calling server.exec or server.batch_exec without X-TOTP-Code must fail with 403
        for tool in ["server.exec", "server.batch_exec"] {
            let err = check_mcp_studio_sensitive_totp(&db, &session_user, tool, &headers)
                .await
                .unwrap_err();
            assert_eq!(err.0, StatusCode::FORBIDDEN);
            assert_eq!(err.1 .0.error.as_deref(), Some("TOTP code is required"));
        }

        // Cookie session calling read-only or fixed-playbook tools does not require TOTP step-up
        for tool in ["cluster.overview", "server.inspect"] {
            assert!(
                check_mcp_studio_sensitive_totp(&db, &session_user, tool, &headers)
                    .await
                    .is_ok()
            );
        }

        // PAT session calling server.exec does not require interactive TOTP header
        assert!(
            check_mcp_studio_sensitive_totp(&db, &pat_user, "server.exec", &headers)
                .await
                .is_ok()
        );
    }
}
