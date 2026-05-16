# Music Streaming Service - Requirements Checklist

**Project Scope**: Home music streaming service for small teams (<100 users) with multi-format support (MP3, FLAC, AAC, WAV), user authentication, and recommendations.

**Last Updated**: May 15, 2026

---

## 🎯 FUNCTIONAL REQUIREMENTS

### 1. CORE AUDIO UPLOAD & STORAGE
- [x] Upload MP3 files via presigned URLs
- [x] Store files in AWS S3
- [ ] **Validate audio format (MP3 only on upload)**
- [ ] Extract audio metadata (bitrate, sample rate, channels)
- [ ] Extract and store album art/cover image
- [ ] Support for large file uploads (streaming/multipart)

### 2. AUDIO TRANSCODING & MULTI-FORMAT SUPPORT
- [ ] Transcode uploaded MP3 to multiple formats:
  - [ ] MP3 @ 128kbps (low quality)
  - [ ] MP3 @ 192kbps (medium quality)
  - [ ] MP3 @ 320kbps (high quality)
  - [ ] FLAC (lossless)
  - [ ] AAC @ 128kbps
  - [ ] WAV (optional - high storage)
- [ ] Queue-based transcoding system (already has skeleton)
- [ ] Background job processing with error handling
- [ ] Transcode job status tracking and monitoring
- [ ] Retry logic for failed transcodes

### 3. METADATA MANAGEMENT
- [x] Create/store Artist info
- [x] Create/store Album info (with artist relationship)
- [x] Create/store Track info (title, duration, artist, album)
- [ ] Track number/order within album
- [ ] Genre tagging
- [ ] Album release date
- [ ] Composer/Producer info (optional)
- [ ] Lyrics storage (optional)

### 4. USER AUTHENTICATION & ACCOUNTS
- [ ] User registration (username/email + password)
- [ ] User login with JWT or session tokens
- [ ] Password hashing and security
- [ ] User profile management
- [ ] Password reset functionality

### 5. USER MUSIC LIBRARY FEATURES
- [ ] Favorites/Likes system:
  - [ ] Add/remove favorite tracks
  - [ ] Add/remove favorite albums
  - [ ] Add/remove favorite artists
  - [ ] List user's favorite tracks
- [ ] Listen history tracking:
  - [ ] Record track plays with timestamp
  - [ ] Play count per user per track
  - [ ] Recently played list
  - [ ] Last played timestamp for resume functionality

### 6. PLAYBACK & STREAMING
- [ ] Get presigned download URLs for streaming (already started)
- [ ] Support streaming all 4+ formats with quality selection
- [ ] Endpoint: GET `/api/v1/streaming/tracks/{track_id}/presigned-url?quality=mp3_320`
- [ ] Return quality metadata (format, bitrate, file size)
- [ ] Direct streaming (not just presigned URLs) - optional but recommended
- [ ] Resume playback from last position
- [ ] Range request support (for seeking in audio files)

### 7. SEARCH & DISCOVERY
- [ ] Full-text search across:
  - [ ] Track names
  - [ ] Album names
  - [ ] Artist names
  - [ ] Genre
- [ ] Filter by artist
- [ ] Filter by album
- [ ] Filter by genre
- [ ] Pagination for search results
- [ ] Sort by popularity, release date, alphabetical

### 8. RECOMMENDATIONS ENGINE
- [ ] Basic recommendations based on:
  - [ ] User's favorite tracks (similar artists)
  - [ ] User's listen history (trending in same genre)
  - [ ] Collaborative filtering (users with similar taste)
  - [ ] New releases from followed artists
- [ ] Recommend endpoint: `GET /api/v1/recommendations`
- [ ] Personalized "Discover" page

### 9. LIBRARY MANAGEMENT
- [ ] Playlists:
  - [ ] Create custom playlists
  - [ ] Add/remove tracks from playlists
  - [ ] Reorder tracks in playlists
  - [ ] Delete playlists
  - [ ] List user's playlists
  - [ ] Public/Private playlists (optional)

### 10. CONTENT DELIVERY
- [ ] Endpoint to list all artists
- [ ] Endpoint to list all albums
- [ ] Endpoint to list all tracks
- [ ] Endpoint to get artist details + albums
- [ ] Endpoint to get album details + track list
- [ ] Endpoint to get track details

---

## ⚙️ NON-FUNCTIONAL REQUIREMENTS

### 1. PERFORMANCE
- [ ] Average API response time < 200ms
- [ ] Support concurrent streaming of 50+ users
- [ ] Database queries optimized with proper indexes
- [ ] Caching layer for frequently accessed data (Redis recommended)
- [ ] CDN/edge caching for audio files
- [ ] Lazy loading for UI (pagination)

### 2. RELIABILITY & AVAILABILITY
- [ ] 99% uptime SLA
- [ ] Graceful error handling and recovery
- [ ] Automatic retry for transient failures
- [ ] Health check endpoints
- [ ] Database connection pooling
- [ ] Circuit breaker pattern for external services

