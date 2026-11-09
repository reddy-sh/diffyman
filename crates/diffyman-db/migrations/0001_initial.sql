-- Initial schema of the diffyman server (docs/adr/0012-local-service-architecture.md).
-- Migrations are forward-only: never edit this file once it is released; add a
-- new one. Needs PostgreSQL 18 (uuidv7()). File content never lives here, only
-- metadata and blob IDs (the SHA-256 that names each object in the blob store).

-- API keys. Only a 12-character prefix (for lookup) and a keyed BLAKE3 hash of
-- the key are stored, never the key itself.
CREATE TABLE api_keys (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    prefix       text NOT NULL CHECK (prefix ~ '^dmk_[0-9a-f]{8}$'),
    hash         bytea NOT NULL UNIQUE CHECK (octet_length(hash) = 32),
    name         text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    scopes       text[] NOT NULL DEFAULT '{}'
                 CHECK (array_to_string(scopes, ',') ~ '^([a-z]+(:[a-z]+)?(,[a-z]+(:[a-z]+)?)*)?$'),
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at   timestamptz
);
CREATE INDEX api_keys_prefix_idx ON api_keys (prefix);

-- One row per object in the blob store. A blob is in use while a job or a diff
-- references it; the foreign keys below refuse to delete it until then.
CREATE TABLE blobs (
    sha256     text PRIMARY KEY CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    size       bigint NOT NULL CHECK (size >= 0),
    media_type text CHECK (char_length(media_type) BETWEEN 1 AND 255),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX blobs_expires_idx ON blobs (expires_at);

-- Saved diffs: a stored Diff Document plus the summary needed to list it.
CREATE TABLE diffs (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    api_key_id    uuid REFERENCES api_keys (id) ON DELETE SET NULL,
    document_blob text NOT NULL REFERENCES blobs (sha256),
    left_blob     text REFERENCES blobs (sha256),
    right_blob    text REFERENCES blobs (sha256),
    status        text NOT NULL CHECK (status IN ('identical', 'different', 'partial', 'error')),
    kind          text NOT NULL CHECK (kind IN ('text', 'code', 'structured', 'table', 'document', 'image', 'binary', 'tree')),
    stats         jsonb CHECK (stats IS NULL OR jsonb_typeof(stats) = 'object'),
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL
);
CREATE INDEX diffs_expires_idx ON diffs (expires_at);
CREATE INDEX diffs_document_blob_idx ON diffs (document_blob);
CREATE INDEX diffs_left_blob_idx ON diffs (left_blob);
CREATE INDEX diffs_right_blob_idx ON diffs (right_blob);

-- Jobs are also the queue: workers claim queued rows with FOR UPDATE SKIP LOCKED
-- and hold a lease that they renew with heartbeats.
CREATE TABLE jobs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    api_key_id       uuid REFERENCES api_keys (id) ON DELETE SET NULL,
    status           text NOT NULL DEFAULT 'queued'
                     CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    options          jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(options) = 'object'),
    left_blob        text NOT NULL REFERENCES blobs (sha256),
    right_blob       text NOT NULL REFERENCES blobs (sha256),
    left_name        text CHECK (char_length(left_name) BETWEEN 1 AND 1024),
    right_name       text CHECK (char_length(right_name) BETWEEN 1 AND 1024),
    result_blob      text REFERENCES blobs (sha256),
    diff_id          uuid REFERENCES diffs (id) ON DELETE SET NULL,
    error_code       text CHECK (error_code ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    error_message    text CHECK (char_length(error_message) <= 2000),
    attempts         integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts     integer NOT NULL DEFAULT 3 CHECK (max_attempts BETWEEN 1 AND 100),
    cancel_requested boolean NOT NULL DEFAULT false,
    created_at       timestamptz NOT NULL DEFAULT now(),
    run_after        timestamptz NOT NULL DEFAULT now(),
    started_at       timestamptz,
    finished_at      timestamptz,
    lease_owner      text CHECK (char_length(lease_owner) BETWEEN 1 AND 200),
    lease_expires_at timestamptz,
    heartbeat_at     timestamptz,
    expires_at       timestamptz NOT NULL,
    CONSTRAINT jobs_lease_iff_running
        CHECK ((status = 'running') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)),
    CONSTRAINT jobs_finished_iff_final
        CHECK ((status IN ('succeeded', 'failed', 'cancelled')) = (finished_at IS NOT NULL)),
    CONSTRAINT jobs_result_iff_succeeded CHECK ((status = 'succeeded') = (result_blob IS NOT NULL)),
    -- A job queued again after a failed attempt keeps that attempt's error.
    CONSTRAINT jobs_failed_has_error CHECK (status <> 'failed' OR error_code IS NOT NULL)
);
CREATE INDEX jobs_queue_idx ON jobs (run_after, id) WHERE status = 'queued';
CREATE INDEX jobs_lease_idx ON jobs (lease_expires_at) WHERE status = 'running';
CREATE INDEX jobs_expires_idx ON jobs (expires_at);
CREATE INDEX jobs_left_blob_idx ON jobs (left_blob);
CREATE INDEX jobs_right_blob_idx ON jobs (right_blob);
CREATE INDEX jobs_result_blob_idx ON jobs (result_blob);
CREATE INDEX jobs_diff_id_idx ON jobs (diff_id);

