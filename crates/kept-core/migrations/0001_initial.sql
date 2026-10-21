-- Kept initial schema. All timestamps are RFC 3339 UTC strings.

CREATE TABLE preferences (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);

-- One row per connected (or previously connected) provider account.
CREATE TABLE accounts (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,             -- google | microsoft | ics_file | import | sample
    category         TEXT NOT NULL,             -- email | chat | calendar | tasks | documents | multi
    display_name     TEXT NOT NULL,
    email            TEXT,
    status           TEXT NOT NULL DEFAULT 'disconnected', -- disconnected|connecting|connected|syncing|attention|error
    status_detail    TEXT,
    scopes           TEXT,                      -- space separated, as granted
    enabled_streams  TEXT NOT NULL DEFAULT '[]',-- json array: ["mail","calendar"]
    connected_at     TEXT,
    last_sync_at     TEXT,
    last_success_at  TEXT,
    next_sync_at     TEXT,
    backoff_until    TEXT,
    error_count      INTEGER NOT NULL DEFAULT 0,
    items_synced     INTEGER NOT NULL DEFAULT 0,
    config_json      TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE sync_cursors (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    stream     TEXT NOT NULL,                   -- mail | sent | calendar
    cursor     TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, stream)
);

-- Normalised messages / chat posts / documents pulled from providers.
-- Bodies are bounded (see ingest::MAX_BODY_CHARS) and purged by retention policy.
CREATE TABLE source_items (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider      TEXT NOT NULL,
    kind          TEXT NOT NULL,                -- email | chat | document | task
    external_id   TEXT NOT NULL,
    thread_key    TEXT,
    sender_name   TEXT,
    sender_email  TEXT,
    to_json       TEXT NOT NULL DEFAULT '[]',
    cc_json       TEXT NOT NULL DEFAULT '[]',
    subject       TEXT,
    body_text     TEXT,
    snippet       TEXT,
    url           TEXT,
    sent_at       TEXT NOT NULL,
    is_from_me    INTEGER NOT NULL DEFAULT 0,
    content_hash  TEXT NOT NULL,
    process_state TEXT NOT NULL DEFAULT 'pending', -- pending | done | skipped | error
    process_note  TEXT,
    processed_at  TEXT,
    created_at    TEXT NOT NULL,
    UNIQUE (account_id, external_id)
);
CREATE INDEX idx_source_thread ON source_items(thread_key);
CREATE INDEX idx_source_state ON source_items(process_state);
CREATE INDEX idx_source_sent ON source_items(sent_at);

CREATE TABLE meetings (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    external_id     TEXT NOT NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    location        TEXT,
    start_at        TEXT NOT NULL,
    end_at          TEXT NOT NULL,
    timezone        TEXT,
    all_day         INTEGER NOT NULL DEFAULT 0,
    organizer_email TEXT,
    organizer_name  TEXT,
    join_url        TEXT,
    url             TEXT,
    status          TEXT NOT NULL DEFAULT 'confirmed', -- confirmed | tentative | cancelled
    my_response     TEXT,                              -- accepted | declined | tentative | needsAction
    prep_state      TEXT NOT NULL DEFAULT 'unknown',   -- unknown | none_needed | needed | ready
    updated_at      TEXT NOT NULL,
    UNIQUE (account_id, external_id)
);
CREATE INDEX idx_meetings_start ON meetings(start_at);

CREATE TABLE meeting_attendees (
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    email      TEXT NOT NULL,
    name       TEXT,
    response   TEXT,
    optional   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (meeting_id, email)
);
CREATE INDEX idx_attendee_email ON meeting_attendees(email);

CREATE TABLE projects (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at TEXT NOT NULL
);

