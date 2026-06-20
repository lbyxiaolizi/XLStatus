-- Task triggers for alert and service failure/recovery events.

ALTER TABLE services
    ADD COLUMN IF NOT EXISTS failure_task_ids_json TEXT,
    ADD COLUMN IF NOT EXISTS recovery_task_ids_json TEXT;

ALTER TABLE alert_rules
    ADD COLUMN IF NOT EXISTS fail_task_ids_json TEXT,
    ADD COLUMN IF NOT EXISTS recover_task_ids_json TEXT;
