CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE CHECK (username ~ '^[a-z0-9_]{3,32}$'),
    admin BOOLEAN NOT NULL DEFAULT FALSE,
    can_invite BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- provider + subject is the extension seam for OAuth and passkeys.
CREATE TABLE identities (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    password_hash TEXT,
    UNIQUE(provider, subject),
    UNIQUE(user_id, provider),
    CHECK ((provider = 'password') = (password_hash IS NOT NULL))
);
CREATE TABLE sessions (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '30 days',
    revoked_at TIMESTAMPTZ
);
CREATE INDEX sessions_user_idx ON sessions(user_id);
CREATE TABLE invites (
    id BIGSERIAL PRIMARY KEY,
    creator_id BIGINT NOT NULL REFERENCES users(id),
    code_hash TEXT NOT NULL UNIQUE,
    max_uses INTEGER NOT NULL CHECK (max_uses BETWEEN 1 AND 200),
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0 AND uses <= max_uses),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invites_creator_idx ON invites(creator_id);
