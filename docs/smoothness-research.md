# Smoothness and latency research

Research date: **2026-09-28, Pacific**. Implementation baseline: [`f1958b5acc525f1d268623448c70df95eb751ea0`](https://github.com/Jakeelamb/iphone-mirror-rs/tree/f1958b5acc525f1d268623448c70df95eb751ea0).

This is a research report, not an implementation plan already executed. No application code, dependencies, phone state, game settings, display configuration, or running viewer were changed. No new game, network, or performance benchmark was run. Three parallel investigations covered input, video presentation, and transport; their findings are consolidated here. Primary documentation, upstream source and research papers support the external claims. Proposed gains remain unmeasured.

## Conclusions and priorities

The strongest next opportunities are **faithful mouse displacement, consistent presentation timing, and visibility into queues before the decoder**. Another wholesale rewrite is not supported by the evidence. More filtering could make motion appear softer while making control less immediate; more buffering could produce steady playback of older pictures. Neither is automatically an improvement.

| Priority | Candidate | Evidence and decision |
| --- | --- | --- |
| First | Preserve mouse displacement through clipping/recentering | Current source has a concrete event-packetization dependency. Test this before adding smoothing. |
| First | Measure displayed cadence and input-to-photon latency | Current metrics end at submission, leaving compositor and display behavior unknown. |
| First | Compare presentation policies and compatible display cadence | Current desktop is fixed 165 Hz with VRR off; historical video is approximately 60 FPS. This is a testable cadence mismatch, not a diagnosed cause. |
| Next | Trace input age and lower-level transport queues | Bounded application queues do not bound all dependency queues. No actual backlog was demonstrated in this research. |
| Next | Qualify USB, then compare it with matched Wi-Fi runs | Removes a radio/TCP-tunnel variable without changing phone capture. Rust USB still needs live qualification. |
| Conditional | Calibrate look region and bounded motion dispatch | Can reduce contact resets or bursts, but must preserve gain, button order and immediate stop. |
| Conditional | Remove copy boundaries; compare same-GPU decode/render | Technically credible, larger engineering cost; historical local video work was already only a few milliseconds. |
| Later | Resampling, One Euro filtering, prediction or frame synthesis | Useful under particular conditions; substantial lag, overshoot, or visual-correctness tradeoffs. |

### How to read the evidence

**Current-code facts** refer to the pinned revision. **Historical measurements** are recorded earlier experiments, not measurements of today's game session. **Host observation** means a read-only query in this research. **Inference/candidate** means a mechanism to test, not an established speedup. Private Apple protocol behavior is attributed to reverse-engineered implementations, not presented as an Apple API guarantee.

## 1. Define smoothness separately from speed

There are at least four independent outcomes:

1. **Input fidelity:** the same physical movement produces the same intended action, independent of event grouping; releases and simultaneous controls remain correct.
2. **Motion cadence:** distinct pictures arrive on screen at consistent intervals, without long repeats or bursts.
3. **Response age:** the time from physical input to visible game response, including both trips through the system.
4. **Sustained stability:** those properties survive heat, queue pressure, focus changes and long sessions.

A 60 FPS average can coexist with repeated pictures and uneven frame holds. A low decoder mean can coexist with stale input, upstream video queues or delayed scanout. Researchers found that frequent frame-time variation affected perceived FPS-game smoothness, with larger variations having a stronger effect in their experiment. That supports measuring the distribution, not importing a universal acceptable-jitter threshold. [Klein et al., CoG 2024](https://research.nvidia.com/publication/2024-08_variable-frame-timing-affects-perception-smoothness-first-person-gaming).

The moment of a hitch matters too: a 38-participant study found targeting-time spikes harmed accuracy and score, while spikes during other actions could still harm experience. Therefore testing only an automatic scroll is insufficient for game controls; include microaim, flicks, reversal, strafing and simultaneous actions. These are methodological lessons, not measurements of Rainbow Six Mobile. [Tokey et al., QoMEX 2025](https://research.nvidia.com/publication/2025-09_timing-matters-impact-event-specific-frametime-spikes-first-person-shooter).

### Pipeline and measurement boundaries

| Stage | What can make motion uneven | What a future trace should establish |
| --- | --- | --- |
| Mouse/keyboard → host callback | Sampling, event grouping, scheduling | Callback intervals and physical-path equivalence; hardware sample time if actually available |
| Mapper → HID worker | Clipping, reanchors, coalescing, queue waits | Intended/emitted displacement, contact IDs, oldest input age, cancellation latency |
| HID transport → phone/game | Stream stalls, touch sampling, game semantics | Local write boundaries plus separately observed phone response |
| Game → capture/encoder | Native hitches, thermal policy, capture cadence, codec reorder | Distinct phone pictures and encoded-picture timing |
| Tunnel → complete access unit | Radio contention, reliable-stream stalls, queues, fragmentation | First/last packet times, queue age, bytes, loss/reorder/recovery |
| Decode → host frame | Decode, GPU download, planar copy | Decode output and transfer timing, retained surfaces |
| GUI → GPU → compositor | Acquire waits, uploads, queueing, redraw scheduling | Selected-frame age, acquisition duration, GPU work, presented/discarded outcomes |
| Panel scanout → photons | Refresh phase, frame holds, pixel response | Optical response at a defined screen position |

Submission is not visible presentation. NVIDIA's latency methodology likewise distinguishes input, system processing and display response; its Windows instrumentation is not a drop-in Linux/iOS measurement tool. [Understanding and Measuring PC Latency](https://developer.nvidia.com/blog/understanding-and-measuring-pc-latency/).

## 2. What is already optimized, and what the old numbers mean

The current implementation already has direct complete-access-unit decoding without demux/parser lookahead, one decoder thread, FFmpeg LOW_DELAY, pooled CPU planes, GPU YUV conversion, a four-access-unit encoded channel and a latest-decoded-picture mailbox. Rendering prefers Immediate, then Mailbox, then FIFO, with desired maximum frame latency one. TCP_NODELAY is already enabled. Input has a separate worker, adjacent-motion coalescing, ordered transitions and priority cancellation. These should not be rediscovered as new optimizations. [Decoder](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/video/decoder.rs), [renderer](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/video/renderer.rs), [session](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/session.rs), [input](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/input.rs).

The earlier scrolling-page qualification at `91b2808`, dated September 27, recorded:

| Historical measurement | Result and limit |
| --- | --- |
| Stream | iPhone 15/iOS 27, 1184×2576 HEVC, approximately 60 FPS |
| Decode | Mean 2.58 ms, including transfer/copy work covered by that counter |
| Completed access-unit receipt → submission | Mean 3.51 ms; p95 ≤6 ms; p99 ≤10 ms |
| Synthetic source marker → submission | Mean 59.17 ms; approximate clock-midpoint uncertainty ±5 ms |
| CPU | 0.129 cores during the recorded steady sampling window |
| GPUs | NVIDIA CUDA/NVDEC decode, AMD Radeon 890M Vulkan render |

These are from [the recorded performance report](performance.md), which remains the authority for conditions and limitations. They precede the current game-control build. They are not click-to-photon measurements, and they do not establish a matched improvement over the Python reference.

**Inference:** optimizing a historical 3–4 ms local tail cannot be promised to remove the whole roughly 59 ms marker age. Nor may those numbers simply be subtracted and called network latency: their boundaries include browser painting, capture, encoding, assembly, queues and clock error. Comparing independent percentile values by subtraction is particularly misleading.

The earlier allocation investigation already removed a specific NVIDIA VAAPI bridge allocation path by choosing direct CUDA. CPU samples reached optimized library copy routines, including AVX512. Neither another language rewrite nor hand-written assembly is justified without a new attributable profile. The failed seven-frame heaptrack capture was not a steady-state memory profile. [Historical profiling evidence](performance.md#decoder-comparison-and-profile-driven-change).

## 3. Input fidelity: highest-confidence code opportunity

### 3.1 Preserve displacement independently of event packetization

The mapper scales each raw mouse event, integrates into a finite look region and lifts/recenters when the next point crosses its boundary. An oversized event is clipped after recentering. **Analytical example:** at default sensitivity 0.002 and a centered region extending 0.16 short-edge units each way, one 100-unit event requests 0.20 but emits at most 0.16. Two 50-unit events can emit 0.10, recenter, then another 0.10. Equal total input can produce different commanded displacement. This is source arithmetic, not a live camera-angle measurement; it excludes the artificial lifted recenter jump. [Pinned motion implementation](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/game.rs).

**Candidate:** represent continuous logical displacement separately from finite touch coordinates. Split motion at a boundary and account explicitly for the remaining displacement. Bound outstanding work and its age so correcting loss does not create a long aiming tail after the mouse stops. A larger calibrated look region may reduce reset frequency without changing gain.

**Risks:** retaining every residual during overload can trade lost motion for stale motion. A larger region can cross HUD buttons. Moving an active finger back to center can generate reverse camera motion; it is not equivalent to lifting. Two overlapping look contacts are not known to provide a seamless handoff in this game.

**Test:** equal paths delivered as single, divided and bursty events, including edge crossings, reversal and stopping. Compare intended versus emitted within-contact displacement, residual age, resets and actual camera endpoint. Repeat while moving and leaning. Require consistent gain and a bounded stop tail, with no accidental controls.

### 3.2 Establish the game's camera transfer function

The mapper assumes relative touch displacement controls the camera. A virtual right stick instead maps displacement from its center to angular velocity. Those models require different mappings. Apple distinguishes mouse delta input from screen-pointer behavior in its game-input guidance; that does not identify this game's exact transfer function. [Apple keyboard and mouse gaming session](https://developer.apple.com/videos/play/wwdc2020/10617/).

**Diagnostic proposal:** move a look touch away from its origin, then hold it stationary. Continued turning suggests rate control; stopping suggests displacement control. Repeat at several offsets/speeds and in ADS, because acceleration or state-dependent sensitivity can confound a single observation. Calibrate the movement deadzone, full-speed radius and sprint threshold separately. Do not infer hidden game coefficients from generic sample code.

### 3.3 Keep contact lifecycle and cancellation correct

At this revision, joystick contact 0 remains down at neutral between WASD gaps. A fresh joystick `AnchorBegin` receives 20 ms of actual delivery separation before movement. **Mouse look uses ordinary Begin and does not receive that dwell on each recenter.** The earlier live observation was that back-to-back joystick center/move failed to establish the desired origin; separating delivery worked. It establishes neither a universal iOS delay requirement nor an optimal mouse-look delay. [Engine](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/game.rs), [worker](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/session.rs), [game-control qualification](game-controls.md).

The pinned HID report supports five stable contact identities with full active-contact snapshots and release coordinates. That is a property of this report implementation, not an iPhone-wide five-finger limit. Its producer encodes low 48-bit nanosecond timestamps; this is not an authoritative receiver-clock specification or synchronized phone sampling time. [idevice HID encoder at `d32c8189`](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/services/core_device/hid.rs).

Movement + look + fire + aim + lean exhaust the five available slots. A sixth action needs an explicit policy, not another thread. A future anchor-delay sweep must test fresh-start reliability, ongoing motion, independent releases and Escape under queued work. Reducing a sleep while restoring lost origins is a regression. Adding that sleep to every camera reanchor is also unproven. Preserve the existing cancellation generation and release of actually delivered contacts.

### 3.4 Cadence, raw units and coalescing

winit 0.30.13 describes raw MouseMotion units as unspecified and device-dependent. Its Wayland backend forwards unaccelerated relative deltas but does not propagate the native relative-pointer timestamps through MouseMotion. Therefore callback time is not necessarily hardware sample time, and calling sensitivity “per pixel” is too strong. Mouse counts, compositor deltas, logical pixels and phone coordinates are separate units. [winit API](https://docs.rs/winit/0.30.13/winit/event/enum.DeviceEvent.html), [exact Wayland backend](https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/linux/wayland/seat/pointer/relative_pointer.rs).

Current coalescing replaces adjacent absolute Move snapshots after the mapper integrates deltas. It does not simply discard all earlier raw movement. However, it can remove intermediate path/velocity detail, while clipping has already occurred upstream. A latest-position policy and a relative-delta accumulator are not interchangeable. [Current input queue](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/input.rs).

**Candidate:** compare event-driven delivery with a bounded cadence that accumulates displacement and emits coherent contact snapshots. Preserve down/up transitions as ordered barriers and cancellation as priority work. Measure callback → enqueue → dequeue → write start/completion and observed response. Increasing reports to 1000 Hz is not useful if the transport or game consumes them more slowly.

**Test:** the same path across polling/burst patterns, with simultaneous controls. Accept only improved consistency without increased stale-input age or stop/reversal error. Local viewer refresh is not a known phone input deadline; do not synchronize to it blindly.

### 3.5 Filters, resampling and prediction

| Technique | Evidence, applicability and cost |
| --- | --- |
| Timestamp-aware low-pass / One Euro | One Euro increases cutoff with estimated speed to trade low-speed jitter suppression against fast-motion lag. It handles variable sample intervals. Evaluate only if unwanted noise remains after conservation/lifecycle fixes; microaim and stopping can become sluggish. [CHI 2012 paper](https://gery.casiez.net/publications/CHI2012-casiez.pdf), [author implementations](https://github.com/casiez/OneEuroFilter). |
| Input resampling | Research shows asynchronous input/output rates can create visual spatial jitter even from smooth input. Estimating position relative to a known refresh can help, but the remote game's next sample time is currently unknown. [UIST 2020 paper](https://storage.googleapis.com/gweb-research2023-media/pubtools/5705.pdf), [publication record](https://research.google/pubs/modeling-and-reducing-spatial-jitter-caused-by-asynchronous-input-and-output-rates/). |
| Bounded extrapolation | Android and Chromium provide examples of constrained resampling with intentional delay/prediction limits. They are design references, not iOS defaults. Test reversals and errors, not only smooth constant-speed motion. [Android source](https://android.googlesource.com/platform/frameworks/native/+/86d048d6f0d02e568fd1f1360d13db2b3fc9d049/libs/input/InputTransport.cpp), [Chromium 132 declaration](https://chromium.googlesource.com/chromium/src/+/refs/tags/132.0.6834.31/ui/base/prediction/linear_resampling.h). |
| Injected-motion prediction | Predictions can overshoot or keep moving after a stop. Apple's predicted touches are provisional visual feedback replaced by real events; an action already applied inside another game cannot be erased the same way. Defer this. [Apple predicted touches](https://developer.apple.com/documentation/uikit/minimizing-latency-with-predicted-touches), [ECC 2016 forecasting paper](https://gery.casiez.net/publications/2016-ECC-latency-compensation.pdf). |

If a filter is eventually tested, apply it to a continuous logical trajectory before touch segmentation, not to discontinuous recentered coordinates. Never smooth button edges, cancellation or neutral stop. Compare stationary noise, slow tracking, flicks, stops and reversals at multiple event rates. Report both target error and settling tail.

Digital WASD changes are intentional steps, not automatically noise. Current diagonals are normalized. Interpolating between two full-radius vectors in Cartesian coordinates cuts inside the circle and can slow movement; angular interpolation preserves radius but must choose a path for a reversal. Direction easing should remain a lower-priority experiment with immediate neutral and measured strafing response. The retained neutral contact already avoids reanchoring without delaying releases. [Current joystick mapping](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/game.rs).

## 4. Presentation cadence and Wayland

### 4.1 The observed 165 Hz / approximately 60 FPS combination

A read-only monitor query during this research reported **2560×1600, 165.000 Hz, scale 1.25, VRR disabled**. No display mode was changed. Supported alternative modes were not established.

**Conditional calculation:** exactly 60 fresh pictures/s on fixed 165 Hz tear-free scanout cannot have uniform integer refresh holds: 165/60 = 2.75. Ideal scheduling mixes two- and three-refresh holds, approximately 12.12 and 18.18 ms. A supported 120 Hz mode could hold each 60 Hz picture for two refreshes, 16.67 ms. This does not prove current stutter: actual source rate, phase, tearing and compositor behavior matter. A lower monitor rate may also increase scanout waiting, so compare response age alongside cadence.

**Candidate:** after a presentation baseline, compare an advertised integer-multiple mode and supported VRR separately. Do not invent a modeline. VRR requires panel/driver/compositor support and an appropriate range; a switch alone does not prove variable scanout. Linux describes VRR_ENABLED as conditional on connector capability. [DRM/KMS documentation](https://docs.kernel.org/6.0/gpu/drm-kms.html). Account for 59.94 versus 60.00 and long-run phase drift.

### 4.2 Measure what was actually presented

Wayland presentation feedback can report presented/discarded commits, output identity, clock and reliability flags. Its zero-copy flag describes compositor presentation, not whether decoding downloaded into RAM. Frame callbacks indicate drawing opportunities; they do not prove photons. Integrating feedback requires care because wgpu owns surface commits. [Presentation-time specification](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/presentation-time/presentation-time.xml), [Wayland frame callbacks](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_surface-request-frame).

Assign frame identities through complete AU → decode → GUI selection → acquire → submit → presented/discarded. Record repeated/omitted pictures, intervals and selected-frame age. GPU timestamps can separate execution from CPU enqueue time, but are not scanout measurements. Vulkan display-timing/present-wait extensions offer related mechanisms only where supported and accessible; they are not automatically exposed by this application's wgpu surface API. [VK_GOOGLE_display_timing](https://docs.vulkan.org/refpages/latest/refpages/source/VK_GOOGLE_display_timing.html), [VK_KHR_present_wait](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_present_wait.html).

### 4.3 Compare policies rather than assuming Immediate wins

Immediate does not wait for vblank; Mailbox replaces a pending picture before presentation; FIFO preserves pending order. These imply different tearing, age and scheduling tradeoffs. [Vulkan present-mode definitions](https://docs.vulkan.org/refpages/latest/refpages/source/VkPresentModeKHR.html).

On Wayland, tearing permission is a compositor hint that may be ignored. Graphics backends may own the relevant protocol object, so creating a competing object is unsafe. Logging Immediate is not physical evidence of tearing or absence of compositor buffering. [Wayland tearing-control specification](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/tearing-control/tearing-control-v1.xml).

Compare supported modes with identical motion and display conditions. Mailbox is a credible low-age, tear-free candidate; FIFO may improve regularity while retaining old frames. Current `desired_maximum_frame_latency = 1` is a backend-clamped hint. Comparing one versus two may expose serialization or GPU starvation at one, at the possible cost of an extra pending picture. Measure acquire waits, p95/p99 age and visible intervals. [wgpu 25.0.2 surface configuration and modes](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu-types/src/lib.rs).

### 4.4 Missing pre-present notification and main-thread stalls

There is no `Window::pre_present_notify()` call in the inspected revision. winit 0.30.13 documents calling it before presentation; on Wayland it requests frame callbacks and throttles redraw delivery. This warrants an isolated comparison, not an automatic claim of lower latency: the additional scheduling constraint could also increase age. [winit documentation](https://docs.rs/winit/0.30.13/winit/window/struct.Window.html#method.pre_present_notify).

The GUI currently selects a frame before synchronous renderer work, and uploads before surface acquisition. If acquire blocks, the selected picture can age while newer output arrives, and GUI input handling can stall. Measure that duration first. Later candidates include selecting the latest picture after an acquisition wait or isolating submission work, but each complicates ownership, window-thread requirements and teardown. Test idle/occlusion, resize, focus-loss release, surface timeouts and reconnect. [Application](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/app.rs), [renderer](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/video/renderer.rs).

### 4.5 Bounded pacing can help, at an explicit age cost

If arrivals are irregular, a small deadline-based decoded-frame allowance might improve cadence. Sweep a fraction of a frame through at most a frame as experiments, not default recommendations; enforce an age ceiling and drop late decoded outputs. Do not arbitrarily discard encoded references. scrcpy defaults to no video buffering and makes extra buffering optional, illustrating the tradeoff. [scrcpy v4.0 buffering](https://github.com/Genymobile/scrcpy/blob/v4.0/doc/video.md#buffering).

Moonlight is a useful implementation reference for bounded pacing, drop accounting and Wayland callbacks. Its constants are not established defaults for this phone. [Pinned Moonlight pacer](https://github.com/moonlight-stream/moonlight-qt/blob/8369d1a0e11b999d4d1598f62ca5f6dea49602fb/app/streaming/video/ffmpeg-renderers/pacer/pacer.cpp), [Wayland timing source](https://github.com/moonlight-stream/moonlight-qt/blob/8369d1a0e11b999d4d1598f62ca5f6dea49602fb/app/streaming/video/ffmpeg-renderers/pacer/waylandvsyncsource.cpp). Android's Frame Pacing library likewise treats queue stuffing and presentation deadlines as distinct concerns; its library is not portable into this stack unchanged. [Android frame pacing](https://developer.android.com/games/sdk/frame-pacing).

## 5. Transport, capture and recovery

### 5.1 Inner UDP does not make the Wi-Fi path unreliable datagrams

The native Wi-Fi path authenticates pairing, opens a TCP connection with NODELAY, wraps it in TLS-PSK, then carries the CoreDevice IP tunnel. Video is inner UDP; HID is inner TCP/RemoteXPC. Both use the tunnel. Missing outer TCP bytes can delay later data, including inner UDP video. NODELAY does not remove ordered delivery or retransmission. [Current device connection](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/device/mod.rs), [pinned native tunnel](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/remote_pairing/tunnel.rs), [TCP standard](https://www.rfc-editor.org/rfc/rfc9293.html).

USB instead uses usbmuxd and CoreDeviceProxy, then constructs the same userspace network adapter over its stream. It removes the Wi-Fi outer path but does not bypass Apple capture, tunneling or all stream queues. [CoreDeviceProxy source](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/services/core_device_proxy.rs).

**Candidate:** qualify native Rust USB lifecycle/input first, then compare repeated USB and Wi-Fi runs with identical phone content, backend and presentation settings. Record charging and thermal differences. A cable connection is not proof of a latency improvement.

### 5.2 Dependency queues and maintenance timing

Pinned jktcp 0.1.7 uses unbounded command and per-socket channels. A stalled consumer can accumulate data before the application's four-AU channel. That is an unbounded-capacity risk, **not evidence of an actual backlog**. The handle flushes on sends and incoming traffic. Its maintenance branch sleeps 250 ms despite a 1 ms comment; it does not impose 250 ms on every normal send. [Exact packaged source commit `216d796f`](https://github.com/jkcoxson/jktcp/blob/216d796f8e5260ab682a9445ea8257899d16fdf7/src/handle.rs#L97-L253).

The inner TCP implementation starts retransmission timing at 200 ms and backs off; normal writes send subject to window/in-flight limits. Inspect real code instead of treating older stop-and-wait comments as current behavior. Outer Linux TCP is a separate layer. [Adapter constants](https://github.com/jkcoxson/jktcp/blob/216d796f8e5260ab682a9445ea8257899d16fdf7/src/adapter.rs#L41-L52), [flush/retry loop](https://github.com/jkcoxson/jktcp/blob/216d796f8e5260ab682a9445ea8257899d16fdf7/src/adapter.rs#L417-L540).

**Instrumentation proposal:** count queued items and bytes, oldest-item age, high-water marks, wake-to-service time and incoming burst lengths. Correlate with first/last AU packets and input age. Use sampled counters or bounded rings rather than formatting every packet.

If queues remain empty, leave them alone. If they grow, define overflow/recovery deliberately. Blocking a shared adapter on a bounded video channel can starve HID; dropping arbitrary compressed packets damages reference chains. A protocol-aware abandon-and-keyframe path or controlled failure is safer than silent corruption. Queue capacity alone never establishes current delay.

### 5.3 HID write completion is not a phone acknowledgment

Pinned `send_report` sends RemoteXPC without requesting an application reply, and the HTTP/2 path flushes. Awaiting it establishes local transport work, not that iOS delivered the touch or the game consumed it. “Remove the per-event RPC round trip” therefore misdescribes this implementation. [HID send](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/services/core_device/hid.rs#L1090-L1100), [RemoteXPC](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/xpc/mod.rs#L176-L201), [HTTP/2](https://github.com/jkcoxson/idevice/blob/d32c8189c51c2789496b0768039419c3705498c3/idevice/src/xpc/http2/mod.rs).

Track local event age separately from observed phone response. Apple offers low-latency dispatch preferences inside a participating UIKit app, but the Linux viewer cannot apply them to another developer's game. [UIUpdateLink preference](https://developer.apple.com/documentation/uikit/uiupdatelink/wantslowlatencyeventdispatch).

### 5.4 Measure loss, reordering and recovery before adding buffers

Current RTP handling invalidates an AU on a forward sequence gap and discards late packets. There is no short reorder window. Receiver reports contain zeroed loss/jitter/sender timing fields, and incoming RTCP is not used for timing analysis. They must not be treated as measured network-quality feedback. [Current RTP implementation](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/rtp.rs).

RTP/RTCP supplies sequence, jitter and sender-clock relationships, but jitter is transit variation, not one-way latency. Correct statistics need per-source state, wrap, duplicate and late-packet handling. Dynamic payload IDs are not universal codec labels. [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550.html).

PLI requests repair of damaged decoding context; it does not guarantee an immediate IDR. NACK can help only if the sender supports retransmission and repair arrives before the deadline. More feedback can increase work without improving freshness. [RFC 4585](https://www.rfc-editor.org/rfc/rfc4585.html#section-6). HEVC fragmentation and reference dependencies require whole-AU integrity; dropping already decoded output is different from dropping compressed references. [RFC 7798](https://www.rfc-editor.org/info/rfc7798/).

**Candidate only if recoverable reordering is observed:** compare bounded 0/2/4 ms reorder windows, reporting repaired AUs and added age. Those are sweep points, not defaults. GStreamer's jitterbuffer documents explicit added latency and defaults to 200 ms; copying that default would conflict with this project's goal. [GStreamer jitterbuffer](https://gstreamer.freedesktop.org/documentation/rtpmanager/rtpjitterbuffer.html). An adaptive policy must shrink again after conditions improve and obey an age ceiling. [Interactive-media congestion requirements](https://www.rfc-editor.org/rfc/rfc8836.html).

### 5.5 Encoder negotiation: request, acceptance and actual behavior differ

The inspected start API does not expose verified FPS, resolution or bitrate controls. Offer construction is reverse-engineered. Its tier table and unexplained integers should not be relabeled as a simple FPS/bitrate switch. [Current offer](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/device/offer.rs), [start request](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/device/mod.rs).

Upstream probe notes distinguish accepted and effective settings: FEC is requested without establishing repair operation, RTX was rejected, and a tile request was accepted but reportedly ignored. The current viewer already disables LTRP and omits VRAE:0, following reference findings. Reported effects on a different phone are not gains for this one. [pymobiledevice3 v11.13.1 offer/probes](https://github.com/doronz88/pymobiledevice3/blob/v11.13.1/pymobiledevice3/remote/core_device/media_stream_offer.py), [DisplayService notes](https://github.com/doronz88/pymobiledevice3/blob/v11.13.1/pymobiledevice3/remote/core_device/display_service.py).

Future negotiation experiments should record requested values, allowlisted returned fields, actual dimensions/codec parameter sets, distinct-picture cadence, bitrate/bursts, keyframe frequency, quality and age. Reverse the change and repeat. Resizing the viewer does not prove capture resolution changed. Encoder low-delay controls in Sunshine are useful concepts, but the Apple source is not Sunshine/NVENC. [Sunshine encoder configuration](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2configuration.html).

### 5.6 Network and power experiments with realistic scope

Compare USB, phone-on-Wi-Fi with the computer wired to the AP, and both devices wireless. Removing one wireless leg may reduce contention, but only if that path is limiting. Record band/channel, signal, competing traffic and retransmission indicators. Base iPhone 15 has Wi-Fi 6 and USB 2 data capability; do not assume Wi-Fi 6E or that a new cable grants USB 3. [Apple iPhone 15 specifications](https://support.apple.com/en-au/111831).

Host Wi-Fi power saving is a candidate for idle-to-first-input tails, not a demonstrated constant cost during continuous streaming. A later per-connection A/B can record and restore the original setting. It says nothing about phone power policy. [Linux dynamic power save](https://wireless.docs.kernel.org/en/latest/en/users/documentation/dynamic-power-save.html), [NetworkManager property](https://networkmanager.dev/docs/api/latest/settings-802-11-wireless.html).

Fair queueing/AQM may help competing traffic at the actual bottleneck. A single encrypted tunnel hides its inner flows, and WAN queue management cannot fix a LAN bottleneck it does not traverse. [FQ-CoDel design](https://www.rfc-editor.org/rfc/rfc8290.html).

QUIC DATAGRAM avoids reliable retransmission for each datagram, but phone support is the gate. Current upstream explicitly rejects its QUIC tunnel on iOS 18.2 and newer, directing users to TCP. Historical QUIC code is not evidence of an available iOS 27 shortcut. [QUIC DATAGRAM standard](https://www.rfc-editor.org/rfc/rfc9221.html), [pymobiledevice3 compatibility path](https://github.com/doronz88/pymobiledevice3/blob/v11.13.1/pymobiledevice3/remote/tunnel_service.py).

## 6. GPU and decoder work: credible, but measure its share first

### 6.1 Remove one CPU copy before full interoperability

The present path downloads hardware frames to CPU memory, copies planes into owned pooled vectors, then uploads to wgpu. It is not zero-copy. A candidate is retaining reference-counted CPU AVFrames in the latest-frame mailbox and uploading with their strides, eliminating the intermediate full planar copy while retaining the hardware download. At 1184×2576 YUV420, one picture is about 4.57 MB, or 275 MB/s of payload per full copy at 60 FPS, before alignment/read-write traffic. This arithmetic is not a latency prediction. [Current decoder](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/src/video/decoder.rs).

Retained frames must outlive GPU upload access, and the decoder must not overwrite shared transfer storage. Test strides, negative linesizes, NV12/planar formats, rotation, resolution changes, mailbox replacement and teardown. Bound retained frames so the optimization does not exhaust decoder surfaces.

wgpu queue writes use staging resources; pooled application vectors do not establish allocation-free uploads. Profile staging before considering a bounded reusable staging ring. Buffer-to-texture copies have alignment requirements, and StagingBelt is not a drop-in planar texture uploader. [wgpu queue](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu/src/api/queue.rs), [core upload implementation](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu-core/src/device/queue.rs), [StagingBelt](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu/src/util/belt.rs). Skipping uploads when a UI-only redraw retains the same video frame is a smaller candidate, probably minor during continuous motion.

### 6.2 Same-GPU selection is a prerequisite to many interop designs

The historical path decoded on NVIDIA and rendered on AMD. NVIDIA's CUDA/Vulkan interoperability guidance requires matching device UUIDs for that shared-resource path. It is not a direct CUDA-to-AMD shortcut. Compare NVIDIA decode/render and AMD VAAPI/render against the existing mixed path; record the scanout adapter too, because the compositor may add another cross-GPU transfer. [CUDA 12.8.1 Vulkan interoperability](https://docs.nvidia.com/cuda/archive/12.8.1/cuda-c-programming-guide/index.html#vulkan-interoperability).

| Design | Potential benefit | Main engineering gates |
| --- | --- | --- |
| VAAPI → DRM PRIME/dma-buf → Vulkan | Avoid CPU download/repack/upload | Supported modifiers/planes, synchronization before reads, FD ownership, retained surface lifetime and compatible GPU |
| NVDEC/CUDA → shared Vulkan surface | Avoid CPU boundary, possibly with one GPU-local copy | Matching NVIDIA device, exportable allocation, external semaphores, layout/queue ownership |
| Shared-device Vulkan Video decode/render | Decode and sample within one API/device | HEVC driver/profile support, queue families/extensions, FFmpeg/wgpu device creation and timeline synchronization |

VAAPI export alone does not synchronize decoder completion. DRM format modifiers, offsets and pitches must match imports. [libva export contract](https://github.com/intel/libva/blob/master/va/va.h), [FFmpeg VAAPI mapping](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavutil/hwcontext_vaapi.c), [Vulkan dma-buf extension](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_external_memory_dma_buf.html), [modifier extension](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_image_drm_format_modifier.html).

A GPU-local copy into a compatible surface may be realistic even when importing decoder-owned CUDA storage directly is not. Label that accurately rather than claiming zero-copy. [NVDEC 13.0 guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvdec-video-decoder-api-prog-guide/index.html). wgpu offers unsafe HAL wrapping, but the texture must belong to the correct internal device and satisfy initialization/layout/lifetime requirements. [wgpu Device](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu/src/api/device.rs), [Vulkan HAL](https://github.com/gfx-rs/wgpu/blob/v25.0.2/wgpu-hal/src/vulkan/device.rs).

Vulkan Video HEVC capability is not universal availability. FFmpeg AVVkFrame carries image state and timeline synchronization that a renderer must honor. [Vulkan HEVC extension](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_video_decode_h265.html), [FFmpeg Vulkan frame contract](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavutil/hwcontext_vulkan.h). Moonlight demonstrates relevant VAAPI and shared-Vulkan integration, but not a measured speedup for this host. [VAAPI reference](https://github.com/moonlight-stream/moonlight-qt/blob/8369d1a0e11b999d4d1598f62ca5f6dea49602fb/app/streaming/video/ffmpeg-renderers/vaapi.cpp), [Vulkan reference](https://github.com/moonlight-stream/moonlight-qt/blob/8369d1a0e11b999d4d1598f62ca5f6dea49602fb/app/streaming/video/ffmpeg-renderers/plvk.cpp).

### 6.3 Decoder switches have limits

Frame threading can improve throughput while increasing output delay; current one-thread decoding already avoids that pipeline expansion. If software decode misses deadlines, compare supported slice/WPP work with frame threading explicitly, including first-output latency. [FFmpeg thread documentation](https://ffmpeg.org/doxygen/8.0/structAVCodecContext.html).

HEVC stream reorder requirements come from the bitstream. LOW_DELAY cannot delete required references. This direct-AVPacket application does not benefit from generic demux probe-size/nobuffer advice. NVDEC's parser display-delay field is not an exposed switch in the current native-HEVC-plus-NVDEC path. [FFmpeg HEVC source](https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavcodec/hevc/hevcdec.c), [FFmpeg NVDEC integration](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavcodec/nvdec.c).

Hardware acceleration can lose its advantage when transfers dominate. A matched software comparison remains worthwhile at reduced resolutions or on different hardware, but offline decoding-to-null throughput is not interactive latency. [FFmpeg hardware-acceleration documentation](https://ffmpeg.org/ffmpeg-all.html#Advanced-Video-options). Keep the proven backend while evaluating alternatives; reducing decoder surface counts until they starve is not an optimization.

## 7. Phone cadence, native input and alternate architectures

### 7.1 Source stability and more than 60 FPS

Game rendering, capture, encoded pictures, decoded outputs and monitor refresh are distinct rates. The qualified stream was approximately 60 FPS. No greater-than-60 CoreDevice stream or verified source-rate control was established. A faster monitor can change scanout waiting and cadence, but cannot create new source pictures. The decoder's synthetic sequence timebase of 1/60 is bookkeeping, not proof of exact source timing.

Apple documents that actual display-link rates depend on policy and conditions. Thermal/power changes can reduce updates. Record game graphics/FPS settings, charging, warmup, ambient conditions and native phone cadence; alternate test order to limit heat drift. [CADisplayLink](https://developer.apple.com/documentation/QuartzCore/CADisplayLink), [Apple power/thermal guidance](https://developer.apple.com/documentation/xcode/responding-to-power-notifications).

Game Mode prioritizes game CPU/GPU access and changes Bluetooth accessory sampling. The latter does not imply doubled CoreDevice touch sampling. Checking its effect is a future source-stability experiment, not a setting changed here. [Apple Game Mode](https://support.apple.com/en-us/105118).

If irregular movement appears on the phone itself, investigate native frame pacing, game response/assists or server correction before attributing it to the mirror. The recent auto-fire symptom had a game-setting explanation; this research did not revisit those settings. Keep input correctness and gameplay behavior separate from video cadence.

### 7.2 Native mouse/controller support could avoid touch recentering

Apple exposes connected keyboard state and raw mouse delta input to participating games. These are consumption APIs, not a systemwide Linux injection service. This particular game's native support and a usable remote input path must both be established. [GCKeyboard](https://developer.apple.com/documentation/gamecontroller/gckeyboard), [GCMouseInput](https://developer.apple.com/documentation/gamecontroller/gcmouseinput).

GCVirtualController adds controls inside its participating app; it is not evidence of a general virtual controller routed into unrelated games. A physical controller also needs game support, and stick input is not inherently equivalent to a relative mouse. [Apple virtual-controller session](https://developer.apple.com/videos/play/wwdc2021/10081/), [controller discovery](https://developer.apple.com/documentation/gamecontroller/discovering-game-controllers).

scrcpy's UHID and AOA modes demonstrate native relative-device forwarding on Android. Linux UHID and Android accessory requests do not establish corresponding iOS interfaces. Copying that code cannot by itself provide GCMouse input to Rainbow Six Mobile. [scrcpy v4.1 mouse modes](https://github.com/Genymobile/scrcpy/blob/v4.1/doc/mouse.md), [Linux UHID](https://docs.kernel.org/hid/uhid.html), [Android AOA2](https://source.android.com/docs/core/interaction/accessories/aoa2). Console/PC Rainbow Six Siege support announcements must not be cited as Mobile compatibility evidence.

### 7.3 Direct display output is a separate hardware path

Apple supports iPhone 15 external-display mirroring over USB-C/DisplayPort, up to 4K60 for the documented connection. A direct display path could bypass this CoreDevice HEVC viewer pipeline. That does not establish the game's output latency or solve keyboard/mouse input. [Apple USB-C display support](https://support.apple.com/en-us/105099).

Bringing that output back into a Linux window through capture hardware would add a different capture/driver/buffering pipeline. It might improve or worsen age; it needs an optical comparison before any purchase or architectural commitment. DisplayPort capability is not proof that the phone exposes a UVC webcam stream. This is an alternative research branch, not a software toggle in this repository.

### 7.4 Late warp and generated frames are speculative here

Research demonstrates that late input and post-render warping can improve aiming in a controlled engine setup. It does not demonstrate a transparent fix for an arbitrary decoded phone stream. [Post-Render Warp, HPG 2020](https://research.nvidia.com/publication/2020-07_post-render-warp-late-input-sampling-improves-aiming-under-high-latency).

Modern frame-generation/warping systems can use engine motion, depth, camera or auxiliary buffers that this viewer does not receive. Pixels and raw mouse events are insufficient to know the game's exact camera transform, recoil, disocclusion or HUD composition. [NVIDIA DLSS research](https://research.nvidia.com/labs/adlr/DLSS4/).

**Inference:** two-frame interpolation needs a later picture; extrapolation can avoid that wait but invents content. Generated intermediate pictures are not new game simulation responses. Warping could make the displayed reticle disagree with actual game state or reveal empty regions. Treat this as a separate optional research project, after fidelity and timing, rather than the route to honest low-latency control.

## 8. Measurement and experiment design

### 8.1 Fix measurement coverage before reporting gains

The existing synthetic page chooses the lowest RTT of six clock requests, uses a midpoint offset and refreshes calibration every 30 seconds. It draws browser animation timestamps into pixels. Its age includes browser scheduling/capture and ends at host submission. The current receive timer starts only after the last AU packet reaches the application, downstream of tunnel queues. [Timestamp workload](https://github.com/Jakeelamb/iphone-mirror-rs/blob/f1958b5acc525f1d268623448c70df95eb751ea0/examples/latency_page.rs), [metric definitions](performance.md#what-the-counters-measure).

A four-timestamp exchange can separate server residence from network RTT and estimate offset, but cannot determine exact one-way delay under asymmetry. Track calibration age/drift and use monotonic clocks for local intervals. Do not claim a sub-millisecond change from a several-millisecond clock uncertainty envelope. [NTP timing model](https://datatracker.ietf.org/doc/html/rfc5905#section-8).

Linux socket timestamping can distinguish kernel scheduling/driver boundaries where supported. TCP acknowledgment timestamps still acknowledge bytes, not game actions; hardware clocks need correlation and stream-byte IDs need correct matching. This is optional deeper instrumentation, not a replacement for content/frame identity. [Kernel timestamping documentation](https://docs.kernel.org/networking/timestamping.html).

For optical ground truth, a proposed rig films an input-linked LED, the phone and the monitor in the same high-speed sequence. Compare trigger→phone, trigger→monitor and phone→monitor at comparable screen positions. At 240 FPS, camera sampling alone is about 4.17 ms per frame, before exposure, rolling shutter and threshold uncertainty. Randomize trigger phase, repeat, and report uncertainty; dual photodiodes provide a stronger timing check. WALT and LDAT provide primary examples of physical sensing methods, not turnkey qualification of this game. [Google WALT](https://github.com/google/walt), [WALT synchronization notes](https://github.com/google/walt/blob/master/android/WALT/app/src/main/jni/README.md), [NVIDIA LDAT](https://developer.nvidia.com/nvidia-latency-display-analysis-tool).

### 8.2 Useful metrics and controlled workloads

| Outcome | Record | Reject misleading substitutes |
| --- | --- | --- |
| Input fidelity | Intended/emitted displacement, contact transitions, gain, stop/reversal error | Only counting HID writes |
| Input age | Callback/enqueue/dequeue/write boundaries and visible response | Calling local write completion a phone acknowledgment |
| Cadence | Distinct frame IDs, interval distributions, repeated holds and stall runs | Average FPS or redraw count alone |
| Pipeline age | First/last packet, oldest queue entry, selected frame, presented result | Treating configured queue capacity as actual occupancy |
| Recovery | Damage event→first valid fresh picture, repair requests/responses | Counting PLI sends as successful repairs |
| Resources | CPU, allocations, RSS, GPU execution and stable thermal conditions | Instrumented CPU numbers compared to an uninstrumented baseline |

Use deterministic scrolling to isolate video, then fixed training-scene input paths for slow aim, flick, stop, reversal, W↔A/D, overlap/all-up gaps, movement+look+lean, and movement+look+fire/ADS. Test release of each independently, followed by Escape/focus loss. Keep the scene and settings fixed within comparisons.

Separate startup, warmup and steady state. Report sample counts, invalid/dropped samples, p50/p95/p99, maxima, visible stall runs and repeated-run variability. Use randomized A/B order and a return to baseline. A short sample cannot support a stable extreme percentile; choose run lengths around the symptom frequency. Correlate per-frame stages rather than subtracting independently aggregated medians.

### 8.3 Scheduling and tracing overhead

If input or tunnel queues age while CPU averages remain low, investigate scheduling stalls rather than assuming insufficient compute. Tokio console exposes task scheduling/polling information and requires instrumentation; compare overhead-on/off separately from release timing. [Tokio console](https://github.com/tokio-rs/console). GPU acquire waits on the GUI thread are another candidate already identified above.

Avoid real-time priority, busy polling, CPU pinning or a different kernel as first interventions. They cannot remove phone capture or network delay and can shift contention elsewhere. Linux documents runtime limits and starvation concerns for real-time scheduling. Consider a narrowly scoped scheduling experiment only after runnable-to-running latency is shown to matter. [Kernel real-time scheduling](https://docs.kernel.org/scheduler/sched-rt-group.html).

### 8.4 Ranked future experiments

These are proposals for a later implementation/testing phase. None ran during this research.

| Order | One variable or question | Required evidence | Stop/reject condition |
| --- | --- | --- | --- |
| 1 | Equal mouse displacement under different event grouping | Emitted path, gain, resets, stop tail; then camera response | Lost movement replaced with unbounded replay |
| 2 | Presentation instrumentation and optical baseline | Presented IDs/intervals plus response uncertainty | Instrumentation materially changes timing |
| 3 | Present mode; separately pre_present_notify; separately latency hint 1/2 | Age and hold distributions, acquire waits, input handling | Smoother averages but worse response tails or release behavior |
| 4 | Advertised integer-multiple refresh; separately supported VRR | Actual presentation cadence and optical age | Unsupported mode, no observed VRR, or harmful latency tradeoff |
| 5 | Queue ages from tunnel through input/video | Byte/item high-water marks, oldest age, burst correlations | Tuning queues without observed growth |
| 6 | Qualified USB versus Wi-Fi; then host-wired AP leg | Repeated matched trials, charging/heat recorded | USB lifecycle/control regression or unmatched source conditions |
| 7 | Safe look-region size and contact cadence | Fewer resets, packetization-independent gain, independent simultaneous controls | HUD activation, wrong control model, delayed stop |
| 8 | Phone graphics/source stability and Game Mode | Native phone and encoded distinct-picture cadence | Only viewer FPS changes, or thermal confounding |
| 9 | Copy removal and same-GPU selection | Transfer/CPU/GPU stage reduction and whole-path result | Hidden copies, surface exhaustion, worse power or age |
| 10 | Real negotiated source resolution/rate/recovery controls | Accepted configuration plus actual bitstream/quality/timing change | RPC success without actual behavior change |
| 11 | Small reorder/presentation buffer only for observed jitter | Repair/stall improvement against explicit age budget | Buffer target only grows or stale age exceeds budget |
| 12 | Optional filtering/resampling | Microaim error versus lag and reversal/stop error | Better-looking path with less faithful or slower control |

## 9. Approaches the evidence does not justify

- Rewriting the project again, adding assembly, or removing the joystick dwell solely because it appears in a profile.
- Applying that joystick dwell to every mouse-look reanchor without a separate experiment.
- Calling all mouse units pixels, interpreting host timestamps as synchronized phone input time, or assuming a faster mouse report rate forces faster game sampling.
- Adding a large blanket jitter buffer, blindly shrinking all queues, or discarding arbitrary HEVC reference packets.
- Treating an Immediate log, a submitted-frame count or an RPC success as proof of physical output behavior.
- Claiming zero-copy because a DMA handle exists, without synchronization/lifetime and actual-copy verification.
- Treating source FPS, generated pictures and monitor refresh as the same quantity.
- Copying Android UHID/AOA, Sunshine encoder flags, or old QUIC code and assuming the current iPhone protocol supports them.
- Promising imperceptible latency or a percentage speedup before matched end-to-end evidence exists.

## 10. Provenance and remaining unknowns

The external sources linked beside claims are official specifications, author papers, vendor documentation or upstream implementation source. Main implementation references were pinned to this project `f1958b5`, idevice `d32c8189`, jktcp 0.1.7 package commit `216d796f`, winit 0.30.13, wgpu 25.0.2, pymobiledevice3 v11.13.1 and the linked Moonlight commit. scrcpy versions are explicit per source. FFmpeg references include n8.0 hardware interfaces and n9.0 HEVC behavior; these are not a fresh audit of all distro patches. Vendor latest/master pages are snapshots accessed on the research date.

Wayland protocol web raw endpoints were unavailable during part of the investigation; the installed canonical XML was read and its upstream locations are linked. The late-warp conclusion uses the authors' publication abstract; inaccessible full-PDF endpoints were not treated as inspected experimental detail. No third-party benchmark was converted into a claimed speedup for this application.

Remaining empirical unknowns include the game's exact camera mapping and sampling phase; useful HID rate and minimum reliable anchor spacing; native game mouse/controller support through a usable connection; real lower-level queue occupancy; phone capture/encode delay; supported/effective encoder controls; current displayed-frame timing; actual VRR/direct-scanout behavior; USB performance; and which interop paths the installed drivers support. Those gaps are the reason for the ordered experiments above.

The research supports concrete next tests, especially displacement conservation and presentation timing. It does not establish that any proposed optimization has already improved the running tool.
