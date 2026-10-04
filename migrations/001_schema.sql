-- Rust str::trim's Unicode White_Space set, with PostgreSQL code-point lengths.
CREATE FUNCTION archive_trim(value TEXT) RETURNS TEXT LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE
AS $$ SELECT btrim(value, U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') $$;
CREATE TABLE people (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL CHECK(name = archive_trim(name) AND char_length(name) BETWEEN 1 AND 200), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE UNIQUE INDEX people_name_unique ON people(lower(name));
CREATE TABLE franchises (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL CHECK(name = archive_trim(name) AND char_length(name) BETWEEN 1 AND 200), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE UNIQUE INDEX franchises_name_unique ON franchises(lower(name));
CREATE TABLE tags (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT NOT NULL CHECK(name = archive_trim(name) AND char_length(name) BETWEEN 1 AND 80), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE UNIQUE INDEX tags_name_unique ON tags(lower(name));
CREATE TABLE association_types (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, key TEXT NOT NULL UNIQUE CHECK(char_length(key) BETWEEN 1 AND 64 AND key ~ '^[a-z][a-z0-9]*(_[a-z0-9]+)*$'), label TEXT NOT NULL CHECK(label = archive_trim(label) AND char_length(label) BETWEEN 1 AND 200), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE characters (id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, franchise_id BIGINT NOT NULL REFERENCES franchises(id) ON DELETE RESTRICT, name TEXT NOT NULL CHECK(name = archive_trim(name) AND char_length(name) BETWEEN 1 AND 200), description TEXT CHECK(description IS NULL OR (char_length(description) BETWEEN 1 AND 10000 AND archive_trim(description) <> '')), created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE UNIQUE INDEX characters_name_unique ON characters(franchise_id, lower(name));
CREATE TABLE person_character_associations (person_id BIGINT NOT NULL REFERENCES people(id) ON DELETE RESTRICT, character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE, association_type_id BIGINT NOT NULL REFERENCES association_types(id) ON DELETE RESTRICT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(person_id, character_id, association_type_id));
CREATE INDEX associations_character ON person_character_associations(character_id);
CREATE INDEX associations_type ON person_character_associations(association_type_id);
CREATE TABLE character_tags (character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE, tag_id BIGINT NOT NULL REFERENCES tags(id) ON DELETE CASCADE, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(character_id, tag_id));
CREATE INDEX character_tags_tag ON character_tags(tag_id);
