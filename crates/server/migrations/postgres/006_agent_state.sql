-- M3: persist the most recent HostState / HostInfo JSON per agent.
-- The full TSDB (crates/tsdb) implementation is deferred to M8 per
-- plan/08-roadmap.md; this column gives the Dashboard a single, fast,
-- always-fresh sample to read for the agent status card.
ALTER TABLE agents ADD COLUMN last_state_json TEXT;
ALTER TABLE agents ADD COLUMN last_state_at TIMESTAMPTZ;
ALTER TABLE agents ADD COLUMN last_info_json TEXT;
ALTER TABLE agents ADD COLUMN last_info_at TIMESTAMPTZ;
