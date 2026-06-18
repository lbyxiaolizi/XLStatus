use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalServerMessage {
    Ready { session_id: String },
    Output { data: String },
    Closed { reason: String },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalBridgeMessage {
    Open { cols: u16, rows: u16 },
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Close { reason: Option<String> },
    Output { data: String },
    Error { message: String },
}
