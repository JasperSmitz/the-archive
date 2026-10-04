CREATE TABLE images (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    storage_key TEXT NOT NULL UNIQUE
        CHECK (storage_key ~ '^[0-9a-f]{32}\.(jpg|png|webp)$'),
    original_filename TEXT NOT NULL
        CHECK (original_filename = archive_trim(original_filename)
            AND char_length(original_filename) BETWEEN 1 AND 255
            AND original_filename !~ '[[:cntrl:]/\\]'),
    content_type TEXT NOT NULL CHECK (content_type IN ('image/jpeg', 'image/png', 'image/webp')),
    byte_size BIGINT NOT NULL CHECK (byte_size BETWEEN 1 AND 20971520),
    sha256 BYTEA NOT NULL CONSTRAINT images_sha256_unique UNIQUE CHECK (octet_length(sha256) = 32),
    width INTEGER NOT NULL CHECK (width BETWEEN 1 AND 12000),
    height INTEGER NOT NULL CHECK (height BETWEEN 1 AND 12000),
    uploaded_by_person_id BIGINT NOT NULL REFERENCES people(id) ON DELETE RESTRICT,
    source_url TEXT CHECK (source_url IS NULL OR (
        source_url = archive_trim(source_url) AND char_length(source_url) BETWEEN 1 AND 2048
        AND source_url ~* '^https?://[^/@[:space:]]+([/?#]|$)')),
    artist TEXT CHECK (artist IS NULL OR (
        artist = archive_trim(artist) AND char_length(artist) BETWEEN 1 AND 200)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (width::bigint * height::bigint <= 40000000)
);
CREATE INDEX images_uploader ON images(uploaded_by_person_id);
CREATE INDEX images_gallery ON images(created_at DESC, id DESC);

CREATE TABLE image_characters (
    image_id BIGINT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (image_id, character_id)
);
CREATE INDEX image_characters_character ON image_characters(character_id, image_id);