-- Append-only usage records: one per API call or job.
CREATE TABLE usage (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    api_key_id  uuid,
    operation   text NOT NULL CHECK (operation ~ '^[a-z]+(\.[a-z]+)*$'),
    outcome     text NOT NULL CHECK (outcome ~ '^[a-z0-9]+(-[a-z0-9]+)*$'),
    bytes_in    bigint NOT NULL DEFAULT 0 CHECK (bytes_in >= 0),
    bytes_out   bigint NOT NULL DEFAULT 0 CHECK (bytes_out >= 0),
    duration_ms bigint NOT NULL DEFAULT 0 CHECK (duration_ms >= 0)
);
CREATE INDEX usage_key_at_idx ON usage (api_key_id, at);

-- Append-only audit records: who did what to which record.
CREATE TABLE audit (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    actor       text NOT NULL CHECK (actor ~ '^(admin|system|key:[0-9a-f-]{36})$'),
    action      text NOT NULL CHECK (action ~ '^[a-z]+(\.[a-z_]+)*$'),
    target_kind text CHECK (target_kind ~ '^[a-z_]+$'),
    target_id   text CHECK (char_length(target_id) <= 200),
    detail      jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(detail) = 'object')
);
CREATE INDEX audit_at_idx ON audit (at);

-- True while a job or a saved diff references the blob.
CREATE FUNCTION diffyman_blob_in_use(blob text) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM jobs WHERE left_blob = blob OR right_blob = blob OR result_blob = blob)
        OR EXISTS (SELECT 1 FROM diffs WHERE document_blob = blob OR left_blob = blob OR right_blob = blob)
$$;

CREATE FUNCTION diffyman_reject_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'table % is append-only', TG_TABLE_NAME
        USING ERRCODE = 'insufficient_privilege';
END
$$;

CREATE TRIGGER usage_append_only BEFORE UPDATE OR DELETE ON usage
    FOR EACH ROW EXECUTE FUNCTION diffyman_reject_change();
CREATE TRIGGER usage_no_truncate BEFORE TRUNCATE ON usage
    FOR EACH STATEMENT EXECUTE FUNCTION diffyman_reject_change();
CREATE TRIGGER audit_append_only BEFORE UPDATE OR DELETE ON audit
    FOR EACH ROW EXECUTE FUNCTION diffyman_reject_change();
CREATE TRIGGER audit_no_truncate BEFORE TRUNCATE ON audit
    FOR EACH STATEMENT EXECUTE FUNCTION diffyman_reject_change();
