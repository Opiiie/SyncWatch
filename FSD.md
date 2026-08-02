# Frontend structure (FSD)

The UI follows Feature-Sliced Design from broad to narrow responsibility:

- `app` — composition, global styles and providers;
- `pages` — complete routes/screens;
- `widgets` — larger assembled interface blocks;
- `features` — user actions such as creating and joining a room;
- `entities` — domain models such as a session;
- `shared` — reusable UI primitives and framework-independent helpers.

Imports must point downward in this list. Network transport and the future Tauri/libmpv bridges will be added behind feature/entity interfaces rather than being called directly by UI components.
