-- Add migration script here
ALTER TABLE metadata.tracks
ADD COLUMN IF NOT EXISTS upload_id UUID NOT NULL,
ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'uploaded';