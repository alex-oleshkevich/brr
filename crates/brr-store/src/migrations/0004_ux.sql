ALTER TABLE proxy ADD COLUMN last_error TEXT;
ALTER TABLE instance ADD COLUMN profile_name TEXT;
UPDATE instance SET profile_name = (SELECT name FROM profile WHERE profile.id = instance.profile_id);
ALTER TABLE template ADD COLUMN description TEXT;
-- A backup outlives its Profile: no foreign key, and the Profile's name is kept with it.
CREATE TABLE backup_new (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, profile_name TEXT NOT NULL, backend TEXT NOT NULL,
  location TEXT NOT NULL, size_bytes INTEGER NOT NULL, created_at INTEGER NOT NULL);
INSERT INTO backup_new SELECT backup.id, backup.profile_id, IFNULL(profile.name, backup.profile_id), backup.backend, backup.location,
  backup.size_bytes, backup.created_at FROM backup LEFT JOIN profile ON profile.id = backup.profile_id;
DROP TABLE backup;
ALTER TABLE backup_new RENAME TO backup;
ALTER TABLE event ADD COLUMN actor_client_id TEXT;
