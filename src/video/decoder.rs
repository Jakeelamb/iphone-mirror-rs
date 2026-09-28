use super::frame::{DecodedFrame, Layout, Plane, PlanePool};
use anyhow::{Context, Result, bail, ensure};
use ffmpeg_sys_next as av;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub enum DecodeMode {
    #[default]
    Auto,
    Cuda,
    Vaapi,
    Software,
}

/// Single-thread-owned FFmpeg decoder. Complete access units bypass demuxer and
/// parser buffering. Packet and presentation storage are pooled after warmup.
pub struct Decoder {
    context: *mut av::AVCodecContext,
    packet: *mut av::AVPacket,
    frame: *mut av::AVFrame,
    transfer: *mut av::AVFrame,
    packet_pool: *mut av::AVBufferPool,
    packet_capacity: usize,
    hardware_format: Box<av::AVPixelFormat>,
    planes: PlanePool,
    timestamps: VecDeque<(i64, Instant)>,
    sequence: i64,
    last_pixel_format: Option<i32>,
}

// The decoder is exclusively accessed through &mut self; FFmpeg contexts may
// move between threads but must never be called concurrently.
unsafe impl Send for Decoder {}

unsafe extern "C" fn choose_format(
    context: *mut av::AVCodecContext,
    formats: *const av::AVPixelFormat,
) -> av::AVPixelFormat {
    // SAFETY: FFmpeg calls this with its live context and a NONE-terminated list.
    unsafe {
        let wanted = *((*context).opaque.cast::<av::AVPixelFormat>());
        let mut current = formats;
        let mut software = av::AVPixelFormat::AV_PIX_FMT_NONE;
        while *current != av::AVPixelFormat::AV_PIX_FMT_NONE {
            if *current == wanted {
                return wanted;
            }
            if matches!(
                *current,
                av::AVPixelFormat::AV_PIX_FMT_YUV420P | av::AVPixelFormat::AV_PIX_FMT_NV12
            ) {
                software = *current;
            }
            current = current.add(1);
        }
        software
    }
}

impl Decoder {
    /// None before the first output frame; then the actual selected backend.
    pub fn hardware_active(&self) -> Option<bool> {
        self.last_pixel_format.map(|format| {
            format == *self.hardware_format as i32
                && *self.hardware_format != av::AVPixelFormat::AV_PIX_FMT_NONE
        })
    }

    pub fn new(mode: DecodeMode) -> Result<Self> {
        // SAFETY: Allocation/free ownership is kept by this struct and Drop.
        unsafe {
            let codec = av::avcodec_find_decoder(av::AVCodecID::AV_CODEC_ID_HEVC);
            ensure!(!codec.is_null(), "FFmpeg has no HEVC decoder");
            let mut decoder = Self {
                context: av::avcodec_alloc_context3(codec),
                packet: av::av_packet_alloc(),
                frame: av::av_frame_alloc(),
                transfer: av::av_frame_alloc(),
                packet_pool: ptr::null_mut(),
                packet_capacity: 0,
                hardware_format: Box::new(av::AVPixelFormat::AV_PIX_FMT_NONE),
                planes: Arc::new(Mutex::new(Vec::with_capacity(4))),
                timestamps: VecDeque::with_capacity(32),
                sequence: 0,
                last_pixel_format: None,
            };
            ensure!(
                !decoder.context.is_null()
                    && !decoder.packet.is_null()
                    && !decoder.frame.is_null()
                    && !decoder.transfer.is_null(),
                "FFmpeg allocation failed"
            );
            (*decoder.context).thread_count = 1;
            (*decoder.context).flags |= av::AV_CODEC_FLAG_LOW_DELAY as i32;
            (*decoder.context).pkt_timebase = av::AVRational { num: 1, den: 60 };
            if !matches!(mode, DecodeMode::Software) {
                decoder.try_hardware(codec, mode);
            }
            check(av::avcodec_open2(decoder.context, codec, ptr::null_mut()))
                .context("open HEVC decoder")?;
            tracing::info!(
                hardware = ?*decoder.hardware_format,
                "HEVC decoder opened; hardware frames are downloaded before wgpu upload"
            );
            Ok(decoder)
        }
    }

