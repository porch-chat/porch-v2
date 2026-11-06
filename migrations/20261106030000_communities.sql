ALTER TABLE users ADD COLUMN official_initialized BOOLEAN NOT NULL DEFAULT FALSE;
CREATE TABLE communities (
 id BIGSERIAL PRIMARY KEY,
 owner_id BIGINT NOT NULL REFERENCES users(id),
 name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
 official BOOLEAN NOT NULL DEFAULT FALSE,
 data JSONB NOT NULL CHECK (jsonb_typeof(data)='object' AND octet_length(data::text)<=2097152)
);
CREATE UNIQUE INDEX communities_official_idx ON communities(official) WHERE official;
CREATE INDEX communities_owner_idx ON communities(owner_id);
CREATE TABLE community_members (
 community_id BIGINT NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
 user_id BIGINT NOT NULL REFERENCES users(id),
 PRIMARY KEY(community_id,user_id)
);
CREATE INDEX community_members_user_idx ON community_members(user_id);
CREATE TABLE community_bans (
 community_id BIGINT NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
 user_id BIGINT NOT NULL REFERENCES users(id),
 PRIMARY KEY(community_id,user_id)
);
CREATE TABLE community_invites (
 id BIGSERIAL PRIMARY KEY,
 community_id BIGINT NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
 creator_id BIGINT NOT NULL REFERENCES users(id),
 code_hash TEXT NOT NULL UNIQUE CHECK(code_hash ~ '^[0-9a-f]{64}$'),
 max_uses INTEGER NOT NULL CHECK(max_uses BETWEEN 1 AND 200),
 uses INTEGER NOT NULL DEFAULT 0 CHECK(uses BETWEEN 0 AND max_uses),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 expires_at TIMESTAMPTZ NOT NULL,
 revoked_at TIMESTAMPTZ,
 CHECK(expires_at > created_at AND expires_at <= created_at + interval '30 days')
);
CREATE INDEX community_invites_community_idx ON community_invites(community_id);
