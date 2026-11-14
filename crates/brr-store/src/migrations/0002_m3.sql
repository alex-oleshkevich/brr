ALTER TABLE extension_version ADD COLUMN size_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE extension_version ADD COLUMN last_referenced_at INTEGER;
CREATE UNIQUE INDEX extension_source_ref ON extension(source, source_ref);
CREATE UNIQUE INDEX proxy_endpoint ON proxy(scheme, host, port, IFNULL(username, ''));