    unsafe fn try_hardware(&mut self, codec: *const av::AVCodec, mode: DecodeMode) {
        // Prefer direct CUDA over the NVIDIA VAAPI bridge: live profiling found
        // the bridge allocates an intermediate image on every hardware download.
        // VAAPI remains the fallback on non-NVIDIA machines. Explicit Vaapi also
        // tries direct AMD nodes before the system driver preference.
        unsafe {
            let preferred = if matches!(mode, DecodeMode::Vaapi) {
                amd_render_nodes(Path::new("/sys/class/drm"), Path::new("/dev/dri"))
            } else {
                Vec::new()
            };
            for kind in hardware_order(mode) {
                let mut index = 0;
                loop {
                    let config = av::avcodec_get_hw_config(codec, index);
                    if config.is_null() {
                        break;
                    }
                    index += 1;
                    if (*config).device_type != kind
                        || (*config).methods & av::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32
                            == 0
                    {
                        continue;
                    }
                    let mut device = ptr::null_mut();
                    if kind == av::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                        for node in &preferred {
                            match create_hardware_device(kind, Some(node)) {
                                Ok(candidate) => {
                                    device = candidate;
                                    tracing::info!(device = %node.display(), driver = "radeonsi",
                                        "selected direct AMD VAAPI decoding");
                                    break;
                                }
                                Err(error) => tracing::debug!(device = %node.display(), %error,
                                    "preferred VAAPI device unavailable"),
                            }
                        }
                    }
                    if device.is_null() {
                        match create_hardware_device(kind, None) {
                            Ok(candidate) => device = candidate,
                            Err(error) => {
                                tracing::debug!(?kind, %error, "hardware decoder unavailable");
                                continue;
                            }
                        }
                    }
                    *self.hardware_format = (*config).pix_fmt;
                    (*self.context).hw_device_ctx = device;
                    (*self.context).opaque =
                        (&mut *self.hardware_format as *mut av::AVPixelFormat).cast();
                    (*self.context).get_format = Some(choose_format);
                    (*self.context).extra_hw_frames = 1;
                    return;
                }
            }
        }
    }

    pub fn decode(
        &mut self,
        annex_b: &[u8],
        received_at: Instant,
        mut emit: impl FnMut(DecodedFrame),
    ) -> Result<()> {
        ensure!(!annex_b.is_empty(), "empty HEVC access unit");
        ensure!(
            annex_b.len() <= 32 * 1024 * 1024,
            "HEVC access unit exceeds 32 MiB limit"
        );
        let started = Instant::now();
        // SAFETY: Packet pool includes FFmpeg's mandatory zero padding. The
        // AVBufferRef attached to the packet keeps data live across send_packet.
        unsafe {
            av::av_packet_unref(self.packet);
            let required = annex_b.len() + av::AV_INPUT_BUFFER_PADDING_SIZE as usize;
            if required > self.packet_capacity {
                av::av_buffer_pool_uninit(&mut self.packet_pool);
                self.packet_capacity = required.next_power_of_two();
                self.packet_pool = av::av_buffer_pool_init(self.packet_capacity, None);
                ensure!(!self.packet_pool.is_null(), "allocate packet buffer pool");
            }
            let buffer = av::av_buffer_pool_get(self.packet_pool);
            ensure!(!buffer.is_null(), "allocate compressed packet buffer");
            (*self.packet).buf = buffer;
            (*self.packet).data = (*buffer).data;
            (*self.packet).size = annex_b.len() as i32;
            ptr::copy_nonoverlapping(annex_b.as_ptr(), (*buffer).data, annex_b.len());
            ptr::write_bytes(
                (*buffer).data.add(annex_b.len()),
                0,
                av::AV_INPUT_BUFFER_PADDING_SIZE as usize,
            );
            self.sequence += 1;
            (*self.packet).pts = self.sequence;
            (*self.packet).dts = self.sequence;
            self.timestamps.push_back((self.sequence, received_at));
            // Broken input must not grow timestamp bookkeeping indefinitely.
            if self.timestamps.len() > 128 {
                self.timestamps.pop_front();
            }
            let mut result = av::avcodec_send_packet(self.context, self.packet);
            if result == -libc::EAGAIN {
                self.drain(&mut emit)?;
                result = av::avcodec_send_packet(self.context, self.packet);
            }
            av::av_packet_unref(self.packet);
            check(result).context("submit HEVC access unit")?;
            self.drain(&mut emit)?;
        }
        tracing::trace!(
            stage = "decode",
            elapsed_us = started.elapsed().as_micros() as u64
        );
        Ok(())
    }

    pub fn flush(&mut self, mut emit: impl FnMut(DecodedFrame)) -> Result<()> {
        // SAFETY: A null packet is FFmpeg's documented end-of-stream marker.
        unsafe {
            let result = av::avcodec_send_packet(self.context, ptr::null());
            if result == -libc::EAGAIN {
                self.drain(&mut emit)?;
                check(av::avcodec_send_packet(self.context, ptr::null()))?;
            } else if result != av::AVERROR_EOF {
                check(result)?;
            }
            self.drain(&mut emit)
        }
    }

    /// Call at a transport discontinuity, then resume at a keyframe with headers.
    pub fn reset(&mut self) {
        // SAFETY: Exclusive live codec context.
        unsafe { av::avcodec_flush_buffers(self.context) };
        self.timestamps.clear();
    }

    unsafe fn drain(&mut self, emit: &mut impl FnMut(DecodedFrame)) -> Result<()> {
        unsafe {
            loop {
                let result = av::avcodec_receive_frame(self.context, self.frame);
                if result == -libc::EAGAIN || result == av::AVERROR_EOF {
                    return Ok(());
                }
                check(result).context("decode HEVC picture")?;
                if self.last_pixel_format != Some((*self.frame).format) {
                    self.last_pixel_format = Some((*self.frame).format);
                    let hardware = (*self.frame).format == *self.hardware_format as i32
                        && *self.hardware_format != av::AVPixelFormat::AV_PIX_FMT_NONE;
                    tracing::info!(
                        hardware,
                        pixel_format = (*self.frame).format,
                        width = (*self.frame).width,
                        height = (*self.frame).height,
                        "active HEVC decoder output"
                    );
                }
                let pts = (*self.frame).pts;
                let received_at = self
                    .timestamps
                    .iter()
                    .position(|(sequence, _)| *sequence == pts)
                    .and_then(|index| self.timestamps.remove(index))
                    .map(|(_, time)| time)
                    .unwrap_or_else(Instant::now);
                let source = if (*self.frame).format == *self.hardware_format as i32
                    && *self.hardware_format != av::AVPixelFormat::AV_PIX_FMT_NONE
                {
                    let transfer_started = Instant::now();
                    // Keep the CPU transfer allocation across frames. FFmpeg
                    // accepts a fully allocated destination for repeated copies.
                    if (*self.transfer).width != (*self.frame).width
                        || (*self.transfer).height != (*self.frame).height
                    {
                        av::av_frame_unref(self.transfer);
                    }
                    check(av::av_hwframe_transfer_data(self.transfer, self.frame, 0))
                        .context("download hardware-decoded picture")?;
                    (*self.transfer).color_range = (*self.frame).color_range;
                    (*self.transfer).colorspace = (*self.frame).colorspace;
                    tracing::trace!(
                        stage = "hardware_download",
                        elapsed_us = transfer_started.elapsed().as_micros() as u64
                    );
                    self.transfer
                } else {
                    self.frame
                };
                let output = self.copy_frame(source, received_at);
                av::av_frame_unref(self.frame);
                emit(output?);
            }
        }
    }

    unsafe fn copy_frame(
        &self,
        source: *const av::AVFrame,
        received_at: Instant,
    ) -> Result<DecodedFrame> {
        unsafe {
            let width = u32::try_from((*source).width).context("invalid picture width")?;
            let height = u32::try_from((*source).height).context("invalid picture height")?;
            ensure!(
                width > 0 && height > 0 && width <= 16384 && height <= 16384,
                "invalid picture dimensions"
            );
            let layout = if (*source).format == av::AVPixelFormat::AV_PIX_FMT_NV12 as i32 {
                Layout::Nv12
            } else if (*source).format == av::AVPixelFormat::AV_PIX_FMT_YUV420P as i32
                || (*source).format == av::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32
            {
                Layout::Planar
            } else {
                bail!(
                    "unsupported decoded pixel format {}; expected 8-bit YUV420P/NV12",
                    (*source).format
                );
            };
            let mut planes = self
                .planes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop()
                .unwrap_or_default();
            copy_plane(source, 0, width, height, &mut planes[0])?;
            let chroma_width = width.div_ceil(2);
            let chroma_height = height.div_ceil(2);
            copy_plane(
                source,
                1,
                chroma_width * if layout == Layout::Nv12 { 2 } else { 1 },
                chroma_height,
                &mut planes[1],
            )?;
            if layout == Layout::Planar {
                copy_plane(source, 2, chroma_width, chroma_height, &mut planes[2])?;
            } else {
                planes[2].bytes.clear();
            }
            Ok(DecodedFrame {
                width,
                height,
                received_at,
                decoded_at: Instant::now(),
                layout,
                full_range: (*source).color_range == av::AVColorRange::AVCOL_RANGE_JPEG,
                bt709: (*source).colorspace != av::AVColorSpace::AVCOL_SPC_BT470BG
                    && (*source).colorspace != av::AVColorSpace::AVCOL_SPC_SMPTE170M,
                planes,
                pool: self.planes.clone(),
            })
        }
    }
}

