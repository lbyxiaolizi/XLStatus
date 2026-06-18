-- Tasks and execution system for M5

-- Notification channels
CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    request_method TEXT NOT NULL DEFAULT 'POST',
    request_type TEXT NOT NULL DEFAULT 'json',
    headers_json TEXT,
    body_template TEXT,
    verify_tls INTEGER NOT NULL DEFAULT 1,
    format_metric_units INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_notifications_owner ON notifications(owner_user_id);

-- Notification groups
CREATE TABLE IF NOT EXISTS notification_groups (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_notification_groups_owner ON notification_groups(owner_user_id);

-- Notification group members
CREATE TABLE IF NOT EXISTS notification_group_members (
    group_id TEXT NOT NULL,
    notification_id TEXT NOT NULL,
    PRIMARY KEY (group_id, notification_id),
    FOREIGN KEY (group_id) REFERENCES notification_groups(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_id) REFERENCES notifications(id) ON DELETE CASCADE
);

-- Alert rules
CREATE TABLE IF NOT EXISTS alert_rules (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_mode TEXT NOT NULL, -- "always", "once"
    notification_group_id TEXT,
    rules_json TEXT NOT NULL, -- Array of rule conditions
    fail_task_ids_json TEXT, -- Array of task IDs to run on failure
    recover_task_ids_json TEXT, -- Array of task IDs to run on recovery
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_group_id) REFERENCES notification_groups(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_alert_rules_owner ON alert_rules(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules(enabled);

-- Tasks
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    task_type TEXT NOT NULL, -- "shell", "http_get", "icmp_ping", "tcp_ping"
    schedule TEXT, -- Cron expression, NULL for manual/triggered tasks
    command TEXT, -- For shell tasks
    payload_json TEXT, -- For other task types
    cover_mode TEXT NOT NULL DEFAULT 'all', -- "all", "any", "specific"
    server_selector_json TEXT NOT NULL, -- Server selection criteria
    push_successful INTEGER NOT NULL DEFAULT 0, -- Notify on success
    notification_group_id TEXT,
    last_executed_at TEXT,
    last_result TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_group_id) REFERENCES notification_groups(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_owner ON tasks(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_tasks_enabled ON tasks(enabled);
CREATE INDEX IF NOT EXISTS idx_tasks_schedule ON tasks(schedule) WHERE schedule IS NOT NULL;

-- Task runs (execution history)
CREATE TABLE IF NOT EXISTS task_runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    status TEXT NOT NULL, -- "success", "failure", "timeout", "offline"
    delay_ms INTEGER,
    output TEXT,
    output_truncated INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY (server_id) REFERENCES servers(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_task_runs_task ON task_runs(task_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_server ON task_runs(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_task_runs_status ON task_runs(status, created_at DESC);

-- File transfers
CREATE TABLE IF NOT EXISTS transfers (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    op TEXT NOT NULL, -- "upload", "download"
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    status TEXT NOT NULL, -- "pending", "in_progress", "completed", "failed"
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (server_id) REFERENCES servers(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_transfers_owner ON transfers(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_transfers_server ON transfers(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_transfers_status ON transfers(status, created_at DESC);

-- Audit logs
CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    api_token_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    server_id TEXT,
    ip TEXT NOT NULL,
    outcome TEXT NOT NULL, -- "success", "failure"
    metadata_json TEXT,
    sensitive_hash TEXT, -- SHA-256 of sensitive data for verification
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (api_token_id) REFERENCES personal_access_tokens(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user ON audit_logs(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_token ON audit_logs(api_token_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource ON audit_logs(resource_type, resource_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_server ON audit_logs(server_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON audit_logs(action, created_at DESC);
