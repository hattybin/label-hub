-- Control-plane schema (PostgreSQL).

CREATE TABLE IF NOT EXISTS nodes (
    id              TEXT PRIMARY KEY,
    site            TEXT NOT NULL DEFAULT '',
    hostname        TEXT NOT NULL DEFAULT '',
    mgmt_port       INTEGER NOT NULL DEFAULT 8081,
    app_version     TEXT NOT NULL DEFAULT '',
    config_version  BIGINT NOT NULL DEFAULT 0,
    node_token      TEXT NOT NULL,
    last_seen       TIMESTAMPTZ,
    queue_depth     INTEGER NOT NULL DEFAULT 0,
    printers_json   JSONB NOT NULL DEFAULT '[]',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Current desired config per node (version bumps on each edit).
CREATE TABLE IF NOT EXISTS node_config (
    node_id         TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    version         BIGINT NOT NULL DEFAULT 1,
    printers        JSONB NOT NULL DEFAULT '[]',
    settings        JSONB NOT NULL DEFAULT '{"auto_print":false}',
    inbound_secret  TEXT NOT NULL DEFAULT '',
    public_url      TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One-time enrollment tokens minted from the dashboard.
CREATE TABLE IF NOT EXISTS enrollment_tokens (
    token        TEXT PRIMARY KEY,
    site         TEXT NOT NULL DEFAULT '',
    note         TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    used_at      TIMESTAMPTZ,
    used_by_node TEXT
);

-- Print/audit event feed reported by nodes.
CREATE TABLE IF NOT EXISTS print_events (
    id       BIGSERIAL PRIMARY KEY,
    node_id  TEXT NOT NULL,
    printer  TEXT NOT NULL DEFAULT '',
    status   TEXT NOT NULL DEFAULT '',
    source   TEXT NOT NULL DEFAULT '',
    error    TEXT,
    at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_print_events_node ON print_events(node_id, at DESC);

-- Operator → site assignments (admins bypass via Entra app role).
CREATE TABLE IF NOT EXISTS user_sites (
    user_email TEXT NOT NULL,
    site       TEXT NOT NULL,
    PRIMARY KEY (user_email, site)
);

-- Dashboard action audit.
CREATE TABLE IF NOT EXISTS audit (
    id     BIGSERIAL PRIMARY KEY,
    actor  TEXT NOT NULL DEFAULT '',
    action TEXT NOT NULL DEFAULT '',
    target TEXT NOT NULL DEFAULT '',
    detail JSONB,
    at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
