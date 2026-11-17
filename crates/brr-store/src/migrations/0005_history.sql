-- What a managed Instance ran with, kept after its Profile or Template changes or goes away.
ALTER TABLE instance ADD COLUMN settings TEXT;
ALTER TABLE instance ADD COLUMN template_id TEXT;
ALTER TABLE instance ADD COLUMN template_name TEXT;
ALTER TABLE instance ADD COLUMN extensions TEXT;
ALTER TABLE instance ADD COLUMN proxy_id TEXT;
ALTER TABLE profile ADD COLUMN template_name TEXT;
UPDATE profile SET template_name = (SELECT name FROM template WHERE template.id = profile.template_id);
CREATE TABLE template_extension (template_id TEXT NOT NULL REFERENCES template(id) ON DELETE CASCADE,
  extension_id TEXT NOT NULL REFERENCES extension(id) ON DELETE CASCADE, pinned_version TEXT, auto_update INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(template_id, extension_id));
