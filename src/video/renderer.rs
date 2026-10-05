use super::frame::{DecodedFrame, Layout};
use super::layout::ViewerLayout;
use super::orientation::{displayed_size, normalized_rotation};
use super::presentation::PresentationOptions;
use anyhow::{Context, Result, ensure};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::window::Window;

#[path = "status.rs"]
mod status;
use status::StatusText;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    viewport: [f32; 4],
    screen: [f32; 4],
    home: [f32; 4],
    footer: [f32; 4],
    style: [f32; 4],
    coefficients: [f32; 4],
    rotation: [f32; 4],
}

struct Textures {
    width: u32,
    height: u32,
    layout: Layout,
    planes: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
}

/// Host-side render attempt timings. These do not measure GPU completion or scanout.
#[derive(Clone, Copy, Debug)]
pub struct RenderSample {
    pub submitted_at: Option<Instant>,
    pub upload: Duration,
    pub surface_acquire: Duration,
    pub total: Duration,
}

/// GPU YUV conversion and presentation; textures are recreated only on format
/// changes. Hardware decode currently includes a CPU download/upload boundary.
pub struct Renderer {
    pub description: String,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    status_uniform: wgpu::Buffer,
    status: StatusText,
    textures: Option<Textures>,
    home_hovered: bool,
    home_pressed: bool,
    rotation: u16,
    pre_present_notify: bool,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, options: PresentationOptions) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .context("create GPU surface")?;
        let adapter = if let Some(vendor) = options.gpu.vendor() {
            let mut matches = instance
                .enumerate_adapters(wgpu::Backends::VULKAN)
                .into_iter()
                .filter(|adapter| {
                    adapter.get_info().vendor == vendor && adapter.is_surface_supported(&surface)
                });
            let adapter = matches
                .next()
                .with_context(|| format!("no surface-compatible {:?} renderer GPU", options.gpu))?;
            ensure!(
                matches.next().is_none(),
                "multiple surface-compatible {:?} GPUs; explicit vendor selection is ambiguous",
                options.gpu
            );
            adapter
        } else {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                })
                .await
                .context("select GPU adapter")?
        };
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mirror GPU"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .context("open GPU device")?;
        let capabilities = surface.get_capabilities(&adapter);
        // Our shader produces transfer-encoded RGB from video YUV. An sRGB
        // attachment would encode it a second time and wash out the picture.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .context("GPU surface has no non-sRGB format")?;
        let present_mode = options.select_mode(&capabilities.present_modes)?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: options.frame_latency,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .context("GPU alpha mode unavailable")?,
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("YUV planes"),
            entries: &[
                texture_binding(0),
                texture_binding(1),
                texture_binding(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("YUV linear sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("YUV conversion parameters"),
            size: std::mem::size_of::<Parameters>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let status = StatusText::default();
        let status_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("control status bitmap"),
            size: StatusText::BYTE_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&status_uniform, 0, status.bytes());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("YUV conversion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("yuv.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("YUV pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("YUV presentation"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        tracing::info!(gpu = %info.name, backend = ?info.backend, ?present_mode,
            supported_present_modes = ?capabilities.present_modes,
            requested_frame_latency = options.frame_latency,
            pre_present_notify = options.pre_present_notify,
            "GPU renderer ready; configuration does not establish physical scanout behavior");
        Ok(Self {
            description: format!("{} / {:?} / {:?}", info.name, info.backend, present_mode),
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            sampler,
            uniform,
            status_uniform,
            status,
            textures: None,
            home_hovered: false,
            home_pressed: false,
            rotation: 0,
            pre_present_notify: options.pre_present_notify,
        })
    }

    /// Updates button feedback without scheduling an idle redraw loop.
    /// The window loop should redraw only when this returns true.
    pub fn set_home_state(&mut self, hovered: bool, pressed: bool) -> bool {
        let changed = self.home_hovered != hovered || self.home_pressed != pressed;
        self.home_hovered = hovered;
        self.home_pressed = pressed;
        changed
    }

    /// Rotate texture coordinates on the GPU; decoded plane storage is unchanged.
    pub fn set_rotation(&mut self, clockwise_degrees: u16) -> bool {
        let rotation = normalized_rotation(clockwise_degrees);
        let changed = self.rotation != rotation;
        self.rotation = rotation;
        changed
    }

    /// Show a transient ASCII status above the Home toolbar. Empty text hides it.
    /// Text is uppercased, bounded to 80 glyphs, and clipped at the window edge.
    /// Returns true when the window should redraw; uploads occur only on changes.
    pub fn set_status(&mut self, text: &str) -> bool {
        if !self.status.set(text) {
            return false;
        }
        self.queue
            .write_buffer(&self.status_uniform, 0, self.status.bytes());
        true
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Timings include skipped timeout attempts; submitted_at is set only after present.
    pub fn render(&mut self, frame: &DecodedFrame) -> Result<RenderSample> {
        let started = Instant::now();
        ensure!(
            frame.width <= self.device.limits().max_texture_dimension_2d
                && frame.height <= self.device.limits().max_texture_dimension_2d,
            "picture exceeds GPU texture limits"
        );
        if self.textures.as_ref().is_none_or(|textures| {
            textures.width != frame.width
                || textures.height != frame.height
                || textures.layout != frame.layout
        }) {
            self.textures = Some(self.make_textures(frame));
        }
        let textures = self.textures.as_ref().context("missing YUV textures")?;
        for (index, plane) in frame.planes.iter().enumerate() {
            if plane.bytes.is_empty() {
                continue;
            }
            let channels = if index == 1 && frame.layout == Layout::Nv12 {
                2
            } else {
                1
            };
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &textures.planes[index],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &plane.bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(plane.width),
                    rows_per_image: Some(plane.height),
                },
                wgpu::Extent3d {
                    width: plane.width / channels,
                    height: plane.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let upload = started.elapsed();
        let scale_factor = self.window.scale_factor();
        let (display_width, display_height) =
            displayed_size(frame.width, frame.height, self.rotation);
        let geometry = ViewerLayout::new(
            self.config.width,
            self.config.height,
            display_width,
            display_height,
            scale_factor,
        );
        let rectangle = |rect: super::layout::Rect| {
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        };
        let parameters = Parameters {
            viewport: [
                self.config.width as f32,
                self.config.height as f32,
                if frame.layout == Layout::Nv12 {
                    1.0
                } else {
                    0.0
                },
                if frame.full_range { 1.0 } else { 0.0 },
            ],
            screen: rectangle(geometry.screen),
            home: rectangle(geometry.home),
            footer: rectangle(geometry.footer),
            style: [
                geometry.corner_radius as f32,
                (geometry.home.width.min(geometry.home.height) / 2.0) as f32,
                if self.home_pressed {
                    2.0
                } else if self.home_hovered {
                    1.0
                } else {
                    0.0
                },
                scale_factor as f32,
            ],
            coefficients: if frame.bt709 {
                [1.5748, -0.187324, -0.468124, 1.8556]
            } else {
                [1.402, -0.344136, -0.714136, 1.772]
            },
            rotation: [f32::from(self.rotation), 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&parameters));
        let acquire_started = Instant::now();
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                self.surface
                    .get_current_texture()
                    .context("reacquire GPU surface")?
            }
            Err(wgpu::SurfaceError::Timeout) => {
                tracing::warn!("GPU surface acquisition timed out; dropping decoded picture");
                return Ok(RenderSample {
                    submitted_at: None,
                    upload,
                    surface_acquire: acquire_started.elapsed(),
                    total: started.elapsed(),
                });
            }
            Err(error) => return Err(error).context("acquire GPU surface"),
        };
        let surface_acquire = acquire_started.elapsed();
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("YUV frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("YUV frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &textures.bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        if self.pre_present_notify {
            self.window.pre_present_notify();
        }
        output.present();
        let submitted_at = Instant::now();
        tracing::trace!(
            stage = "present_submit",
            elapsed_us = started.elapsed().as_micros() as u64,
            receive_to_submit_us = frame.received_at.elapsed().as_micros() as u64,
            decode_to_submit_us = frame.decoded_at.elapsed().as_micros() as u64,
            upload_us = upload.as_micros() as u64,
            surface_acquire_us = surface_acquire.as_micros() as u64,
            "presentation submitted; excludes capture, network and compositor scanout"
        );
        Ok(RenderSample {
            submitted_at: Some(submitted_at),
            upload,
            surface_acquire,
            total: submitted_at.duration_since(started),
        })
    }

    fn make_textures(&self, frame: &DecodedFrame) -> Textures {
        let planes = std::array::from_fn(|index| {
            let chroma = index != 0;
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("YUV plane"),
                size: wgpu::Extent3d {
                    width: if chroma {
                        frame.width.div_ceil(2)
                    } else {
                        frame.width
                    },
                    height: if chroma {
                        frame.height.div_ceil(2)
                    } else {
                        frame.height
                    },
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if index == 1 && frame.layout == Layout::Nv12 {
                    wgpu::TextureFormat::Rg8Unorm
                } else {
                    wgpu::TextureFormat::R8Unorm
                },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let views = planes
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("YUV planes"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.status_uniform.as_entire_binding(),
                },
            ],
        });
        Textures {
            width: frame.width,
            height: frame.height,
            layout: frame.layout,
            planes,
            bind_group,
        }
    }
}

fn texture_binding(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    #[test]
    fn shader_is_valid_wgsl() {
        let module = naga::front::wgsl::parse_str(include_str!("yuv.wgsl")).expect("parse shader");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("validate shader");
    }
}
