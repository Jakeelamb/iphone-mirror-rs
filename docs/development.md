# Development

For setup and pull requests, start with [CONTRIBUTING.md](../CONTRIBUTING.md).
Use `cargo build --release --locked` for performance work. Release builds retain
debug information for profiling and use thin LTO. Keep `Cargo.lock` alongside the
source because `idevice`'s native CoreDevice APIs are pinned to a Git revision.

## Source map

| Module | Responsibility |
| --- | --- |
| `src/device/` | Existing pairing import, USB/Wi-Fi discovery, authenticated tunnel, display and HID services |
| `src/rtp.rs` | Bounded HEVC access-unit assembly, loss recovery and RTCP reports |
| `src/session.rs` | Service lifecycle, ordered encoded handoff, decoder worker, input dispatch |
| `src/video/decoder.rs` | FFmpeg packet pool, hardware selection, decoded-frame transfer and buffer reuse |
| `src/video/frame.rs` | Recyclable decoded planes and one-frame presentation mailbox |
| `src/video/renderer.rs` | wgpu YUV textures, rounded screen and Home button rendering, surface presentation |
| `src/video/orientation.rs` | Interface orientation to GPU rotation and displayed dimensions |
| `src/video/layout.rs` | Shared DPI-aware screen/button geometry and pointer hit testing |
| `src/input.rs` | HID encoding, contact/key state and bounded input queue |
| `src/game.rs` | Calibrated five-contact mapping, full touch snapshots and profile persistence |
| `src/game/bindings.rs` | Input overrides, physical key/mouse names and aggregate alias ownership |
| `src/game_ui.rs` | Calibration steps and physical game-key bindings |
| `src/app.rs` | winit window events, coordinate mapping and controls |
| `src/metrics.rs` | Fixed-size counters, timing histograms and optional synthetic timestamp recognition |

The process uses two Tokio workers, a blocking decoder task and the window's
event loop. Device/GPU decoder initialization completes before the phone starts
producing frames. A separate task polls interface orientation with bounded
requests and retry backoff, without blocking video or HID writes. Encoded packets are never arbitrarily replaced: dependent HEVC
pictures need earlier reference pictures. Only decoded pictures may replace an
older unpresented picture. On detected stream loss, recovery waits for a
keyframe; a stalled decoder handoff ends the session instead of growing memory.

## Validation

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

The normal suite includes RTP loss/order cases, HID transitions, pairing
validation, immediate HEVC output from complete synthetic access units, decoded
storage recycling, histogram boundaries and WGSL validation. Home button tests
cover click cancellation, interrupted scrolling and F1 ownership; layout tests
check rounded corners, touch mapping and
DPI scaling. Orientation tests cover both landscape directions, already-rotated
buffers, touch release at rotation and footer-aware window fitting. These tests
do not require a connected iPhone.

Game-control tests compare active/released frames with the pinned upstream
multitouch encoder, preserve held contacts through look recentering, bound
button capacity and check cleanup at Escape, focus loss and rotation. These
are protocol/state tests; use the [touch diagnostic](game-controls.md#diagnostic-page)
and a game's training HUD to qualify actual device delivery and control semantics.

Probe local decoder hardware explicitly:

```sh
cargo test --lib video::decoder::tests::automatic_decoder_matches_software_picture_content \
  -- --ignored --nocapture
```

This compares synthetic decoded luma against software output. Read the printed
`actual hardware active` result: a passing comparison with `false` validates the
fallback, not hardware acceleration.

The offline surface viewer exercises window presentation:

```sh
cargo run --example render_fixture
cargo run --example render_fixture -- --hardware
cargo run --example render_fixture -- --rotation 270
cargo run --release --example render_fixture -- --animate --fps 60 --frames 180 \
  --present-mode fifo --pre-present-notify on --frame-latency 1
```

Use an isolated desktop for routine UI verification. Resize between wide and
narrow windows to inspect both letterbox directions. Synthetic captures can be
compared against FFmpeg's RGB conversion using the same color-matrix choice.
An isolated compositor may select llvmpipe; a correct screenshot there does not
qualify physical GPU performance.

`--animate` loops the complete synthetic sequence; the default repeats its first
picture for stable screenshots. `--fps 1..240` controls the fixture's target rate
(default 30), and `--frames N` closes it one source interval after N fixture
steps, even if the compositor stops delivering redraws. A main-loop timer
advances the source; UI redraws only render the retained newest picture.
Main-thread blocking can still delay that timer. The closing counters distinguish
source steps, distinct submitted pictures and pictures replaced before submission.
These are local synthetic pictures, not phone capture-rate controls. The normal test suite
verifies immediate output and content changes across two complete loops without
opening a window. All presentation flags use the viewer's shared validation.

For a device change, separately verify discovery, authentication, media, actual
decoder selection, rendering, input, loss recovery and clean shutdown. USB
support in source is not proof that a USB live run passed. A service-opening log
also does not prove that a tap or key reached the intended app.

## Diagnostics and privacy

Application tracing records timing, counters, service stage labels and graphics
metadata. The entry point disables `idevice` and `jktcp` tracing because upstream
debug output can contain protocol payloads or credentials. Do not remove these
filters while using real pairing records.

`--trace PATH` creates a new, mode-0600 file and refuses to overwrite an existing
one. `RUST_LOG=iphone_mirror_rs=trace` enables detailed stage timing; default
application info logging is preferable for comparison runs. The optional marker
probe reads synthetic pixels transiently and does not save them. Pairing files
remain in the user's existing data directory, outside the repository.
