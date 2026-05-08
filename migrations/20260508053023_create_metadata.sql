-- Add migration script here
create schema metadata

-- Enable UUID extension if you prefer UUIDs over Serial IDs
-- CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- 1. Artists Table: The root entity
CREATE TABLE metadata.artists (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 2. Albums Table: Belongs to an Artist
CREATE TABLE metadata.albums (
    id BIGSERIAL PRIMARY KEY,
    artist_id BIGINT NOT NULL REFERENCES metadata.artists(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    release_date DATE,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    
    -- Prevents duplicate albums by the same artist
    UNIQUE(artist_id, title)
);

-- 3. Tracks Table: Belongs to an Album
CREATE TABLE metadata.tracks (
    id BIGSERIAL PRIMARY KEY,
    album_id BIGINT NOT NULL REFERENCES metadata.albums(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL CHECK (duration_seconds > 0),
    track_number INTEGER, -- Optional: Order within the album
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Optimization Indexes
CREATE INDEX idx_albums_artist_id ON metadata.albums(artist_id);
CREATE INDEX idx_tracks_album_id ON metadata.tracks(album_id);
CREATE INDEX idx_tracks_title_search ON metadata.tracks USING gin (to_tsvector('english', title));