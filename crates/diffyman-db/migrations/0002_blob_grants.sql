-- Blob grants: who may use a blob (docs/adr/0012-local-service-architecture.md).
-- Forward-only, like every migration.
--
-- Blobs are content-addressed, so several API keys can upload the same bytes
-- and share one object. A grant records that one key uploaded it, with that
-- upload's own expiry; api_key_id is NULL when authentication is off. A key
-- can start a job only on a blob it holds a live grant for, and a blob with a
-- live grant is never deleted, so one key's DELETE cannot remove another
-- key's upload. Blobs uploaded before this migration have no grant: saved
-- diffs and jobs that reference them keep working, and they expire as before.
CREATE TABLE blob_grants (
    sha256     text NOT NULL REFERENCES blobs (sha256) ON DELETE CASCADE,
    api_key_id uuid REFERENCES api_keys (id) ON DELETE CASCADE,
    media_type text CHECK (char_length(media_type) BETWEEN 1 AND 255),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CONSTRAINT blob_grants_once UNIQUE NULLS NOT DISTINCT (sha256, api_key_id)
);
CREATE INDEX blob_grants_expires_idx ON blob_grants (expires_at);
CREATE INDEX blob_grants_key_idx ON blob_grants (api_key_id);

-- True while a job or a saved diff references the blob, or a key holds a
-- grant for it that has not expired.
CREATE FUNCTION diffyman_blob_held(blob text) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT diffyman_blob_in_use(blob)
        OR EXISTS (SELECT 1 FROM blob_grants WHERE sha256 = blob AND expires_at > now())
$$;

-- The advisory lock that orders deleting a blob's object against storing it
-- again: the first 64 bits of its SHA-256.
CREATE FUNCTION diffyman_blob_lock_key(blob text) RETURNS bigint
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT ('x' || substr(blob, 1, 16))::bit(64)::bigint
$$;
