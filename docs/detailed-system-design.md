# SoundZone — Detailed System Design

> **Status.** Drafted 2026-05-16. Source of truth: [`docs/REQUIREMENTS.md`](./REQUIREMENTS.md)
> (rewritten the same day from the current code under `src/`). This document
> defines the **target** module layout and contracts for the backend; it is not a
> description of the code as it exists today. Where the current code differs,
> [§11 Migration from current code](#11-migration-from-current-code) maps the gap.
>
> Confirmed design decisions (recorded so the rest of the doc reads as a single
> coherent choice):
>
> | Decision | Value |
> |---|---|
> | Process topology | **Single binary** runs API and transcode worker in the same process (a CLI flag could be added later if needed). |
> | Session model | **Stateless JWT only** — short access token + refresh token, no server-side session table. |
> | OAuth / OIDC | **Keycloak** as the only provider for v1; verifier sits behind a trait so a second IdP can be added later. |
> | HLS segment delivery | **Hybrid**: presigned MinIO URLs are the default; an in-process byte-proxy is available behind a feature flag for cases that need per-segment auth or play-count accuracy. |
> | Read-path caching | **HTTP-level only** — `Cache-Control` + `ETag` on catalog and playlist responses. No in-process or Redis cache. |
> | FFmpeg integration | **Subprocess** (`ffmpeg` / `ffprobe` invoked via `tokio::process::Command`), called through a thin `Transcoder` trait so the queue handler is unit-testable without an ffmpeg binary. |

---

## Table of contents

1. [Design goals and non-goals](#1-design-goals-and-non-goals)
2. [Architecture at a glance](#2-architecture-at-a-glance)
3. [Module map](#3-module-map)
4. [Dependency rules](#4-dependency-rules)
5. [Cross-cutting foundation modules](#5-cross-cutting-foundation-modules)
   - 5.1 `config` · 5.2 `error` · 5.3 `telemetry` · 5.4 `http`
   - 5.5 `db` · 5.6 `storage` · 5.7 `auth` · 5.8 `jobs`
6. [Domain modules](#6-domain-modules)
   - 6.1 `catalog` · 6.2 `ingest` · 6.3 `transcode` · 6.4 `streaming`
   - 6.5 `library` · 6.6 `search` · 6.7 `recommendations` · 6.8 `admin` · 6.9 `ops`
7. [Consolidated database schema](#7-consolidated-database-schema)
8. [HTTP API surface (consolidated)](#8-http-api-surface-consolidated)
9. [Process topology, configuration, deployment](#9-process-topology-configuration-deployment)
10. [Testing strategy](#10-testing-strategy)
11. [Migration from current code](#11-migration-from-current-code)
12. [Open questions / risks](#12-open-questions--risks)

---

## 1. Design goals and non-goals

### 1.1 Goals (derived from the requirements)

- **Decoupled modules.** Every domain capability (catalog, ingest, transcode,
  streaming, library, search, recommendations) is a self-contained module that
  exposes a narrow public surface and depends only on foundation modules and
  on other domains via explicit interfaces. No module reaches into another
  module's tables.
- **Independently testable.** Every module is testable without booting the
  whole app. Side-effecting collaborators (Postgres, S3/MinIO, OIDC, ffmpeg,
  the job queue, the clock) sit behind traits with both a production and a
  fake implementation.
- **Right-sized for the deployment.** Target hardware is a Raspberry Pi (or
  one Linux box) running Postgres + MinIO + the binary. No Redis, no CDN, no
  Kafka, no service mesh.
- **Restart-safe background work.** Transcode jobs survive process restarts
  (Postgres-backed `underway` queue) and reach a terminal `failed` state on
  unrecoverable errors.
- **Auth as a cross-cutting concern.** A single auth extractor produces a
  typed `AuthenticatedUser` that every domain handler can require. Role gating
  (`admin` vs `listener`) is done via a typed guard, not by ad-hoc string
  checks.
- **Industrial-standard error handling.** `thiserror` for typed domain errors,
  a single `AppError → axum::Response` mapping, no `.unwrap()` /
  `.expect()` on the request path or in the worker.

### 1.2 Non-goals (v1)

- No frontend, no CDN, no DRM, no native mobile clients, no public sharing,
  no genre tagging, no album/track art, no lyrics, no email/password auth, no
  collaborative-filtering ML. See [§8 of `REQUIREMENTS.md`](./REQUIREMENTS.md#8-out-of-scope-v1--explicitly-deferred).
- No horizontal sharding. We design statelessly so a second instance is
  *possible* once `underway` lands, but multi-node is not a v1 target.

---

## 2. Architecture at a glance

```
                          ┌─────────────────────────────────┐
   HTTPS (TLS-terminated  │           nginx / Caddy         │
   by reverse proxy)      └────────────────┬────────────────┘
                                           │ HTTP
                          ┌────────────────▼────────────────┐
                          │           bin/api               │
                          │  Axum router + middleware       │
                          │                                 │
                          │  routes ─► services ─► repos    │
                          │     │         │         │       │
                          │     │         │         ▼       │
                          │     │         │     Postgres ───┼──► sqlx pool
                          │     │         ▼                 │
                          │     │     Object store (S3) ────┼──► aws-sdk-s3
                          │     │         │                 │       (MinIO)
                          │     ▼         ▼                 │
                          │  OIDC verifier (JWK cache) ─────┼──► Keycloak (JWKS)
                          │                                 │
                          │  underway worker (same process) │
                          │     │                           │
                          │     ▼                           │
                          │  Transcoder trait ──► ffmpeg ───┼──► subprocess
                          └─────────────────────────────────┘
                                           │
                                           ▼
                                     pg_dump / mc mirror
                                     (backup, out of band)
```

Per [§decisions](#soundzone--detailed-system-design) the API HTTP server and
the underway worker run in **one** OS process. The worker uses Postgres for
its queue, so a future split into `bin/api` and `bin/transcode-worker` is a
deployment change with no code rewrites required.

---

## 3. Module map

A flat list of modules and the single responsibility each owns. Foundation
modules have no domain knowledge; domain modules depend on foundation
modules and on each other only via the explicit interfaces called out below.

| # | Module | Layer | Owns (data / behavior) |
|---|---|---|---|
| 5.1 | `config` | foundation | Typed config tree, env + file merge, validation. |
| 5.2 | `error` | foundation | `AppError`, `Result<T>` alias, `IntoResponse` mapping, error codes. |
| 5.3 | `telemetry` | foundation | `tracing` subscriber init, request-ID middleware, log fields. |
| 5.4 | `http` | foundation | Axum app builder, middleware stack, router composition, OpenAPI mount. |
| 5.5 | `db` | foundation | `PgPool` construction, migration runner, `Tx` newtype, retry helpers. |
| 5.6 | `storage` | foundation | `ObjectStore` trait, S3 implementation, presigned URL helpers, byte-range reader. |
| 5.7 | `auth` | foundation | `OidcVerifier` trait + Keycloak impl, JWT verification, `AuthenticatedUser` extractor, `RequireRole` guard. |
| 5.8 | `jobs` | foundation | `JobQueue` trait + `underway` adapter, job spec types, worker bootstrap. |
| 6.1 | `catalog` | domain | Artists, albums, tracks read/write (excluding ingest-side fields). |
| 6.2 | `ingest` | domain | Upload sessions, presigned PUT, format validation, source probing, dedupe. |
| 6.3 | `transcode` | domain | Transcode job state machine, ffmpeg orchestration, HLS ladder output, `transcode.outputs` table. |
| 6.4 | `streaming` | domain | HLS master + variant playlist endpoints, segment delivery (presigned or proxied). |
| 6.5 | `library` | domain | Favorites, listen history, playback positions, playlists. |
| 6.6 | `search` | domain | Full-text search over catalog (`tsvector` queries, ranking). |
| 6.7 | `recommendations` | domain | `recently-added`, `most-played`, `for-you` endpoints. |
| 6.8 | `admin` | domain | Admin-only endpoints (transcode job inspection, audit log read). |
| 6.9 | `ops` | domain | `/healthz`, `/readyz`, `/metrics`. |

### 3.1 Source layout

A single Cargo crate, single binary, organized strictly by module. Each domain
module is a *directory module* with the same internal shape; this is the
convention every module follows so that adding a new one is mechanical.

```
src/
├── main.rs                       # bin entrypoint: parse args, init telemetry, build AppState, serve
├── lib.rs                        # pub use of public surface; #[cfg(test)] test helpers
│
├── config/                       # 5.1
│   ├── mod.rs                    # `Config`, `Config::load`
│   └── sections.rs               # `ServerConfig`, `DatabaseConfig`, ... (typed)
│
├── error.rs                      # 5.2  `AppError`, `Result`, IntoResponse
├── telemetry.rs                  # 5.3  init_tracing, request_id middleware
├── http/                         # 5.4
│   ├── mod.rs                    # `build_router`, middleware stack
│   ├── middleware.rs             # request_id, auth, error-mapping, cors
│   └── openapi.rs                # utoipa doc + Swagger UI mount
│
├── db/                           # 5.5
│   ├── mod.rs                    # `PgPool`, `connect`, `Tx`
│   └── migrate.rs                # `run_migrations`
│
├── storage/                      # 5.6
│   ├── mod.rs                    # `ObjectStore` trait, `Presigner`, error types
│   ├── s3.rs                     # `S3ObjectStore`
│   └── fake.rs                   # `#[cfg(test)] InMemoryObjectStore`
│
├── auth/                         # 5.7
│   ├── mod.rs                    # `AuthenticatedUser`, `Role`, extractor, guard
│   ├── oidc.rs                   # `OidcVerifier` trait, Keycloak impl, JWKS cache
│   ├── jwt.rs                    # token verify + claims model
│   └── users.rs                  # `UserRepo`, upsert-on-login
│
├── jobs/                         # 5.8
│   ├── mod.rs                    # `JobQueue` trait, `JobSpec`, retry policy
│   ├── underway_adapter.rs       # production impl
│   ├── inproc.rs                 # `#[cfg(test)] InMemoryJobQueue`
│   └── worker.rs                 # bootstrap the worker tasks
│
├── catalog/                      # 6.1
│   ├── mod.rs                    # re-exports
│   ├── domain.rs                 # `Artist`, `Album`, `Track`, `TrackStatus`
│   ├── repo.rs                   # `CatalogRepo` trait + Postgres impl
│   ├── service.rs                # `CatalogService` (pure, no axum)
│   └── routes.rs                 # axum handlers; depend only on `CatalogService`
│
├── ingest/                       # 6.2  (same shape: domain.rs / repo.rs / service.rs / routes.rs)
├── transcode/                    # 6.3
│   ├── mod.rs
│   ├── domain.rs                 # `TranscodeJobSpec`, `TranscodeStatus`, `Variant`, `Ladder`
│   ├── repo.rs                   # `TranscodeRepo` (status updates, outputs table)
│   ├── ffmpeg.rs                 # `Transcoder` trait + ffmpeg impl + fake impl
│   ├── worker.rs                 # job handler (registered with `jobs::JobQueue`)
│   └── service.rs                # enqueue + read job status
│
├── streaming/                    # 6.4
│   ├── domain.rs                 # `MasterPlaylist`, `VariantPlaylist`, `Segment`
│   ├── playlist.rs               # build playlists from `transcode.outputs`
│   ├── delivery.rs               # `SegmentDelivery` trait: Presigned | Proxied
│   ├── service.rs
│   └── routes.rs
│
├── library/                      # 6.5
│   ├── favorites/  history/  playback/  playlists/   (each its own sub-module)
│   ├── service.rs
│   └── routes.rs
│
├── search/                       # 6.6
├── recommendations/              # 6.7
├── admin/                        # 6.8
└── ops/                          # 6.9

migrations/                       # one .sql per migration; one numbered group per module
tests/                            # integration tests (axum + sqlx::test + testcontainers)
```

The internal triple `domain.rs / repo.rs / service.rs / routes.rs` is the
mandated convention for **every** domain module:

- `domain.rs` — data types and value objects. No I/O.
- `repo.rs` — DB queries behind a trait, with a Postgres implementation. No
  business rules.
- `service.rs` — business logic. Depends on the *traits* from `repo.rs`,
  `storage`, `auth`, `jobs`. Has no `axum::*` imports.
- `routes.rs` — axum handlers. Owns the HTTP shape; converts DTOs to/from the
  service's domain types; maps `AppError → Response` (via the foundation).

This is the key decoupling rule: **services do not import axum, repos do not
import services, domain does not import either.** That is what makes
per-module testing trivial (you instantiate the service with fake
repos/storage/clock and call methods directly).

---

## 4. Dependency rules

Allowed dependency directions, top-to-bottom (a module may depend on
*anything below it* on this list, never above):

```
            routes/<domain>
                  │
                  ▼
            service/<domain>
                  │
       ┌──────────┼──────────┐
       ▼          ▼          ▼
     repo/    storage/     jobs/         ← these are foundation traits
       │          │          │
       └────►   db/   ◄──────┘
                                          auth/, error/, telemetry/, config/, http/
                                          are leaf foundations — anything may use them
```

Forbidden:

- Domain `A.service` calling domain `B.repo` directly. Cross-domain reads go
  through `B.service` (or through a stable public read trait that `B`
  exposes — see §6.4 `streaming` for an example consuming
  `transcode::OutputsReader`).
- Any `repo.rs` taking an `axum::*` type as an argument.
- Any `service.rs` importing `axum`, `tower`, or any HTTP type.
- Any module accessing another module's tables (enforced by code review and by
  using Postgres schemas — see §7).

---

## 5. Cross-cutting foundation modules

### 5.1 `config`

**Responsibility.** Load typed configuration from `config.toml` and environment
variables, validate it, expose it as an `Arc<Config>`.

**Public surface.**

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub s3: S3Config,
    pub jwt: JwtConfig,        // signing key, ttl
    pub oidc: OidcConfig,      // Keycloak issuer, audience, jwks_url
    pub transcode: TranscodeConfig,
    pub streaming: StreamingConfig,
    pub limits: LimitsConfig,  // body sizes, presign TTLs, rate limits
    pub telemetry: TelemetryConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError>;
    pub fn from_env() -> Result<Self, ConfigError>;     // for tests
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid configuration: {0}")] Invalid(String),
    #[error("missing required field: {0}")] Missing(&'static str),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}
```

**Key sub-types.**

```rust
pub struct OidcConfig {
    pub issuer_url: String,        // e.g. https://kc.example.com/realms/soundzone
    pub audience: String,          // client id
    pub jwks_cache_ttl_secs: u64,  // how long to cache JWKS
    pub admin_email_allowlist: Vec<String>,  // bootstrap first admin
}

pub struct StreamingConfig {
    pub segment_presign_ttl_secs: u64,  // ≤300 (5 min)
    pub default_delivery: DeliveryMode,  // Presigned | Proxied
    pub enable_proxy_fallback: bool,
}

pub struct TranscodeConfig {
    pub worker_size: usize,           // parallelism
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub ladder: Vec<LadderRung>,      // pre-validated list of (variant_name, codec, bitrate_kbps)
    pub max_attempts: u32,
    pub originals_prefix: String,     // e.g. "originals/"
    pub hls_prefix: String,           // e.g. "hls/"
    pub tmp_dir: PathBuf,
}
```

**Removals vs current `config.rs`.** The `RedisConfig` section is removed
(§9 of REQUIREMENTS); `JwtConfig` is kept; `OidcConfig`, `StreamingConfig`,
`LimitsConfig`, `TelemetryConfig` are new.

**Test seam.** `Config::from_env()` lets every integration test build a
config without a TOML file. Internal helpers like `LadderRung::parse` are
pure functions and trivially unit-testable.

---

### 5.2 `error`

**Responsibility.** Single application-wide error type. Maps to HTTP
responses in exactly one place. Domain modules define their own typed errors
and bubble them up via `#[from]` conversions.

**Public surface.**

```rust
pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]                        NotFound,
    #[error("conflict: {0}")]                    Conflict(String),
    #[error("not ready: {0}")]                   NotReady(String),       // → 425 / 409
    #[error("validation failed: {0}")]           Validation(String),     // → 422
    #[error("forbidden")]                        Forbidden,              // → 403
    #[error("unauthorized")]                     Unauthorized,           // → 401
    #[error("rate limited")]                     RateLimited,            // → 429
    #[error("upstream storage error: {0}")]      Storage(#[from] storage::StorageError),
    #[error("database error: {0}")]              Db(#[from] sqlx::Error),
    #[error("oidc error: {0}")]                  Oidc(#[from] auth::OidcError),
    #[error("transcode error: {0}")]             Transcode(#[from] transcode::TranscodeError),
    #[error("internal: {0}")]                    Internal(String),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response { /* uniform JSON body */ }
}

#[derive(serde::Serialize)]
pub struct ErrorBody<'a> {
    pub code: &'a str,        // stable machine-readable code, e.g. "track.not_ready"
    pub message: String,      // user-facing message (safe to expose)
    pub request_id: String,   // from request_id middleware
}
```

**Mapping rules.**

| Variant | HTTP status | `code` prefix |
|---|---|---|
| `NotFound` | 404 | `*.not_found` |
| `Conflict` | 409 | `*.conflict` |
| `NotReady` | 425 (or 409 depending on stream context) | `track.not_ready`, `job.not_ready` |
| `Validation` | 422 | `validation.<field>` |
| `Forbidden` | 403 | `auth.forbidden` |
| `Unauthorized` | 401 | `auth.unauthorized` |
| `RateLimited` | 429 | `rate_limited` |
| `Storage` | 502 | `storage.*` |
| `Db` | 500 (logged) | `internal` |
| `Oidc` | 401 | `auth.token_invalid` etc. |
| `Transcode` | 500 / 422 depending on subtype | `transcode.*` |
| `Internal` | 500 (logged) | `internal` |

**Test seam.** `into_response()` is pure (no I/O); a unit test per variant
asserts the status code, the JSON body shape, and that 5xx variants log at
ERROR while 4xx variants log at INFO.

---

### 5.3 `telemetry`

**Responsibility.** Structured logging and request correlation. No metric or
trace exporter wiring beyond what is necessary; `/metrics` lives in `ops`.

**Public surface.**

```rust
pub fn init_tracing(cfg: &TelemetryConfig) -> Result<TracingHandle, TelemetryError>;

#[derive(Clone)]
pub struct RequestId(pub String);     // UUID v7 string, propagated via header

pub fn request_id_layer() -> tower::layer::util::Stack<...>;
```

`init_tracing` installs a `tracing_subscriber::Registry` with:

- a JSON layer (for production) or pretty layer (for dev), selected by config,
- a layer that adds `request_id` and the current `user_id` (if any) to every
  span,
- `EnvFilter` driven by `RUST_LOG` with a documented sensible default
  (`info,sqlx=warn,aws_smithy_http=warn`).

**Decision.** All logs go through `tracing`. `println!` / `eprintln!` are
banned in non-`main` code (a Clippy lint will enforce this — see §10.5).

---

### 5.4 `http`

**Responsibility.** Build the Axum router and the middleware stack. This is
the **only** module that imports `axum` outside of `*/routes.rs` and
`auth/mod.rs`.

**Public surface.**

```rust
pub fn build_router(state: AppState) -> axum::Router;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub storage: Arc<dyn ObjectStore>,
    pub jobs: Arc<dyn JobQueue>,
    pub oidc: Arc<dyn OidcVerifier>,
    pub clock: Arc<dyn Clock>,         // injected for testability
}
```

**Middleware stack (outer → inner).**

1. `TraceLayer::new_for_http()` with a custom `make_span` that pulls
   `RequestId` and `user_id` (set by 2 and 3).
2. Request-ID middleware (`telemetry::request_id_layer()`). Reads
   `X-Request-Id` from incoming requests or generates a fresh UUID v7.
3. CORS (`tower_http::cors::CorsLayer`) configured with a typed origin
   allowlist from `LimitsConfig::cors_allowed_origins`.
4. Body size limit from `LimitsConfig::max_body_bytes` (e.g. 1 MiB for JSON
   payloads; the upload path uses presigned PUT and bypasses this).
5. Request timeout from `LimitsConfig::request_timeout_secs`.
6. Auth extractor (per-route; not a global layer — see 5.7).
7. Centralized error-mapping happens inside `AppError::into_response`, not as
   a middleware.

**Router composition.**

```rust
pub fn build_router(state: AppState) -> axum::Router {
    let v1 = axum::Router::new()
        .merge(catalog::routes::router(&state))
        .merge(ingest::routes::router(&state))
        .merge(streaming::routes::router(&state))
        .merge(library::routes::router(&state))
        .merge(search::routes::router(&state))
        .merge(recommendations::routes::router(&state))
        .merge(admin::routes::router(&state));

    axum::Router::new()
        .nest("/api/v1", v1)
        .merge(ops::routes::router(&state))
        .merge(http::openapi::router())
        .with_state(state)
        .layer(/* middleware stack 1..5 */)
}
```

---

### 5.5 `db`

**Responsibility.** Postgres pool, migration runner, transaction helper. No
domain queries live here.

**Public surface.**

```rust
pub async fn connect(cfg: &DatabaseConfig) -> Result<PgPool, DbError>;
pub async fn run_migrations(pool: &PgPool) -> Result<(), DbError>;

pub struct Tx<'a>(sqlx::Transaction<'a, sqlx::Postgres>);

impl<'a> Tx<'a> {
    pub async fn begin(pool: &PgPool) -> Result<Self, DbError>;
    pub async fn commit(self) -> Result<(), DbError>;
    pub async fn rollback(self) -> Result<(), DbError>;
}
```

`run_migrations` uses `sqlx::migrate!("./migrations")`. The destructive
`DROP TABLE` block in the current first migration is removed; dev seeding
moves to a separate `seeds/` directory invoked from `make seed`.

**Schema per module.** Each domain module owns a Postgres *schema*:
`auth`, `catalog`, `ingest`, `transcode`, `library`. This is more than
cosmetic — it lets us grant per-module DB roles later, and code review can
mechanically check that `catalog/repo.rs` only touches `catalog.*` tables.

---

### 5.6 `storage`

**Responsibility.** Abstract MinIO/S3 behind a trait so services can be
tested without a real object store, and so the same trait powers presigned
URLs *and* range-byte proxying for the streaming module.

**Public surface.**

```rust
#[async_trait::async_trait]
pub trait ObjectStore: Send + Sync {
    async fn head(&self, key: &str) -> Result<ObjectMeta, StorageError>;
    async fn put_object(&self, key: &str, body: Bytes, content_type: &str)
        -> Result<(), StorageError>;
    async fn get_object_range(
        &self,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<ObjectStream, StorageError>;
    async fn delete_object(&self, key: &str) -> Result<(), StorageError>;
    async fn delete_prefix(&self, prefix: &str) -> Result<u64, StorageError>;

    async fn presign_put(&self, key: &str, ttl: Duration, content_type: Option<&str>)
        -> Result<PresignedUrl, StorageError>;
    async fn presign_get(&self, key: &str, ttl: Duration)
        -> Result<PresignedUrl, StorageError>;
}

pub struct ObjectMeta {
    pub size_bytes: u64,
    pub etag: String,
    pub content_type: Option<String>,
    pub last_modified: DateTime<Utc>,
}

pub struct ByteRange { pub start: u64, pub end_inclusive: Option<u64> }

pub struct ObjectStream {
    pub content_length: Option<u64>,
    pub content_range: Option<String>,  // "bytes 0-1023/4096"
    pub content_type: Option<String>,
    pub bytes: Pin<Box<dyn Stream<Item = Result<Bytes, StorageError>> + Send>>,
}

pub struct PresignedUrl { pub url: String, pub expires_at: DateTime<Utc> }

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("not found")]                   NotFound,
    #[error("permission denied")]           Forbidden,
    #[error("invalid key: {0}")]            InvalidKey(String),
    #[error("upstream: {0}")]               Upstream(String),
}
```

**Implementations.**

- `S3ObjectStore` — wraps `aws_sdk_s3::Client`. Knows the bucket name **from
  config** (never hard-coded; closes bug #3 from REQUIREMENTS §6).
- `InMemoryObjectStore` (test-only) — a `HashMap<String, Bytes>` with
  matching presign mocks that return `http://fake.local/{key}?sig=...`.

**Key naming convention.** Single canonical scheme, used by every module:

```
originals/<upload_id>/<safe_filename>      # the source file, preserved
hls/<track_id>/master.m3u8                 # master playlist
hls/<track_id>/<variant>/index.m3u8        # variant playlist
hls/<track_id>/<variant>/seg-<n>.m4s       # CMAF segment
hls/<track_id>/<variant>/init.mp4          # CMAF init segment
```

This replaces today's ad-hoc `uploads/...` / `music/...` paths.

**Test seam.** Every service that touches storage takes `Arc<dyn
ObjectStore>`. Integration tests can choose the in-memory impl or real MinIO
via testcontainers.

---

### 5.7 `auth`

**Responsibility.** Verify Keycloak-issued JWT access tokens, expose
`AuthenticatedUser` as an Axum extractor, gate routes by role, and
upsert the user row on first contact.

> **Note.** The user mentioned they have stronger opinions on the auth side
> and may revise this. The interface boundary (the traits and the extractor)
> is what other modules depend on, so substitutions inside `auth` should not
> ripple outward.

**Token model — stateless JWT only.**

- The backend does **not** issue its own tokens for v1. It accepts JWT
  access tokens minted by Keycloak.
- Access tokens are short-lived (≤15 min). Refresh is handled client-side
  against Keycloak directly (`/realms/.../protocol/openid-connect/token`).
- There is **no** `auth.sessions` table — logout is "drop the token client
  side; wait ≤15 min for revocation to bite via expiry". If true revocation
  is needed later, we add a small denylist of `jti`s without changing the
  extractor contract.

**Required claims.**

| Claim | Required | Source | Used as |
|---|---|---|---|
| `iss` | yes | Keycloak realm URL | validated against `OidcConfig::issuer_url` |
| `aud` | yes | client id | validated against `OidcConfig::audience` |
| `exp`, `iat`, `nbf` | yes | standard | standard validity window |
| `sub` | yes | Keycloak user id | persisted as `users.oauth_subject` |
| `email` | yes | profile | persisted as `users.email` |
| `preferred_username` or `name` | yes | profile | persisted as `users.display_name` |
| `realm_access.roles` | optional | Keycloak realm roles | mapped to `Role::Admin` if it contains `"soundzone-admin"`, otherwise `Role::Listener` |

**Bootstrap admin.** Independent of realm roles, an email in
`OidcConfig::admin_email_allowlist` is always promoted to `Role::Admin` on
upsert. This is the on-ramp for the first deploy.

**Public surface.**

```rust
#[async_trait::async_trait]
pub trait OidcVerifier: Send + Sync {
    async fn verify(&self, bearer_token: &str) -> Result<VerifiedClaims, OidcError>;
}

pub struct KeycloakVerifier { /* jwks cache, http client */ }

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("token missing")]               Missing,
    #[error("token malformed")]             Malformed,
    #[error("signature invalid")]           BadSignature,
    #[error("token expired")]               Expired,
    #[error("issuer mismatch")]             BadIssuer,
    #[error("audience mismatch")]           BadAudience,
    #[error("jwks unreachable: {0}")]       JwksFetch(String),
}

#[derive(Debug, Clone)]
pub struct VerifiedClaims {
    pub sub: String,
    pub email: String,
    pub display_name: String,
    pub realm_roles: Vec<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role { Admin, Listener }

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: i64,                // local users.id
    pub oauth_subject: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
}

impl<S> axum::extract::FromRequestParts<S> for AuthenticatedUser
where Arc<AppState>: FromRef<S>, S: Send + Sync
{ /* extracts bearer, calls OidcVerifier, upserts via UserRepo */ }

pub struct RequireRole<const R: u8>;  // newtype guard: RequireRole::<{Role::Admin as u8}>

impl<const R: u8, S> FromRequestParts<S> for RequireRole<R> { /* 403 unless role matches */ }
```

**User upsert on first contact.**

```rust
#[async_trait::async_trait]
pub trait UserRepo: Send + Sync {
    async fn upsert_from_claims(&self, claims: &VerifiedClaims, admin_emails: &[String])
        -> Result<UserRow, sqlx::Error>;
    async fn by_subject(&self, subject: &str) -> Result<Option<UserRow>, sqlx::Error>;
}
```

`upsert_from_claims` uses `INSERT ... ON CONFLICT (oauth_provider, oauth_subject) DO UPDATE`
and computes the role each time (so role changes in Keycloak take effect on
the next request, plus the admin-allowlist applies).

**Test seam.** `OidcVerifier` is a trait. Tests use a `StubVerifier` that
returns canned `VerifiedClaims` for chosen bearer strings. `UserRepo` is
also a trait; in unit tests a `FakeUserRepo` (HashMap) lets us verify the
extractor's behavior without Postgres.

---

### 5.8 `jobs`

**Responsibility.** Abstract `underway` so the transcode module's worker is
testable in-process without a real Postgres queue, and so other future
background jobs (e.g. cleanup of orphaned HLS prefixes) can plug in.

**Public surface.**

```rust
#[async_trait::async_trait]
pub trait JobQueue: Send + Sync {
    async fn enqueue<S: JobSpec>(&self, spec: S) -> Result<JobId, JobError>;
    async fn status(&self, id: JobId) -> Result<JobStatus, JobError>;
}

pub trait JobSpec: Serialize + DeserializeOwned + Send + 'static {
    const TASK_NAME: &'static str;
    type Output: Send;
}

pub struct JobId(pub uuid::Uuid);

#[derive(Debug, Clone)]
pub enum JobStatus {
    Pending,
    Running { started_at: DateTime<Utc> },
    Succeeded { finished_at: DateTime<Utc> },
    Failed { finished_at: DateTime<Utc>, error: String, attempts: u32 },
}

pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_factor: f32,
}

#[async_trait::async_trait]
pub trait JobHandler<S: JobSpec>: Send + Sync {
    async fn handle(&self, spec: S, ctx: JobCtx) -> Result<S::Output, JobHandlerError>;
}

pub struct JobCtx {
    pub job_id: JobId,
    pub attempt: u32,
    pub cancel: tokio_util::sync::CancellationToken,
}
```

**Implementations.**

- `UnderwayQueue` — wraps `underway::Queue` and `underway::Worker`. Owns the
  retry policy. Persists jobs in `underway`'s own schema (separate from
  `transcode.*`).
- `InMemoryJobQueue` — used by tests. Runs handlers inline or via a single
  background task; deterministic.

**Worker bootstrap.** A single function called from `main.rs`:

```rust
pub fn spawn_workers(
    queue: Arc<dyn JobQueue>,
    handlers: WorkerRegistry,
    shutdown: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()>;
```

Each domain module that produces jobs publishes a registration function:

```rust
// transcode/worker.rs
pub fn register(reg: &mut WorkerRegistry, deps: TranscodeWorkerDeps);
```

This keeps `jobs/` ignorant of the catalog of jobs — the queue and the
registry are general; the job *types* live in each domain module.

---

## 6. Domain modules

Each domain module is documented to the same template:

- **Scope** — what it owns and explicitly does not own.
- **Data ownership** — which tables/schemas.
- **Public API (Rust)** — service trait, important types, repo trait.
- **HTTP endpoints** — method, path, auth requirement, request, response,
  error codes.
- **External dependencies** — which foundation modules and which other
  domain modules' public traits it consumes.
- **Test seams** — how to test it in isolation.

### 6.1 `catalog`

**Scope.** Read and write of artist / album / track metadata in the
"librarian's view" — title, artist, album, track number, release date,
duration. Owns the *metadata* of a track, but not its file/upload/transcode
state — those belong to `ingest` and `transcode`.

**Data ownership.** Postgres schema `catalog`:

```sql
CREATE TABLE catalog.artists (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE catalog.albums (
    id           BIGSERIAL PRIMARY KEY,
    artist_id    BIGINT NOT NULL REFERENCES catalog.artists(id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    release_date DATE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artist_id, title)
);

CREATE TABLE catalog.tracks (
    id               BIGSERIAL PRIMARY KEY,
    album_id         BIGINT NOT NULL REFERENCES catalog.albums(id) ON DELETE CASCADE,
    artist_id        BIGINT NOT NULL REFERENCES catalog.artists(id) ON DELETE CASCADE,
    title            TEXT NOT NULL,
    track_number     INTEGER,
    duration_seconds INTEGER NOT NULL CHECK (duration_seconds > 0),
    source_key       TEXT,         -- originals/<upload_id>/<file>   (set by ingest)
    hls_master_key   TEXT,         -- hls/<track_id>/master.m3u8     (set by transcode)
    status           TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending','transcoding','ready','failed')),
    failure_reason   TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (album_id, track_number)
);

CREATE INDEX idx_albums_artist_id    ON catalog.albums(artist_id);
CREATE INDEX idx_tracks_album_id     ON catalog.tracks(album_id);
CREATE INDEX idx_tracks_status       ON catalog.tracks(status);
CREATE INDEX idx_tracks_title_search ON catalog.tracks USING gin (to_tsvector('english', title));
CREATE INDEX idx_artists_name_search ON catalog.artists USING gin (to_tsvector('english', name));
CREATE INDEX idx_albums_title_search ON catalog.albums  USING gin (to_tsvector('english', title));
```

**Domain types.**

```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Artist { pub id: i64, pub name: String, pub created_at: DateTime<Utc> }

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Album {
    pub id: i64,
    pub artist_id: i64,
    pub title: String,
    pub release_date: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Track {
    pub id: i64,
    pub album_id: i64,
    pub artist_id: i64,
    pub title: String,
    pub track_number: Option<i32>,
    pub duration_seconds: i32,
    pub status: TrackStatus,
    pub hls_master_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, sqlx::Type, serde::Serialize)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum TrackStatus { Pending, Transcoding, Ready, Failed }
```

**Repo trait.**

```rust
#[async_trait::async_trait]
pub trait CatalogRepo: Send + Sync {
    async fn create_artist(&self, name: &str) -> Result<Artist, sqlx::Error>;
    async fn list_artists(&self, page: Page) -> Result<PagedResult<Artist>, sqlx::Error>;
    async fn get_artist(&self, id: i64) -> Result<Option<Artist>, sqlx::Error>;

    async fn create_album(&self, artist_id: i64, req: NewAlbum) -> Result<Album, sqlx::Error>;
    async fn list_albums_by_artist(&self, artist_id: i64, page: Page)
        -> Result<PagedResult<Album>, sqlx::Error>;
    async fn get_album(&self, id: i64) -> Result<Option<Album>, sqlx::Error>;

    // Note: track *creation* is owned by `ingest`; `catalog` only reads tracks
    // and offers metadata-edit endpoints (title, track_number, release_date).
    async fn list_tracks_by_album(&self, album_id: i64, page: Page)
        -> Result<PagedResult<Track>, sqlx::Error>;
    async fn get_track(&self, id: i64) -> Result<Option<Track>, sqlx::Error>;
    async fn update_track_metadata(&self, id: i64, patch: TrackPatch)
        -> Result<Track, sqlx::Error>;
    async fn delete_track(&self, id: i64) -> Result<bool, sqlx::Error>;
}

pub struct Page { pub limit: u32, pub offset: u32 }
pub struct PagedResult<T> { pub items: Vec<T>, pub total: u64 }

pub struct NewAlbum { pub title: String, pub release_date: Option<NaiveDate> }
pub struct TrackPatch {
    pub title: Option<String>,
    pub track_number: Option<i32>,
    pub artist_id: Option<i64>,
    pub album_id: Option<i64>,
}
```

**Service.**

```rust
pub struct CatalogService<R: CatalogRepo> { repo: R }

impl<R: CatalogRepo> CatalogService<R> {
    pub async fn create_artist(&self, actor: &AuthenticatedUser, name: &str) -> Result<Artist>;
    pub async fn list_artists(&self, page: Page) -> Result<PagedResult<Artist>>;
    pub async fn create_album(&self, actor: &AuthenticatedUser, req: CreateAlbumCmd)
        -> Result<Album>;
    pub async fn list_albums_by_artist(&self, artist_id: i64, page: Page)
        -> Result<PagedResult<Album>>;
    pub async fn list_tracks_by_album(&self, album_id: i64, page: Page)
        -> Result<PagedResult<Track>>;
    pub async fn get_track(&self, id: i64) -> Result<Track>;            // 404 → AppError::NotFound
    pub async fn update_track(&self, actor: &AuthenticatedUser, id: i64, patch: TrackPatch)
        -> Result<Track>;
    pub async fn delete_track(&self, actor: &AuthenticatedUser, id: i64) -> Result<()>;
}
```

The service is the only place that enforces "admin only for writes" — but it
does that by *requiring* an `&AuthenticatedUser` and matching on `role`, not
by reading from any thread-local. Read methods do not take `&AuthenticatedUser`.

**HTTP endpoints.**

| Method | Path | Auth | Body / Query | Response | Errors |
|---|---|---|---|---|---|
| `POST` | `/api/v1/catalog/artists` | admin | `{name}` | `201 Artist` | 422, 409, 401, 403 |
| `GET`  | `/api/v1/catalog/artists` | listener | `?limit&offset` | `200 PagedResult<Artist>` | 401 |
| `GET`  | `/api/v1/catalog/artists/{id}` | listener | — | `200 Artist` | 404, 401 |
| `POST` | `/api/v1/catalog/artists/{id}/albums` | admin | `{title, release_date?}` | `201 Album` | 422, 409, 404 |
| `GET`  | `/api/v1/catalog/artists/{id}/albums` | listener | `?limit&offset` | `200 PagedResult<Album>` | 404 |
| `GET`  | `/api/v1/catalog/albums/{id}` | listener | — | `200 Album` | 404 |
| `GET`  | `/api/v1/catalog/albums/{id}/tracks` | listener | `?limit&offset` | `200 PagedResult<Track>` | 404 |
| `GET`  | `/api/v1/catalog/tracks/{id}` | listener | — | `200 Track` | 404 |
| `PATCH` | `/api/v1/catalog/tracks/{id}` | admin | `TrackPatch` | `200 Track` | 404, 422 |
| `DELETE` | `/api/v1/catalog/tracks/{id}` | admin | — | `204` | 404 |

`DELETE` on a track also enqueues a `CleanupTrackAssets` job (defined in
`transcode`) so the HLS prefix in MinIO is removed asynchronously.

**Dependencies.**

| Depends on | Why |
|---|---|
| `auth::AuthenticatedUser`, `auth::Role` | role gating on writes |
| `db::PgPool` | the Postgres repo impl |
| `jobs::JobQueue` | enqueue `CleanupTrackAssets` on delete |
| `error::AppError` | bubble up errors |

Catalog does **not** depend on `storage`, `transcode`, or `ingest`. Its read
endpoints expose `hls_master_key` so `streaming` can build playlists, but
catalog itself never opens an HLS playlist.

**Test seams.**

- `CatalogService` is generic over `CatalogRepo`. Unit tests use a
  `FakeCatalogRepo` (in-memory `BTreeMap`s).
- `routes.rs` is integration-tested via `axum::Router` + `tower::ServiceExt`
  with a `FakeCatalogRepo`, a `StubVerifier`, and an in-memory `JobQueue`.
- A separate `sqlx::test` group exercises the real Postgres repo against a
  testcontainer.

---

### 6.2 `ingest`

**Scope.** Everything from "admin wants to upload a track" up to "the track
row exists in `catalog.tracks` with `status='pending'` and a `source_key`,
and a `Transcode` job is enqueued". `ingest` does **not** transcode.

**Data ownership.** Postgres schema `ingest`:

```sql
CREATE TABLE ingest.upload_sessions (
    id              UUID PRIMARY KEY,
    actor_user_id   BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    file_name       TEXT NOT NULL,
    expected_mime   TEXT,
    object_key      TEXT NOT NULL,                       -- originals/<id>/<file>
    status          TEXT NOT NULL DEFAULT 'awaiting_put'
                       CHECK (status IN ('awaiting_put','confirmed','expired','rejected')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    confirmed_at    TIMESTAMPTZ,
    sha256_hex      CHAR(64),                            -- filled at confirm-time
    UNIQUE (sha256_hex)                                  -- dedupe across the catalog
);

CREATE INDEX idx_upload_sessions_actor ON ingest.upload_sessions(actor_user_id);
CREATE INDEX idx_upload_sessions_status ON ingest.upload_sessions(status);
```

`tracks.source_key` mirrors `ingest.upload_sessions.object_key` for the
confirmed session — that lets `streaming` use only `catalog.tracks` for its
queries.

**Domain types.**

```rust
pub struct UploadSession {
    pub id: Uuid,
    pub actor_user_id: i64,
    pub file_name: String,
    pub object_key: String,
    pub status: UploadStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub enum UploadStatus { AwaitingPut, Confirmed, Expired, Rejected }

pub struct SourceProbe {
    pub codec: SourceCodec,            // Flac | Wav | Mp3
    pub duration_seconds: u32,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub bit_depth: Option<u8>,
    pub bitrate_kbps: Option<u32>,
}

pub enum SourceCodec { Flac, Wav, Mp3 }
```

**Service.**

```rust
pub struct IngestService<U, S, P, J>
where
    U: UploadRepo,
    S: ObjectStore,
    P: SourceProber,
    J: JobQueue,
{
    uploads: U,
    storage: Arc<S>,
    prober:  Arc<P>,
    jobs:    Arc<J>,
    catalog_writer: Arc<dyn CatalogWriter>,   // narrow port owned here (see below)
    cfg:     IngestConfig,
    clock:   Arc<dyn Clock>,
}

#[derive(Debug)]
pub struct CreateUploadCmd {
    pub file_name: String,
    pub expected_mime: Option<String>,
}

#[derive(Debug)]
pub struct ConfirmUploadCmd {
    pub upload_id: Uuid,
    pub album_id: i64,
    pub artist_id: i64,
    pub title: Option<String>,
    pub track_number: Option<i32>,
}

impl<...> IngestService<...> {
    pub async fn create_upload(&self, actor: &AuthenticatedUser, cmd: CreateUploadCmd)
        -> Result<PresignedUploadResponse>;
    pub async fn confirm_upload(&self, actor: &AuthenticatedUser, cmd: ConfirmUploadCmd)
        -> Result<Track>;
    pub async fn expire_stale_uploads(&self) -> Result<u64>;   // called by a cron job
}

pub struct PresignedUploadResponse {
    pub upload_id: Uuid,
    pub presigned_url: String,
    pub object_key: String,
    pub expires_at: DateTime<Utc>,
    pub headers: BTreeMap<String, String>,   // headers the client must echo on PUT
}
```

**`CatalogWriter` port.** To avoid `ingest` reaching into `catalog`'s repo,
`catalog` publishes a small write-only port that `ingest` consumes:

```rust
// catalog/mod.rs (public)
#[async_trait::async_trait]
pub trait CatalogWriter: Send + Sync {
    async fn create_track_pending(
        &self,
        album_id: i64,
        artist_id: i64,
        title: &str,
        track_number: Option<i32>,
        duration_seconds: i32,
        source_key: &str,
    ) -> Result<Track, sqlx::Error>;
}
```

This is the **only** way another module touches `catalog.tracks` for writes.
`catalog::PostgresCatalogRepo` implements both `CatalogRepo` and
`CatalogWriter`.

**Probing.**

```rust
#[async_trait::async_trait]
pub trait SourceProber: Send + Sync {
    async fn probe(&self, src: ProbeSource<'_>) -> Result<SourceProbe, ProbeError>;
}

pub enum ProbeSource<'a> {
    Path(&'a Path),
    Stream(Pin<Box<dyn AsyncRead + Send + 'a>>),
}

pub struct FfprobeProber { ffprobe_path: PathBuf }   // production
pub struct FakeProber(SourceProbe);                  // tests
```

The default flow downloads the head of the object (≤4 MiB is enough for
headers in FLAC/WAV/MP3) into a temp file and runs `ffprobe -of json`. We
do not need the entire object to probe duration/codec — fixing bug #4 from
REQUIREMENTS §6.

**End-to-end ingest sequence.**

1. `POST /api/v1/ingest/uploads` (admin) — `IngestService::create_upload`:
   1. Validate `file_name` (length, allowed characters), reject if
      extension ∉ {flac, wav, mp3}.
   2. Insert `upload_sessions` row with status `awaiting_put`,
      `expires_at = now + 1h`.
   3. Call `storage.presign_put(object_key, ttl=1h, content_type=mime)`.
   4. Return `PresignedUploadResponse` (the response also tells the client
      which `Content-Type` header it must send on the PUT, so the bucket
      records it correctly).
2. Client PUTs the bytes directly to MinIO.
3. `POST /api/v1/ingest/uploads/{id}/confirm` (admin) —
   `IngestService::confirm_upload`:
   1. Validate session exists, is `awaiting_put`, and is not expired.
   2. Validate album_id/artist_id exist (cheap repo check).
   3. `storage.head(object_key)` to confirm the object was actually uploaded
      and to read its size + ETag.
   4. Download head bytes, run `prober.probe(...)` → `SourceProbe`.
   5. Reject if codec ∉ supported list, or if `sha256(head+tail)` matches an
      existing `upload_sessions.sha256_hex` (cheap dedupe). (Whole-file
      sha256 is recorded on the *originals* object during transcode, which
      runs anyway.)
   6. Begin DB transaction:
      - Mark `upload_sessions` row `confirmed`, store sha256.
      - `catalog_writer.create_track_pending(...)` → returns `Track { status:
        Pending }`.
      - Enqueue `TranscodeJobSpec { track_id, source_key, source_probe }`
        via `jobs.enqueue`.
      - Commit.
   7. Return the new `Track`.

`expire_stale_uploads` is a separate cron-style job (also registered with
`jobs`) that runs every 15 min and:

- marks `awaiting_put` sessions past `expires_at` as `expired`,
- deletes the corresponding object from MinIO if it exists,
- never touches `confirmed` rows.

**HTTP endpoints.**

| Method | Path | Auth | Body | Response | Errors |
|---|---|---|---|---|---|
| `POST` | `/api/v1/ingest/uploads` | admin | `{file_name, expected_mime?}` | `200 PresignedUploadResponse` | 422, 401, 403 |
| `POST` | `/api/v1/ingest/uploads/{id}/confirm` | admin | `{album_id, artist_id, title?, track_number?}` | `201 Track` | 404, 409, 422 |
| `GET`  | `/api/v1/ingest/uploads/{id}` | admin | — | `200 UploadSession` | 404 |
| `DELETE` | `/api/v1/ingest/uploads/{id}` | admin | — | `204` | 404, 409 |

The `DELETE` only works on `awaiting_put` or `rejected` sessions; once
`confirmed`, the track exists and must be deleted via `catalog`.

**Dependencies.**

| Depends on | Why |
|---|---|
| `auth::AuthenticatedUser` | admin gating |
| `storage::ObjectStore` | presign PUT, head, range-get |
| `jobs::JobQueue` | enqueue `TranscodeJobSpec` |
| `catalog::CatalogWriter` | create the pending track row |
| `db::PgPool` | own `ingest.upload_sessions` |
| `error::AppError` | error bubbling |

**Test seams.**

- `IngestService` is generic over its four collaborators; unit tests inject
  `InMemoryObjectStore`, `FakeProber`, `InMemoryJobQueue`,
  `FakeUploadRepo`, and a `StubCatalogWriter` (HashMap-backed). The happy
  path, all rejection paths, expiry, and dedupe are unit-testable.
- Integration test: real MinIO testcontainer + real Postgres + real
  `FfprobeProber` (skip if `ffprobe` isn't on PATH).

---

### 6.3 `transcode`

**Scope.** Take a `Track` in status `pending` whose `source_key` is set,
produce an HLS ladder (3 AAC-LC variants + a master playlist) in MinIO under
`hls/<track_id>/...`, write one row per variant to `transcode.outputs`, set
`catalog.tracks.status='ready'` and `hls_master_key='hls/<track_id>/master.m3u8'`.
On unrecoverable failure, set `status='failed'` and a `failure_reason`.

**Data ownership.** Postgres schema `transcode`:

```sql
CREATE TABLE transcode.outputs (
    id                  BIGSERIAL PRIMARY KEY,
    track_id            BIGINT NOT NULL REFERENCES catalog.tracks(id) ON DELETE CASCADE,
    variant             TEXT NOT NULL,         -- "low" | "mid" | "high"
    codec               TEXT NOT NULL,         -- "aac_lc"
    container           TEXT NOT NULL,         -- "fmp4"
    bitrate_kbps        INTEGER NOT NULL,
    hls_playlist_key    TEXT NOT NULL,         -- hls/<track_id>/<variant>/index.m3u8
    hls_init_key        TEXT NOT NULL,         -- hls/<track_id>/<variant>/init.mp4
    byte_size           BIGINT NOT NULL,
    segment_count       INTEGER NOT NULL,
    target_segment_secs INTEGER NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (track_id, variant)
);

CREATE INDEX idx_outputs_track_id ON transcode.outputs(track_id);
```

(The `underway` library manages its own queue tables in its own schema — we
don't model them here.)

**Domain types.**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscodeJobSpec {
    pub track_id: i64,
    pub source_key: String,
    pub source_probe: SourceProbe,
}

impl JobSpec for TranscodeJobSpec {
    const TASK_NAME: &'static str = "transcode.encode_ladder";
    type Output = ();
}

#[derive(Debug, Clone, Copy)]
pub enum VariantName { Low, Mid, High }

#[derive(Debug, Clone)]
pub struct LadderRung {
    pub name: VariantName,
    pub bitrate_kbps: u32,
    pub target_segment_secs: u32,
}

#[derive(Debug, Clone)]
pub struct TranscodeOutput {
    pub track_id: i64,
    pub variant: VariantName,
    pub hls_playlist_key: String,
    pub hls_init_key: String,
    pub byte_size: u64,
    pub segment_count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CleanupTrackAssetsSpec { pub track_id: i64, pub hls_prefix: String }
impl JobSpec for CleanupTrackAssetsSpec {
    const TASK_NAME: &'static str = "transcode.cleanup_track_assets";
    type Output = ();
}
```

**`Transcoder` trait — the test seam for ffmpeg.**

```rust
#[async_trait::async_trait]
pub trait Transcoder: Send + Sync {
    /// Read source bytes from `src`, write the full set of HLS outputs
    /// (init + segments + variant playlist + master playlist) into `dst`.
    async fn encode_hls_ladder(
        &self,
        src: TranscodeSource<'_>,
        dst: &mut dyn HlsWriter,
        ladder: &[LadderRung],
        ctx: &TranscodeCtx,
    ) -> Result<Vec<TranscodeOutput>, TranscodeError>;
}

pub enum TranscodeSource<'a> {
    Path(&'a Path),                                          // ffmpeg reads a local file
    Stream(Pin<Box<dyn AsyncRead + Send + 'a>>),             // ffmpeg reads from stdin
}

#[async_trait::async_trait]
pub trait HlsWriter: Send + Sync {
    async fn write(&mut self, key: &str, body: Bytes, content_type: &str)
        -> Result<(), TranscodeError>;
}

pub struct TranscodeCtx {
    pub track_id: i64,
    pub tmp_dir: PathBuf,
    pub cancel: tokio_util::sync::CancellationToken,
}
```

**Implementations.**

- `FfmpegTranscoder` — production. Downloads the source object to a tmp
  file (we do not stream into ffmpeg's stdin for v1 — seeking is friendlier
  for ffmpeg, especially for FLAC), invokes `ffmpeg` with one
  `-map 0:a` per rung and `-f hls -hls_segment_type fmp4` per rung,
  outputting into per-variant tmp dirs. Then iterates the produced files
  and calls `HlsWriter::write` for each. Generates the master playlist
  in-process (deterministic; we know the bitrates).
- `FakeTranscoder` — test-only. Emits one fake "playlist" and one fake
  "segment" per rung, into the supplied writer. Lets us test `worker.rs`
  without ffmpeg.

The `HlsWriter` adapter that the worker uses in production wraps an
`Arc<dyn ObjectStore>` and `put_object`s under the `hls/<track_id>/...`
prefix. Tests use an in-memory `Vec<(String, Bytes)>` writer to assert that
the expected keys were produced.

**Worker.**

```rust
// transcode/worker.rs
pub struct TranscodeJobHandler<R, S, T>
where R: TranscodeRepo, S: ObjectStore, T: Transcoder
{
    repo: R,
    storage: Arc<S>,
    transcoder: Arc<T>,
    catalog_writer: Arc<dyn CatalogStatusWriter>,
    cfg: TranscodeConfig,
}

#[async_trait::async_trait]
impl<...> JobHandler<TranscodeJobSpec> for TranscodeJobHandler<...> {
    async fn handle(&self, spec: TranscodeJobSpec, ctx: JobCtx) -> Result<(), JobHandlerError> {
        // 1. mark track 'transcoding'
        // 2. download source_key → tmp file
        // 3. transcoder.encode_hls_ladder(...) → Vec<TranscodeOutput>
        // 4. insert into transcode.outputs (in a DB transaction)
        // 5. write master playlist via storage
        // 6. catalog_writer.set_track_ready(track_id, hls_master_key)
        // On any error: catalog_writer.set_track_failed(track_id, reason)
        //   - returning Err from handle() lets `underway` apply RetryPolicy
        //     for transient errors; permanent errors return Ok(()) after
        //     marking failed (so we don't retry forever).
    }
}
```

`CatalogStatusWriter` is the second narrow port `catalog` exposes:

```rust
// catalog/mod.rs (public)
#[async_trait::async_trait]
pub trait CatalogStatusWriter: Send + Sync {
    async fn set_track_transcoding(&self, track_id: i64) -> Result<(), sqlx::Error>;
    async fn set_track_ready(&self, track_id: i64, hls_master_key: &str)
        -> Result<(), sqlx::Error>;
    async fn set_track_failed(&self, track_id: i64, reason: &str) -> Result<(), sqlx::Error>;
}
```

**Cleanup handler.** A separate `JobHandler<CleanupTrackAssetsSpec>` that
calls `storage.delete_prefix("hls/{track_id}/")` (and the original under
`originals/<upload_id>/...`). It is enqueued by `catalog::delete_track`.

**Retry policy.** `RetryPolicy { max_attempts: 3, initial_backoff: 30s,
backoff_factor: 4.0, max_backoff: 30min }`. Errors are classified:

| Error | Classification | Action |
|---|---|---|
| `StorageError::Upstream` | transient | retry |
| `TranscodeError::FfmpegSpawn` | transient | retry |
| `TranscodeError::FfmpegExit { code }` (non-zero) | permanent | mark failed, do not retry |
| `TranscodeError::InvalidSource` | permanent | mark failed |
| `sqlx::Error` (timeout, pool exhaustion) | transient | retry |
| `sqlx::Error` (unique violation) | permanent | mark failed (race condition; shouldn't happen) |

**HTTP endpoints.** None public. The admin module exposes read-only views
on jobs (§6.8).

**Dependencies.**

| Depends on | Why |
|---|---|
| `jobs::JobHandler` | register itself with the worker |
| `storage::ObjectStore` | download source, upload HLS, delete on cleanup |
| `catalog::CatalogStatusWriter` | flip track status |
| `db::PgPool` | own `transcode.outputs` |
| `config::TranscodeConfig` | ffmpeg path, ladder, prefixes |

**Test seams.**

- `TranscodeJobHandler::handle` is generic over its three collaborators;
  unit tests with `FakeTranscoder`, `InMemoryObjectStore`,
  `FakeCatalogStatusWriter`, `FakeTranscodeRepo` cover happy path,
  every classified error, and idempotency on retry.
- A separate integration test boots `underway` against a real Postgres,
  enqueues a job, and asserts that the handler is invoked and the track
  reaches `ready`.
- An end-to-end smoke test (gated by `RUN_E2E=1`) uses the real ffmpeg on
  a known-good 10-second WAV and asserts that all three variants exist in
  MinIO and that ffprobe can parse the master playlist.

---

### 6.4 `streaming`

**Scope.** Serve HLS playlists and (optionally) HLS segments for tracks
that are in status `ready`. Reject not-ready tracks with `425 Too Early`.
Authenticate playlist fetches; segments are either presigned (default) or
proxied through this module.

**Data ownership.** None. Streaming is a *read* module over
`catalog.tracks` and `transcode.outputs`.

**Domain types.**

```rust
pub struct MasterPlaylist {
    pub track_id: i64,
    pub variants: Vec<VariantEntry>,
}

pub struct VariantEntry {
    pub name: VariantName,
    pub bitrate_kbps: u32,
    pub codecs: &'static str,         // "mp4a.40.2" for AAC-LC
    pub uri: String,                  // absolute URL to variant playlist
}

pub struct VariantPlaylist {
    pub target_duration_secs: u32,
    pub init_uri: String,             // absolute (presigned or local proxy URL)
    pub segments: Vec<SegmentEntry>,
}

pub struct SegmentEntry { pub duration_secs: f32, pub uri: String }
```

**Ports.**

`streaming` does not query `transcode` or `catalog` tables directly. They
each publish a narrow read trait:

```rust
// catalog/mod.rs (public)
#[async_trait::async_trait]
pub trait CatalogReadForStreaming: Send + Sync {
    async fn get_ready_track(&self, id: i64) -> Result<Option<ReadyTrack>, sqlx::Error>;
}
pub struct ReadyTrack { pub id: i64, pub hls_master_key: String, pub status: TrackStatus }

// transcode/mod.rs (public)
#[async_trait::async_trait]
pub trait OutputsReader: Send + Sync {
    async fn list_outputs(&self, track_id: i64) -> Result<Vec<TranscodeOutput>, sqlx::Error>;
}
```

**Delivery mode.**

```rust
#[derive(Debug, Clone, Copy)]
pub enum DeliveryMode { Presigned, Proxied }

#[async_trait::async_trait]
pub trait SegmentDelivery: Send + Sync {
    /// Convert an object key into a URL suitable for embedding in a playlist.
    async fn url_for(&self, key: &str) -> Result<String, AppError>;

    /// Optional proxy path: stream bytes for a given key with Range support.
    /// Returns Err(AppError::NotFound) if this delivery mode does not proxy.
    async fn proxy(&self, key: &str, range: Option<ByteRange>) -> Result<ObjectStream, AppError>;
}

pub struct PresignedDelivery { storage: Arc<dyn ObjectStore>, ttl: Duration }
pub struct ProxiedDelivery   { storage: Arc<dyn ObjectStore>, base_url: String }
```

`StreamingService` is constructed with **both** deliveries; per request it
picks based on a query parameter (`?delivery=presigned|proxied`) or the
config default. The proxied path is only enabled if
`StreamingConfig::enable_proxy_fallback = true`.

**Service.**

```rust
pub struct StreamingService<C, T>
where C: CatalogReadForStreaming, T: OutputsReader
{
    catalog: C,
    transcode: T,
    presigned: Arc<PresignedDelivery>,
    proxied: Option<Arc<ProxiedDelivery>>,
    cfg: StreamingConfig,
}

impl<...> StreamingService<...> {
    pub async fn master_playlist(&self, track_id: i64, mode: DeliveryMode)
        -> Result<MasterPlaylist>;
    pub async fn variant_playlist(&self, track_id: i64, variant: VariantName, mode: DeliveryMode)
        -> Result<VariantPlaylist>;
    pub async fn segment(&self, track_id: i64, variant: VariantName, seg_name: &str,
                         range: Option<ByteRange>)
        -> Result<ObjectStream>;
}
```

If the requested track has `status != Ready` → `AppError::NotReady` →
HTTP `425 Too Early`. This closes bug #10 from REQUIREMENTS §6.

**Variant playlist construction.** When the worker writes the per-variant
`index.m3u8` to MinIO, it writes it with *relative* segment URIs
(`seg-001.m4s`, ...). When `streaming` serves a variant playlist, it either:

- *Presigned mode:* reads the playlist from MinIO, parses out segment names,
  rewrites each to a fresh presigned URL with TTL =
  `StreamingConfig::segment_presign_ttl_secs` (default 300s). The init
  segment URI is rewritten the same way.
- *Proxied mode:* rewrites each segment name to
  `https://<base_url>/api/v1/streaming/tracks/{track_id}/{variant}/{seg-name}`,
  a path served by `StreamingService::segment` (which proxies bytes).

We do not serve `streaming.outputs` rows as the playlist source — we render
the playlist text in-process from the stored `outputs` row. This is faster
and avoids paying for an extra MinIO GET per request.

**HTTP endpoints.**

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| `GET` | `/api/v1/streaming/tracks/{id}/master.m3u8` | listener | — | `200 application/vnd.apple.mpegurl` (or `425` if not ready) |
| `GET` | `/api/v1/streaming/tracks/{id}/{variant}/index.m3u8` | listener | `?delivery=` | `200 application/vnd.apple.mpegurl` |
| `GET` | `/api/v1/streaming/tracks/{id}/{variant}/init.mp4` | listener | — (proxied only) | `200 video/iso.segment` |
| `GET` | `/api/v1/streaming/tracks/{id}/{variant}/{segment}` | listener | `Range:` | `200`/`206 video/iso.segment` (proxied only) |

`Cache-Control` headers:

| Endpoint | Cache-Control |
|---|---|
| master.m3u8 | `public, max-age=300` |
| index.m3u8 (presigned) | `public, max-age=120` (must be ≤ presign TTL) |
| index.m3u8 (proxied) | `public, max-age=300` |
| segment / init (proxied) | `public, max-age=31536000, immutable` |

Playlist responses also carry `ETag` (sha256 of the rendered body, weak ETag)
so a smart client can `If-None-Match` and we can answer `304`.

**Recording a play.** `streaming::master_playlist` optionally fires an event
into `library::history` (`record_play`) when called by an authenticated
listener. This is gated by a query param `?record=true` so the client
controls when a "play" actually counts; alternatively, the client can call
`POST /api/v1/me/history` itself. See §6.5.

**Dependencies.**

| Depends on | Why |
|---|---|
| `auth::AuthenticatedUser` | playlist endpoints require a listener |
| `catalog::CatalogReadForStreaming` | look up a ready track |
| `transcode::OutputsReader` | list variants |
| `storage::ObjectStore` | proxy bytes + presign segments |
| `library::HistoryWriter` (optional) | record a play |

**Test seams.**

- Service tests with a `FakeCatalogReadForStreaming`, `FakeOutputsReader`,
  and `InMemoryObjectStore` cover playlist rendering, not-ready handling,
  range-request math, and presign URL substitution. No HTTP needed.
- One integration test using `axum::Router` validates the `425` and
  `Cache-Control` headers.

---

### 6.5 `library`

**Scope.** All per-user data: favorites, listen history, playback
positions, playlists. Internally split into four sub-modules sharing the
same `library` Postgres schema. Each sub-module follows the same
`domain.rs / repo.rs / service.rs` shape; one combined `routes.rs` exposes
all `/me/...` endpoints.

**Data ownership.** Postgres schema `library`:

```sql
CREATE TABLE library.favorite_tracks (
    user_id    BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    track_id   BIGINT NOT NULL REFERENCES catalog.tracks(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, track_id)
);
CREATE INDEX idx_fav_tracks_user ON library.favorite_tracks(user_id, created_at DESC);
-- analogous: favorite_albums (user_id, album_id), favorite_artists (user_id, artist_id)

CREATE TABLE library.listen_history (
    id                          BIGSERIAL PRIMARY KEY,
    user_id                     BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    track_id                    BIGINT NOT NULL REFERENCES catalog.tracks(id) ON DELETE CASCADE,
    played_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    duration_listened_seconds   INTEGER NOT NULL DEFAULT 0,
    counted_as_play             BOOLEAN NOT NULL DEFAULT false   -- true if ≥ 50% of track
);
CREATE INDEX idx_history_user_time  ON library.listen_history(user_id, played_at DESC);
CREATE INDEX idx_history_track_time ON library.listen_history(track_id, played_at DESC);

CREATE TABLE library.playback_positions (
    user_id          BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    track_id         BIGINT NOT NULL REFERENCES catalog.tracks(id) ON DELETE CASCADE,
    position_seconds INTEGER NOT NULL CHECK (position_seconds >= 0),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, track_id)
);
CREATE INDEX idx_positions_user_recent ON library.playback_positions(user_id, updated_at DESC);

CREATE TABLE library.playlists (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, name)
);

CREATE TABLE library.playlist_tracks (
    id          BIGSERIAL PRIMARY KEY,
    playlist_id BIGINT NOT NULL REFERENCES library.playlists(id) ON DELETE CASCADE,
    track_id    BIGINT NOT NULL REFERENCES catalog.tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL CHECK (position >= 0),
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (playlist_id, position)
);
CREATE INDEX idx_playlist_tracks_pl ON library.playlist_tracks(playlist_id, position);
```

**Services.**

```rust
pub trait FavoritesService { /* track/album/artist add/remove/list */ }
pub trait HistoryService {
    async fn record_play(&self, user: &AuthenticatedUser, track_id: i64,
                         duration_listened_seconds: u32) -> Result<()>;
    async fn list_history(&self, user: &AuthenticatedUser, page: Page)
        -> Result<PagedResult<HistoryEntry>>;
    async fn top_tracks(&self, user: &AuthenticatedUser, window: Window, limit: u32)
        -> Result<Vec<TrackPlayCount>>;
}
pub trait PlaybackService {
    async fn upsert_position(&self, user: &AuthenticatedUser, track_id: i64,
                             position_seconds: u32) -> Result<()>;
    async fn recent(&self, user: &AuthenticatedUser, limit: u32)
        -> Result<Vec<PlaybackPosition>>;
}
pub trait PlaylistService {
    async fn create(&self, user: &AuthenticatedUser, name: &str) -> Result<Playlist>;
    async fn list_mine(&self, user: &AuthenticatedUser) -> Result<Vec<Playlist>>;
    async fn get(&self, user: &AuthenticatedUser, id: i64) -> Result<PlaylistWithTracks>;
    async fn rename(&self, user: &AuthenticatedUser, id: i64, name: &str) -> Result<Playlist>;
    async fn delete(&self, user: &AuthenticatedUser, id: i64) -> Result<()>;
    async fn add_track(&self, user: &AuthenticatedUser, id: i64, track_id: i64)
        -> Result<PlaylistTrack>;
    async fn reorder(&self, user: &AuthenticatedUser, id: i64, order: Vec<i64>) -> Result<()>;
    async fn remove_track(&self, user: &AuthenticatedUser, id: i64, playlist_track_id: i64)
        -> Result<()>;
}
```

`record_play` computes `counted_as_play = duration_listened_seconds * 2 >=
track.duration_seconds`. It calls `catalog::CatalogReadForStreaming::get_ready_track`
to fetch the duration; it never reads `catalog.tracks` directly.

**`HistoryWriter` port** (exposed publicly by `library` for `streaming` and
for `recommendations` to consume read-side):

```rust
#[async_trait::async_trait]
pub trait HistoryWriter: Send + Sync {
    async fn record_play(&self, user_id: i64, track_id: i64,
                         duration_listened_seconds: u32) -> Result<(), sqlx::Error>;
}
```

**HTTP endpoints (all require listener).**

| Method | Path | Notes |
|---|---|---|
| `POST` / `DELETE` | `/api/v1/me/favorites/tracks/{id}` | |
| `POST` / `DELETE` | `/api/v1/me/favorites/albums/{id}` | |
| `POST` / `DELETE` | `/api/v1/me/favorites/artists/{id}` | |
| `GET` | `/api/v1/me/favorites/{tracks\|albums\|artists}` | paginated |
| `POST` | `/api/v1/me/history` | `{track_id, duration_listened_seconds}` |
| `GET` | `/api/v1/me/history` | paginated |
| `GET` | `/api/v1/me/stats/top-tracks?window=30d&limit=20` | |
| `PUT` | `/api/v1/me/playback/{track_id}` | `{position_seconds}` |
| `GET` | `/api/v1/me/playback/recent?limit=20` | |
| `POST` | `/api/v1/me/playlists` | |
| `GET` | `/api/v1/me/playlists` | |
| `GET` | `/api/v1/me/playlists/{id}` | |
| `PATCH` | `/api/v1/me/playlists/{id}` | rename + reorder |
| `DELETE` | `/api/v1/me/playlists/{id}` | |
| `POST` | `/api/v1/me/playlists/{id}/tracks` | |
| `DELETE` | `/api/v1/me/playlists/{id}/tracks/{playlist_track_id}` | |

**Dependencies.**

| Depends on | Why |
|---|---|
| `auth::AuthenticatedUser` | identifies the owner; gates ownership checks |
| `catalog::CatalogReadForStreaming` | duration for "counted_as_play"; existence checks |
| `db::PgPool` | owns `library.*` |

`library` does not depend on `storage`, `transcode`, or `streaming`.

**Test seams.** All services generic over their repos; per-service test files
exercise ownership rules (e.g. can't read another user's playlist),
reorder math, and `counted_as_play` threshold.

---

### 6.6 `search`

**Scope.** Single search endpoint over the catalog. Uses Postgres
`tsvector` GIN indexes already defined in `catalog` (§6.1).

**Data ownership.** None (read-only over `catalog`).

**Service.**

```rust
pub struct SearchService { repo: Arc<dyn SearchRepo> }

#[derive(Debug, Clone, Copy)]
pub enum SearchType { Track, Album, Artist, All }

pub struct SearchRequest {
    pub q: String,
    pub kind: SearchType,
    pub page: Page,
}

pub struct SearchResults {
    pub tracks:  Option<PagedResult<TrackHit>>,
    pub albums:  Option<PagedResult<AlbumHit>>,
    pub artists: Option<PagedResult<ArtistHit>>,
}

pub struct TrackHit  { pub track: Track,   pub rank: f32 }
pub struct AlbumHit  { pub album: Album,   pub rank: f32 }
pub struct ArtistHit { pub artist: Artist, pub rank: f32 }

#[async_trait::async_trait]
pub trait SearchRepo: Send + Sync {
    async fn search_tracks (&self, q: &str, page: Page) -> Result<PagedResult<TrackHit>,  sqlx::Error>;
    async fn search_albums (&self, q: &str, page: Page) -> Result<PagedResult<AlbumHit>,  sqlx::Error>;
    async fn search_artists(&self, q: &str, page: Page) -> Result<PagedResult<ArtistHit>, sqlx::Error>;
}
```

Each search uses `plainto_tsquery('english', $1)` with `ts_rank_cd` for the
rank score. The repo also LIMITs to the page and uses a window function or
two-query fan-out (one for hits, one for `COUNT(*) OVER ()`) so the
response carries `total`.

**Why `search` is its own module and not in `catalog`.** Two reasons:

1. Different query shape — search is full-text and cross-entity; the
   `CatalogRepo` is keyed reads. Mixing them muddies the trait surface and
   complicates testing.
2. It is the only foreseeable place we'd swap implementations (e.g. swap
   Postgres FTS for Meilisearch in v2). Keeping it isolated means that swap
   is a single-trait substitution.

`SearchRepo`'s Postgres impl issues `SELECT ... FROM catalog.tracks ...`,
which is the **single** exception to "no cross-schema queries". It's
read-only and well-bounded.

**HTTP endpoint.**

| Method | Path | Auth | Query | Response |
|---|---|---|---|---|
| `GET` | `/api/v1/search` | listener | `q, type=track\|album\|artist\|all, limit, offset` | `200 SearchResults` |

`Cache-Control: public, max-age=60` on responses.

---

### 6.7 `recommendations`

**Scope.** Three v1 endpoints, all derived from data already in the
catalog and library tables.

**Service.**

```rust
pub trait RecommendationsService {
    async fn recently_added(&self, limit: u32) -> Result<Vec<Track>>;
    async fn most_played(&self, window: Window, limit: u32) -> Result<Vec<TrackPlayCount>>;
    async fn for_you(&self, user: &AuthenticatedUser, limit: u32) -> Result<Vec<Track>>;
}
```

`Window ∈ {Last7Days, Last30Days, AllTime}`.

`for_you` algorithm (cheap, deterministic, no ML):

1. Pull the user's top N artists from `library.listen_history`
   (most plays in last 90d).
2. Pull every ready track by those artists.
3. Subtract every track the user has listened to in the last 14d.
4. Sort by per-artist play count desc, then random within tie.
5. Return up to `limit`.

**Data ownership.** None. Reads `catalog.tracks`, `library.listen_history`,
`library.favorite_artists`.

**Cross-module reads.** Adds another narrow port:

```rust
// library/mod.rs (public)
#[async_trait::async_trait]
pub trait HistoryReader: Send + Sync {
    async fn top_artists_for_user(&self, user_id: i64, window: Window, limit: u32)
        -> Result<Vec<(i64, u64)>, sqlx::Error>;   // (artist_id, play_count)
    async fn recent_track_ids(&self, user_id: i64, since: DateTime<Utc>)
        -> Result<Vec<i64>, sqlx::Error>;
    async fn top_tracks_global(&self, window: Window, limit: u32)
        -> Result<Vec<TrackPlayCount>, sqlx::Error>;
}
```

**HTTP endpoints.**

| Method | Path | Auth | Response |
|---|---|---|---|
| `GET` | `/api/v1/recommendations/recently-added?limit=20` | listener | `Vec<Track>` |
| `GET` | `/api/v1/recommendations/most-played?window=30d&limit=20` | listener | `Vec<TrackPlayCount>` |
| `GET` | `/api/v1/recommendations/for-you?limit=20` | listener | `Vec<Track>` |

`Cache-Control: public, max-age=300` on `recently-added` and `most-played`
(both global). `for-you` is `private, max-age=60`.

---

### 6.8 `admin`

**Scope.** Endpoints only an admin should see. Visibility into transcode
jobs and audit log read are the two v1 surfaces.

**Data ownership.** `admin.audit_log`:

```sql
CREATE TABLE admin.audit_log (
    id          BIGSERIAL PRIMARY KEY,
    actor_id    BIGINT NOT NULL REFERENCES auth.users(id) ON DELETE SET NULL,
    action      TEXT NOT NULL,            -- "track.create" | "track.delete" | "user.role_change" | ...
    target_kind TEXT,                     -- "track" | "user" | ...
    target_id   TEXT,                     -- stringified id (may be UUID)
    metadata    JSONB,                    -- arbitrary structured context
    at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_actor ON admin.audit_log(actor_id, at DESC);
CREATE INDEX idx_audit_action ON admin.audit_log(action, at DESC);
```

**`AuditLogger` port** (so any module can emit audit events without
depending on the `admin` module's repo):

```rust
// admin/mod.rs (public)
#[async_trait::async_trait]
pub trait AuditLogger: Send + Sync {
    async fn record(&self, event: AuditEvent) -> Result<(), sqlx::Error>;
}

pub struct AuditEvent {
    pub actor_id: i64,
    pub action: &'static str,
    pub target_kind: Option<&'static str>,
    pub target_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}
```

`catalog::CatalogService` and `ingest::IngestService` are wired with
`Arc<dyn AuditLogger>` and call it on writes. Tests use a `NoopAuditLogger`.

**HTTP endpoints (all require admin).**

| Method | Path | Response |
|---|---|---|
| `GET` | `/api/v1/admin/transcode/jobs?status=&limit=&offset=` | paginated job rows from `underway` joined with `catalog.tracks` |
| `POST` | `/api/v1/admin/transcode/jobs/{track_id}/retry` | re-enqueue for failed tracks |
| `GET` | `/api/v1/admin/audit?actor_id=&action=&limit=&offset=` | paginated audit events |
| `PATCH` | `/api/v1/admin/users/{id}/role` | `{role}` — promote/demote |

---

### 6.9 `ops`

**Scope.** Liveness, readiness, metrics, version. No auth required for
liveness and readiness; metrics is gated by a network ACL at the reverse
proxy (or by a shared secret query parameter in dev).

**HTTP endpoints.**

| Method | Path | Auth | Response |
|---|---|---|---|
| `GET` | `/healthz` | none | `200 ok` (does not depend on DB or S3) |
| `GET` | `/readyz` | none | `200` only if `db.ping()` and `storage.head("readyz-canary")` both succeed within 1s; otherwise `503` with a JSON body listing what failed |
| `GET` | `/version` | none | `{ version, git_sha, built_at }` (baked in at build time) |
| `GET` | `/metrics` | proxy ACL | Prometheus exposition (via `axum-prometheus`) |

Health endpoints have a 1s internal timeout and never block on a long DB
operation; we use `try_acquire` on the pool to detect saturation.

**Implementation note.** `ops` is the only module besides `http` that
*imports* every foundation service (it calls `db.ping`, `storage.head`,
`oidc.jwks_status`, etc.). That's fine because it has no business logic and
no schema.

---

## 7. Consolidated database schema

Schemas: `auth`, `catalog`, `ingest`, `transcode`, `library`, `admin`.
`underway` owns its own schema (default: `underway`).

```sql
-- =============== auth ===============
CREATE SCHEMA IF NOT EXISTS auth;
CREATE TABLE auth.users (
    id              BIGSERIAL PRIMARY KEY,
    oauth_provider  TEXT NOT NULL,                 -- "keycloak"
    oauth_subject   TEXT NOT NULL,
    email           TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('admin','listener')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (oauth_provider, oauth_subject)
);
CREATE INDEX idx_users_email ON auth.users(lower(email));

-- =============== catalog ===============
CREATE SCHEMA IF NOT EXISTS catalog;
-- (artists, albums, tracks — see §6.1)

-- =============== ingest ===============
CREATE SCHEMA IF NOT EXISTS ingest;
-- (upload_sessions — see §6.2)

-- =============== transcode ===============
CREATE SCHEMA IF NOT EXISTS transcode;
-- (outputs — see §6.3)

-- =============== library ===============
CREATE SCHEMA IF NOT EXISTS library;
-- (favorite_tracks/albums/artists, listen_history, playback_positions, playlists, playlist_tracks — see §6.5)

-- =============== admin ===============
CREATE SCHEMA IF NOT EXISTS admin;
-- (audit_log — see §6.8)
```

### 7.1 Migration plan

The first migration in the current tree is destructive. Replace with one
migration per schema, non-destructive, ordered so foreign-key targets exist
before referrers:

```
migrations/
  001_init_auth_schema.sql
  002_init_catalog_schema.sql              # artists, albums, tracks (NEW shape)
  003_init_ingest_schema.sql
  004_init_transcode_schema.sql
  005_init_library_schema.sql
  006_init_admin_schema.sql
  007_install_underway.sql                 # CREATE SCHEMA underway; \i underway.sql
```

Migrations from the *current* tree are NOT preserved as-is — they reflect a
different schema (single `metadata.tracks` table with `file_path`/`status`
inline). For a brownfield deploy:

- If the current DB has zero data: drop, re-init with the new migrations.
- If the current DB has data: write a one-shot data migration script (out
  of scope for this document) that copies `metadata.*` → `catalog.*` and
  reconstructs `transcode.outputs` from existing `file_path` values.

---

## 8. HTTP API surface (consolidated)

```
/healthz                                                       GET
/readyz                                                        GET
/version                                                       GET
/metrics                                                       GET

/api/v1/catalog/artists                                        GET, POST(admin)
/api/v1/catalog/artists/{id}                                   GET
/api/v1/catalog/artists/{id}/albums                            GET, POST(admin)
/api/v1/catalog/albums/{id}                                    GET
/api/v1/catalog/albums/{id}/tracks                             GET
/api/v1/catalog/tracks/{id}                                    GET, PATCH(admin), DELETE(admin)

/api/v1/ingest/uploads                                         POST(admin)
/api/v1/ingest/uploads/{id}                                    GET(admin), DELETE(admin)
/api/v1/ingest/uploads/{id}/confirm                            POST(admin)

/api/v1/streaming/tracks/{id}/master.m3u8                      GET(listener)
/api/v1/streaming/tracks/{id}/{variant}/index.m3u8             GET(listener)
/api/v1/streaming/tracks/{id}/{variant}/init.mp4               GET(listener)  # proxy only
/api/v1/streaming/tracks/{id}/{variant}/{segment}              GET(listener)  # proxy only

/api/v1/me/favorites/{kind}                                    GET
/api/v1/me/favorites/{kind}/{id}                               POST, DELETE
/api/v1/me/history                                             GET, POST
/api/v1/me/stats/top-tracks                                    GET
/api/v1/me/playback/{track_id}                                 PUT
/api/v1/me/playback/recent                                     GET
/api/v1/me/playlists                                           GET, POST
/api/v1/me/playlists/{id}                                      GET, PATCH, DELETE
/api/v1/me/playlists/{id}/tracks                               POST
/api/v1/me/playlists/{id}/tracks/{playlist_track_id}           DELETE

/api/v1/search                                                 GET(listener)
/api/v1/recommendations/recently-added                         GET(listener)
/api/v1/recommendations/most-played                            GET(listener)
/api/v1/recommendations/for-you                                GET(listener)

/api/v1/admin/transcode/jobs                                   GET(admin)
/api/v1/admin/transcode/jobs/{track_id}/retry                  POST(admin)
/api/v1/admin/audit                                            GET(admin)
/api/v1/admin/users/{id}/role                                  PATCH(admin)
```

All write endpoints accept and return `application/json`. All playlist
endpoints return `application/vnd.apple.mpegurl`. All proxied segment
endpoints return `video/iso.segment` and support `Range`.

---

## 9. Process topology, configuration, deployment

### 9.1 Process model (v1)

A single binary, started as:

```
$ music-backend --config /etc/soundzone/config.toml
```

The binary spawns:

- the Axum HTTP server (tokio task),
- the `underway` worker pool, sized from `TranscodeConfig::worker_size`,
- a cron-like task that calls `IngestService::expire_stale_uploads` every
  `LimitsConfig::upload_session_cleanup_interval_secs` (default 900s).

All tasks share the `AppState`; all read from the same `Config`.

Graceful shutdown: a `tokio_util::sync::CancellationToken` is signalled on
`SIGTERM`/`SIGINT`; the HTTP server uses `axum::serve(..).with_graceful_shutdown`;
the worker exits its loop when the token fires and waits for in-flight
handlers up to `LimitsConfig::shutdown_timeout_secs` (default 30s).

### 9.2 Splitting later

Because the worker speaks to `underway` via Postgres and the API speaks to
the worker only through `JobQueue::enqueue`, splitting the binary later is
a deployment change. The intended path:

- Add a `--role api|worker|both` CLI flag (default `both`).
- `main.rs` picks which subsystems to spawn based on the role.
- No module code changes.

### 9.3 Configuration

```toml
# /etc/soundzone/config.toml

[server]
host = "0.0.0.0"
port = 3000

[database]
url = "postgres://soundzone:***@localhost:5432/soundzone"
max_connections = 10

[s3]
endpoint = "http://localhost:9000"
region = "us-east-1"
access_key = "..."
secret_key = "..."
bucket = "soundzone"
use_path_style = true

[jwt]
# Currently unused for issuance (Keycloak issues the access token); kept here
# in case we need to issue our own backend-scoped tokens later.
secret = ""
expiration_seconds = 900

[oidc]
issuer_url = "https://kc.example.com/realms/soundzone"
audience = "soundzone-backend"
jwks_cache_ttl_secs = 600
admin_email_allowlist = ["me@example.com"]

[streaming]
default_delivery = "presigned"
segment_presign_ttl_secs = 300
enable_proxy_fallback = true

[transcode]
worker_size = 2
ffmpeg_path = "/usr/bin/ffmpeg"
ffprobe_path = "/usr/bin/ffprobe"
max_attempts = 3
originals_prefix = "originals/"
hls_prefix = "hls/"
tmp_dir = "/var/lib/soundzone/tmp"

[[transcode.ladder]]
name = "low"
bitrate_kbps = 96
target_segment_secs = 6

[[transcode.ladder]]
name = "mid"
bitrate_kbps = 160
target_segment_secs = 6

[[transcode.ladder]]
name = "high"
bitrate_kbps = 256
target_segment_secs = 6

[limits]
max_json_body_bytes = 1_048_576
request_timeout_secs = 30
cors_allowed_origins = ["https://music.example.com"]
upload_session_cleanup_interval_secs = 900
shutdown_timeout_secs = 30

[telemetry]
format = "json"   # "json" | "pretty"
default_filter = "info,sqlx=warn,aws_smithy_http=warn"
```

A `config.toml.example` ships in the repo; the real `config.toml` is
gitignored.

### 9.4 Container image

Multi-stage Dockerfile:

1. `rust:1.84-slim` build stage — `cargo build --release --locked`.
2. `debian:bookworm-slim` runtime — install `ffmpeg`, copy the binary,
   `USER soundzone`, `CMD ["music-backend", "--config", "/etc/soundzone/config.toml"]`.

`docker-compose.yml` brings up Postgres, MinIO, the backend, and (for dev)
a Keycloak container.

---

## 10. Testing strategy

### 10.1 Test pyramid

```
       /\        e2e (manual + 1 gated smoke; tests/e2e_*.rs with RUN_E2E=1)
      /  \
     /----\      integration (axum::Router + sqlx::test + testcontainers MinIO + Keycloak stub)
    /      \
   /--------\    service-level (each domain's service.rs with fakes; no I/O)
  /          \
 /------------\  unit (pure functions: playlist render, ladder parse, role mapping, error mapping)
```

### 10.2 What lives where

| Test kind | Location | Tooling | Runs on every PR |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` inside each `*.rs` | `cargo test` | yes |
| Service-level | `src/<module>/service.rs` (in the same `#[cfg(test)]` block) | `cargo test` with fakes | yes |
| Integration | `tests/<module>_test.rs` | `axum::Router`, `tower::ServiceExt`, `sqlx::test`, `testcontainers` | yes |
| End-to-end | `tests/e2e_*.rs` (skipped unless `RUN_E2E=1`) | docker compose stack | nightly |

### 10.3 Required fakes (shipped in `#[cfg(test)]` modules)

- `storage::fake::InMemoryObjectStore`
- `auth::oidc::StubVerifier`, `auth::users::FakeUserRepo`
- `jobs::inproc::InMemoryJobQueue`
- `transcode::ffmpeg::FakeTranscoder`
- `ingest::FakeProber`
- One `FakeXxxRepo` per repo trait
- `clock::FixedClock` for any time-sensitive logic

Fakes live in the same module as the trait they implement. This keeps the
trait and its fake co-located and means changes to a trait are obviously
breaking for the fake too.

### 10.4 Concrete test obligations per module

| Module | Minimum tests |
|---|---|
| `config` | parse a sample TOML, env override, validation failures |
| `error` | each variant → expected status + JSON body |
| `auth` | role mapping (admin allowlist + realm role), expired token, bad iss/aud, JWKS rotation |
| `storage` | (against real MinIO via testcontainer) put + head + presign + range get + delete prefix |
| `jobs` | retry policy executes on transient errors, gives up on permanent ones |
| `catalog` | create artist/album, paginated list, update track, delete track triggers cleanup enqueue, admin-only writes |
| `ingest` | reject bad extension, presign issuance, confirm path full flow, expired session rejection, dedupe by sha256 |
| `transcode` | handler: pending → transcoding → ready (happy), pending → failed (permanent ffmpeg error), retry on transient |
| `streaming` | playlist render correctness, `425` on not-ready, presign rewrite, range proxy, `ETag` 304 |
| `library` | favorites idempotency, history `counted_as_play` threshold, playlist reorder, ownership enforcement |
| `search` | pagination + ranking ordering |
| `recommendations` | for-you exclusion of recent tracks, deterministic ordering of ties |
| `admin` | non-admin gets 403, audit-log read |
| `ops` | `/readyz` returns 503 when DB or storage is unreachable |

### 10.5 Tooling enforcement

`Makefile` (currently empty) gains:

```
make fmt          # cargo fmt --all
make lint         # cargo clippy --all-targets --all-features -- -D warnings
make test         # cargo test --all-features
make e2e          # RUN_E2E=1 cargo test --test 'e2e_*'
make migrate      # sqlx migrate run
make run          # cargo run -- --config config.toml
```

A `clippy.toml` bans `println!` and `eprintln!` outside of `src/main.rs`
via `disallowed-macros`. Custom Clippy lints (or a tiny `cargo-deny`
config) enforce:

- no module imports `axum` if its filename is not `routes.rs`, `http/*`, or
  `auth/mod.rs`,
- no module file under `src/<x>/repo.rs` imports another module's `repo.rs`.

(The lint isn't strictly required, but the convention should be reviewed
on every PR.)

---

## 11. Migration from current code

Concrete mapping from today's `src/` to the target shape. Each row is a
bite-sized PR.

| # | Current location | Target location | Action |
|---|---|---|---|
| 1 | `src/main.rs` | unchanged | parse `--config`, install `telemetry::init_tracing`, hand off to `http::build_router` |
| 2 | `src/config.rs` | `src/config/` directory | drop `RedisConfig`; add `OidcConfig`, `StreamingConfig`, `LimitsConfig`, `TelemetryConfig`; validate ladder |
| 3 | `src/state.rs::AppState` | `src/http/mod.rs::AppState` | replace concrete `S3Client` field with `Arc<dyn ObjectStore>`; add `oidc`, `jobs`, `clock` |
| 4 | (new) | `src/error.rs` | introduce `AppError` and `IntoResponse` |
| 5 | (new) | `src/telemetry.rs` | install `tracing-subscriber`; delete every `println!`/`eprintln!` |
| 6 | `src/services/transcode_services.rs` (`get_upload_presigned_url`) | `src/ingest/service.rs::create_upload` | use `Arc<dyn ObjectStore>::presign_put`; remove hard-coded `"soundzone"` |
| 7 | `src/services/metadata.rs::create_track` | `src/ingest/service.rs::confirm_upload` + `catalog::CatalogWriter::create_track_pending` | also fixes duplicate `head_object` (bug #9) |
| 8 | `src/services/transcode_services.rs::get_mp3_duration` | `src/ingest/prober.rs::FfprobeProber` | replace MP3-only path with `ffprobe`; download only header range |
| 9 | `src/services/transcode/queue.rs` | `src/transcode/worker.rs` (`JobHandler<TranscodeJobSpec>`) + `src/jobs/underway_adapter.rs` | replace in-memory mpsc with `underway`; replace `.expect()` with classified retry + `set_track_failed` |
| 10 | `src/services/streaming.rs` | `src/streaming/service.rs` | implement master + variant + segment endpoints; return `AppError::NotReady` instead of generic 500 (bug #10) |
| 11 | `src/routes/metadata.rs::get_artists` (stub) | `src/catalog/routes.rs::list_artists` | call the actual repo (bug #1) |
| 12 | `src/routes/metadata.rs::create_album` (empty body) | `src/catalog/routes.rs::create_album` | repo uses `RETURNING`; service returns the row (bug #2) |
| 13 | `src/repositories/metadata.rs::create_track` (`duration_ms` parameter) | `src/catalog/repo.rs::create_track_pending` | rename to `duration_seconds` (bug #11) |
| 14 | `migrations/2026050805*.sql` | `migrations/00*_init_<schema>.sql` | new non-destructive schema split; existing data path documented separately (bug #13) |
| 15 | `tower_http::cors::CorsLayer::permissive()` | `src/http/middleware.rs` | typed origin allowlist (bug #8) |
| 16 | `Cargo.toml` `mp3-duration` | (removed) | not needed once `ffprobe` is in place |
| 17 | `Cargo.toml` adds | `tracing`, `tracing-subscriber`, `async-trait`, `thiserror`, `anyhow`, `chrono`, `tokio-util`, `axum-prometheus`, `validator`, `jsonwebtoken`, `reqwest`, `utoipa`, `utoipa-swagger-ui`, `testcontainers`, `wiremock`, `sha2`, `bytes` |

Phase mapping (loose alignment with REQUIREMENTS §7):

- **Phase 0 (hygiene).** Rows 4, 5, 15, plus bug fixes 1, 2, 11.
- **Phase 1 (auth).** Module 5.7 in full + `auth.users` migration + row 3 partial.
- **Phase 2 (catalog).** Module 6.1 + remaining read endpoints + pagination.
- **Phase 3 (transcode).** Rows 8, 9 + module 6.3 in full + `transcode.outputs` migration.
- **Phase 4 (HLS playback).** Module 6.4 in full + row 10.
- **Phase 5 (library).** Module 6.5 + library schema.
- **Phase 6 (search + recs).** Modules 6.6 + 6.7 + GIN indexes for `albums.title`, `artists.name`.
- **Phase 7 (ops polish).** Module 6.9 + audit log + rate limiting + OpenAPI.

---

## 12. Open questions / risks

Items that are *design risks* — places where the design as written above
needs validation before code lands.

1. **Auth substitution.** You indicated you may revise the auth module.
   The interface other modules see (`AuthenticatedUser` extractor + `Role`
   guard + `OidcVerifier` trait) is what they depend on; if the
   substitution preserves those, the other 8 domain modules need no
   changes. Concretely: anything you change inside `src/auth/` is fine;
   any change to those three public items will ripple.
2. **Range-request math on the proxy path** is easy to get wrong, especially
   `Content-Range` headers and `206 Partial Content` semantics. The test
   plan in §10.4 covers it, but plan extra review time.
3. **`underway` API stability.** It's a `0.2.x` crate. The `JobQueue` trait
   in `src/jobs/` insulates the rest of the code, but a major-version bump
   may force changes inside `underway_adapter.rs`.
4. **FFmpeg flag drift.** The exact ffmpeg invocation for fMP4 HLS with
   three audio rungs in one pass is non-trivial. A small fixture-based
   integration test (10-second WAV → assert ladder exists) is mandatory
   before declaring transcoding "done".
5. **Pi I/O budget.** Three parallel ffmpeg encodes plus a Postgres plus a
   MinIO on a Pi 4/5 can saturate disk I/O. The `tmp_dir` placement
   (preferably on the same SSD MinIO uses, or on a tmpfs sized to the
   largest source file) matters for the 2× real-time target.
6. **Schema split for brownfield deploys.** The migration plan in §7.1
   assumes either a green field or a one-off data migration. If there is
   live data in the current `metadata.*` tables in any environment, we owe
   a separate copy script.
7. **`record_play` source of truth.** Recording a play from the
   master-playlist fetch is cheap but can over-count (multiple master
   fetches per playback session) and under-count (HLS clients may cache
   the master). The cleanest source is the client posting to
   `POST /api/v1/me/history` with elapsed time; the master-fetch hook is
   a fallback. Recommend: design ships both, default to client-driven.
