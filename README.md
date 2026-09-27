# iPhone Mirror Rust

A new native Rust iPhone mirror, developed against a real iPhone 15 on iOS 27.
This repository is under active construction. The initial scaffold does not yet
mirror a device; functionality and performance claims require live validation.

The intended path is native `idevice` transport, HEVC access units sent directly
to FFmpeg, and a wgpu YUV renderer. The display consumes the newest decoded frame
instead of accumulating an old-frame queue. Keyboard and pointer input use the
phone's developer HID services. Python and MPV are not runtime dependencies.

The protocol and behavior reference is
[omarchy-iphone-mirror](https://github.com/daniellemky/omarchy-iphone-mirror).
Existing pairing records stay outside this repository. Traces must contain only
timings, counters and fixed diagnostic labels—never keys, screen content, pairing
credentials or arbitrary device responses.

Build prerequisites: Rust, pkg-config, Clang/libclang, FFmpeg development
libraries, and a Vulkan-capable graphics driver. Validation commands:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Original implementation work is GPL-3.0-or-later. Dependencies retain their own
licenses; `idevice` is MIT and FFmpeg's license depends on the installed build.
