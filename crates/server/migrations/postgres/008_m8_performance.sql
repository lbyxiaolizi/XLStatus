-- M8 high-IO helpers for PostgreSQL.
--
-- This migration is intentionally safe to apply to an existing schema.
-- The current base migrations create heap tables, and PostgreSQL cannot
-- convert a populated heap table to a partitioned parent in-place without
-- an operator-planned data move. For that reason this file provides:
--
--   * idempotent retention policies for high-IO tables
--   * covering indexes for recent-window queries
--   * JSONB batch insert helper functions
--   * partition management functions that no-op unless a table is already
--     a RANGE-partitioned parent on created_at
--
-- Fresh high-scale deployments can change the parent CREATE TABLE shape
-- in 003/005 to `PARTITION BY RANGE (created_at)` before data lands; the
-- `xlstatus_ensure_high_io_partitions` function below then creates the
-- monthly child tables.

CREATE TABLE IF NOT EXISTS xlstatus_metric_retention_policies (
    table_name TEXT PRIMARY KEY,
    retention_days INTEGER NOT NULL CHECK (retention_days > 0),
    partition_interval TEXT NOT NULL DEFAULT 'monthly',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO xlstatus_metric_retention_policies
    (table_name, retention_days, partition_interval)
VALUES
    ('service_results', 30, 'monthly'),
    ('task_runs', 30, 'monthly'),
    ('audit_logs', 90, 'monthly'),
    ('transfers', 30, 'monthly')
ON CONFLICT (table_name) DO NOTHING;

DO $$
BEGIN
    IF to_regclass('public.service_results') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS idx_service_results_service_created_desc
            ON service_results(service_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_service_results_server_created_desc
            ON service_results(server_id, created_at DESC)
            WHERE server_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_service_results_status_created_desc
            ON service_results(status, created_at DESC);
    END IF;

    IF to_regclass('public.task_runs') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS idx_task_runs_task_created_desc
            ON task_runs(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_task_runs_server_created_desc
            ON task_runs(server_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_task_runs_status_created_desc
            ON task_runs(status, created_at DESC);
    END IF;

    IF to_regclass('public.audit_logs') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS idx_audit_logs_created_desc
            ON audit_logs(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_created_desc
            ON audit_logs(resource_type, resource_id, created_at DESC);
    END IF;

    IF to_regclass('public.transfers') IS NOT NULL THEN
        CREATE INDEX IF NOT EXISTS idx_transfers_owner_created_desc
            ON transfers(owner_user_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_transfers_server_created_desc
            ON transfers(server_id, created_at DESC);
    END IF;
END $$;

CREATE OR REPLACE FUNCTION xlstatus_relation_is_partitioned(parent_table REGCLASS)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
AS $$
    SELECT EXISTS (
        SELECT 1
        FROM pg_partitioned_table
        WHERE partrelid = parent_table
    );
$$;

CREATE OR REPLACE FUNCTION xlstatus_ensure_month_partition(
    parent_table REGCLASS,
    month_start DATE
)
RETURNS BOOLEAN
LANGUAGE plpgsql
AS $$
DECLARE
    parent_schema TEXT;
    parent_name TEXT;
    partition_name TEXT;
    start_at TIMESTAMPTZ;
    end_at TIMESTAMPTZ;
BEGIN
    IF NOT xlstatus_relation_is_partitioned(parent_table) THEN
        RAISE NOTICE 'Skipping %. It is not a partitioned parent.', parent_table;
        RETURN FALSE;
    END IF;

    SELECT n.nspname, c.relname
      INTO parent_schema, parent_name
      FROM pg_class c
      JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE c.oid = parent_table;

    start_at := date_trunc('month', month_start)::TIMESTAMPTZ;
    end_at := start_at + INTERVAL '1 month';
    partition_name := format('%s_%s', parent_name, to_char(start_at, 'YYYY_MM'));

    EXECUTE format(
        'CREATE TABLE IF NOT EXISTS %I.%I PARTITION OF %s FOR VALUES FROM (%L) TO (%L)',
        parent_schema,
        partition_name,
        parent_table,
        start_at,
        end_at
    );

    RETURN TRUE;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_ensure_high_io_partitions(
    start_month DATE DEFAULT CURRENT_DATE,
    months_ahead INTEGER DEFAULT 4
)
RETURNS TABLE(parent_name TEXT, partition_month DATE, partition_ready BOOLEAN)
LANGUAGE plpgsql
AS $$
DECLARE
    table_names TEXT[] := ARRAY[
        'service_results',
        'task_runs',
        'audit_logs',
        'transfers'
    ];
    table_name TEXT;
    parent REGCLASS;
    month_offset INTEGER;
BEGIN
    IF months_ahead < 1 THEN
        RAISE EXCEPTION 'months_ahead must be >= 1';
    END IF;

    FOREACH table_name IN ARRAY table_names LOOP
        parent := to_regclass(format('public.%I', table_name));
        IF parent IS NULL THEN
            CONTINUE;
        END IF;

        FOR month_offset IN 0..(months_ahead - 1) LOOP
            parent_name := table_name;
            partition_month :=
                (date_trunc('month', start_month)::DATE + (month_offset || ' months')::INTERVAL)::DATE;
            partition_ready := xlstatus_ensure_month_partition(parent, partition_month);
            RETURN NEXT;
        END LOOP;
    END LOOP;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_apply_high_io_retention(
    now_at TIMESTAMPTZ DEFAULT NOW()
)
RETURNS TABLE(table_name TEXT, cutoff_at TIMESTAMPTZ, rows_deleted BIGINT)
LANGUAGE plpgsql
AS $$
DECLARE
    policy RECORD;
    parent REGCLASS;
BEGIN
    FOR policy IN
        SELECT p.table_name, p.retention_days
          FROM xlstatus_metric_retention_policies p
         WHERE p.table_name IN ('service_results', 'task_runs', 'audit_logs', 'transfers')
    LOOP
        parent := to_regclass(format('public.%I', policy.table_name));
        IF parent IS NULL THEN
            CONTINUE;
        END IF;

        table_name := policy.table_name;
        cutoff_at := now_at - make_interval(days => policy.retention_days);
        EXECUTE format('DELETE FROM %s WHERE created_at < $1', parent)
            USING cutoff_at;
        GET DIAGNOSTICS rows_deleted = ROW_COUNT;
        RETURN NEXT;
    END LOOP;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_insert_service_results_batch(rows_json JSONB)
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    inserted INTEGER;
BEGIN
    INSERT INTO service_results (
        id, service_id, server_id, status, delay_ms, status_code,
        error, cert_fingerprint, cert_not_after, created_at
    )
    SELECT
        r.id,
        r.service_id,
        r.server_id,
        r.status,
        r.delay_ms,
        r.status_code,
        r.error,
        r.cert_fingerprint,
        r.cert_not_after,
        COALESCE(r.created_at, NOW())
    FROM jsonb_to_recordset(rows_json) AS r(
        id UUID,
        service_id UUID,
        server_id UUID,
        status TEXT,
        delay_ms INTEGER,
        status_code INTEGER,
        error TEXT,
        cert_fingerprint TEXT,
        cert_not_after TIMESTAMPTZ,
        created_at TIMESTAMPTZ
    )
    ON CONFLICT DO NOTHING;

    GET DIAGNOSTICS inserted = ROW_COUNT;
    RETURN inserted;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_insert_task_runs_batch(rows_json JSONB)
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    inserted INTEGER;
BEGIN
    INSERT INTO task_runs (
        id, task_id, server_id, status, delay_ms, output,
        output_truncated, error, created_at
    )
    SELECT
        r.id,
        r.task_id,
        r.server_id,
        r.status,
        r.delay_ms,
        r.output,
        COALESCE(r.output_truncated, FALSE),
        r.error,
        COALESCE(r.created_at, NOW())
    FROM jsonb_to_recordset(rows_json) AS r(
        id TEXT,
        task_id TEXT,
        server_id UUID,
        status TEXT,
        delay_ms INTEGER,
        output TEXT,
        output_truncated BOOLEAN,
        error TEXT,
        created_at TIMESTAMPTZ
    )
    ON CONFLICT DO NOTHING;

    GET DIAGNOSTICS inserted = ROW_COUNT;
    RETURN inserted;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_insert_transfers_batch(rows_json JSONB)
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    inserted INTEGER;
BEGIN
    INSERT INTO transfers (
        id, owner_user_id, server_id, op, path, size, status,
        error, created_at, completed_at
    )
    SELECT
        r.id,
        r.owner_user_id,
        r.server_id,
        r.op,
        r.path,
        r.size,
        r.status,
        r.error,
        COALESCE(r.created_at, NOW()),
        r.completed_at
    FROM jsonb_to_recordset(rows_json) AS r(
        id TEXT,
        owner_user_id UUID,
        server_id UUID,
        op TEXT,
        path TEXT,
        size BIGINT,
        status TEXT,
        error TEXT,
        created_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ
    )
    ON CONFLICT DO NOTHING;

    GET DIAGNOSTICS inserted = ROW_COUNT;
    RETURN inserted;
END;
$$;

CREATE OR REPLACE FUNCTION xlstatus_insert_audit_logs_batch(rows_json JSONB)
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    inserted INTEGER;
BEGIN
    INSERT INTO audit_logs (
        id, user_id, api_token_id, action, resource_type, resource_id,
        server_id, ip, outcome, metadata_json, sensitive_hash, created_at
    )
    SELECT
        r.id,
        r.user_id,
        r.api_token_id,
        r.action,
        r.resource_type,
        r.resource_id,
        r.server_id,
        r.ip,
        r.outcome,
        r.metadata_json,
        r.sensitive_hash,
        COALESCE(r.created_at, NOW())
    FROM jsonb_to_recordset(rows_json) AS r(
        id TEXT,
        user_id UUID,
        api_token_id UUID,
        action TEXT,
        resource_type TEXT,
        resource_id TEXT,
        server_id TEXT,
        ip TEXT,
        outcome TEXT,
        metadata_json TEXT,
        sensitive_hash TEXT,
        created_at TIMESTAMPTZ
    )
    ON CONFLICT DO NOTHING;

    GET DIAGNOSTICS inserted = ROW_COUNT;
    RETURN inserted;
END;
$$;