### 3. SECURITY
- [ ] HTTPS/TLS for all communications
- [ ] JWT token expiration and refresh
- [ ] Input validation on all endpoints
- [ ] SQL injection prevention (using SQLx prepared statements ✓)
- [ ] Rate limiting per user/IP
- [ ] CORS configuration (currently permissive - needs hardening)
- [ ] File upload validation (virus scanning optional)
- [ ] Secure storage of sensitive data (API keys, secrets)
- [ ] Audit logging for user actions

### 4. SCALABILITY
- [ ] Horizontal scaling capability
- [ ] Stateless API design
- [ ] Database replication for read scaling
- [ ] Async task processing (transcoding, recommendations)
- [ ] Message queue for job distribution (optional)
- [ ] Load balancing ready

### 5. MAINTAINABILITY
- [ ] API documentation (Swagger/OpenAPI)
- [ ] Structured logging (tracing setup exists, needs implementation)
- [ ] Error codes and messages standardized
- [ ] Code comments for complex logic
- [ ] Database migration strategy
- [ ] Monitoring and alerting setup

### 6. DATA INTEGRITY
- [ ] Database transaction management
- [ ] Orphaned file cleanup
- [ ] Duplicate detection (same track uploaded twice)
- [ ] Cascade delete handling (artist → albums → tracks)
- [ ] Data backup strategy

### 7. STORAGE & BANDWIDTH
- [ ] S3 storage optimization (lifecycle policies)
- [ ] Bandwidth cost management
- [ ] Delete old versions/formats
- [ ] Compression for non-audio files

### 8. DEPLOYMENT
- [ ] Docker containerization
- [ ] CI/CD pipeline
- [ ] Environment configuration (dev/staging/prod)
- [ ] Database migrations automation
- [ ] Rollback strategy
- [ ] Monitoring dashboard

---

## 📊 DATABASE REQUIREMENTS

### Current Status:
- [x] Artist table created
- [x] Album table created
- [x] Track table created
- [x] Track status column added
- [x] Upload ID tracking added
- [ ] **User table** - MISSING
- [ ] **User favorites table** - MISSING
- [ ] **User listen history table** - MISSING
- [ ] **Playlist table** - MISSING
- [ ] **Playlist tracks junction table** - MISSING
- [ ] **Transcoded versions table** - MISSING (need to track different quality files)
- [ ] **Genre table** - MISSING
- [ ] **Track genre junction table** - MISSING

### Additional Indexes Needed:
- [ ] User login index (email/username)
- [ ] Listen history timestamp index
- [ ] Favorite tracks index
- [ ] Genre search index

---

## 🔧 CURRENT CODEBASE STATUS

### ✅ Already Implemented:
- Basic project structure (Axum web framework)
- S3 integration with presigned URLs
- MP3 duration extraction
- Artist/Album/Track metadata endpoints (create only)
- Database schema (partial)
- Transcoding queue skeleton
- Configuration management (figment)
- CORS and request size limiting

### ⚠️ Partially Implemented:
- Transcoding queue (exists but doesn't actually transcode - just copies files)
- Streaming endpoint (returns presigned URL, but no quality selection)
- Track status tracking

### ❌ Not Implemented:
- User authentication system
- Search functionality
- Recommendations engine
- Favorites/Likes system
- Listen history
- Playlists
- Audio format conversion (needs FFmpeg integration)
- Input validation
- Error handling middleware
- Logging/monitoring
- API documentation

---

## 🎬 RECOMMENDED IMPLEMENTATION ORDER

1. **Phase 1 - Authentication & Core Security**
   - User registration/login endpoints
   - JWT token management
   - Input validation middleware
   - Error handling middleware

2. **Phase 2 - Audio Transcoding**
   - FFmpeg integration
   - Implement actual transcoding (not just copying)
   - Add quality selection endpoint
   - Transcode job monitoring

3. **Phase 3 - User Library Features**
   - Favorites/Likes endpoints
   - Listen history tracking
   - User profile endpoints

4. **Phase 4 - Search & Discovery**
   - Full-text search implementation
   - Genre tagging system
   - Filter endpoints

5. **Phase 5 - Recommendations**
   - Basic recommendation algorithms
   - Recommendation endpoint
   - Trending/popular tracks

6. **Phase 6 - Polish & Optimization**
   - Caching layer (Redis)
   - Performance optimization
   - API documentation
   - Monitoring/alerting

---

## 📝 NOTES

- Current request body limit: 10MB (suitable for MP3 uploads)
- Current request timeout: 30 seconds
- S3 bucket name in code: "soundzone"
- Database: PostgreSQL with migrations
- Note: CORS is currently permissive (`CorsLayer::permissive()`) - should be restricted in production

---

## ❓ CLARIFICATIONS CONFIRMED WITH USER

✅ Multi-format support: MP3, FLAC, AAC, WAV  
✅ User features: Authentication, Favorites, Listen History  
✅ Scale: Personal/Small team (<100 users)  
✅ Search: Full-text search required  
✅ Recommendations: Basic algorithm needed
