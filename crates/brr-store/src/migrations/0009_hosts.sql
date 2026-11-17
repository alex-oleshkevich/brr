CREATE TABLE host (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  docker_url TEXT NOT NULL,
  max_instances INTEGER NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL
);
ALTER TABLE instance ADD COLUMN host_id TEXT REFERENCES host(id) ON DELETE SET NULL;
CREATE INDEX instance_host ON instance(host_id);
