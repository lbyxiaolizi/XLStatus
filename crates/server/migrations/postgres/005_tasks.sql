-- Tasks and execution system for M5

-- Notification channels
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    owner_user_id UUID NOT NULL,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    request_method TEXT NOT NULL DEFAULT 'POST',
    request_type TEXT NOT NULL DEFAULT 'json',
    headers_json TEXT,
    body_template TEXT,
    verify_tls BOOLEAN NOT NULL DEFAULT true,
    format_metric_units BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_notifications_owner ON notifications(owner_user_id);

-- Notification groups
CREATE TABLE notification_groups (
    id TEXT PRIMARY KEY,
    owner_user_id UUID NOT NULL,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_notification_groups_owner ON notification_groups(owner_user_id);

-- Notification group members
CREATE TABLE notification_group_members (
    group_id TEXT NOT NULL,
    notification_id TEXT NOT NULL,
    PRIMARY KEY (group_id, notification_id),
    FOREIGN KEY (group_id) REFERENCES notification_groups(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_id) REFERENCES notifications(id) ON DELETE CASCADE
);

-- Alert rules
CREATE TABLE alert_rules (
    id TEXT PRIMARY KEY,
    owner_user_id UUID NOT NULL,
    name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    trigger_mode TEXT NOT NULL, -- "always", "once"
    notification_group_id TEXT,
    rules_json TEXT NOT NULL, -- Array of rule conditions
    fail_task_ids_json TEXT, -- Array of task IDs to run on failure
    recover_task_ids_json TEXT, -- Array of task IDs to run on recovery
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_group_id) REFERENCES notification_groups(id) ON DELETE SET NULL
);

CREATE INDEX idx_alert_rules_owner ON alert_rules(owner_user_id);
CREATE INDEX idx_alert_rules_enabled ON alert_rules(enabled) WHERE enabled = true;

-- Tasks
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    owner_user_id UUID NOT NULL,
    name TEXT NOT NULL,
    task_type TEXT NOT NULL, -- "shell", "http_get", "icmp_ping", "tcp_ping"
    schedule TEXT, -- Cron expression, NULL for manual/triggered tasks
    command TEXT, -- For shell tasks
    payload_json TEXT, -- For other task types
    cover_mode TEXT NOT NULL DEFAULT 'all', -- "all", "any", "specific"
    server_selector_json TEXT NOT NULL, -- Server selection criteria
    push_successful BOOLEAN NOT NULL DEFAULT false, -- Notify on success
    notification_group_id TEXT,
    last_executed_at TIMESTAMPTZ,
    last_result TEXT,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (notification_group_id) REFERENCES notification_groups(id) ON DELETE SET NULL
);

CREATE INDEX idx_tasks_owner ON tasks(owner_user_id);
CREATE INDEX idx_tasks_enabled ON tasks(enabled) WHERE enabled = true;
CREATE INDEX idx_tasks_schedule ON tasks(schedule) WHERE schedule IS NOT NULL;

-- Task runs (execution history) - partitioned by created_at
CREATE TABLE task_runs (
    id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    server_id UUID NOT NULL,
    status TEXT NOT NULL, -- "success", "failure", "timeout", "offline"
    delay_ms INTEGER,
    output TEXT,
    output_truncated BOOLEAN NOT NULL DEFAULT false,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY (server_id) REFERENCES servers(id) ON DELETE CASCADE
);
-- Note: monthly partitioning deferred to M8 (high-IO performance) per plan/08-roadmap.md.

CREATE INDEX idx_task_runs_task ON task_runs(task_id, created_at DESC);
CREATE INDEX idx_task_runs_server ON task_runs(server_id, created_at DESC);
CREATE INDEX idx_task_runs_status ON task_runs(status, created_at DESC);

-- File transfers - partitioned by created_at
CREATE TABLE transfers (
    id TEXT NOT NULL,
    owner_user_id UUID NOT NULL,
    server_id UUID NOT NULL,
    op TEXT NOT NULL, -- "upload", "download"
    path TEXT NOT NULL,
    size BIGINT NOT NULL,
    status TEXT NOT NULL, -- "pending", "in_progress", "completed", "failed"
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (server_id) REFERENCES servers(id) ON DELETE CASCADE
);
-- Note: monthly partitioning deferred to M8 per plan/08-roadmap.md.

CREATE INDEX idx_transfers_owner ON transfers(owner_user_id);
CREATE INDEX idx_transfers_server ON transfers(server_id, created_at DESC);
CREATE INDEX idx_transfers_status ON transfers(status, created_at DESC);

-- Audit logs - partitioned by created_at
CREATE TABLE audit_logs (
    id TEXT NOT NULL,
    user_id UUID,
    api_token_id UUID,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    server_id TEXT,
    ip TEXT NOT NULL,
    outcome TEXT NOT NULL, -- "success", "failure"
    metadata_json TEXT,
    sensitive_hash TEXT, -- SHA-256 of sensitive data for verification
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (api_token_id) REFERENCES personal_access_tokens(id) ON DELETE SET NULL
);
-- Note: monthly partitioning deferred to M8 per plan/08-roadmap.md.

CREATE INDEX idx_audit_logs_user ON audit_logs(user_id, created_at DESC);
CREATE INDEX idx_audit_logs_token ON audit_logs(api_token_id, created_at DESC);
CREATE INDEX idx_audit_logs_resource ON audit_logs(resource_type, resource_id, created_at DESC);
CREATE INDEX idx_audit_logs_server ON audit_logs(server_id, created_at DESC);
CREATE INDEX idx_audit_logs_action ON audit_logs(action, created_at DESC);