fn hardware_order(mode: DecodeMode) -> [av::AVHWDeviceType; 2] {
    let vaapi = av::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI;
    let cuda = av::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA;
    if matches!(mode, DecodeMode::Auto | DecodeMode::Cuda) {
        [cuda, vaapi]
    } else {
        [vaapi, cuda]
    }
}

fn amd_render_nodes(sysfs: &Path, devices: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sysfs) else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_text) = name.to_str() else {
            continue;
        };
        let Some(number) = name_text.strip_prefix("renderD") else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if std::fs::read_to_string(entry.path().join("device/vendor"))
            .is_ok_and(|vendor| vendor.trim() == "0x1002")
        {
            nodes.push(devices.join(name));
        }
    }
    nodes.sort();
    nodes
}

unsafe fn create_hardware_device(
    kind: av::AVHWDeviceType,
    amd_node: Option<&Path>,
) -> Result<*mut av::AVBufferRef> {
    // SAFETY: Strings and option dictionary remain live throughout FFmpeg's
    // synchronous initialization. FFmpeg owns the returned reference; the codec
    // receives ownership on success. No process-wide environment is mutated.
    unsafe {
        let node = amd_node
            .map(|path| CString::new(path.as_os_str().as_bytes()))
            .transpose()
            .context("invalid VAAPI device path")?;
        let mut options = ptr::null_mut();
        if node.is_some() {
            let result = av::av_dict_set(&mut options, c"driver".as_ptr(), c"radeonsi".as_ptr(), 0);
            if result < 0 {
                av::av_dict_free(&mut options);
                check(result).context("set VAAPI driver")?;
            }
        }
        let mut device = ptr::null_mut();
        let result = av::av_hwdevice_ctx_create(
            &mut device,
            kind,
            node.as_ref().map_or(ptr::null(), |node| node.as_ptr()),
            options,
            0,
        );
        av::av_dict_free(&mut options);
        if result < 0 {
            av::av_buffer_unref(&mut device);
            check(result).context("initialize hardware device")?;
        }
        Ok(device)
    }
}

