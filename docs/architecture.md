# Project Architecture
**Language:** Rust  
**Platform:** Windows (1080p)  
**Related docs:** `image_matrix_optimisation.md` · `ai_web_automation_workflow.md`

---

## 1. Guiding Principles

These rules apply across every file and module in this project.

1. **Pure functions have no side effects.** Comparison, prompt building, and output formatting functions do not log, click the mouse, read from disk, or write to disk. They take data in and return data out. All I/O and logging belongs at the call site.

2. **Platform-specific code is quarantined.** Anything that touches the Windows API (`screenshots`, `windows`, `enigo`, `winapi`) lives only in `capture.rs` and `human_simulations.rs`. Every other module compiles and tests cleanly on any platform.

3. **One responsibility per file.** If you find yourself writing `use crate::X` for more than two unrelated modules inside a single file, it is doing too much.

4. **Assets are versioned alongside code.** Reference images, session data, and AI output all live in predictable, documented locations. Nothing is hardcoded to an absolute path.

5. **The end goal travels with every prompt.** As defined in `ai_web_automation_workflow.md`, the `end_goal` and `hard_constraints` fields from `session.json` are injected into every AI prompt without exception.

---

## 2. Full Directory Tree

```
project_root/
│
├── Cargo.toml
├── Cargo.lock
├── build.bat                         — Windows build helper
│
├── src/
│   ├── main.rs                       — entry point; boots GUI or CLI intake
│   ├── session.rs                    — SessionConfig load/save (session.json)
│   │
│   ├── image_matrix/                 — pure image comparison library
│   │   ├── mod.rs                    — public re-exports only
│   │   ├── types.rs                  — RgbMatrix, GrayMatrix, MatchResult, Tolerance
│   │   ├── convert.rs                — rgb_to_gray, gray_to_rgb, pixel_to_gray
│   │   ├── io.rs                     — load/save PNG, JSON
│   │   ├── capture.rs                — capture_rgb_matrix, capture_gray_matrix (Windows)
│   │   └── compare.rs                — all comparison functions (pure, no side effects)
│   │
│   ├── intake/                       — staged user intake (Stages 1–5)
│   │   ├── mod.rs
│   │   ├── stage_1_source.rs         — repo URL, branch, content type
│   │   ├── stage_2_goal.rs           — end goal + success criteria
│   │   ├── stage_3_constraints.rs    — hard constraints + context
│   │   ├── stage_4_tasks.rs          — task list + iteration config
│   │   └── stage_5_review.rs         — review mode + debate config
│   │
│   ├── prompt/                       — prompt assembly (pure)
│   │   ├── mod.rs
│   │   ├── builder.rs                — standard prompt template
│   │   └── debate.rs                 — bull / bear / judge prompt variants
│   │
│   ├── runner/                       — execution loop
│   │   ├── mod.rs
│   │   ├── site_workflow.rs          — site-specific setup + adapter construction
│   │   ├── task_runner.rs            — outer task + iteration loop
│   │   ├── debate_runner.rs          — bull → bear → judge orchestration
│   │   └── constraint_check.rs       — post-response violation detection
│   │
│   ├── browser/                      — browser and UI automation (Windows)
│   │   ├── mod.rs
│   │   ├── navigation.rs             — search_for_youtube_in_url, url bar control
│   │   ├── detection.rs              — check_if_youtube_logo, contains_searchbar, etc.
│   │   ├── playback.rs               — is_video_paused, is_ad_playing, monitor_video
│   │   └── ai_interface.rs           — submit prompt to AI tab, capture response text
│   │
│   ├── human_simulations.rs          — mouse movement, typing, scrolling (Windows)
│   ├── gui.rs                        — eframe GUI entry point
│   ├── gui_components.rs             — reusable GUI widgets
│   ├── gui_theme.rs                  — colours, fonts, spacing constants
│   ├── json_functions.rs             — read viewbot_settings.json, details.json
│   ├── discord_control.rs            — Discord bot control logic
│   ├── discord_notifications.rs      — send Discord messages
│   ├── dolphin_anty.rs               — Dolphin Anty profile management
│   ├── purge_repopulate_profiles.rs  — profile reset logic
│   ├── bot_single_video.rs           — single-video bot loop
│   ├── bot_multiple_videos.rs        — multi-video bot loop
│   ├── bot_livestream.rs             — livestream bot loop
│   └── watch_related.rs              — related video discovery
│
├── assets/
│   ├── templates/                    — reference images for screen comparison
│   │   ├── ui/                       — browser chrome (logos, buttons, bars)
│   │   │   ├── youtube_1080_125.png
│   │   │   ├── youtube_1080_100.png
│   │   │   ├── youtube_1080_extra.png
│   │   │   ├── youtube_1080_event.png
│   │   │   ├── searchbar_unclicked_1080_125.png
│   │   │   ├── searchbar_unclicked_1080_100.png
│   │   │   ├── videotab_unclicked_1080_125.png
│   │   │   ├── videotab_unclicked_1080_100.png
│   │   │   ├── videotab_unclicked_1080_extra.png
│   │   │   ├── videotab_clicked_1080_125.png
│   │   │   ├── videotab_clicked_1080_100.png
│   │   │   ├── videotab_clicked_1080_extra.png
│   │   │   ├── share_button_1080.png
│   │   │   ├── browser_exit_fullscreen_button.png
│   │   │   ├── browser_set_fullscreen_button.png
│   │   │   ├── browser_tab_x_button.png
│   │   │   ├── browser_tab_x_button_2.png
│   │   │   └── browser_tab_x_button_3.png
│   │   ├── popups/                   — popup and overlay images
│   │   │   ├── default_browser_chromium_1080_100.png
│   │   │   ├── restore_chromium_popup_1080_100.png
│   │   │   ├── youtube_tos_1080_100.png
│   │   │   ├── tos_accept_all_button.png
│   │   │   ├── youtube_side_panel_ad.png
│   │   │   └── youtube_side_panel_ad_extra.png
│   │   └── channel/                  — channel-specific images (per-project)
│   │       └── belongs_to_channel_1080_125.png
│   │
│   └── thumbnails/                   — pre-captured video thumbnail matrices
│       └── (JSON or bincode files per video, named by video ID or index)
│
├── config/
│   ├── viewbot_settings.json         — single-video bot settings
│   └── bot_multiple_video_details/
│       └── details.json              — multi-video target list
│
├── session/
│   ├── session.json                  — active session config (written by intake)
│   └── session.lock                  — prevents double-launch
│
├── outputs/                          — AI prompt outputs (written at runtime)
│   ├── task_1/
│   │   ├── iter_1_raw.md
│   │   ├── iter_1_self_review.md
│   │   └── iter_1_debate/
│   │       ├── round_1_bull.md
│   │       ├── round_1_bear.md
│   │       └── round_1_judge.md
│   └── summary.md
│
├── debug/                            — screen captures saved during error conditions
│   ├── restore_browser.png
│   ├── share_button_check.png
│   └── browser_tab_x_button.png
│
└── tests/
    ├── compare_tests.rs              — unit tests for image_matrix::compare
    ├── prompt_tests.rs               — unit tests for prompt::builder and prompt::debate
    └── fixtures/
        ├── haystack_1920x1080.png    — test screen captures
        └── needle_50x30.png
```

