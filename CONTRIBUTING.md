# Contributing

Small, focused fixes and reproducible compatibility reports are welcome. Start
with the [README](README.md) for dependencies and
[development notes](docs/development.md) for the source map.

## Change and check

Fork the repository, create a branch, and keep unrelated edits out of your pull
request. Use the locked dependencies. No phone is needed for the normal checks:

```sh
cargo +1.96.0 fmt --all --check
cargo +1.96.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.96.0 test --locked --all-targets
cargo +1.96.0 build --locked --release
```

CI runs the non-hardware checks on Arch Linux. A green build does not establish
phone, GPU or desktop compatibility. The opt-in hardware and synthetic viewer
checks are described in [development notes](docs/development.md).

Keep runtime dependencies small, queues bounded and credentials out of logs.
Reuse storage in video hot paths. Prefer a measured bottleneck over speculative
optimization. Include tests for changed behavior and update relevant docs.

In a pull request, explain the problem, the change and what you verified. Label
live-phone results separately from synthetic tests. Performance claims need the
workload, hardware, transport, exact revision and measurement boundary; use the
[profiling guide](docs/performance.md).

## Report a problem

[Open an issue](https://github.com/Jakeelamb/iphone-mirror-rs/issues/new) with:

- The command and steps to reproduce, plus expected and actual behavior.
- Linux distribution, desktop/session type, GPU and driver.
- Phone model, iOS and developer-image versions, and USB or Wi-Fi transport.
- `rustc --version`, `ffmpeg -version`, and the viewer revision.
- Relevant, reviewed trace lines from `--trace /path/to/new-file.log`.

Do not upload pairing records, developer images, complete data directories,
private screen captures or credentials. Keep local profiles under the ignored
`profiles/` directory. Synthetic fixtures under `src/` are intentional test data.

Contributions use the project's [GPL-3.0-or-later license](LICENSE); preserve
upstream notices when adapting code.
