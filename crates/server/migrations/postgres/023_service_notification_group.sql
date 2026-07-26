ALTER TABLE services
    ADD COLUMN IF NOT EXISTS notification_group_id UUID;