---

## 3. What Goes Where — Rules by Category

### 3.1 Reference Images (`assets/templates/`)

All PNG files used as comparison templates live here — never in `src/` or the project root.

| Subdirectory | Contents | Naming convention |
|---|---|---|
| `assets/templates/ui/` | Browser chrome, YouTube UI elements | `{element}_{resolution}_{scale}.png` |
| `assets/templates/popups/` | Popups, overlays, TOS dialogs | `{element}_{resolution}_{scale}.png` |
| `assets/templates/channel/` | Channel icons, per-project targets | `{element}_{resolution}_{scale}.png` |
| `assets/thumbnails/` | Pre-captured video thumbnails as matrix data | `{video_id}.json` or `{video_id}.bin` |

**Naming convention breakdown:**
- `{element}` — what the image shows, snake_case, e.g. `youtube`, `searchbar_unclicked`, `videotab_clicked`
- `{resolution}` — vertical pixel count of the screen it was captured on, e.g. `1080`
- `{scale}` — browser zoom level as a percentage, e.g. `100`, `125`

Example: `searchbar_unclicked_1080_125.png` → unclicked search bar, captured at 1080p, 125% browser zoom.

**Why separate subdirectories?** Templates are loaded via `OnceLock` statics at the call site. Grouping by function means adding a new popup image doesn't require scrolling past 30 UI images to find the right group.

