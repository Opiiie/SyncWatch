# SyncWatch

Desktop application for synchronized video watching over a local network or VPN. The UI uses React and Tauri; local playback uses the libmpv C API.

## Development

```text
pnpm install
pnpm tauri dev
```

## libmpv on Windows

SyncWatch dynamically loads a 64-bit `libmpv-2.dll`. This keeps normal Rust and frontend builds independent from a machine-specific import library.

For development, place the DLL at `src-tauri/resources/libmpv-2.dll` or provide an absolute path through `SYNCWATCH_LIBMPV_PATH`.

Windows installers and normal application updates do not contain libmpv. Shortly after startup SyncWatch checks the pinned runtime in its local application-data directory. When it is missing, the application first migrates a compatible DLL from an older installation and otherwise downloads it once from the immutable `runtime-v1` GitHub Release. Both downloaded and manually selected files must match the pinned SHA-256 checksum before they are activated. A failed download can be retried from the in-app notice, and an offline user can select a compatible `libmpv-2.dll` manually.

The official mpv installation page links maintained Windows builds: <https://mpv.io/installation/>. Select a development/libmpv archive containing `libmpv-2.dll`; an archive containing only `mpv.exe` is insufficient.

When the DLL is unavailable or incompatible, the rest of the room remains usable while the runtime notice offers recovery actions. Player creation waits for an already running runtime preparation instead of starting another download.

## Player controls

- Play, pause, seek and playback speed are authoritative room commands and are applied to libmpv after the server broadcasts the new state.
- Volume is local, persisted on the current computer, and never sent to the room.
- Audio track and subtitle selection are local to each participant.
- Preferred audio and subtitle tracks are remembered by language and title and are reapplied when a matching track exists in the next playlist item.
- Fullscreen is local to the application window.
- The progress bar displays libmpv's current position and media duration and sends seek commands through the room.
- Playback shortcuts are stored locally by physical keyboard code, so they do not change with the active keyboard layout. They are active only while focus is inside the player.
- Previous and next playlist items can be selected from the player controls or with configurable Page Up / Page Down shortcuts.
- The seek step is configurable from 1 to 10 seconds; Escape always exits fullscreen.

The controls fade out after two seconds without pointer or keyboard activity and immediately when the pointer leaves the player.

The native video surface keeps the exact player dimensions. A separate clipped black backdrop sits underneath it to cover Windows/DPI composition seams without stretching or cropping the video, and both native surfaces are hidden or clipped when the player scrolls outside the application viewport.

## Network playback

The host registers local media paths directly with the Rust server; these paths are never included in WebSocket messages. After joining, a viewer receives a random room media token and opens the active item in libmpv through `GET /media/{roomCode}/{itemId}?token=...` on the same automatically selected port as the WebSocket server.

The media endpoint streams the file, supports single HTTP byte ranges, returns `206 Partial Content`, and advertises `Accept-Ranges: bytes`. Invalid tokens and media identifiers return `404`. libmpv handles network buffering, seeking, container demuxing, and embedded audio and subtitle tracks on each participant's computer.

Room media is paced in 256 KiB chunks through one shared adaptive bandwidth budget. The server continuously updates its bitrate target when the file duration or viewer count changes, keeps additional headroom for bitrate spikes, and adjusts unused capacity from the worst current viewer RTT instead of oscillating between individual reports. All viewers share the budget, preventing several simultaneous streams from independently saturating the host's upload without starving high-bitrate files. libmpv uses a bounded 90-second/512 MiB forward cache with hysteresis and waits for a useful reserve before resuming after an underrun.

Clock synchronization keeps a rolling sample window and uses the least-delayed sample for the clock offset, so temporary network queues do not move the room timeline. A new latency sample does not trigger a seek by itself. Playback drift is checked separately: large drift is corrected exactly, while small drift is removed with a temporary speed adjustment.

External `.srt`, `.ass`, `.ssa`, `.vtt` and `.sub` files with the same base name as a video are detected automatically. A host can also attach files to the active playlist item from the subtitle controls. Attachments are persisted with saved sessions and served to viewers through authenticated `/subtitles/{roomCode}/{itemId}/{subtitleId}` requests. Each viewer's selected subtitle track remains local.

## Finding rooms

The host advertises active rooms inside the local network and VPN adapters. A viewer can either enter a room code or open the room browser; addresses and ports are resolved automatically and are not shown in the interface. Discovery uses UDP port `45892`, directed broadcast and the local multicast group `239.255.77.77`. Only the room code, host display name, participant count, video presence and server port are advertised. Media tokens, file names and local paths are never included.

Windows Firewall must allow SyncWatch to receive local UDP and TCP connections. Radmin VPN discovery depends on broadcast or multicast support in the installed network configuration; the application tries both mechanisms.

### Testing with one computer

Debug builds accept `--allow-multiple-instances`, while normal builds remain single-instance. Start two copies of `src-tauri/target/debug/syncwatch.exe` with that flag, create a room in the first one, and use either code lookup or the room browser in the second one. Loopback discovery is enabled specifically for this test. This verifies two independent WebSocket clients, HTTP Range streaming and playback through a second libmpv instance, but does not verify Windows Firewall or real LAN/VPN routing.

## Updates and releases

Packaged builds check the latest GitHub Release shortly after startup. When an update is available, the user can download and install it from a small in-app notice. Installation is disabled while the user is connected to a room so playback is never interrupted unexpectedly. Update bundles are signed with the Tauri updater key and installed quietly on Windows; SyncWatch still shows its own download progress before the required restart.

`libmpv-2.dll` is not stored in Git or application releases. Local developers can restore the pinned DLL from the `runtime-v1` GitHub prerelease by running `scripts/fetch-libmpv.ps1`; the script rejects a download whose SHA-256 checksum does not match. Application releases fetch the same runtime on the user's computer only when needed. See `RELEASING.md` for the release procedure.

## Playlist

A host can create an empty room, add several videos later, remove and reorder them, or select another item without recreating the session. Playlist metadata, per-item progress and the active item are synchronized through the room; absolute local paths remain on the host computer. Selecting an item resumes its saved position, and reaching the end selects the next item automatically when one exists.

Non-empty host sessions are persisted in local application storage with their playlist, local file paths and last known positions. The home screen can recreate or delete one of the most recently used sessions after an application or computer restart. At most 30 sessions are retained.
