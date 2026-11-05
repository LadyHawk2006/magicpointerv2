# AGENTS.md

## Project Vision & Overview
**Project Codename:** `magicpointerv2`  
**Target Environment:** Arch Linux | Hyprland (Wayland)  
**Core Goal:** Build an intent-aware, spatial desktop intelligence pipeline inspired by multimodal vision overlays (e.g., Google's Magic Pointer). 

The system operates as a zero-subprocess, high-performance background daemon implemented entirely in **Rust**. It queries compositor state via native IPC, provides an integrated GTK4 Wayland layer-shell selection canvas, captures visual bounding boxes directly from compositor buffers into RAM via `libwayshot`, and streams multimodal payloads to AI backends (Gemini API)—rendering streaming responses in floating overlays directly anchored to the cursor.

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
│     └── Zero subprocesses, zero disk I/O                  │
│                                                           │
│  3. Multimodal Streaming Dispatch                         │
│     └── Stream memory buffer to Gemini Flash (reqwest/SSE)│
│                                                           │
│  4. Floating Card Display & Clipboard staging             │
│     └── Exits cleanly once dismissed                      │
└───────────────────────────────────────────────────────────┘
           │
           ▼
┌───────────────────────────────────────────────────────────┐
│ Wayland Layer-Shell Overlay                               │
│  - gtk4 + gtk4-layer-shell-rs                             │
│  - Anchored dynamically adjacent to Cursor (X,Y)          │
│  - Real-time streaming response cards & action buttons    │
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
* **Overlay Window Manager:** `gtk4` + `gtk4-layer-shell-rs` (anchored floating response card)
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

### Phase 2: Native In-Memory Screencopy
* **Goal:** Capture targeted screen pixels directly into memory buffers with zero disk writes and zero subprocess overhead.
* **Modules to Implement:**
  * `screencapture.rs`: Integrates `libwayshot` to bind to `zwlr_screencopy_v1`, capturing the target geometry directly into an in-memory `image::DynamicImage` or raw RGBA buffer.

### Phase 3: Multimodal Streaming Dispatcher
* **Goal:** Encode the in-memory buffer and prompt intents into a multimodal payload, streaming tokens back with minimal latency.
* **Modules to Implement:**
  * `ai_agent.rs`: Asynchronous client using `reqwest` targeting Gemini Flash. Encodes in-memory image bytes (JPEG/PNG) into payload, consumes SSE token streams, and pipes tokens to the UI via `tokio::sync::mpsc` channels.

### Phase 4: Wayland Floating Card & Action Pipeline
* **Goal:** Display streaming AI output and actions in an interactive floating UI anchored near the target cursor position.
* **Modules to Implement:**
  * `overlay_ui.rs`: Floating `gtk4-layer-shell` window anchored near cursor `(x, y)` on the `Overlay` layer. Dynamically updates text buffers from the SSE stream.
  * `executor.rs`: Handles quick actions like clipboard copies (via `arboard`) and smooth dismissal/cleanup of overlay windows.

---

## Key Milestone Checklist

- [x] Initialize Cargo workspace with `tokio`, `hyprland-rs`, `libwayshot`, `gtk4`, and `gtk4-layer-shell-rs`.
- [x] Implement Hyprland IPC client to query `cursorpos` and `activewindow`.
- [x] Build native GTK4 layer-shell drag-selection overlay yielding `(x, y, w, h)` coordinates.
- [x] Connect `libwayshot` to capture selected coordinates directly into an in-memory `DynamicImage`.
- [x] Connect `reqwest` SSE client to Gemini Flash with multimodal image payload.
- [x] Build floating `gtk4-layer-shell-rs` response window anchored adjacent to cursor `(x, y)`.
- [x] Stream AI tokens directly into GTK4 text buffer in real-time.
- [x] Wire clipboard staging (`arboard`) and add Hyprland keybind trigger in `hyprland.conf`.
