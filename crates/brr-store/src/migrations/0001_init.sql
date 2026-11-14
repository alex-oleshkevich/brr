CREATE TABLE build (id TEXT PRIMARY KEY, source TEXT NOT NULL, engine TEXT NOT NULL, name TEXT NOT NULL, version TEXT NOT NULL,
  channel TEXT, path TEXT NOT NULL, state TEXT NOT NULL, sha256 TEXT, size_bytes INTEGER NOT NULL DEFAULT 0,
  capabilities TEXT NOT NULL, installed_at INTEGER, last_referenced_at INTEGER, UNIQUE(source, version));
CREATE TABLE asset (id TEXT PRIMARY KEY, kind TEXT NOT NULL, version TEXT NOT NULL, path TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE template (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, builtin INTEGER NOT NULL, settings TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE profile (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, template_id TEXT, build_id TEXT, channel TEXT,
  settings TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, notes TEXT NOT NULL DEFAULT '', data_dir TEXT, disk_quota_bytes INTEGER,
  backup_schedule TEXT, backup_keep INTEGER NOT NULL DEFAULT 7, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE profile_tag (profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE, tag TEXT NOT NULL, PRIMARY KEY(profile_id, tag));
CREATE TABLE extension (id TEXT PRIMARY KEY, engine TEXT NOT NULL, source TEXT NOT NULL, source_ref TEXT NOT NULL, name TEXT NOT NULL);
CREATE TABLE extension_version (extension_id TEXT NOT NULL REFERENCES extension(id) ON DELETE CASCADE, version TEXT NOT NULL,
  path TEXT NOT NULL, fetched_at INTEGER NOT NULL, PRIMARY KEY(extension_id, version));
CREATE TABLE profile_extension (profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE,
  extension_id TEXT NOT NULL REFERENCES extension(id) ON DELETE CASCADE, pinned_version TEXT, auto_update INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(profile_id, extension_id));
CREATE TABLE credential (id TEXT PRIMARY KEY, site TEXT NOT NULL, login_url TEXT, username_enc BLOB NOT NULL, password_enc BLOB NOT NULL,
  totp_enc BLOB, notes_enc BLOB, selectors TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE credential_profile (credential_id TEXT NOT NULL REFERENCES credential(id) ON DELETE CASCADE,
  profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE, PRIMARY KEY(credential_id, profile_id));
CREATE TABLE login_check (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE, site TEXT NOT NULL,
  url TEXT NOT NULL, expect_selector TEXT, expect_text TEXT, credential_id TEXT REFERENCES credential(id) ON DELETE SET NULL,
  auto_relogin INTEGER NOT NULL DEFAULT 0, last_status TEXT NOT NULL DEFAULT 'unknown', last_checked_at INTEGER);
CREATE TABLE proxy (id TEXT PRIMARY KEY, scheme TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, username TEXT, password_enc BLOB,
  healthy INTEGER, exit_ip TEXT, country TEXT, latency_ms INTEGER, checked_at INTEGER);
CREATE TABLE proxy_pool (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, mode TEXT NOT NULL);
CREATE TABLE pool_member (pool_id TEXT NOT NULL REFERENCES proxy_pool(id) ON DELETE CASCADE,
  proxy_id TEXT NOT NULL REFERENCES proxy(id) ON DELETE CASCADE, PRIMARY KEY(pool_id, proxy_id));
CREATE TABLE pool_sticky (pool_id TEXT NOT NULL REFERENCES proxy_pool(id) ON DELETE CASCADE,
  profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE, proxy_id TEXT NOT NULL REFERENCES proxy(id) ON DELETE CASCADE,
  PRIMARY KEY(pool_id, profile_id));
CREATE TABLE instance (id TEXT PRIMARY KEY, kind TEXT NOT NULL, profile_id TEXT REFERENCES profile(id) ON DELETE SET NULL,
  build_id TEXT REFERENCES build(id) ON DELETE SET NULL, state TEXT NOT NULL, display TEXT NOT NULL, priority INTEGER NOT NULL,
  admission TEXT NOT NULL, pid INTEGER, log_path TEXT, cdp_url TEXT, data_dir TEXT, ephemeral INTEGER NOT NULL, promote_to_name TEXT,
  restart_count INTEGER NOT NULL DEFAULT 0, last_activity_at INTEGER NOT NULL, queued_at INTEGER NOT NULL, started_at INTEGER,
  ended_at INTEGER, end_reason TEXT, error TEXT);
CREATE INDEX instance_state ON instance(state);
CREATE INDEX instance_profile ON instance(profile_id);
CREATE TABLE tab (id TEXT PRIMARY KEY, instance_id TEXT NOT NULL REFERENCES instance(id) ON DELETE CASCADE, target_id TEXT NOT NULL,
  state TEXT NOT NULL, url TEXT NOT NULL, title TEXT NOT NULL, opened_by_brr INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0,
  last_activity_at INTEGER NOT NULL, created_at INTEGER NOT NULL, closed_at INTEGER);
CREATE INDEX tab_instance ON tab(instance_id);
CREATE TABLE client (id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL, current_tab_id TEXT, created_at INTEGER NOT NULL, ended_at INTEGER);
CREATE TABLE lease (id TEXT PRIMARY KEY, client_id TEXT NOT NULL, client_kind TEXT NOT NULL, client_label TEXT NOT NULL,
  instance_id TEXT NOT NULL, tab_id TEXT, exclusive INTEGER NOT NULL, acquired_at INTEGER NOT NULL, released_at INTEGER, release_reason TEXT);
CREATE UNIQUE INDEX lease_active_tab ON lease(tab_id) WHERE released_at IS NULL AND tab_id IS NOT NULL;
CREATE TABLE artifact (id TEXT PRIMARY KEY, public_id TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, profile_id TEXT, instance_id TEXT, tab_id TEXT,
  mime TEXT NOT NULL, size_bytes INTEGER NOT NULL, backend TEXT NOT NULL, location TEXT NOT NULL, sha256 TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE backup (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profile(id) ON DELETE CASCADE, backend TEXT NOT NULL,
  location TEXT NOT NULL, size_bytes INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE captcha_solve (id TEXT PRIMARY KEY, ts INTEGER NOT NULL, profile_id TEXT, tab_id TEXT, site TEXT NOT NULL, type TEXT NOT NULL,
  provider TEXT NOT NULL, cost_usd_micros INTEGER NOT NULL, duration_ms INTEGER NOT NULL, result TEXT NOT NULL, error TEXT);
CREATE TABLE event (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, kind TEXT NOT NULL, subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL, data TEXT NOT NULL);
CREATE INDEX event_kind ON event(kind);
CREATE INDEX event_subject ON event(subject_type, subject_id);
CREATE INDEX event_ts ON event(ts);
CREATE TABLE secret (name TEXT PRIMARY KEY, value_enc BLOB NOT NULL, updated_at INTEGER NOT NULL);