**Do not store** debug captures, error screenshots, or runtime outputs in `assets/`. Those go in `debug/` and `outputs/` respectively.

---

### 3.2 Rust Source Files (`src/`)

| File / directory | Allowed dependencies | Must NOT depend on |
|---|---|---|
| `image_matrix/compare.rs` | `image_matrix/types.rs`, `rayon` | `screenshots`, `enigo`, `windows`, `chrono`, `image` |
| `image_matrix/convert.rs` | `image_matrix/types.rs` | Everything else |
| `image_matrix/io.rs` | `image_matrix/types.rs`, `image`, `serde_json` | `screenshots`, `enigo`, `windows` |
| `image_matrix/capture.rs` | `image_matrix/types.rs`, `screenshots` | `enigo`, prompt, runner, browser |
| `prompt/builder.rs` | `session.rs` | `image_matrix`, `browser`, `enigo` |
| `prompt/debate.rs` | `session.rs`, `prompt/builder.rs` | `image_matrix`, `browser`, `enigo` |
| `runner/task_runner.rs` | `prompt/`, `session.rs`, `output/` | `image_matrix`, `enigo` directly |
| `runner/debate_runner.rs` | `prompt/`, `runner/task_runner.rs` | `image_matrix`, `enigo` directly |
| `browser/detection.rs` | `image_matrix/`, `human_simulations.rs` | `prompt/`, `runner/`, `session.rs` |
| `browser/ai_interface.rs` | `human_simulations.rs` | `image_matrix/`, `session.rs` |
| `human_simulations.rs` | `enigo`, `winapi`, `rand` | Everything above |
| `gui.rs` | `eframe`, `session.rs`, `intake/` | `runner/`, `browser/` directly |

The dependency arrows only flow in one direction. `compare.rs` never knows about the browser; `browser/` never knows about prompts.

---

### 3.3 Session and Config Data (`config/` and `session/`)

| File | Written by | Read by | Contents |
|---|---|---|---|
| `config/viewbot_settings.json` | User, manually | `json_functions.rs` | Single-video target, watch duration range |
| `config/bot_multiple_video_details/details.json` | User, manually | `json_functions.rs` | Multi-video target list with per-video settings |
| `session/session.json` | `intake/` at startup | `session.rs`, `runner/`, `prompt/` | Full `SessionConfig` struct (see `ai_web_automation_workflow.md`) |
| `session/session.lock` | `main.rs` on launch | `main.rs` on launch | Prevents two instances running at once |

`session.json` and `viewbot_settings.json` are different things. `viewbot_settings.json` controls which YouTube video to watch and for how long. `session.json` controls the AI automation workflow — the goal, constraints, tasks, and review mode.

---

### 3.4 Runtime Outputs (`outputs/` and `debug/`)

| Directory | Written by | Cleared by |
|---|---|---|
| `outputs/task_{n}/iter_{i}_raw.md` | `output/writer.rs` | User manually, or new session |
| `outputs/task_{n}/iter_{i}_debate/` | `runner/debate_runner.rs` | User manually |
| `outputs/summary.md` | `output/summary.rs` | Overwritten each session |
| `debug/` | `browser/detection.rs` on error | User manually |

