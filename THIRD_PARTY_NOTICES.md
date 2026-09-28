# Third-party notices

This repository is licensed under GPL-3.0-or-later; see [LICENSE](LICENSE).
That license does not replace the licenses or notices of dependencies.

## Protocol and behavior references

[omarchy-iphone-mirror](https://github.com/daniellemky/omarchy-iphone-mirror),
by Daniel Lemky, is the behavior reference for connection lifecycle, input and
orientation handling. Its original code is MIT licensed. The notice is retained
below for adapted behavior/code.

[pymobiledevice3](https://github.com/doronz88/pymobiledevice3/tree/v11.13.1)
(version 11.13.1, GPL-3.0-or-later) is a protocol reference for display
negotiation and HID report layouts. The video-offer test fixture was generated
from its default offer builder, not from a phone capture. See
[src/device/fixtures/README.md](src/device/fixtures/README.md).
The Rust viewer does not import or launch pymobiledevice3; the optional setup
guide uses it as a separate preparation tool.

### Daniel Lemky's MIT notice

```text
MIT License

Copyright (c) 2026 Daniel Lemky

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Runtime and build dependencies

- [idevice](https://github.com/jkcoxson/idevice) is MIT licensed. Its revision is
  pinned in Cargo.toml and Cargo.lock.
- [wgpu](https://github.com/gfx-rs/wgpu) and
  [winit](https://github.com/rust-windowing/winit) are MIT/Apache-2.0 licensed.
- [FFmpeg](https://ffmpeg.org/legal.html) is linked from the system. Its license
  depends on the enabled build components; GPL-enabled builds have different
  distribution requirements from LGPL builds.
- Other Rust dependencies retain the licenses in their source packages.
  Cargo.lock records the dependency versions; this list is not a complete
  third-party license inventory.

## Repository scope

The repository contains source and small synthetic test fixtures. It does not
bundle FFmpeg, GPU drivers, Apple developer images, pairing credentials, Python
environments or prebuilt application binaries. Distributing a combined binary
requires preserving the notices and corresponding source required by its
included components; this source repository alone is not a binary license bundle.
