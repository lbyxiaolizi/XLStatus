use serde::{Deserialize, Serialize};

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP tool execution request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolRequest {
    pub tool: String,
    pub arguments: serde_json::Value,
}

/// MCP tool execution response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResponse {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Available MCP tools
pub fn get_available_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "meta.whoami".to_string(),
            description: "Get current user and system information".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        McpTool {
            name: "server.list".to_string(),
            description: "List all servers accessible to the current user".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of servers to return",
                        "default": 50
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Number of servers to skip",
                        "default": 0
                    }
                },
                "required": []
            }),
        },
        McpTool {
            name: "server.get".to_string(),
            description: "Get detailed information about a specific server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID or name"
                    }
                },
                "required": ["server_id"]
            }),
        },
        McpTool {
            name: "server.exec".to_string(),
            description: "Execute a command on a server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to execute",
                        "minLength": 1,
                        "maxLength": 8192
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds",
                        "default": 30,
                        "minimum": 1,
                        "maximum": 60
                    }
                },
                "required": ["server_id", "command"]
            }),
        },
        McpTool {
            name: "fs.list".to_string(),
            description: "List files in a directory on a server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory path"
                    }
                },
                "required": ["server_id", "path"]
            }),
        },
        McpTool {
            name: "fs.read".to_string(),
            description: "Read a file from a server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "max_size": {
                        "type": "integer",
                        "description": "Maximum file size in bytes",
                        "default": 1048576
                    }
                },
                "required": ["server_id", "path"]
            }),
        },
        McpTool {
            name: "fs.write".to_string(),
            description: "Write content to a file on a server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "content": {
                        "type": "string",
                        "description": "File content"
                    },
                    "mode": {
                        "type": "string",
                        "description": "Write mode: 'overwrite' or 'append'",
                        "default": "overwrite"
                    }
                },
                "required": ["server_id", "path", "content"]
            }),
        },
        McpTool {
            name: "fs.delete".to_string(),
            description: "Delete a file from a server".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    }
                },
                "required": ["server_id", "path"]
            }),
        },
        McpTool {
            name: "fs.download_url".to_string(),
            description: "Generate a temporary download URL for a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "expires_in": {
                        "type": "integer",
                        "description": "URL expiration time in seconds",
                        "default": 3600
                    }
                },
                "required": ["server_id", "path"]
            }),
        },
        McpTool {
            name: "fs.upload_url".to_string(),
            description: "Generate a temporary upload URL for a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "server_id": {
                        "type": "string",
                        "description": "Server ID"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path"
                    },
                    "expires_in": {
                        "type": "integer",
                        "description": "URL expiration time in seconds",
                        "default": 3600
                    }
                },
                "required": ["server_id", "path"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_tools() {
        let tools = get_available_tools();
        assert_eq!(tools.len(), 10);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"meta.whoami"));
        assert!(tool_names.contains(&"server.list"));
        assert!(tool_names.contains(&"fs.read"));
    }
}
