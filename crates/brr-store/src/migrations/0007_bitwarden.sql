-- SQLite cannot drop NOT NULL, so the table is rebuilt. Foreign keys are off while migrations run
-- (see Store::init): dropping `credential` with them on would cascade into credential_profile and
-- null login_check.credential_id.
CREATE TABLE credential_new (
  id TEXT PRIMARY KEY,
  site TEXT NOT NULL,
  login_url TEXT,
  username_enc BLOB,
  password_enc BLOB,
  totp_enc BLOB,
  notes_enc BLOB,
  selectors TEXT NOT NULL DEFAULT '{}',
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  source TEXT NOT NULL DEFAULT 'local',
  bitwarden_item_id TEXT,
  CHECK (source <> 'local' OR (username_enc IS NOT NULL AND password_enc IS NOT NULL)),
  CHECK (source <> 'bitwarden' OR bitwarden_item_id IS NOT NULL)
);
INSERT INTO credential_new (id, site, login_url, username_enc, password_enc, totp_enc, notes_enc, selectors, created_at, updated_at)
  SELECT id, site, login_url, username_enc, password_enc, totp_enc, notes_enc, selectors, created_at, updated_at FROM credential;
DROP TABLE credential;
ALTER TABLE credential_new RENAME TO credential;
