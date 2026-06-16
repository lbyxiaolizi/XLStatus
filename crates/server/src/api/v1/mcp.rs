use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::auth::middleware::AuthUser;
use crate::db::Db;
use crate::mcp::executor::McpExecutor;
use crate::mcp::tools::{get_available_tools, McpTool, McpToolRequest, McpToolResponse};

#[derive(Debug, Serialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpTool>,
}

#[derive(Debug, Deserialize)]
pub struct McpExecuteRequest {
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// List available MCP tools
pub async fn list_mcp_tools(
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let tools = get_available_tools();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(McpToolsResponse { tools }),
        error: None,
    }))
}

/// Execute an MCP tool
pub async fn execute_mcp_tool(
    State(db): State<Db>,
    AuthUser { user, .. }: AuthUser,
    Json(req): Json<McpExecuteRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let executor = McpExecutor::new(db);

    let tool_request = McpToolRequest {
        tool: req.tool,
        arguments: req.arguments,
    };

    let response = executor.execute(&user.id.0.to_string(), tool_request).await;

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

/// Get MCP server information
pub async fn get_mcp_info(
    auth_user: AuthUser,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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
