# Performance measurements

Measure a release build on the same phone, transport, workload and desktop when
comparing changes. Record actual decode backend, rendering adapter, resolution,
presentation mode, run duration and the workload's clock-sync result. A smooth
frame rate does not by itself establish low end-to-end latency.

## Current evidence

The initial native Wi-Fi GUI run on an iPhone 15/iOS 27 negotiated 1184×2576
HEVC, reported actual VAAPI output, and rendered with the AMD Radeon 890M Vulkan
adapter. Its trace showed sustained decoding and presentation. That initial run
ended with a `mediastreamstop` response error, so it is evidence for the media
path rather than a clean-shutdown qualification. Later bounded runs should
report their own exit status.

Separately, synthetic hardware-decoded pictures matched software luma exactly.
The isolated surface fixture matched an independent FFmpeg BT709 RGB reference
within 2/255 per sampled channel. Three window sizes preserved the image aspect
within one raster pixel, and complete VAAPI/NV12 and software screenshots were
identical. The isolated rendering adapter was llvmpipe; these are correctness
checks, not physical GPU timing results.

There is no matched before/after performance result against the Python reference
yet. CPU percentages or exploratory profiles collected with different motion,
renderers or profiling overhead must not be presented as a speedup.

## Repeatable scrolling workload

The repository includes a dependency-free Rust HTTP example serving synthetic
scrolling content and a clock endpoint:

```sh
cargo run --release --example latency_page -- 0.0.0.0:52415
```

Open `http://HOST_LAN_IP:52415/` in Safari on the phone. The host's selected port
must already be reachable from the phone. The page scrolls automatically and
draws a full-width binary timestamp between green and magenta markers. It uses
six clock requests and chooses the lowest round-trip sample to align the
browser's monotonic clock with the host. Wait for the clock-sync label to show a
round-trip time; if it says the clock is unavailable, motion still works but no
valid timestamp samples are available. The server defaults to localhost when
no bind address is supplied. Stop it with Ctrl+C when the run finishes.

Run the normal viewer with the opt-in marker probe:

```sh
mkdir -p profiles
./target/release/iphone-mirror-rs --connection wifi --duration 60 \
  --measure-stamp --trace profiles/scroll-run.log
```

Use a new trace filename for each run. Keep the marker visible across the whole
encoded picture; browser zoom, rotation, hidden content or interrupted sync can
make recognition fail. The probe accepts only recognizable markers with an age
between zero and five seconds. Always report `stamp_samples`, including zero or
missing samples, alongside any latency statistic. The clock offset is refreshed
when the page becomes visible again, not continuously during a long run.

## What the counters measure

| Field | Meaning and boundary |
| --- | --- |
| `decoded` | Complete pictures emitted by FFmpeg |
| `replaced_decoded_frames` | Older decoded pictures replaced before the GUI consumed them; in headless mode this is expected |
| `submitted` | Fresh pictures successfully submitted to the presentation path; not completed display scanouts |
| `decode_mean_ms` | Mean wall time of a decoder call, including hardware download, decoded-plane copy and mailbox publication |
| `receive_to_submit_*` | Time from receipt of the last packet completing an access unit at the application stream receiver to presentation submission |
| `source_to_submit_*` | Recognized synthetic browser timestamp age at submission, using the page's estimated host-clock offset |

The receive timer starts after earlier packet assembly, phone capture/encoding,
network travel and lower-level tunnel buffering. It measures local remaining
work, not total phone-to-display latency. The synthetic timestamp covers much
more of that path, but also includes browser scheduling/painting and clock-sync
error. Neither metric measures compositor scheduling, display scanout, photon
arrival or click-to-response latency. Midpoint clock synchronization has
uncertainty from request asymmetry and scheduling; record the page's selected
RTT and its approximate half-RTT uncertainty with the result.

Histograms and counts accumulate from startup; the tool does not discard a
warmup interval automatically. Percentiles report upper edges of 1 ms buckets.
The value 256 is an overflow marker for samples at least 255 ms, not an exact
256 ms latency. A zero statistic can mean there were no samples. Periodic
counters are sampled concurrently and can briefly differ by a frame.

`--headless` exercises transport and decoding without a window or presentation.
Its submit and source-stamp metrics remain empty; it cannot stand in for a GUI
latency run. `--duration` includes connection setup rather than specifying a
fixed number of seconds of received video.

## CPU and allocation profiles

Collect production-workload profiles separately from clean timing runs. For
example, while the synthetic page is moving:

```sh
perf record -F 199 --call-graph dwarf -o profiles/cpu.data -- \
  ./target/release/iphone-mirror-rs --connection wifi --duration 60
perf report -i profiles/cpu.data

heaptrack -o profiles/heaptrack -- \
  ./target/release/iphone-mirror-rs --connection wifi --duration 30
```

Profiler access depends on the host. Preserve any permission failure as a
limitation rather than changing system-wide security settings. Heap tracing
adds overhead and includes allocator calls from FFmpeg and graphics drivers;
it does not account for all GPU memory. Process RSS is resident memory, not an
allocation-rate profile. CPU samples show where CPU time is spent; they do not
directly identify time waiting for the phone or network.

The implementation pools compressed packets, decoded planes and hardware
download storage and reuses textures until dimensions or layout change. That
does not make the whole pipeline allocation-free. Hardware decode still crosses
a download/upload boundary. Optimize the dominant measured stage, then repeat a
clean timing run before making an improvement claim.