`debug/` images are saved when a detection check fails, so you can inspect what the screen actually looked like. They are named after the function that failed, e.g. `debug/restore_browser.png`.

---

## 4. Module Dependency Graph

```
main.rs
  ├── gui.rs
  │     └── intake/ ──────────────────────────────► session.rs
  │
  └── runner/
        ├── task_runner.rs
        │     ├── prompt/builder.rs ──────────────► session.rs
        │     ├── browser/ai_interface.rs
        │     │     └── human_simulations.rs
        │     └── output/writer.rs
        │
        └── debate_runner.rs
              ├── prompt/debate.rs ───────────────► session.rs
              ├── browser/ai_interface.rs
              └── output/writer.rs


browser/detection.rs
  ├── image_matrix/
  │     ├── compare.rs  ◄── pure, no external deps
  │     ├── convert.rs  ◄── pure, no external deps
  │     ├── io.rs       ◄── image, serde_json only
  │     └── capture.rs  ◄── screenshots (Windows only)
  └── human_simulations.rs  ◄── enigo, winapi (Windows only)


(bot_single_video.rs, bot_multiple_videos.rs, bot_livestream.rs)
  ├── browser/detection.rs
  ├── browser/navigation.rs
  ├── browser/playback.rs
  ├── human_simulations.rs
  └── json_functions.rs
```

---

## 5. File Naming Conventions

### Rust files
- All lowercase, words separated by underscores: `task_runner.rs`, `constraint_check.rs`
- Modules that contain only type definitions end in `types.rs`
- Modules that are purely I/O end in `io.rs`
- Platform-specific files are named for their purpose, not their platform: `capture.rs` not `windows_capture.rs`

### Image template files
```
{element}_{resolution}_{scale_or_variant}.png
```
- `element` — snake_case description of what is shown
- `resolution` — screen height in pixels (`1080`)
- `scale_or_variant` — browser zoom (`100`, `125`) or a short qualifier (`extra`, `event`)

Avoid generic names like `image_1.png` or `template.png`. The filename must tell you exactly what template it is without opening the file.

### JSON data files
- Settings files: `{purpose}_settings.json` e.g. `viewbot_settings.json`
- Detail/list files: `details.json` inside a named subdirectory
- Session file: always `session/session.json`
- Thumbnail matrices: `{video_id}.json` or `{video_id}.bin`

### Output files
- Raw AI responses: `iter_{n}_raw.md`
- Self-review responses: `iter_{n}_self_review.md`
- Debate files: `round_{n}_{role}.md` where role is `bull`, `bear`, or `judge`
- Summary: always `outputs/summary.md`

---

## 6. Cargo.toml Dependency Map

```toml
[package]
name    = "web_ai_automation"
version = "0.1.0"
edition = "2024"

[dependencies]

# ── Image comparison ─────────────────────────────────────────────
screenshots = "0.8"          # screen capture (capture.rs only)
image       = "0.25"         # PNG load/save (io.rs only)
rayon       = "1.10"         # parallelism (compare.rs only)

# ── Serialisation ────────────────────────────────────────────────
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"            # human-readable config and session files
bincode     = "2"            # fast binary format for thumbnail matrices

# ── GUI ──────────────────────────────────────────────────────────
eframe      = "0.24"         # immediate-mode GUI (gui.rs only)
rfd         = "0.13"         # native file picker dialogs

# ── Browser / mouse automation ───────────────────────────────────
enigo       = "0.1"          # cross-platform input simulation
winapi      = { version = "0.3", features = ["winuser", "windef"] }
windows     = "0.51"         # GetPixel, GetDC, GetCursorPos

# ── Bot / timing ─────────────────────────────────────────────────
rand        = "0.8"
chrono      = "0.4"
tokio       = { version = "1", features = ["rt-multi-thread", "macros"] }
reqwest     = { version = "0.12", features = ["json", "blocking"] }

# ── Discord ──────────────────────────────────────────────────────
serenity    = { version = "0.12", default-features = false,
                features = ["client", "gateway", "model", "rustls_backend"] }

# ── Utilities ────────────────────────────────────────────────────
colored         = "2.1"
crossterm       = "0.26"
crossbeam-channel = "0.5"
once_cell       = "1.19"
futures         = "0.3"

[dev-dependencies]
# Tests for pure modules run on any platform.
# No screenshots, enigo, or windows crates in dev-dependencies.
```

