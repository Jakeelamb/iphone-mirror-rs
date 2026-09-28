# Performance measurements

Measure a release build on the same phone, transport, workload and desktop when
comparing changes. Record actual decode backend, rendering adapter, resolution,
presentation mode, run duration and the workload's clock-sync result. A smooth
frame rate does not by itself establish low end-to-end latency.

## Current evidence

Release validation at revision `91b2808` on 2026-09-27 (Pacific), using an iPhone 15/iOS 27,
1184×2576 HEVC, direct CUDA/NVDEC, AMD Radeon 890M Vulkan rendering and Immediate
presentation on the physical Hyprland desktop:

| Measurement | Result |
| --- | --- |
| Run duration | 45 seconds including automatic Wi-Fi discovery; exit 0 |
| Decoded / submitted frames | 2,355 / 2,355, approximately 60 FPS while streaming |
| Decode mean | 2.58 ms |
| Last-packet receipt to submission | Mean 3.51 ms; p50 ≤4 ms, p95 ≤6 ms, p99 ≤10 ms |
| Synthetic source to submission | Mean 59.17 ms; p50 ≤59 ms, p95 ≤63 ms; 2,355 samples |
| Source clock calibration | Fresh RTT 10 ms, approximate midpoint uncertainty ±5 ms; recalibrated every 30 s |
| Process CPU | 0.129 cores averaged over the last 30.04 seconds |
| Process resident memory | Mean 301.19 MiB, peak 301.73 MiB in that interval |

Resource sampling reads `/proc/PID/stat` every 250 ms and discards the first
15 seconds for discovery/warmup. Timing counters include startup. These figures
come from the installed 11 MiB binary in an uninstrumented release run with the optional synthetic timestamp
probe; CPU sampling and allocation tracing run separately. They do not establish
imperceptible latency or click-to-photon latency.

The profiling build passed formatting, all-target Clippy, 49 normal tests, and
explicit hardware image comparisons. Two consecutive short sessions confirmed clean stop and reconnect. A separate
90-second isolated GUI run confirmed tap, wheel, Home, Spotlight, keyboard URL
entry including colons, successful navigation and clean shutdown. USB has not
been live-qualified in the Rust application.

The subsequent rounded-screen/Home and orientation validation passed 67 normal
tests, formatting and all-target Clippy at revision `3094863`. A live Wi-Fi session confirmed orientation 3
with a portrait-encoded buffer displayed correctly in landscape, a tap opening
the game's Settings, vertical wheel scrolling, and a tap returning to Training.
All four GPU rotations also passed isolated synthetic pixel checks (within
2/255 of reference colors), with a pixel-identical upright Home strip. The other
landscape direction and upside-down portrait have synthetic coverage, not live
phone qualification. The timing/resource table above predates these UI changes;
it is not a new performance measurement of the orientation build.

Separately, synthetic hardware-decoded pictures matched software luma exactly.
The isolated surface fixture matched an independent FFmpeg BT709 RGB reference
within 2/255 per sampled channel. Three window sizes preserved the image aspect
within one raster pixel, and complete VAAPI/NV12 and software screenshots were
identical. The isolated rendering adapter was llvmpipe; these are correctness
checks, not physical GPU timing results.

There is no matched before/after performance result against the Python reference
yet. CPU percentages or exploratory profiles collected with different motion,
renderers or profiling overhead must not be presented as a speedup.

## Decoder comparison and profile-driven change

One 45-second release run per backend used the same phone, scrolling page,
physical AMD renderer and sampling method (last 30 seconds for CPU/RSS):

| Decoder path | Decode mean | Receipt→submit mean | CPU cores | Mean RSS |
| --- | ---: | ---: | ---: | ---: |
| System VAAPI through NVIDIA bridge | 2.91 ms | 3.97 ms | 0.146 | 324.64 MiB |
| Direct AMD VAAPI | 6.42 ms | 7.38 ms | 0.221 | 196.23 MiB |
| Direct CUDA/NVDEC | 2.60 ms | 3.80 ms | 0.128 | 305.71 MiB |

All three sustained approximately 60 FPS and exited successfully. These are
individual observations, not confidence intervals or a universal GPU ranking.
Direct CUDA became the default because it reduced local decode time, CPU and
resident memory on this host. AMD remains selectable for its lower memory use.
Source timestamp ages from these sequential experiments are not used to rank
backends: clock calibration age and startup conditions differed.

A separate CPU profile collected 928 user-cycle samples after a five-second
delay, with no lost samples. Memory-copy routines and GPU-driver calls dominated
the visible leaves. Disassembly showed the sampled glibc copy/clear routines
already used AVX512; incomplete driver call-chain unwinding prevents attributing
every copy to a specific application stage. Hand-written assembly was not added.

Full heaptrack instrumentation exceeded the bounded decoder startup deadline
and ended after seven frames, so that capture is **not** a steady-workload
allocation result. Sampled jemalloc profiling then completed two 25-second
live phone-video runs: 1,441 decoded frames with NVIDIA VAAPI and 1,446 with
CUDA. They started from the scrolling workload, but did not collect per-frame
marker validation, so exact app content throughout those runs is unverified.
The VAAPI profile identified 1,440 allocations totalling approximately
7.55 GB at `nvCreateImage` beneath `av_hwframe_transfer_data`, roughly 5 MiB per
frame. Disassembly confirmed the driver allocation call. The CUDA profile
contains neither the bridge library nor that stack: direct decoding removes
this specific allocation path. It does not make all drivers or rendering
allocation-free. Other large sampled stacks could not be attributed reliably
because their modules were unloaded; no total allocation-reduction percentage
is claimed. Sampling changes the allocator and adds overhead, so those runs are
not the latency benchmark.

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
every 30 seconds and when the page becomes visible again. Overlapping refreshes
are suppressed; a failed refresh retains the last estimate and displays a
warning. Start qualified timing runs only after a successful calibration.

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

heaptrack -o profiles/heaptrack \
  ./target/release/iphone-mirror-rs --connection wifi --duration 30
```

For lower-overhead allocation sampling when jemalloc profiling is installed:

```sh
env LD_PRELOAD=/usr/lib/libjemalloc.so.2 \
  MALLOC_CONF=prof:true,prof_accum:true,prof_final:true,lg_prof_sample:18,prof_prefix:profiles/alloc \
  ./target/release/iphone-mirror-rs --connection wifi --duration 25
jeprof --text --alloc_space ./target/release/iphone-mirror-rs profiles/alloc.PID.0.f.heap
```

Retain the exact executable with each profile for symbolization. The sampled
allocation totals represent requested allocation traffic, not live memory or
GPU memory. Driver symbols can be missing or misleading; inspect mappings and
call sites before assigning a hotspot to a function.

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
