-- Per-user identity + bring-your-own-key (BYOK) MCP credentials.
--
-- Users authenticate with their own API key (LiteLLM_VerificationTokenTable),
-- and store per-server upstream credentials (LiteLLM_MCPUserCredentialTable)
-- that the gateway injects into MCP calls on their behalf. Credentials are
-- encrypted at rest with AES-256-GCM (see src/db/managed_agents/mcp_credentials/crypto.rs).

CREATE TABLE IF NOT EXISTS "LiteLLM_UserTable" (
  user_id    TEXT PRIMARY KEY,
  user_alias TEXT,
  created_at BIGINT NOT NULL
);

-- v0 stores the API key verbatim as the primary key. LiteLLM hashes keys; that
-- is deferred (see PR notes). The key is the bearer token a user presents.
CREATE TABLE IF NOT EXISTS "LiteLLM_VerificationTokenTable" (
  token      TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES "LiteLLM_UserTable" (user_id) ON DELETE CASCADE,
  key_alias  TEXT,
  created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS "LiteLLM_VerificationToken_user_idx"
  ON "LiteLLM_VerificationTokenTable" (user_id);

-- One credential per (user, MCP server). credential_type discriminates between
-- a pasted static/BYOK token and an OAuth2 token set. All secret columns are
-- AES-256-GCM ciphertext (12-byte nonce || ciphertext).
CREATE TABLE IF NOT EXISTS "LiteLLM_MCPUserCredentialTable" (
  user_id           TEXT NOT NULL REFERENCES "LiteLLM_UserTable" (user_id) ON DELETE CASCADE,
  server_id         TEXT NOT NULL,
  credential_type   TEXT NOT NULL,          -- 'static' | 'oauth'
  credential_enc    BYTEA,                  -- static/BYOK token
  access_token_enc  BYTEA,                  -- oauth access token
  refresh_token_enc BYTEA,                  -- oauth refresh token (optional)
  expires_at        BIGINT,                 -- oauth access token expiry (ms epoch)
  scopes            TEXT,                   -- space-joined oauth scopes
  created_at        BIGINT NOT NULL,
  updated_at        BIGINT NOT NULL,
  PRIMARY KEY (user_id, server_id)
);