**Dependency ownership — each crate should only be used inside its designated module:**

| Crate | Owned by |
|---|---|
| `screenshots` | `image_matrix/capture.rs` |
| `image` | `image_matrix/io.rs` |
| `rayon` | `image_matrix/compare.rs` |
| `enigo` | `human_simulations.rs` |
| `winapi` / `windows` | `human_simulations.rs`, `image_matrix/capture.rs` |
| `eframe` / `rfd` | `gui.rs`, `gui_components.rs` |
| `serde` / `serde_json` | `session.rs`, `json_functions.rs`, `image_matrix/io.rs` |
| `bincode` | `image_matrix/io.rs` |
| `serenity` / `tokio` | `discord_control.rs`, `discord_notifications.rs` |
| `reqwest` | `dolphin_anty.rs`, `browser/ai_interface.rs` |

---

## 7. Adding a New Template Image

1. Capture the image at the target resolution and browser zoom level.
2. Name it following the convention: `{element}_{resolution}_{scale}.png`
3. Place it in the correct `assets/templates/` subdirectory.
4. Add a `load_gray_from_image(…)` call inside the relevant `OnceLock` initialiser in `browser/detection.rs`.
5. Do **not** add a path string anywhere in `compare.rs` or `image_matrix/` — those modules never reference file paths.

---

## 8. Adding a New Bot Function

A "bot function" is something that checks the screen and/or moves the mouse.

1. The pure detection logic (does this template appear on screen?) goes in `browser/detection.rs`, using functions from `image_matrix/compare.rs`.
2. The mouse movement or click goes in `human_simulations.rs` or is called from the call site.
3. The orchestration (call detection, decide what to do, call mouse) goes in the relevant bot file (`bot_single_video.rs`, etc.) or `browser/playback.rs`.
4. No logging inside `image_matrix/`. Log the result at the call site in step 3.

---

## 9. Adding a New AI Prompt Stage

1. Add any new fields to `SessionConfig` in `session.rs`.
2. Add the corresponding intake screen in a new file under `intake/` following the stage pattern in `ai_web_automation_workflow.md`.
3. Add the prompt template string in `prompt/builder.rs` or `prompt/debate.rs`.
4. The runner (`runner/task_runner.rs` or `runner/debate_runner.rs`) picks up the new fields from `SessionConfig` — it does not know about the intake UI.

---

## 10. Cross-Document Reference

| Topic | Canonical document |
|---|---|
| Complete image comparison function implementations | `image_matrix_optimisation.md` |
| Staged intake UI screens and field definitions | `ai_web_automation_workflow.md` |
| `SessionConfig` struct and all enums | `ai_web_automation_workflow.md` Part 2 |
| Prompt templates (standard, bull, bear, judge) | `ai_web_automation_workflow.md` Part 3 |
| Execution loop and output file structure | `ai_web_automation_workflow.md` Parts 4–5 |
| `RgbMatrix`, `GrayMatrix`, `Tolerance`, `MatchResult` | `image_matrix_optimisation.md` Section 4 |
| All comparison function signatures | `image_matrix_optimisation.md` Section 8 |
| Performance rationale for greyscale pipeline | `image_matrix_optimisation.md` Sections 2–3 |
