-- No cascading deletion: disk reclamation must be explicit and coordinated.
CREATE TABLE uploads (
 id TEXT PRIMARY KEY CHECK(id ~ '^[0-9a-f]{32}$'),
 owner_id BIGINT NOT NULL,
 conversation_id BIGINT NOT NULL,
 filename TEXT NOT NULL CHECK(octet_length(filename) BETWEEN 1 AND 180),
 bytes BIGINT NOT NULL CHECK(bytes BETWEEN 1 AND 1073741824),
 received BIGINT NOT NULL DEFAULT 0 CHECK(received BETWEEN 0 AND bytes),
 sha256 TEXT NOT NULL CHECK(sha256 ~ '^[0-9a-f]{64}$'),
 mime TEXT NOT NULL DEFAULT 'application/octet-stream',
 state TEXT NOT NULL DEFAULT 'receiving' CHECK(state IN ('receiving','ready','attached')),
 message_id BIGINT,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '24 hours',
 CHECK((state='receiving') OR received=bytes),
 CHECK((state='attached')=(message_id IS NOT NULL))
);
CREATE INDEX uploads_owner_idx ON uploads(owner_id);
CREATE INDEX uploads_message_idx ON uploads(conversation_id,message_id);
CREATE INDEX uploads_expiry_idx ON uploads(expires_at) WHERE message_id IS NULL;
