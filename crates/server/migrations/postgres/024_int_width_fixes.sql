-- Widen INT4 columns that Rust decodes as i64. sqlx postgres rejects an i64
-- read from an INT4 column at decode time, so any row that reaches these code
-- paths turned into a runtime error (sqlite masked this: its INTEGER is
-- always 8 bytes). Guarded so the batch runner can replay this file on every
-- startup without rewriting already-migrated tables.
--
-- service_results.delay_ms intentionally stays INT4: its readers decode i32
-- (public status, service history, monitor writes); the two i64 readers in
-- alerts/engine.rs cast with ::bigint in SQL instead.

DO $$
BEGIN
    -- int4 -> int8 rewrites the table under an ACCESS EXCLUSIVE lock. The
    -- whole migration batch runs as one implicit transaction, so a
    -- DATABASE_URL-configured statement_timeout would abort it mid-rewrite
    -- and leave the server in a restart loop. Lift the timeout for this
    -- transaction and bound the lock wait instead.
    PERFORM set_config('statement_timeout', '0', true);
    PERFORM set_config('lock_timeout', '30s', true);

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'waf_bans'
          AND column_name = 'failed_count'
          AND data_type = 'integer'
    ) THEN
        ALTER TABLE waf_bans ALTER COLUMN failed_count TYPE BIGINT;
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'task_runs'
          AND column_name = 'delay_ms'
          AND data_type = 'integer'
    ) THEN
        ALTER TABLE task_runs ALTER COLUMN delay_ms TYPE BIGINT;
    END IF;
END $$;
