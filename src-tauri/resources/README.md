# Video runtime

SyncWatch loads the 64-bit `libmpv-2.dll` and ANGLE runtime dynamically. These binaries are intentionally excluded from Git history and application installers. For local development, `libmpv-2.dll` can be placed in this directory; `SYNCWATCH_LIBMPV_PATH` and `SYNCWATCH_ANGLE_PATH` can also point to exact runtime files.

Missing components are downloaded from the `runtime-v1` GitHub release and verified against pinned SHA-256 checksums before use. Existing verified files are reused after application updates.
