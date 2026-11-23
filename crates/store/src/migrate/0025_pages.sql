CREATE TABLE board_pages (
    id BLOB PRIMARY KEY NOT NULL,
    workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slot TEXT NOT NULL,
    -- Copied out of the document so the list doesn't parse every page.
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    -- '' or 'theme', and the theme's id as text. No foreign key: a
    -- page must outlive the plan layer.
    anchor_kind TEXT NOT NULL DEFAULT '',
    anchor TEXT NOT NULL DEFAULT '',
    doc_json TEXT NOT NULL,
    schema_v INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    actor TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    resource_version INTEGER NOT NULL,
    UNIQUE (workspace_id, slot)
);

CREATE TABLE page_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    slot TEXT NOT NULL,
    actor TEXT NOT NULL,
    kind TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    shape TEXT NOT NULL DEFAULT ''
);
CREATE INDEX page_events_by_slot ON page_events (workspace_id, slot, at);