CREATE TABLE tasks (
    id                   TEXT PRIMARY KEY,
    title                TEXT NOT NULL,
    description          TEXT,
    task_type            TEXT NOT NULL DEFAULT 'action', -- action|reply|approval|deliverable|followup|prep|review
    my_role              TEXT NOT NULL DEFAULT 'owner',  -- owner|collaborator|requester|mentioned
    owner_name           TEXT,
    status               TEXT NOT NULL,
    priority             INTEGER NOT NULL DEFAULT 3,     -- 1 critical .. 4 low
    priority_reason      TEXT,
    priority_locked      INTEGER NOT NULL DEFAULT 0,     -- user set it explicitly
    confidence           REAL NOT NULL DEFAULT 1.0,
    origin               TEXT NOT NULL,                  -- manual | rules | ai | external
    needs_confirmation   INTEGER NOT NULL DEFAULT 0,
    due_at               TEXT,
    due_kind             TEXT NOT NULL DEFAULT 'none',   -- none|explicit|inferred|suggested|confirmed
    due_phrase           TEXT,
    committed_at         TEXT,
    meeting_id           TEXT REFERENCES meetings(id) ON DELETE SET NULL,
    project_id           TEXT REFERENCES projects(id) ON DELETE SET NULL,
    expected_deliverable TEXT,
    completion_criteria  TEXT,
    parent_id            TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    recurrence           TEXT,                           -- daily | weekly | monthly | null
    effort_minutes       INTEGER,
    people_json          TEXT NOT NULL DEFAULT '[]',
    counterpart_email    TEXT,                           -- whom it is owed to / waiting on
    fingerprint          TEXT NOT NULL,
    duplicate_of         TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    possible_duplicate_of TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    conflict_note        TEXT,
    review_reason        TEXT,                           -- why it is in the review queue
    completion_evidence  TEXT,
    last_update_at       TEXT,
    completed_at         TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_due ON tasks(due_at);
CREATE INDEX idx_tasks_meeting ON tasks(meeting_id);
CREATE INDEX idx_tasks_parent ON tasks(parent_id);

-- Deadline history keeps explicit / inferred / suggested / confirmed dates separate and traceable.
CREATE TABLE task_deadlines (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL,      -- explicit | inferred | suggested | confirmed
    due_at         TEXT NOT NULL,
    phrase         TEXT,
    source_item_id TEXT REFERENCES source_items(id) ON DELETE SET NULL,
    reason         TEXT,
    created_at     TEXT NOT NULL,
    superseded_at  TEXT
);
CREATE INDEX idx_deadlines_task ON task_deadlines(task_id);

-- Evidence links: why a task exists, what updated it, what completed it.
CREATE TABLE task_sources (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    source_item_id TEXT REFERENCES source_items(id) ON DELETE SET NULL,
    meeting_id     TEXT REFERENCES meetings(id) ON DELETE SET NULL,
    relation       TEXT NOT NULL,      -- origin | update | evidence | followup | completion | duplicate
    evidence_quote TEXT,
    provider       TEXT,
    url            TEXT,
    source_label   TEXT,               -- survives source deletion: "Email from Sara · Oct 14"
    added_at       TEXT NOT NULL
);
CREATE INDEX idx_task_sources_task ON task_sources(task_id);
CREATE INDEX idx_task_sources_item ON task_sources(source_item_id);

CREATE TABLE task_dependencies (
    task_id    TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    note       TEXT,
    PRIMARY KEY (task_id, depends_on)
);

-- Every commitment sentence we recognised, including other people's promises (waiting-for).
CREATE TABLE commitments (
    id             TEXT PRIMARY KEY,
    source_item_id TEXT REFERENCES source_items(id) ON DELETE CASCADE,
    direction      TEXT NOT NULL,      -- mine | request_to_me | theirs | followup_on_me
    who_name       TEXT,
    who_email      TEXT,
    text           TEXT NOT NULL,
    due_at         TEXT,
    status         TEXT NOT NULL DEFAULT 'open', -- open | fulfilled | dismissed
    task_id        TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_commitments_status ON commitments(status);

-- Metadata only (no content) about every AI call, for cost and privacy disclosure.
CREATE TABLE ai_runs (
    id             TEXT PRIMARY KEY,
    provider       TEXT NOT NULL,
    model          TEXT NOT NULL,
    purpose        TEXT NOT NULL,
    source_ids     TEXT NOT NULL DEFAULT '[]',
    request_chars  INTEGER NOT NULL,
    response_chars INTEGER NOT NULL DEFAULT 0,
    items_returned INTEGER NOT NULL DEFAULT 0,
    items_rejected INTEGER NOT NULL DEFAULT 0,
    ok             INTEGER NOT NULL,
    error          TEXT,
    duration_ms    INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);

CREATE TABLE notifications (
    id             TEXT PRIMARY KEY,
    fingerprint    TEXT NOT NULL UNIQUE,  -- stable: kind + entity + stage
    category       TEXT NOT NULL,         -- deadline | overdue | meeting_prep | review | followup | sync | digest | system
    priority       INTEGER NOT NULL,      -- 1 critical .. 4 low
    title          TEXT NOT NULL,
    body           TEXT NOT NULL,
    entity_type    TEXT,
    entity_id      TEXT,
    group_key      TEXT,
    state          TEXT NOT NULL,         -- pending | held | delivered | snoozed | dismissed | expired | failed | in_app
    deliver_after  TEXT NOT NULL,
    delivered_at   TEXT,
    delivery_error TEXT,
    read_at        TEXT,
    acted_at       TEXT,
    snoozed_until  TEXT,
    expires_at     TEXT,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_notif_state ON notifications(state);
CREATE INDEX idx_notif_entity ON notifications(entity_type, entity_id);

CREATE TABLE notification_rules (
    category      TEXT PRIMARY KEY,
    enabled       INTEGER NOT NULL DEFAULT 1,
    desktop       INTEGER NOT NULL DEFAULT 1,
    lead_minutes  INTEGER NOT NULL DEFAULT 0,
    repeat_minutes INTEGER NOT NULL DEFAULT 0,
    min_priority  INTEGER NOT NULL DEFAULT 4,
    bypass_quiet  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE audit_events (
    id          TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_id   TEXT NOT NULL,
    action      TEXT NOT NULL,
    from_value  TEXT,
    to_value    TEXT,
    actor       TEXT NOT NULL,       -- user | agent | rules | ai | connector
    reason      TEXT,
    created_at  TEXT NOT NULL,
    undone      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_audit_entity ON audit_events(entity_type, entity_id);
CREATE INDEX idx_audit_created ON audit_events(created_at);

-- Pending outbound actions that need explicit user confirmation (none execute automatically).
CREATE TABLE external_actions (
    id          TEXT PRIMARY KEY,
    account_id  TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    action      TEXT NOT NULL,        -- e.g. create_external_task
    destination TEXT NOT NULL,
    payload     TEXT NOT NULL,
    effect      TEXT NOT NULL,
    state       TEXT NOT NULL DEFAULT 'proposed', -- proposed | confirmed | executed | rejected | failed
    created_at  TEXT NOT NULL,
    decided_at  TEXT
);

INSERT INTO notification_rules(category, enabled, desktop, lead_minutes, repeat_minutes, min_priority, bypass_quiet) VALUES
  ('deadline',     1, 1, 1440, 0,   3, 0),
  ('overdue',      1, 1, 0,    720, 3, 0),
  ('meeting_prep', 1, 1, 120,  0,   4, 0),
  ('review',       1, 0, 0,    0,   4, 0),
  ('followup',     1, 1, 0,    0,   3, 0),
  ('sync',         1, 1, 0,    0,   4, 0),
  ('digest',       1, 1, 0,    0,   4, 0),
  ('critical',     1, 1, 120,  60,  1, 1);
