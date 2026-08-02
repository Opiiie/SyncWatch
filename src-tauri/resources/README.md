# libmpv runtime

SyncWatch loads the 64-bit `libmpv-2.dll` dynamically at runtime. Put the DLL in this directory for development, next to `syncwatch.exe` for a packaged application, or set `SYNCWATCH_LIBMPV_PATH` to its absolute path.

Windows builds are linked from the official mpv installation page. Use a build that contains the libmpv development runtime, not only `mpv.exe`.
