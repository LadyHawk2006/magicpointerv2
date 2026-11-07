# Project Guidelines

## Current Project State

Magic Pointer v2 is a user-triggered Rust desktop application for selecting
screen content on a Hyprland Wayland session, asking Google's Gemini API about
it, and viewing the streamed answer in a GTK chat window. Work against the
implemented behavior; do not treat proposed architecture or future features as
existing functionality.

The application currently:

- Uses Hyprland IPC for cursor position, window/client geometry, titles, and
  monitor/workspace information.
- Uses GTK4 Layer Shell for window/region/full-monitor selection, the prompt
  composer, and annotation modal; captures selected pixels into memory with
  libwayshot.
- Sends user-selected image(s) and text to Gemini over HTTPS using reqwest and
  SSE streaming. There is no local model, daemon, or persistent chat history.
- Displays responses in a regular GTK4 window. The compositor controls its
  placement; it is not guaranteed to appear beside the selection.
- Stores preferences and API-key metadata in JSON and API-key values in Secret
  Service using `secret-tool`. The app intentionally invokes that executable;
  it is not a zero-subprocess application.
- Creates and updates a managed Hyprland Lua module for the chat-window rule
  and Home key binding. It appends a `require("magicpointerv2")` line to the
  user's `hyprland.lua` only when missing, and reloads Hyprland when managed
  configuration changes.
- Supports saved model attempt ordering, request-generation settings, and
  connection/response/stream-idle timeouts. Spatial hardware context reads the
  installed Linux distribution from os-release files instead of assuming
  Arch.

See [README.md](./README.md) for user-facing setup and behavior, and
[description.txt](./description.txt) for the detailed scope, architecture,
implementation map, and technology overview. Keep those descriptions aligned
with code changes.

## Architecture and Boundaries

- `src/main.rs` orchestrates startup, settings/credential setup, selection,
  capture, prompt, and chat.
- `src/selection_overlay/` implements selection modes, Hyprland window
  targeting, cycling, geometry, snapping, and Cairo rendering.
- `src/hypr_ipc.rs` queries Hyprland. `src/screencapture.rs` captures a logical
  screen rectangle with libwayshot.
- `src/prompt_overlay/` implements cursor-adjacent placement, prompt entry,
  quick actions, and in-memory annotation.
- `src/ui/` builds the normal GTK chat window, attachment flow, Markdown
  presentation, and response indicator.
- `src/ai_agent.rs` owns image encoding, in-memory conversation history,
  Gemini request/fallback behavior, and SSE parsing.
- `src/settings/` owns settings persistence, Secret Service access, appearance,
  opt-in spatial context, Hyprland Lua generation/installation, and the
  collapsible settings UI. `src/auth.rs` resolves environment and saved keys.
- `src/styles/` bundles GTK CSS at compile time. The user-provided CSS setting
  is separate.

Keep GTK widget access on the GTK/GLib main thread. Run network work on the
existing Tokio runtime and deliver response chunks to the UI through channels
or the established GLib integration; do not block the GTK event loop.
Hyprland and GTK Layer Shell are current platform dependencies, not optional
abstractions. Do not describe the regular response window as a layer-shell
overlay.

## Implementation Conventions

- Follow existing Rust patterns and module ownership. Prefer focused changes
  that preserve current UX and error handling.
- Use `cargo fmt --all` for formatting. Keep types explicit and handle
  fallible operations with contextual errors or established logging; avoid
  silent fallbacks and broad catches.
- Keep credentials, captured image contents, and sensitive user context out of
  logs. Preserve the current separation between settings metadata and keyring
  secret values.
- Captures and chat history are in memory. Do not add disk persistence for
  screenshots or conversations unless explicitly requested.
- Spatial context is opt-in and disabled by default. Preserve that default and
  do not attach extra context unless the relevant settings enable it.
- `models.txt` is a bundled model-picker catalog; the user's ordered model
  attempts and generation/timeout settings are persisted in `AppSettings`.
- Hyprland response-window settings are persisted in `AppSettings`; position
  presets are calculated from the focused monitor and its reserved work area
  using Hyprland IPC. Keep the application-managed file
  `$XDG_CONFIG_HOME/hypr/magicpointerv2.lua` (or
  `~/.config/hypr/magicpointerv2.lua` when unset) synchronized with those
  controls. Match chat sizing rules by initial title as well as application
  class, and keep Settings in its separate centered rule.
- Do not replace or rewrite unrelated user Hyprland configuration. The
  integration only appends its own `require("magicpointerv2")` line when
  missing; generated rule changes belong in `magicpointerv2.lua`.
- Spatial context is opt-in. The detected OS name comes from `/etc/os-release`
  or `/usr/lib/os-release`; do not replace it with a fixed distribution name.
- Keep documentation accurate: environment keys precede saved keys; saved
  primary key precedes saved fallbacks; model fallback behavior is implemented
  in `src/ai_agent.rs`.

## Build and Test

Run the smallest relevant validation for the change. Common project checks:

```sh
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
```

The test suite includes unit tests for selection geometry, prompt positioning,
settings normalization, model-catalog filtering, key ordering, OS-release
parsing, spatial context, Hyprland rule/position generation, image preparation,
and SSE parsing. UI execution and screen-capture validation require a graphical
Wayland session with Hyprland, GTK4 Layer Shell, and the required screencopy
protocol.

Builds require a Rust toolchain supporting edition 2024, Cargo, a C toolchain,
pkg-config, GTK4 and GTK4 Layer Shell development libraries, and the native
dependencies of the Rust crates. Runtime keyring use additionally requires
`secret-tool` and an available Secret Service. Do not add dependencies or
tooling unless the requested work needs them.
