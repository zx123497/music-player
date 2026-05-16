# Music Streaming Service — Requirements

> **Status of this document.** Rewritten on 2026-05-16 from a direct read of the
> source tree (`src/`, `migrations/`, `Cargo.toml`). Every checkbox is backed by
> code evidence (file + line reference) or explicitly marked as not implemented.
> The previous version of this file was inaccurate in several places — those
> corrections are summarized in [§9 Corrections vs. prior doc](#9-corrections-vs-prior-doc).

---

## 1. Project Overview

**SoundZone** is a self-hosted music streaming backend, written in Rust
(Axum + SQLx + AWS S3 SDK), targeted at a small private deployment (PPLab
home lab).

| Aspect | Decision (confirmed with owner) |
|---|---|
| Deployment | Raspberry Pi (or single Linux box), MinIO for object storage, Postgres on same host |
| Scale target | <100 registered users, ~10–20 concurrent streams, best-effort uptime (no formal SLA) |
| Streaming model | **HLS adaptive bitrate** (segmented `.m3u8` playlists + AAC-LC chunks; client auto-switches quality) |
| Authentication | **OAuth-only** (third-party identity provider; backend issues short-lived session/JWT for its own API) |
| Roles | Two roles: **admin** (uploads, manages catalog) and **listener** (browse, stream, favorite, playlist) |
| Scope | Backend HTTP API only — no UI in this repo |
| Job queue | `underway` (Postgres-backed) — already a dependency in `Cargo.toml`, currently unused |

### 1.1 Recommended transcoding ladder

Because the source format question was left to recommendation, and because the
playback path is HLS, the proposed ladder is:

| Source accepted | Output (HLS variants, AAC-LC in fragmented MP4 / CMAF) | Notes |
|---|---|---|
| FLAC, WAV (preferred — true lossless masters) | 96 kbps · 160 kbps · 256 kbps | Standard 3-rung ladder; covers cellular → home Wi-Fi |
| MP3 (fallback) | 96 kbps · 160 kbps · (source-bitrate cap) | Never claim "lossless" output from a lossy source |

> Future option: also publish an "original" download URL (presigned, admin-only or
> opt-in) for users who want the source file. Out of scope for v1.

### 1.2 Glossary (domain terms used below)

- **HLS (HTTP Live Streaming)** — Apple's segmented streaming protocol. A
  master playlist (`master.m3u8`) lists several *variant* playlists
  (one per bitrate). Each variant playlist points at small media segments
  (typically 2–10 s of audio). The client decides which variant to fetch
  based on measured bandwidth.
- **Transcoding ladder** — the fixed set of (codec, bitrate) outputs
  produced from each source track. Each rung is one HLS variant.
- **Presigned URL** — a short-lived signed S3/MinIO URL that grants read or
  write access without exposing credentials.
- **CMAF / fMP4** — fragmented MP4 container format used by modern HLS;
  preferred over `.ts` for new deployments.

---

## 2. Architecture Snapshot (as built today)

```
                ┌──────────────────────────────────────┐
                │           Axum (src/main.rs)         │
                │  /api/v1/metadata/*    /api/v1/streaming/* │
                └───────────────┬──────────────────────┘
                                │
        ┌───────────────────────┼────────────────────────────┐
        │                       │                            │
        ▼                       ▼                            ▼
 PgPool (max=10)        S3Client (aws-sdk)         in-mem mpsc + Semaphore
 src/state.rs:20        src/state.rs:43-58         src/services/transcode/queue.rs
                                                   (jobs lost on restart)
```

Confirmed wiring in `src/state.rs`:

```16:41:src/state.rs
pub async fn create_app_state(config: Config) -> Arc<AppState> {
    let s3_client = create_s3_client(&config.s3).await;
    ...
    let transcode_thread_pool = ThreadPool::new(
        worker_size,
        s3_client.clone(),
        pg_pool.clone(),
        bucket,
    );
    ...
}
```

---

## Legend

- `[x]` — implemented and verified against source
- `[~]` — partially implemented (details inline)
- `[ ]` — not implemented

---

## 3. Functional Requirements

### 3.1 Upload & Ingest (admin only)

- `[~]` **Generate presigned PUT URL for direct-to-S3 upload** — works, but
  uses a hard-coded bucket name and has no auth/role check.
  - Endpoint: `POST /api/v1/metadata/tracks/presigned-url` → `src/routes/metadata.rs:81-104`
  - Service: `src/services/transcode_services.rs:8-28` (bucket hard-coded as `"soundzone"` on line 23 — should use `state.config.s3.bucket`)
- `[~]` **Confirm upload + create track metadata** — works for happy path; missing
  format validation, missing auth, calls `head_object` twice (route + service).
  - Endpoint: `POST /api/v1/metadata/tracks` → `src/routes/metadata.rs:106-141`
  - Service: `src/services/metadata.rs:35-84` (also hard-codes bucket `"soundzone"` on line 49)
- `[ ]` **Admin role check on upload endpoints** — there is no auth at all yet.
- `[ ]` **Reject non-audio files** (MIME sniff + magic-byte check on the uploaded object).
- `[ ]` **Reject formats outside the accepted source list** (FLAC/WAV/MP3).
- `[ ]` **Extract source metadata** (sample rate, channels, bit depth, codec) — only duration is extracted today (`mp3-duration`, MP3-only).
- `[ ]` **Extract/store embedded cover art** (ID3v2 APIC frame, FLAC `METADATA_BLOCK_PICTURE`).
- `[ ]` **Detect duplicates** (e.g. SHA-256 of source bytes, or audio fingerprint).
- `[ ]` **Support large uploads** — currently capped at **10 MB** by `RequestBodyLimitLayer` in `src/main.rs:21`. Direct-to-S3 presigned PUT bypasses this limit, so the practical cap is whatever the client + S3 allow; document this explicitly.
- `[ ]` **Multipart/resumable upload** for files >100 MB (FLAC/WAV can exceed this).

### 3.2 Transcoding (HLS ladder)

- `[~]` **Background job queue** — exists, but is in-memory only and uses
  `.expect()` everywhere (any S3/DB hiccup crashes the worker task).
  - `src/services/transcode/queue.rs:31-46` (channel + semaphore)
  - `process_transcode_job` at `src/services/transcode/queue.rs:49-101` — every fallible call is `.expect()`.
- `[ ]` **Replace in-memory queue with `underway`** (Postgres-backed; already in `Cargo.toml:20`) so jobs survive restarts and can retry.
- `[ ]` **Actually transcode** — current "transcode" is an S3 copy+delete:
  `src/services/transcode/queue.rs:71-86`. Needs FFmpeg integration (spawn `ffmpeg` subprocess; or `ffmpeg-next` Rust bindings).
- `[ ]` **Produce HLS ladder per track**:
  - `[ ]` AAC-LC @ 96 kbps variant playlist + segments
  - `[ ]` AAC-LC @ 160 kbps variant playlist + segments
  - `[ ]` AAC-LC @ 256 kbps variant playlist + segments
  - `[ ]` Master playlist (`master.m3u8`) referencing all variants
- `[ ]` **Upload HLS outputs to MinIO** under a deterministic prefix (e.g. `tracks/{track_id}/hls/{variant}/...`).
- `[ ]` **Job status state machine**: `pending → transcoding → ready | failed`. Today only `uploaded → transcoding → transcoded` (no `failed`); see `src/services/transcode/queue.rs:55-65`.
- `[ ]` **Retry with backoff** for failed jobs (e.g. 3 attempts, exponential backoff). None today.
- `[ ]` **Visibility/observability** — `GET /api/v1/admin/transcode/jobs` (admin), per-job logs/errors.
- `[ ]` **Preserve original source file** in a separate prefix (e.g. `originals/`) for re-transcoding when the ladder changes; current code deletes the original (`queue.rs:81-86`).
- `[ ]` **Probe source on ingest** (`ffprobe`) and record codec/bitrate/duration/sample-rate in the DB before queueing.

### 3.3 Catalog metadata

- `[x]` **Create artist** — `POST /api/v1/metadata/artist` → `src/routes/metadata.rs:26-47`, service `src/services/metadata.rs:6-12`, repo `src/repositories/metadata.rs:6-21`.
- `[~]` **Create album** — endpoint exists, but the route returns an empty 201 because the service throws away the inserted row.
  - Route returns `serde_json::to_string(&album).unwrap()` on a `()` value — `src/routes/metadata.rs:58-79`.
  - Service: `src/services/metadata.rs:19-25` returns `Result<(), _>`; repo at `src/repositories/metadata.rs:34-46` does not use `RETURNING`.
- `[x]` **Create track** — `POST /api/v1/metadata/tracks` → `src/routes/metadata.rs:106-141`.
- `[ ]` **List all artists** — endpoint *exists* but is **stubbed**: it returns the hard-coded string `"List of artists"` and does not query the DB.
  - `src/routes/metadata.rs:49-56`. The working repo function `get_all_artists` at `src/repositories/metadata.rs:23-32` is never wired up.
- `[ ]` **List albums for an artist** — repo function `get_albums_by_artist` exists (`src/repositories/metadata.rs:48-62`); **no route** uses it.
- `[ ]` **List tracks for an album** — repo function `get_tracks_by_album` exists (`src/repositories/metadata.rs:91-102`); **no route** uses it.
- `[ ]` **Get track details** — repo function `get_track_by_id` exists (`src/repositories/metadata.rs:104-115`) and is used internally by the streaming service, but no public `GET /metadata/tracks/{id}` route exists.
- `[~]` **Album release date** — column exists (`migrations/20260508053023_create_metadata.sql:24`) but no API reads or writes it.
- `[~]` **Track number** — column exists (`migrations/20260508053023_create_metadata.sql:37`) but no API reads or writes it, and there is no `UNIQUE (album_id, track_number)` constraint.
- `[ ]` **Genre tagging** — no schema, no API. (Not listed as required, but commonly expected; documenting as optional.)
- `[ ]` **Update / delete** artist / album / track — no `PATCH` or `DELETE` endpoints anywhere.
- `[ ]` **Cover art per album / per track** — no schema, no API, not extracted.

### 3.4 Authentication & Authorization (OAuth, two roles)

- `[ ]` **OAuth login flow** — choose provider(s) (Google / GitHub / Apple). Backend implements `/auth/{provider}/login` redirect and `/auth/{provider}/callback`.
- `[ ]` **Backend session token** after successful OAuth — short-lived JWT (config exists at `src/config.rs:49-53`, unused) **or** a server-side opaque session token.
- `[ ]` **Refresh tokens** (or sliding sessions) so users aren't bounced back to OAuth daily.
- `[ ]` **Logout** (revoke session / blacklist JWT jti).
- `[ ]` **`users` table** with at minimum: `id`, `oauth_provider`, `oauth_subject`, `email`, `display_name`, `role` (`admin` | `listener`), `created_at`.
- `[ ]` **Role-based middleware** that gates `/metadata/*` write endpoints to `admin` only.
- `[ ]` **Authenticated user extractor** for routes that need `user_id` (favorites, history, playlists, recommendations).
- `[ ]` **Bootstrap first admin** (e.g. config-driven allowlist of admin emails, or CLI `cargo run --bin grant-admin <email>`).

### 3.5 Streaming / Playback (HLS)

- `[~]` **Track streaming URL endpoint** — exists but returns a single presigned URL to the raw uploaded file. Not HLS, no quality selection, no auth, no error if track isn't transcoded.
  - Endpoint: `GET /api/v1/streaming/tracks/{track_id}/presigned-url` → `src/routes/streaming.rs:9-33`
  - Service: `src/services/streaming.rs:5-28`. Note `track.file_path.is_empty()` check at line 11 — if a track is still transcoding the service returns `Err`, but the route then returns a generic 500 (`src/routes/streaming.rs:28-31`).
- `[ ]` **HLS master playlist endpoint** — `GET /api/v1/streaming/tracks/{track_id}/master.m3u8` returns the master playlist with variant URLs.
- `[ ]` **Variant playlist endpoints** — `GET /api/v1/streaming/tracks/{track_id}/{variant}/index.m3u8`.
- `[ ]` **Segment delivery** — either:
  - (a) signed URLs embedded in the variant playlist pointing directly at MinIO, **or**
  - (b) backend proxies bytes from MinIO with Range request support.
  Pick one and document; (a) is cheaper, (b) lets you enforce per-request auth and play counts.
- `[ ]` **Auth on stream URLs** — today anyone with the presigned URL can play. With HLS + presigned segment URLs, gate the playlist endpoint behind the session token; segments inherit security via short presign TTL (e.g. 5 min).
- `[ ]` **Block streaming of non-ready tracks** (return `409 Conflict` or `425 Too Early`, not 500).
- `[ ]` **Range request support** for non-HLS fallback download (S3/MinIO supports `Range` natively if you redirect; explicit only matters for the proxy path).
- `[ ]` **Record a "play"** event when a stream is opened (feeds history + recommendations). Likely a `POST /api/v1/me/history` from the client, or server-side on master-playlist fetch.
- `[ ]` **Resume playback** — store `last_position_seconds` per (user, track) in DB. Endpoints: `PUT /api/v1/me/playback/{track_id}` and `GET /api/v1/me/playback/recent`.

### 3.6 User Library (per-user features)

- `[ ]` **Favorites**
  - `[ ]` `POST /api/v1/me/favorites/tracks/{id}` / `DELETE`
  - `[ ]` `POST /api/v1/me/favorites/albums/{id}` / `DELETE`
  - `[ ]` `POST /api/v1/me/favorites/artists/{id}` / `DELETE`
  - `[ ]` `GET /api/v1/me/favorites/{tracks|albums|artists}` (paginated)
- `[ ]` **Listen history**
  - `[ ]` Record one row per play with `user_id`, `track_id`, `played_at`, `duration_listened_seconds` (for ≥X% counts as a "real" play)
  - `[ ]` `GET /api/v1/me/history` (paginated, recent-first)
  - `[ ]` `GET /api/v1/me/stats/top-tracks` (most-played in last N days)
- `[ ]` **Playlists**
  - `[ ]` `POST /api/v1/me/playlists` (create)
  - `[ ]` `GET /api/v1/me/playlists` (list)
  - `[ ]` `GET /api/v1/playlists/{id}` (read — owner only for v1; public-flag later)
  - `[ ]` `PATCH /api/v1/playlists/{id}` (rename / reorder via `position`)
  - `[ ]` `DELETE /api/v1/playlists/{id}`
  - `[ ]` `POST /api/v1/playlists/{id}/tracks` (add)
  - `[ ]` `DELETE /api/v1/playlists/{id}/tracks/{playlist_track_id}`
- `[ ]` **Resume** — see §3.5.

### 3.7 Search

- `[ ]` **Full-text search across tracks, albums, artists**.
  - DB groundwork: `idx_tracks_title_search` GIN index on `to_tsvector('english', title)` exists (`migrations/20260508053023_create_metadata.sql:44`), but **no equivalent index for albums or artists** and **no endpoint**.
- `[ ]` `GET /api/v1/search?q=<query>&type=<track|album|artist|all>&page=...&page_size=...`
- `[ ]` Pagination (cursor or offset/limit) — none today on any endpoint.
- `[ ]` Sort by relevance / play count / recency.

### 3.8 Recommendations

For ~100 users, full collaborative filtering will be sparse and unhelpful.
Recommended v1 algorithms (cheap, no ML deps):

- `[ ]` `GET /api/v1/recommendations/recently-added` — newest tracks/albums.
- `[ ]` `GET /api/v1/recommendations/most-played` — global top tracks last 30 days.
- `[ ]` `GET /api/v1/recommendations/for-you` — top tracks by artists you've favorited or played frequently, excluding tracks you've already heard recently.
- `[ ]` *(stretch)* "Because you played X" — same artist + same album peers.
- `[ ]` *(stretch)* Item-item co-occurrence using the `listen_history` table (precompute nightly into a small `track_similarity` table).

### 3.9 Health & operational endpoints

- `[ ]` `GET /healthz` — liveness (process is up).
- `[ ]` `GET /readyz` — readiness (DB ping + S3/MinIO ping succeed).
- `[ ]` `GET /api/v1/admin/transcode/jobs` — list/inspect queue (admin).
- `[ ]` `GET /metrics` — Prometheus exposition (optional, but cheap with `axum-prometheus` or similar).

---

## 4. Non-Functional Requirements

> Targets are sized for a Raspberry Pi class deployment with <100 users.

### 4.1 Performance

- `[ ]` p95 API latency < 300 ms for catalog reads (Pi-class CPU; not 200 ms).
- `[ ]` Support **10–20 concurrent HLS streams** without dropping segments.
- `[ ]` Transcoding throughput: at least **2× real-time per track** on the Pi (i.e. a 4-min track finishes its full ladder in <2 min). Tune `transcode.worker_size` to match available CPU cores.
- `[~]` DB connection pool sized — currently fixed at `max_connections(10)` (`src/state.rs:22`). Adequate for this scale; document the choice.
- `[ ]` DB query indexes on every FK + every search/sort column actually used (see §6).
- `[ ]` HLS segment caching headers (`Cache-Control: public, max-age=...`) on segment responses.

### 4.2 Reliability

- `[ ]` Transcoder must not panic on S3/DB errors — replace every `.expect()` in `src/services/transcode/queue.rs` with proper error handling and a status-row update.
- `[ ]` Job retries with backoff (`underway` supports this).
- `[ ]` Graceful shutdown: drain in-flight jobs before exit; current `tokio::spawn` workers will be terminated abruptly.
- `[ ]` Health endpoints (§3.9).
- `[ ]` Restart-safe queue (covered by underway switch).

### 4.3 Security

- `[ ]` **HTTPS / TLS terminated in front** (nginx / Caddy / traefik). Backend itself stays HTTP.
- `[ ]` OAuth flow (§3.4). No passwords stored.
- `[ ]` JWT signed with secret from `config.jwt.secret`; respect `expiration_seconds`.
- `[ ]` Role middleware on admin endpoints.
- `[~]` SQL injection — currently safe by virtue of `sqlx::query_as` with bound parameters everywhere (`src/repositories/metadata.rs`). Keep this discipline; ban string concatenation.
- `[ ]` **Input validation** on all request bodies (e.g. `validator` crate). Currently `CreateArtistRequest` etc. accept any string of any length.
- `[ ]` **Tighten CORS** — `src/main.rs:20` uses `CorsLayer::permissive()`. Restrict to known origins in production.
- `[ ]` **Rate limiting** per IP and per user (especially on auth and presigned-URL endpoints).
- `[ ]` File-type validation on uploads (§3.1).
- `[ ]` Short-lived (≤5 min) presigned URLs for segments; longer (≤1 h) for downloads only if downloads are a feature.
- `[ ]` Secrets in env / config file outside the repo (`.env` and `config.toml` already in `.gitignore`).
- `[ ]` Audit log for admin actions (uploads, deletes, role changes).

### 4.4 Scalability (right-sized)

- `[x]` Stateless API design — session state lives in DB/JWT, not in memory. (Verified: no per-process user state in `AppState`.)
- `[ ]` Horizontal scaling possible (currently the in-memory queue is per-process; switching to `underway` enables N backend instances).
- `[ ]` Background workers separable from API process (after underway switch, run `bin/transcode-worker` separately).
- `[ ]` No CDN required at this scale; rely on browser caching + MinIO range support.

### 4.5 Observability & maintainability

- `[ ]` **Structured logging** with `tracing` + `tracing-subscriber`. **The `tracing` crate is not currently a dependency** — `TraceLayer::new_for_http()` (`src/main.rs:23`) compiles but emits nothing because no subscriber is installed. All current logging is `println!`/`eprintln!` (e.g. `src/routes/metadata.rs:30, 31, 43, 50, 62, 63, 75, 85, 100`, `src/routes/streaming.rs:20, 21, 29`). Replace.
- `[ ]` Error type — adopt `thiserror` for domain errors and `anyhow` (or `eyre`) for top-level. Today many functions return `Box<dyn std::error::Error>` (e.g. `src/services/metadata.rs:42`, `src/services/streaming.rs:5-8`) which loses type info.
- `[ ]` Centralized error → HTTP-status mapping (avoid the current pattern of mapping every error to `500` in each handler).
- `[ ]` OpenAPI / Swagger spec (e.g. `utoipa`).
- `[ ]` `README.md` expanded beyond the current single-line title.
- `[ ]` Code comments only where intent is non-obvious (avoid narrating).
- `[~]` DB migrations: framework in place (`sqlx migrate`, three files in `migrations/`). The first migration (`20260508053023_create_metadata.sql:6-7`) does `DROP TABLE IF EXISTS` of all tables — destructive; remove for production migrations or gate behind a "dev_reset" file.

### 4.6 Data integrity

- `[x]` `ON DELETE CASCADE` from `artists → albums → tracks` (`migrations/20260508053023_create_metadata.sql:22, 34`).
- `[x]` `UNIQUE(artist_id, title)` on albums prevents dup albums per artist (`migrations/20260508053023_create_metadata.sql:28`).
- `[x]` `UNIQUE` on `artists.name` (`migrations/20260508053023_create_metadata.sql:15`).
- `[ ]` Treat `track.file_path` consistently. Today it's stored as empty string at create-time (`src/services/metadata.rs:66`), then overwritten by the worker (`src/services/transcode/queue.rs:88-97`). This works but is brittle — prefer making it nullable, or split into `source_path` + `hls_master_path`.
- `[ ]` Orphaned-object cleanup: if `tracks` row deleted, delete corresponding HLS prefix from MinIO.
- `[ ]` Duplicate-upload detection (track-level checksum).
- `[ ]` Backup strategy for both Postgres (`pg_dump` cron) and MinIO bucket (`mc mirror` to a second disk).

### 4.7 Deployment

- `[ ]` Dockerfile for the backend (multi-stage; final image based on `debian:slim` with `ffmpeg` installed).
- `[ ]` `docker-compose.yml` bringing up Postgres + MinIO + backend together.
- `[ ]` `Makefile` targets (the file exists but is empty — `Makefile`). At minimum: `make run`, `make migrate`, `make test`, `make lint`.
- `[ ]` Documented configuration: example `config.toml.example` checked into the repo (real `config.toml` is `.gitignore`d).
- `[ ]` Migrations run automatically on container start (or a documented one-shot step).
- `[ ]` CI: at minimum `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

### 4.8 Testing

- `[ ]` Unit tests on services and repositories (none today — `cargo test` will compile zero test binaries).
- `[ ]` Integration tests using `axum::Router` + `sqlx::test` + a test Postgres + `testcontainers` (or local MinIO).
- `[ ]` A smoke test for the full flow: presign → upload → create track → wait for transcode → fetch master playlist → fetch a segment.

---

## 5. Database — Required Schema Work

### 5.1 Present today (from `migrations/`)

- `[x]` `metadata.artists (id, name UNIQUE, created_at)`
- `[x]` `metadata.albums (id, artist_id FK, title, release_date, created_at, UNIQUE(artist_id, title))`
- `[x]` `metadata.tracks (id, album_id FK, title, duration_seconds, track_number, created_at, artist_id FK, file_path, upload_id, status)`
- `[x]` Indexes: `idx_albums_artist_id`, `idx_tracks_album_id`, `idx_tracks_title_search` (GIN)

### 5.2 To add

- `[ ]` `auth.users (id, oauth_provider, oauth_subject, email, display_name, role, created_at)` + `UNIQUE (oauth_provider, oauth_subject)`
- `[ ]` `auth.sessions` (or rely entirely on JWT — pick one)
- `[ ]` `library.favorite_tracks (user_id, track_id, created_at, PRIMARY KEY(user_id, track_id))` and analogous `favorite_albums`, `favorite_artists`
- `[ ]` `library.listen_history (id, user_id, track_id, played_at, duration_listened_seconds)` + index on `(user_id, played_at DESC)` and `(track_id, played_at DESC)`
- `[ ]` `library.playback_positions (user_id, track_id, position_seconds, updated_at, PRIMARY KEY(user_id, track_id))`
- `[ ]` `library.playlists (id, user_id, name, created_at, updated_at)`
- `[ ]` `library.playlist_tracks (id, playlist_id, track_id, position, added_at, UNIQUE(playlist_id, position))`
- `[ ]` `transcode.outputs (id, track_id, variant, codec, bitrate_kbps, container, hls_playlist_key, byte_size, created_at)` — one row per ladder rung; `UNIQUE(track_id, variant)`
- `[ ]` Add GIN full-text indexes on `albums.title` and `artists.name` for §3.7.
- `[ ]` Add `tracks.source_key` (the `originals/` location) and `tracks.hls_master_key` (nullable until ready) — clearer than the current `file_path`.
- `[ ]` Make first migration non-destructive (remove `DROP TABLE` statements, or split dev-seed from schema).

---

## 6. Bugs / Inaccuracies in current code

These are concrete defects found while writing this document. They are listed
so they don't get lost when planning Phase 1.

1. **`GET /metadata/artists` is a stub.** Returns the literal string `"List of artists"` instead of querying the DB. The repo function it should call (`get_all_artists`) exists and is unused. — `src/routes/metadata.rs:49-56`.
2. **`POST /metadata/album` returns empty body.** Service signature is `Result<(), _>`; route then serializes `()` into JSON. The repo `INSERT` does not `RETURNING`. — `src/routes/metadata.rs:58-79`, `src/services/metadata.rs:19-25`, `src/repositories/metadata.rs:34-46`.
3. **Bucket name hard-coded as `"soundzone"`** in two places, ignoring `config.s3.bucket`:
   - `src/services/transcode_services.rs:23`
   - `src/services/metadata.rs:49`
4. **Misleading comment in `get_mp3_duration`.** Says "Download first 256KB" but actually downloads the entire S3 object (`src/services/transcode_services.rs:34-43`). For long FLAC/WAV that becomes seriously wasteful.
5. **Transcoder worker panics on any error.** Every `.expect()` in `src/services/transcode/queue.rs:55-100` will crash the spawned task with no DB status update; the job is silently lost.
6. **Track status never reaches a terminal `failed` state.** Only `uploaded → transcoding → transcoded`. No way for a client to see a failed transcode.
7. **`tracing` crate is missing from dependencies.** `TraceLayer::new_for_http()` (`src/main.rs:23`) installs the middleware but no subscriber, so HTTP traces are dropped.
8. **`CorsLayer::permissive()` in production** (`src/main.rs:20`) — fine in dev, must be tightened.
9. **Duplicate `head_object` call** on the create-track path: once in `src/routes/metadata.rs:113-121` and again in `src/services/metadata.rs:45-57` (the latter against the wrong, hard-coded bucket).
10. **Streaming endpoint returns 500 for tracks that aren't transcoded yet.** Should be `409 Conflict` / `425 Too Early` with a structured error body. — `src/services/streaming.rs:11-13` then `src/routes/streaming.rs:28-31`.
11. **`Track.duration_seconds` is named `duration_ms` inside `create_track`** (`src/repositories/metadata.rs:70`) — just a misleading parameter name, the values are seconds. Confusing for future maintainers.
12. **`underway` and `mp3-duration` dependencies are declared but underused / soon-to-be-replaced.** Clean up `Cargo.toml` once FFmpeg + underway are wired in.
13. **First migration is destructive** (`DROP TABLE IF EXISTS` on every table) — running it against an existing prod DB would wipe data.

---

## 7. Recommended Implementation Phases

Sized to be reviewable in small PRs.

**Phase 0 — Hygiene (no new features).**
Fix bugs #1–#3, #5, #7, #9 above. Add `tracing` + `tracing-subscriber` and
replace `println!`/`eprintln!`. Add a `Dockerfile`, a `docker-compose.yml`
(Postgres + MinIO + app), and a `config.toml.example`. Add `cargo fmt` + `cargo
clippy` + a trivial integration test that boots the app and hits `/healthz`.

**Phase 1 — Auth + Roles.**
OAuth flow, `users` table, role middleware, `/me` endpoint. Gate all
`/metadata/*` write endpoints behind the `admin` role. Wire JWT issuance using
existing `JwtConfig`.

**Phase 2 — Catalog completion.**
Implement the missing read endpoints (`GET /metadata/artists`, list albums by
artist, list tracks by album, get track). Add pagination. Add release-date /
track-number to the create/update payloads. Validate inputs.

**Phase 3 — Real transcoding + durable queue.**
Switch the queue to `underway`. Integrate FFmpeg subprocess. Produce HLS
ladder (3 AAC-LC variants) and master playlist. Store rungs in
`transcode.outputs`. Preserve the original source. Add retry + a terminal
`failed` state.

**Phase 4 — HLS playback API.**
Master playlist endpoint, variant playlist endpoint, segment delivery (start
with presigned MinIO URLs in the playlist). Auth on playlist fetch. Return
`409` for not-ready tracks.

**Phase 5 — User library.**
Favorites, listen history, playback positions, playlists. Indexes per §5.2.

**Phase 6 — Search + recommendations.**
Search endpoint backed by GIN indexes across tracks/albums/artists.
Recommendations endpoints (recently-added, most-played, for-you).

**Phase 7 — Operational polish.**
Rate limiting, tightened CORS, audit log, backups, OpenAPI spec, Prometheus
`/metrics`, runbook docs.

---

## 8. Out of Scope (v1 — explicitly deferred)

These were either dropped during the requirements interview or never raised.
Listed here so they're not accidentally rebuilt later:

- Lyrics storage and display.
- Album art / per-track cover image (recommended to revisit; many clients expect it).
- Genre tagging and browse-by-genre.
- User-uploaded content from non-admin listeners.
- Public/shareable playlists.
- DRM, watermarking, geo-blocking.
- Native mobile clients.
- A frontend UI (React/Svelte/etc.).
- CDN / edge caching.
- Collaborative-filtering ML for recommendations (too sparse at <100 users).
- Multi-tenancy / multi-library separation.
- Email-based / password-based auth (OAuth-only was chosen).

---

## 9. Corrections vs. prior doc

The previous `REQUIREMENTS.md` claimed several items were `[x]` complete that
aren't, and listed assumptions that no longer match this round of requirements
gathering. Corrected:

- ❌ "Upload MP3 files via presigned URLs" was marked complete; actually
  partial — works but hard-codes the bucket, no auth, no format validation.
- ❌ "Create/store Artist/Album/Track info" was marked complete; **the
  list-artists endpoint is a stub** and **create-album returns an empty body**.
- ❌ Streaming model was described as "presigned URLs with quality selection";
  now confirmed as **HLS adaptive bitrate** with no quality selection in the
  URL.
- ❌ Multi-format output table (MP3 128/192/320, FLAC, AAC 128, WAV) replaced
  with a single AAC-LC HLS ladder at 96/160/256 kbps.
- ❌ Email/password + JWT auth replaced with **OAuth-only**.
- ❌ "Support concurrent streaming of 50+ users" / "99% uptime SLA" replaced
  with the right-sized targets in §4.1.
- ❌ Redis as a required caching layer dropped — not needed at this scale; the
  `redis` config section in `src/config.rs:29-32` should be removed or marked
  optional.
- ❌ Old "CLARIFICATIONS CONFIRMED WITH USER" section listed FLAC/AAC/WAV as
  output formats; that conflated source vs. delivery. Now clarified in §1.1.
