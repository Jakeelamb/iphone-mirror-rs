# iPhone Mirror Rust

Mirror and control an iPhone from Linux with a native Rust viewer, FFmpeg
hardware decoding and wgpu rendering. Supports mouse input, keyboard input,
Home, and automatic portrait/landscape rotation. No Python or MPV is needed to
**run the viewer**; first-time phone preparation uses a separate setup tool.

**Experimental.** Live-tested over Wi-Fi on an iPhone 15 running iOS 27, with
Arch Linux/Omarchy on x86-64. USB is implemented but has not been live-tested in
this Rust viewer. Other phones, iOS releases, desktops and GPU combinations are
not yet qualified. Omarchy is not a runtime dependency.

## Quick start

### 1. Install build dependencies

You need Rust **1.96 or newer**, a C compiler, `pkg-config`, Clang/libclang,
FFmpeg development libraries (`libavcodec` and `libavutil`), and a working Vulkan
renderer. Live phone tests used **FFmpeg 9.0.1**; CI checks builds and software
decoding against Arch's current FFmpeg package. Other live configurations remain
unverified. Use [rustup](https://rustup.rs/) if your distribution's Rust is too old.

On Arch Linux/Omarchy:

```sh
sudo pacman -Syu --needed base-devel clang ffmpeg git pkgconf rustup usbmuxd
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
```

Keep the GPU driver appropriate for your hardware installed. CUDA/NVDEC and
VAAPI decoding are optional: decoding falls back to software, but the window
still needs a Vulkan renderer. Other distributions need equivalent development
packages; no Debian/Ubuntu or Fedora build has been verified yet.

### 2. Prepare the phone

The phone must be unlocked, trust this computer, have Developer Mode enabled,
and have a compatible developer image mounted. Wi-Fi also needs a saved
CoreDevice pairing record and a reachable local network.

Follow [phone setup](docs/phone-setup.md) if this computer is not already ready.
The viewer does not create pairing records, download images or mount them.

### 3. Clone, build and run

```sh
git clone https://github.com/Jakeelamb/iphone-mirror-rs.git
cd iphone-mirror-rs
cargo +1.96.0 build --release --locked
./target/release/iphone-mirror-rs --connection wifi
```

Close the window or press Ctrl+C in the launching terminal to stop. Only one
viewer should control a phone at a time. The connection is selected at startup;
close and reopen the viewer to change transports.

Optional installation:

```sh
install -Dm755 -s target/release/iphone-mirror-rs ~/.local/bin/iphone-mirror-rs
~/.local/bin/iphone-mirror-rs --connection wifi
```

To invoke it as `iphone-mirror-rs`, include `~/.local/bin` in your PATH.
Rebuild and repeat that install command after updating. To uninstall, remove
`~/.local/bin/iphone-mirror-rs`. Pairing records and developer images are separate
from the application and remain on the computer.

## Controls

| Input | Action |
| --- | --- |
| Left click / drag | Single-finger tap / drag |
| Vertical mouse wheel | Vertical swipe at the pointer |
| Keyboard | ASCII text, modifiers and navigation keys |
| House button below the picture, or F1 | Home |
| F2 | Spotlight |
| F8 (with `--game-profile`) | Enter / leave game mouse capture |
| Escape in game mode | Release all controls and restore the pointer |
| F9 (with `--game-profile`) | Calibrate touchscreen targets |

Focus the window before using input. Leaving the window ends a drag; losing
focus releases held input. The Home button activates on release inside it.
Margins and clipped corners do not send touches.

Orientation follows the phone's **interface**, so an app locked to portrait
stays portrait. Rotation is checked about every 400 ms; video, taps and wheel
swipes use the same transformation. The Home strip stays upright. The window
requests a matching size, but tiling or maximization can override it and leave
letterboxing. Resize or float the window in your desktop if needed.

Optional [game controls](docs/game-controls.md) map WASD, action keys and relative
mouse motion to up to five independent touchscreen contacts. They require a
calibrated HUD profile and a compositor that supports pointer locking.

Not implemented: audio, clipboard injection or Unicode/IME composition. There is no automatic reconnection
after a failed session.

## Options

```sh
# Try USB first, then Wi-Fi (the default).
iphone-mirror-rs --connection auto

# Select a decoder preference; unavailable hardware falls back.
iphone-mirror-rs --connection wifi --decoder vaapi
iphone-mirror-rs --connection wifi --decoder software

# Select a saved pairing explicitly.
iphone-mirror-rs --connection wifi --pairing-file /path/to/remote_DEVICE.plist

# Bounded diagnostic run; duration includes connection setup.
iphone-mirror-rs --connection wifi --duration 30 --trace /tmp/mirror-run.log

iphone-mirror-rs --help
```

Use `./target/release/iphone-mirror-rs` instead if you did not install the binary.
Trace files must not already exist. `--serial ID` selects among saved devices;
`--address IP:PORT` supplies a known Wi-Fi pairing endpoint when discovery is
unavailable. `--headless` tests transport/decoding without a window.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| No saved pairing / several pairings | Follow [phone setup](docs/phone-setup.md), or select `--pairing-file` / `--serial`. |
| Wi-Fi discovery fails | Unlock the phone; use the same reachable LAN. Guest-network isolation can block discovery. |
| Developer/display service fails to open | Check Developer Mode, the mounted image and iOS compatibility. Successful pairing alone does not prove mirroring support. |
| Decoder initialization or driver errors | Try `--decoder software`. For VAAPI try `--decoder vaapi`; the selected backend is logged. |
| GPU window fails | Check that Vulkan works in the current desktop session. Software decoding does not replace the renderer. |
| Black margins | The image preserves its aspect ratio. Floating/resizing can let the window fit the phone. |
| Another viewer is running | Close the existing viewer before starting another. |

For a bug report, include the command, OS, phone/iOS version, transport, GPU,
FFmpeg version and relevant trace lines. Never attach pairing records or
private screen content. See [contributing](CONTRIBUTING.md).

## Design and performance

Complete HEVC pictures go directly into FFmpeg. Encoded pictures keep their
reference order in a bounded queue; presentation retains only the newest
complete decoded picture. Packet buffers, decoded planes and GPU textures are
reused. Rotation runs in the shader.

Automatic decoding prefers CUDA/NVDEC, then VAAPI, then software. The current
hardware path downloads frames to CPU memory and uploads their YUV planes to
wgpu: **it is not zero-copy**. Immediate presentation is preferred where supported
and can trade tearing for less queueing.

An earlier build sustained about 60 FPS in the tested scrolling workload, with
about 59 ms synthetic source-to-GPU-submission delay. That is **not**
click-to-photon latency or a guarantee for other systems. There is no matched
speedup comparison against the Python reference. See [measurements and their
limits](docs/performance.md) and [architecture/testing](docs/development.md).

## Credits and license

Built using [idevice](https://github.com/jkcoxson/idevice),
[FFmpeg](https://ffmpeg.org/), [wgpu](https://wgpu.rs/) and
[winit](https://github.com/rust-windowing/winit).
[Daniel Lemky's omarchy-iphone-mirror](https://github.com/daniellemky/omarchy-iphone-mirror)
and [pymobiledevice3](https://github.com/doronz88/pymobiledevice3) provided protocol
and behavior references.

Source is licensed under [GPL-3.0-or-later](LICENSE). See
[third-party notices](THIRD_PARTY_NOTICES.md) for attribution and dependency scope.
This is an independent project, not affiliated with Apple.
