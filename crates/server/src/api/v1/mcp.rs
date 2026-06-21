use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::AppState;
use crate::auth::middleware::AuthUser;
use crate::mcp::executor::McpExecutor;
use crate::mcp::tools::{get_available_tools, McpTool, McpToolRequest};

#[derive(Debug, Serialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Deserialize)]
pub struct McpExecuteRequest {
    pub tool: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

/// List available MCP tools
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

/// Execute an MCP tool
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
        if batch.is_empty() {
            return Ok(Json(jsonrpc_error(Value::Null, -32600, "Invalid Request")));
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

/// Get MCP server information
pub async fn get_mcp_info(
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    auth_user.require_pat().map_err(forbidden)?;
    Ok(Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({
            "name": "XLStatus MCP Server",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": "1.0",
            "capabilities": {
                "tools": true,
                "resources": false,
                "prompts": false,
            },
            "tools_count": get_available_tools().len(),
        })),
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
                    "tools": {}
                }
            }),
        ),
        "tools/list" => jsonrpc_success(
            id,
            json!({
                "tools": get_available_tools()
                    .into_iter()
                    .map(|tool| json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
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
    if let Some(scope) = required_mcp_scope(&tool) {
        auth_user.require_scope(scope)?;
    }

    let executor = McpExecutor::new(
        state.db.clone(),
        state.session_registry.clone(),
        state.config.security.session_secret.clone(),
    );

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

fn required_mcp_scope(tool: &str) -> Option<&'static str> {
    match tool {
        "meta.whoami" => None,
        "server.list" | "server.get" => Some("server:read"),
        "server.exec" => Some("server:exec"),
        "fs.list" | "fs.read" | "fs.download_url" => Some("transfer:read"),
        "fs.write" | "fs.delete" | "fs.upload_url" => Some("transfer:write"),
        _ => Some("admin:*"),
    }
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
