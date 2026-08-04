# libmpv runtime

SyncWatch loads the 64-bit `libmpv-2.dll` dynamically at runtime. The DLL is intentionally excluded from Git history. Run `scripts/fetch-libmpv.ps1` to restore the pinned runtime for development and release builds. You can also set `SYNCWATCH_LIBMPV_PATH` to its absolute path.

The download is verified against its pinned SHA-256 checksum before it is used. Packaged builds include the DLL automatically, so application users do not need to install it separately.
