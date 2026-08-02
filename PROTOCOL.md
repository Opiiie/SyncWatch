# SyncWatch room protocol v2

The local Tauri server binds to `0.0.0.0` on an operating-system-selected available port and exposes:

- `GET /health` — health check;
- `GET /media/{roomCode}/{itemId}?token=...` — authenticated media stream with HTTP Range support;
- `GET /subtitles/{roomCode}/{itemId}/{subtitleId}?token=...` — authenticated external subtitle file;
- `WS /ws` — room protocol.

JSON messages use a discriminated envelope: `{ "type": string, "payload": object }`.

The first client message must be `create_room` or `join_room`. Both carry `version: 2`, a room code, participant id and display name. `create_room` also supplies a random media access token. The server confirms membership with `room_snapshot`, which gives joined viewers that token so libmpv can open room media.

After joining, the client can send:

- `playback_command` with action `play`, `pause` or `seek` and a position in seconds;
- `playback_rate` with the room-wide playback speed and current position;
- `playlist_update` with the host-owned ordered media metadata, per-item progress and active item;
- `ping` with the client's Unix time in milliseconds.
- `latency_report` with the measured WebSocket round-trip time in milliseconds.

The server broadcasts:

- `playback_state` — authoritative playback state with a monotonically increasing revision;
- `playlist_state` — the ordered playlist and active media item after a host update;
- `participant_count` — the current number of connections;
- `participant_joined` — the id and display name of a newly connected participant;
- `participant_list` — the current ordered list with display names, host markers and last measured pings;
- `room_closed` — the host disconnected and every viewer must leave the room;
- `pong` — server time used for automatic and manual clock-offset correction;
- `error` — stable machine-readable code plus a user-facing message.

Only the host may update the playlist. Local file paths are never transmitted: each item contains a generated id, display name, saved position, duration and safe metadata for attached external subtitles. The server clamps progress values and resumes the authoritative playback position when the active item changes. Volume, audio track and subtitle selection remain local and are intentionally absent from the room protocol. Playback speed is part of the authoritative room state and is applied to every participant.

The host registers the item-id-to-path mapping through a local Tauri command rather than through WebSocket. The HTTP endpoint checks that the room is active, the item belongs to its current playlist, and the bearer token matches. It supports one `bytes` range per request, including open-ended and suffix ranges; multipart ranges are rejected with `416 Range Not Satisfiable`.

Media responses share a room-level adaptive bandwidth budget. Its lower working bound is derived from file size, known duration, and the number of viewers, with extra headroom for container overhead and bitrate peaks. This keeps Range requests made after seeking from being throttled below the rate required for continuous playback while retaining RTT-based congestion protection.

## Local room discovery

Room discovery is a separate UDP protocol on port `45892`. A viewer sends a versioned JSON query to every active IPv4 adapter using its directed broadcast address, limited broadcast and multicast group `239.255.77.77`. The query may contain a room code; without one, every active room is returned for the browser.

The host replies directly to the source address. A response contains only the room code, host display name, participant count, whether the playlist has an active video, and the shared HTTP/WebSocket port. The viewer uses the response packet's source IP instead of trusting an advertised address. Access tokens, media metadata and local paths are not part of discovery. The room's normal WebSocket handshake remains mandatory after discovery.
