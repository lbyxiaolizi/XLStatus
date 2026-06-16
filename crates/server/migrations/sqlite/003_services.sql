-- M5 任务执行相关表

-- 服务监控配置
CREATE TABLE IF NOT EXISTS services (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL, -- 'http', 'tcp', 'icmp'
    target TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL DEFAULT 60,
    timeout_seconds INTEGER NOT NULL DEFAULT 10,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_services_enabled ON services(enabled);

-- 服务监控历史
CREATE TABLE IF NOT EXISTS service_history (
    id TEXT PRIMARY KEY NOT NULL,
    service_id TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    latency_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    checked_at TEXT NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX idx_service_history_service ON service_history(service_id, checked_at);
CREATE INDEX idx_service_history_checked ON service_history(checked_at);
