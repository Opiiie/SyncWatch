# SyncWatch releases

## One-time GitHub setup

1. Create an immutable prerelease tagged `runtime-v1` and attach the file `libmpv-2.dll`.
2. Keep the runtime asset at `https://github.com/Opiiie/SyncWatch/releases/download/runtime-v1/libmpv-2.dll`.
3. Add repository secrets named `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
4. Keep an offline backup of both the private updater key and its password. The public key is already stored in `src-tauri/tauri.conf.json`.

The pinned runtime URL and checksum are stored in both `scripts/fetch-libmpv.ps1` and `src-tauri/src/mpv_runtime.rs`. When libmpv is replaced, publish a new immutable runtime release and update both locations. Application installers and updater artifacts must remain independent from this DLL.

## Publishing an application update

1. Update the same SemVer version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, then refresh lockfiles.
2. Run the TypeScript build, Rust tests, and a signed Tauri bundle build.
3. Commit the release and push `main`.
4. Create and push a matching tag such as `v0.2.0`.

The `Выпуск SyncWatch` workflow downloads and verifies libmpv, builds the Windows installers, signs the updater artifacts, creates the GitHub Release, and uploads `latest.json`. Installed copies use that file to discover the new version.