unsafe fn copy_plane(
    source: *const av::AVFrame,
    index: usize,
    width: u32,
    height: u32,
    destination: &mut Plane,
) -> Result<()> {
    unsafe {
        let stride = (*source).linesize[index] as isize;
        ensure!(
            !(*source).data[index].is_null() && stride.unsigned_abs() >= width as usize,
            "invalid decoded plane"
        );
        destination.width = width;
        destination.height = height;
        destination
            .bytes
            .resize(width as usize * height as usize, 0);
        for row in 0..height as usize {
            ptr::copy_nonoverlapping(
                (*source).data[index].offset(row as isize * stride),
                destination.bytes.as_mut_ptr().add(row * width as usize),
                width as usize,
            );
        }
        Ok(())
    }
}

fn check(code: i32) -> Result<()> {
    ensure!(code >= 0, "{}", av_error(code));
    Ok(())
}

fn av_error(code: i32) -> String {
    let mut bytes = [0i8; 256];
    // SAFETY: Buffer is writable, bounded and initialized with a NUL terminator.
    unsafe {
        av::av_strerror(code, bytes.as_mut_ptr(), bytes.len());
        CStr::from_ptr(bytes.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: These free functions accept null pointers and clear each owned
        // pointer. The codec releases all retained packet and hardware refs.
        unsafe {
            av::avcodec_free_context(&mut self.context);
            av::av_packet_free(&mut self.packet);
            av::av_frame_free(&mut self.frame);
            av::av_frame_free(&mut self.transfer);
            av::av_buffer_pool_uninit(&mut self.packet_pool);
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn explicit_preferences_keep_fallback_order() {
        use av::AVHWDeviceType::{AV_HWDEVICE_TYPE_CUDA as CUDA, AV_HWDEVICE_TYPE_VAAPI as VAAPI};
        assert_eq!(hardware_order(DecodeMode::Auto), [CUDA, VAAPI]);
        assert_eq!(hardware_order(DecodeMode::Cuda), [CUDA, VAAPI]);
        assert_eq!(hardware_order(DecodeMode::Vaapi), [VAAPI, CUDA]);
    }

    #[test]
    fn amd_node_discovery_filters_vendor_and_ignores_non_render_entries() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "mirror-vaapi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir_all(&path)?;
        let result = (|| -> Result<()> {
            for (name, vendor) in [
                ("renderD128", "0x10de\n"),
                ("renderD129", "0x1002\n"),
                ("card2", "0x1002\n"),
                ("renderDinvalid", "0x1002\n"),
            ] {
                std::fs::create_dir_all(path.join(name).join("device"))?;
                std::fs::write(path.join(name).join("device/vendor"), vendor)?;
            }
            assert_eq!(
                amd_render_nodes(&path, Path::new("/dev/dri")),
                vec![PathBuf::from("/dev/dri/renderD129")]
            );
            assert!(amd_render_nodes(&path.join("missing"), Path::new("/dev/dri")).is_empty());
            Ok(())
        })();
        std::fs::remove_dir_all(&path)?;
        result
    }

    fn access_units() -> Vec<&'static [u8]> {
        split_units(include_bytes!("fixtures/motion-64x96.hevc"))
    }

    fn split_units(data: &'static [u8]) -> Vec<&'static [u8]> {
        let mut boundaries: Vec<_> = data
            .windows(6)
            .enumerate()
            .filter_map(|(index, bytes)| {
                (bytes[..4] == [0, 0, 0, 1] && bytes[4] >> 1 == 35).then_some(index)
            })
            .collect();
        boundaries.push(data.len());
        boundaries
            .windows(2)
            .map(|range| &data[range[0]..range[1]])
            .collect()
    }

    #[test]
    fn complete_access_units_decode_without_a_parser_frame_delay() {
        let mut decoder = Decoder::new(DecodeMode::Software).expect("decoder");
        let units = access_units();
        assert_eq!(units.len(), 6);
        let mut count = 0;
        let mut checksums = Vec::new();
        for unit in units {
            let before = count;
            decoder
                .decode(unit, Instant::now(), |frame| {
                    assert_eq!((frame.width, frame.height), (64, 96));
                    assert_eq!(frame.planes[0].bytes.len(), 64 * 96);
                    assert!(frame.decoded_at >= frame.received_at);
                    checksums.push(
                        frame.planes[0]
                            .bytes
                            .iter()
                            .map(|&value| u64::from(value))
                            .sum::<u64>(),
                    );
                    count += 1;
                })
                .expect("decode");
            // The zero-B-frame fixture should emit immediately for each AU.
            assert_eq!(count, before + 1);
        }
        decoder.flush(|_| count += 1).expect("flush");
        assert_eq!(count, 6);
        assert!(checksums.windows(2).any(|values| values[0] != values[1]));
        assert!(decoder.planes.lock().expect("pool").len() <= 1);
        decoder.reset();
        let mut restarted = 0;
        decoder
            .decode(access_units()[0], Instant::now(), |_| restarted += 1)
            .expect("decode after reset");
        assert_eq!(restarted, 1);
    }

    #[test]
    fn rejects_empty_packet() {
        let mut decoder = Decoder::new(DecodeMode::Software).expect("decoder");
        assert!(decoder.decode(&[], Instant::now(), |_| {}).is_err());
    }

    #[test]
    #[ignore = "probes local GPU devices; run explicitly during hardware qualification"]
    fn automatic_decoder_matches_software_picture_content() {
        for mode in [DecodeMode::Auto, DecodeMode::Cuda, DecodeMode::Vaapi] {
            let mut hardware = Decoder::new(mode).expect("automatic decoder");
            let mut software = Decoder::new(DecodeMode::Software).expect("software decoder");
            for unit in split_units(include_bytes!("fixtures/motion-256x384.hevc")) {
                let mut hardware_picture = None;
                let mut software_picture = None;
                hardware
                    .decode(unit, Instant::now(), |frame| hardware_picture = Some(frame))
                    .expect("automatic decode");
                software
                    .decode(unit, Instant::now(), |frame| software_picture = Some(frame))
                    .expect("software decode");
                let hardware_picture = hardware_picture.expect("automatic picture");
                let software_picture = software_picture.expect("software picture");
                assert_eq!(
                    hardware_picture.planes[0].bytes,
                    software_picture.planes[0].bytes
                );
            }
            eprintln!(
                "mode {mode:?}: actual hardware active: {:?}",
                hardware.hardware_active()
            );
        }
    }
}
