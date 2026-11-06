# AGENTS.md

## Project Vision & Overview
**Project Codename:** `magicpointerv2`  
**Target Environment:** Arch Linux | Hyprland (Wayland)  
**Core Goal:** Build an intent-aware, spatial desktop intelligence pipeline inspired by multimodal vision overlays (e.g., Google's Magic Pointer).

The application is implemented in **Rust**. It queries compositor state via IPC, provides GTK4 Wayland layer-shell surfaces for selection and the cursor-adjacent prompt composer, captures visual bounding boxes directly from compositor buffers into RAM via `libwayshot`, and streams multimodal payloads to Gemini. The response is shown in a regular GTK window managed by Hyprland. OAuth browser launch and Secret Service keyring integration currently use desktop command-line utilities, so the complete application is not zero-subprocess.

---

## System Architecture Blueprint

```
[ System Trigger (Wiggle detection / Hotkey / Script) ]
           │
           ▼
┌───────────────────────────────────────────────────────────┐
│ magicpointerv2 Binary Invocation                          │
│                                                           │
│  1. Spatial Selection (GTK4 Layer-Shell Selection Canvas) │
│     └── Fullscreen transparent overlay                    │
│     └── Interactive box drag, corner highlights, esc exit │
│                                                           │
│  2. In-Memory Frame Capture (libwayshot)                  │
│     └── Direct wlr-screencopy to RAM buffer (image crate) │
│     └── No screenshot file required                      │
│                                                           │
│  3. Cursor-Adjacent Prompt (GTK4 Layer-Shell)             │
│     └── Prominent thumbnail, question field, send         │
│     └── Position near cursor; flip/clamp to monitor bounds│
│                                                           │
│  4. Gemini Streaming (reqwest/SSE)                        │
│     └── Resize/JPEG encode capture and send with prompt   │
│     └── Stream answer to the regular chat window          │
└───────────────────────────────────────────────────────────┘
           │
           ▼
┌───────────────────────────────────────────────────────────┐
│ Native GTK Response Window                               │
│  - Normal Wayland window managed by Hyprland             │
│  - Floating, sizing, opacity and placement via rules     │
│  - Real-time Markdown response & action buttons           │
└───────────────────────────────────────────────────────────┘
           │
           ▼
┌───────────────────────────────────────────────────────────┐
│ Execution Pipeline                                        │
│  - Clipboard staging (arboard / native Wayland clipboard) │
│  - Target injection / dismissal                           │
└───────────────────────────────────────────────────────────┘
```

---

## Technical Stack & Dependencies

### Core Runtime & System Libraries
* **Language Runtime:** Rust (2024 edition or latest stable, `tokio` async runtime)
* **Compositor IPC:** `hyprland-rs` (querying Hyprland's request socket for cursor coordinates, active window geometry, monitors)
* **Screen Selection Overlay:** Native `gtk4` + `gtk4-layer-shell-rs` transient drawing area (interactive drag-selection and window snap, replacing `slurp`)
* **In-Memory Screencopy:** `libwayshot` (direct `zwlr_screencopy_v1` capture into `image::DynamicImage` / `RgbaImage` buffers, replacing `grim`)
* **Multimodal Client:** `reqwest` (with `stream` & TLS support) targeting Gemini API endpoints via SSE
* **Response Window:** Native GTK4 window managed by Hyprland window rules; the selection canvas remains a GTK4 layer-shell surface.
* **Prompt Composer:** Compact GTK4 layer-shell surface positioned by the cursor monitor's geometry and edge-aware margins.
* **Clipboard Control:** `arboard` (cross-platform / Wayland clipboard crate)

---

## Module Breakdown & Development Phases

### Phase 1: IPC & Native Spatial Target Selection
* **Goal:** Query Hyprland state and provide a seamless, native selection interface without external CLI dependencies.
* **Modules to Implement:**
  * `hypr_ipc.rs`: Connects to Hyprland's request socket via `hyprland-rs` to inspect active window geometry `(x, y, w, h)`, window class, title, and current cursor position on demand.
  * `selection_overlay.rs`: Transient fullscreen `gtk4-layer-shell` canvas supporting:
    * Interactive drag-to-select bounding box via `GestureDrag` and custom Cairo drawing.
    * Instant window-snap mode leveraging geometry from `hypr_ipc.rs`.
    * Yields target coordinates `(x, y, w, h)` directly to the capture pipeline.
  * `prompt_overlay.rs`: Cursor-adjacent GTK4 layer-shell prompt with a capture thumbnail, text entry, Enter/send submission, and Escape dismissal. The prompt chooses the monitor containing the cursor and flips/clamps its position against that monitor's edges.

### Phase 2: Native In-Memory Screencopy
* **Goal:** Capture targeted screen pixels directly into memory buffers with zero disk writes and zero subprocess overhead.
* **Modules to Implement:**
  * `screencapture.rs`: Integrates `libwayshot` to bind to `zwlr_screencopy_v1`, capturing the target geometry directly into an in-memory `image::DynamicImage` or raw RGBA buffer.

### Phase 3: Multimodal Streaming Dispatcher
* **Goal:** Encode the in-memory buffer and prompt intents into a multimodal payload, streaming tokens back with minimal latency.
* **Modules to Implement:**
  * `ai_agent.rs`: Asynchronous client using `reqwest` targeting Gemini Flash. Encodes in-memory image bytes (JPEG/PNG) into payload, consumes SSE token streams, and pipes tokens to the UI via `tokio::sync::mpsc` channels.

### Phase 4: Wayland Floating Card & Action Pipeline
* **Goal:** Ask about the capture in a cursor-adjacent layer-shell composer, then display streaming AI output in a Hyprland-managed chat window.
* **Modules to Implement:**
  * `prompt_overlay.rs`: Native GTK4 layer-shell prompt composer positioned near the cursor and kept within the selected monitor's bounds.
  * `ui.rs`: Native GTK4 response window with CSS styling and pulldown-cmark Markdown rendering. Hyprland manages floating state, placement, opacity, and interactive resizing.
  * `executor.rs`: Handles quick actions like clipboard copies (via `arboard`) and smooth dismissal/cleanup of overlay windows.

---

## Key Milestone Checklist

- [x] Initialize Cargo workspace with `tokio`, `hyprland-rs`, `libwayshot`, `gtk4`, and `gtk4-layer-shell-rs`.
- [x] Implement Hyprland IPC client to query `cursorpos` and `activewindow`.
- [x] Build native GTK4 layer-shell drag-selection overlay yielding `(x, y, w, h)` coordinates.
- [x] Connect `libwayshot` to capture selected coordinates directly into an in-memory `DynamicImage`.
- [x] Connect `reqwest` SSE client to Gemini Flash with multimodal image payload.
- [x] Build native GTK4 response window for Hyprland-managed floating and resizing.
- [x] Stream AI tokens directly into GTK4 text buffer in real-time.
- [x] Wire clipboard staging (`arboard`) and add Hyprland keybind trigger in `hyprland.conf`.

### Hyprland response-window rules

Hyprland 0.57 uses a Lua configuration at `~/.config/hypr/hyprland.lua`. The GTK application ID is `com.magicpointerv2.tool`; verify the window's reported class with `hyprctl clients` and adjust the class regex if needed.

Add a window rule to the Lua config to float and center the response card and apply active/inactive opacity. The app supplies its initial size; omit a compositor `size` rule so the window remains interactively resizable:

```lua
hl.window_rule({
    match = { class = [[^com\.magicpointerv2\.tool$]] },
    float = true,
    center = true,
    opacity = "0.96 override 0.90 override",
})
```

To move and resize floating windows with the mouse, add bindings such as:

```lua
hl.bind("ALT + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind("ALT + mouse:273", hl.dsp.window.resize(), { mouse = true })
```

These are Hyprland 0.57 Lua APIs, not legacy `windowrule = ...` directives. Since the response card is a regular Wayland window, the compositor controls its placement; it is not guaranteed to appear adjacent to the selected region.
