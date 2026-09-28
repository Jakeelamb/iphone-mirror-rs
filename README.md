# iPhone Mirror Rust

A native Linux iPhone viewer with keyboard and pointer control. Rust handles the
authenticated device connection, HEVC packet assembly, input and window loop;
FFmpeg decodes video and wgpu renders YUV directly. Python and MPV are not runtime
dependencies.

The native Wi-Fi path has streamed a real iPhone 15 on iOS 27 at 1184×2576 using
VAAPI decoding and an AMD Vulkan renderer. See [validation and measurement
limits](docs/performance.md) before interpreting latency counters. A measured
speedup over the reference application has not been established.

## Build and run

Requirements: a current stable Rust toolchain, `pkg-config`, Clang/libclang,
FFmpeg development libraries (`libavcodec` and `libavutil`), and a Vulkan driver.
USB discovery additionally needs `usbmuxd`. The tested system uses FFmpeg 9.0.1;
the build generates bindings against the installed headers. The tested Rust
compiler is 1.96.0; a minimum supported Rust version has not been qualified.

The iPhone must already be paired, unlocked, and have Developer Mode enabled and
its developer services available. The tested remote-control service requires
iOS 27. This tool currently uses existing pairing records; it does not perform
first-time pairing or mount a developer image.

```sh
cargo build --release --locked
./target/release/iphone-mirror-rs --connection wifi
```

Wi-Fi uses the existing CoreDevice pairing under
`$XDG_DATA_HOME/pymobiledevice3`, defaulting to
`~/.local/share/pymobiledevice3`. Keep the phone on the same reachable local
network. Pairing records are read without being changed. With multiple saved
devices, select `--serial ID` or `--pairing-file PATH`.

```sh
./target/release/iphone-mirror-rs --connection usb
./target/release/iphone-mirror-rs --software --connection wifi
./target/release/iphone-mirror-rs --headless --duration 30 --connection wifi
./target/release/iphone-mirror-rs --help
```

The default connection mode is `auto`: try USB, then Wi-Fi. `--address IP:PORT`
selects a Wi-Fi remote-pairing endpoint explicitly. Only one instance runs at a
time. Close the window or use Ctrl+C to end the session. `--duration SECONDS`
limits total run time, including connection setup.

## Controls

| Computer input | iPhone action |
| --- | --- |
| Left click and drag | Single-finger touch and drag |
| Vertical mouse wheel | A short vertical swipe at the pointer position |
| Keyboard | ASCII text keys, modifiers and navigation keys over HID |
| F1 | Home button |
| F2 | Spotlight shortcut, Command+Space |

The window must be focused. Clicks in the black margins do not start touches.
Leaving the window ends a drag; losing focus releases held input. ASCII typing
follows the host logical character, including shifted punctuation, while
modifier and key-release state remains tied to physical keys. Unicode/IME
composition, custom game keymaps, simultaneous touch contacts, clipboard text
injection and audio are not implemented.

## Architecture and development

Complete HEVC access units go straight into FFmpeg, avoiding a byte-stream
demuxer/parser waiting for a subsequent frame. Encoded frames retain dependency
order in a bounded handoff. The presentation mailbox holds only the newest
complete decoded picture. Compressed packet buffers, decoded planes and GPU
textures are reused.

Hardware decoding prefers VAAPI, then CUDA, with software fallback. The current
hardware path downloads decoded frames to reusable CPU buffers and uploads YUV
planes to wgpu; it is **not zero-copy**. The renderer supports 8-bit YUV420P and
NV12 and selects Immediate, Mailbox, then FIFO presentation according to surface
support. Immediate presentation can trade tearing for shorter queueing.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo run --example render_fixture
cargo run --example render_fixture -- --hardware
```

The fixture viewer uses synthetic content and never connects to a phone.
[Development notes](docs/development.md) explain module boundaries and hardware
qualification. [Performance notes](docs/performance.md) describe CPU/allocation
profiling, the scrolling workload, and `--measure-stamp`.

## Reference and license

The protocol and behavior reference is
[omarchy-iphone-mirror](https://github.com/daniellemky/omarchy-iphone-mirror).
The native transport uses `idevice`, pinned to the revision in `Cargo.toml`.

This project is [GPL-3.0-or-later](LICENSE). Dependencies retain their own
licenses; `idevice` is MIT, and the installed FFmpeg build determines its enabled
components and license configuration. Keep pairing credentials, screen captures
and local profiling artifacts out of Git.
