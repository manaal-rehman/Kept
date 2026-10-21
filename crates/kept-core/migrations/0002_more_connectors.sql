-- Documents (metadata only) from Google Drive and OneDrive/SharePoint.
CREATE TABLE documents (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider      TEXT NOT NULL,
    external_id   TEXT NOT NULL,
    name          TEXT NOT NULL,
    mime_type     TEXT,
    url           TEXT,
    modified_at   TEXT,
    modified_by   TEXT,
    owner         TEXT,
    updated_at    TEXT NOT NULL,
    UNIQUE (account_id, external_id)
);
CREATE INDEX idx_documents_modified ON documents(modified_at);

-- Tasks assigned to you in Jira / Asana / Todoist / Linear, mirrored into local tasks.
CREATE TABLE external_tasks (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider     TEXT NOT NULL,
    external_id  TEXT NOT NULL,
    key          TEXT,
    title        TEXT NOT NULL,
    status       TEXT,
    is_done      INTEGER NOT NULL DEFAULT 0,
    due_at       TEXT,
    url          TEXT,
    project      TEXT,
    task_id      TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE (account_id, external_id)
);
CREATE INDEX idx_external_tasks_task ON external_tasks(task_id);
