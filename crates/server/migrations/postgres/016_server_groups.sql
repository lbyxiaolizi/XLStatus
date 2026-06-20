CREATE TABLE IF NOT EXISTS server_groups (
    id UUID PRIMARY KEY NOT NULL,
    owner_user_id UUID NOT NULL,
    name TEXT NOT NULL,
    color TEXT,
    display_order INTEGER,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(owner_user_id, name)
);

CREATE TABLE IF NOT EXISTS server_group_members (
    group_id UUID NOT NULL,
    agent_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (group_id, agent_id),
    FOREIGN KEY (group_id) REFERENCES server_groups(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_server_groups_owner ON server_groups(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_server_group_members_agent ON server_group_members(agent_id);
