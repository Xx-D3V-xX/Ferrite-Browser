//! Draws the page's picture straight from the engine's pixels into one GPU
//! texture that lives as long as the page keeps its size.
//!
//! iced's `image` widget is built for icons: every new picture is a new handle,
//! and a new handle means a new atlas allocation, a padded copy of the whole
//! picture on the CPU, and (for a picture wider or taller than the atlas's 2048
//! pixels, which a page on a Retina screen always is) one more full-size upload
//! buffer per 2048-pixel tile. For a page that animates, that ran sixty times a
//! second. This widget writes each new picture into the texture it already has
//! (`queue.write_texture`, one copy) and draws it pixel for pixel, flipping
//! OpenGL's bottom-to-top rows in the shader so the engine does not have to. On
//! macOS the engine's bytes are BGRA; a BGRA texture takes them as they are.

use iced::widget::shader::{self, wgpu, Viewport};
use iced::{mouse, Rectangle};

pub use ferrite_servo::session::SharedFrame as PageFrame;

/// Whether to draw the page with iced's `image` widget instead
/// (`FERRITE_PAGE_DRAW=image`): slower, but it also works when iced fell back
/// to its software renderer, which cannot run this widget's shader.
pub fn use_image_widget() -> bool {
    static CHOICE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHOICE.get_or_init(|| {
        std::env::var("FERRITE_PAGE_DRAW").is_ok_and(|v| v.trim().eq_ignore_ascii_case("image"))
    })
}

/// The `shader::Program` for one page picture.
pub struct PageView {
    frame: PageFrame,
}

impl PageView {
    pub fn new(frame: PageFrame) -> Self {
        Self { frame }
    }
}

impl<Message> shader::Program<Message> for PageView {
    type State = ();
    type Primitive = PagePrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> PagePrimitive {
        PagePrimitive {
            frame: self.frame.clone(),
        }
    }
}

#[derive(Debug)]
pub struct PagePrimitive {
    frame: PageFrame,
}

/// GPU state kept between frames in iced's primitive storage.
struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// Whether the window's target is sRGB: the page texture matches it, so
    /// the engine's bytes pass through unchanged.
    srgb: bool,
    target: Option<Target>,
}

/// The texture the picture lives in, and what was last written to it.
struct Target {
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    seq: u64,
}

/// `rect`: the picture's left, top, right and bottom edges in clip space;
/// `options.x`: 1 when the rows are stored bottom first.
#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    rect: [f32; 4],
    options: [f32; 4],
}

impl Uniforms {
    fn bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, value) in self.rect.iter().chain(self.options.iter()).enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&value.to_ne_bytes());
        }
        out
    }
}

const SHADER: &str = r"
struct Uniforms { rect: vec4<f32>, options: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var picture_sampler: sampler;

struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Out {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: Out;
    out.position = vec4<f32>(mix(u.rect.x, u.rect.z, corner.x), mix(u.rect.y, u.rect.w, corner.y), 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, select(corner.y, 1.0 - corner.y, u.options.x > 0.5));
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    return textureSample(picture, picture_sampler, in.uv);
}
";

impl Pipeline {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ferrite page"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ferrite page"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ferrite page"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ferrite page"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        // Pixel for pixel: nearest keeps text sharp; the picture is only
        // stretched for the moment a resize takes to reach the engine.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ferrite page"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ferrite page"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            srgb: target_format.is_srgb(),
            target: None,
        }
    }

    /// The texture format for a frame: the engine's byte order (a BGRA texture
    /// samples as RGBA, so the GPU does the swizzle), sRGB to match the target.
    fn format_for(&self, frame: &PageFrame) -> wgpu::TextureFormat {
        match (frame.bgra, self.srgb) {
            (true, true) => wgpu::TextureFormat::Bgra8UnormSrgb,
            (true, false) => wgpu::TextureFormat::Bgra8Unorm,
            (false, true) => wgpu::TextureFormat::Rgba8UnormSrgb,
            (false, false) => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &PageFrame) {
        let expected = frame.width as usize * frame.height as usize * 4;
        if frame.width == 0 || frame.height == 0 || frame.pixels.len() < expected {
            return;
        }
        let format = self.format_for(frame);
        let reuse = self.target.as_ref().is_some_and(|t| {
            t.width == frame.width && t.height == frame.height && t.format == format
        });
        if !reuse {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ferrite page"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ferrite page"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.target = Some(Target {
                width: frame.width,
                height: frame.height,
                format,
                texture,
                bind_group,
                seq: u64::MAX,
            });
        }
        let Some(target) = self.target.as_mut() else {
            return;
        };
        if target.seq == frame.seq {
            return;
        }
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.pixels[..expected],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        target.seq = frame.seq;
    }
}

impl shader::Primitive for PagePrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        if !storage.has::<Pipeline>() {
            storage.store(Pipeline::new(device, format));
        }
        let Some(pipeline) = storage.get_mut::<Pipeline>() else {
            return;
        };
        pipeline.upload(device, queue, &self.frame);

        // The picture's own size in physical pixels, from the area's top-left
        // corner: shown one to one, never scaled to fit.
        let scale = viewport.scale_factor() as f32;
        let physical = viewport.physical_size();
        let (vw, vh) = (physical.width.max(1) as f32, physical.height.max(1) as f32);
        let left = (bounds.x * scale).round();
        let top = (bounds.y * scale).round();
        let right = left + self.frame.width as f32;
        let bottom = top + self.frame.height as f32;
        let uniforms = Uniforms {
            rect: [
                left / vw * 2.0 - 1.0,
                1.0 - top / vh * 2.0,
                right / vw * 2.0 - 1.0,
                1.0 - bottom / vh * 2.0,
            ],
            options: [f32::from(u8::from(self.frame.bottom_up)), 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&pipeline.uniforms, 0, &uniforms.bytes());
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(pipeline) = storage.get::<Pipeline>() else {
            return;
        };
        let Some(page) = pipeline.target.as_ref() else {
            return;
        };
        if clip_bounds.width == 0 || clip_bounds.height == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ferrite page"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &page.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}
