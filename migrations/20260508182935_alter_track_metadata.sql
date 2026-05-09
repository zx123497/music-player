-- Add migration script here
ALTER TABLE metadata.tracks
ADD COLUMN IF NOT EXISTS artist_id BIGINT NOT NULL REFERENCES metadata.artists(id) ON DELETE CASCADE,
ADD COLUMN IF NOT EXISTS file_path TEXT NOT NULL;