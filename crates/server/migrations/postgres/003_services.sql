-- M5 任务执行相关表

-- 服务监控配置
CREATE TABLE IF NOT EXISTS services (
    id UUID PRIMARY KEY NOT NULL,
    name VARCHAR(255) NOT NULL,
    type VARCHAR(50) NOT NULL,
    target TEXT NOT NULL,
    interval_seconds INTEGER NOT NULL DEFAULT 60,
    timeout_seconds INTEGER NOT NULL DEFAULT 10,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_services_enabled ON services(enabled);

-- 服务监控历史
CREATE TABLE IF NOT EXISTS service_history (
    id UUID PRIMARY KEY NOT NULL,
    service_id UUID NOT NULL,
    success BOOLEAN NOT NULL,
    latency_ms INTEGER,
    status_code INTEGER,
    error TEXT,
    checked_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);

CREATE INDEX idx_service_history_service ON service_history(service_id, checked_at);
CREATE INDEX idx_service_history_checked ON service_history(checked_at);
