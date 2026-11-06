CREATE TABLE conversations (
 id BIGSERIAL PRIMARY KEY,
 community_id BIGINT REFERENCES communities(id) ON DELETE CASCADE,
 channel_id BIGINT,
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 100),
 owner_id BIGINT REFERENCES users(id),
 participants BIGINT[] NOT NULL DEFAULT '{}',
 dm_pair TEXT UNIQUE,
 revision BIGINT NOT NULL DEFAULT 0 CHECK(revision>=0),
 CHECK ((community_id IS NOT NULL AND channel_id IS NOT NULL AND owner_id IS NULL AND cardinality(participants)=0 AND dm_pair IS NULL) OR
 (community_id IS NULL AND channel_id IS NULL AND owner_id IS NOT NULL AND cardinality(participants) BETWEEN 1 AND 20 AND owner_id=ANY(participants))),
 UNIQUE(community_id,channel_id)
);
CREATE INDEX conversations_participants_idx ON conversations USING GIN(participants);
INSERT INTO conversations(community_id,channel_id,name)
 SELECT c.id,(ch->>'id')::bigint,ch->>'name' FROM communities c,
 LATERAL jsonb_array_elements(c.data->'channels') ch WHERE ch->>'kind'='text';
CREATE TABLE messages (
 id BIGSERIAL PRIMARY KEY,
 conversation_id BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 author_id BIGINT NOT NULL REFERENCES users(id),
 nonce TEXT NOT NULL CHECK(length(nonce) BETWEEN 16 AND 64),
 fingerprint TEXT NOT NULL CHECK(length(fingerprint)=64),
 reply_to BIGINT,
 data JSONB NOT NULL CHECK(jsonb_typeof(data)='object' AND octet_length(data::text)<=65536),
 source TEXT NOT NULL DEFAULT '' CHECK(octet_length(source)<=8000),
 search TSVECTOR GENERATED ALWAYS AS(to_tsvector('simple',source)) STORED,
 deleted BOOLEAN NOT NULL DEFAULT FALSE,
 mentions BIGINT[] NOT NULL DEFAULT '{}',
 mention_everyone BOOLEAN NOT NULL DEFAULT FALSE,
 pinned BOOLEAN NOT NULL DEFAULT FALSE,
 revision BIGINT NOT NULL CHECK(revision>0),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 UNIQUE(conversation_id,id),
 UNIQUE(conversation_id,author_id,nonce),
 FOREIGN KEY(conversation_id,reply_to) REFERENCES messages(conversation_id,id)
);
CREATE INDEX messages_history_idx ON messages(conversation_id,id DESC);
CREATE INDEX messages_delta_idx ON messages(conversation_id,revision);
CREATE INDEX messages_search_idx ON messages USING GIN(search) WHERE NOT deleted;
CREATE INDEX messages_pins_idx ON messages(conversation_id,id DESC) WHERE pinned AND NOT deleted;
CREATE TABLE message_reads (
 conversation_id BIGINT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 user_id BIGINT NOT NULL REFERENCES users(id),
 message_id BIGINT NOT NULL,
 PRIMARY KEY(conversation_id,user_id),
 FOREIGN KEY(conversation_id,message_id) REFERENCES messages(conversation_id,id)
);
