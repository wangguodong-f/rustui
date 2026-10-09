//! Per window renderer: one pipeline, instanced quads, batched by texture and clip (§17-§21).
//!
//! ```text
//! Painter -> PaintCommand[] -> tessellate() -> Instance[] + Batch[] -> wgpu draw calls
//! ```
//!
//! Clipping uses scissor rectangles, corner rounding happens in the fragment shader (with an
//! optional inset that draws borders as rings), and blending is premultiplied alpha so the result
//! composites correctly over the window.

use std::collections::HashMap;

use rustui_core::{GlyphSource, PaintCommand, Result, TextArena, TextureId, UiError};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendComponent, BlendFactor,
    BlendOperation, BlendState, Buffer, BufferDescriptor, BufferUsages, Color as GpuColor,
    ColorTargetState, ColorWrites, CommandEncoderDescriptor, CurrentSurfaceTexture, Extent3d,
    FilterMode, FragmentState, LoadOp, MapMode, MipmapFilterMode, MultisampleState, Operations,
    Origin3d, PipelineCompilationOptions, PipelineLayoutDescriptor, PolygonMode, PrimitiveState,
    PrimitiveTopology, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, Surface, SurfaceConfiguration,
    SurfaceTexture, TexelCopyBufferInfo, TexelCopyBufferLayout, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureViewDescriptor, VertexAttribute, VertexBufferLayout, VertexFormat, VertexState,
    VertexStepMode,
};

use crate::gpu::GpuContext;
use crate::tess::{tessellate, Instance, Tessellation, SLOT_GLYPH_ATLAS, SLOT_WHITE};

/// The single WGSL program used by every draw call.
const SHADER: &str = include_str!("shader.wgs");

/// Stride of one instance in bytes.
pub const INSTANCE_STRIDE: u64 = std::mem::size_of::<Instance>() as u64;

/// Smallest instance buffer we keep around (a few hundred quads).
const MIN_INSTANCE_BYTES: u64 = 64 * 1024;

/// Pixel format of an offscreen target: the sRGB one a window surface normally picks.
const OFFSCREEN_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

/// Creates an `RENDER_ATTACHMENT | COPY_SRC` texture an offscreen renderer draws into.
fn create_target_texture(
    ctx: &GpuContext,
    format: TextureFormat,
    width: u32,
    height: u32,
) -> Texture {
    ctx.device().create_texture(&TextureDescriptor {
        label: Some("rustui-offscreen"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Everything the renderer needs for one frame.
pub struct RenderInput<'a> {
    /// Recorded paint commands, in painter order.
    pub commands: &'a [PaintCommand],
    /// Frame string arena referenced by [`PaintCommand::Text`].
    pub arena: &'a TextArena,
    /// Display scale factor (physical pixels per logical pixel).
    pub scale: f32,
    /// Text engine providing glyph quads and the atlas; `None` drops text.
    pub glyphs: Option<&'a dyn GlyphSource>,
}

impl<'a> RenderInput<'a> {
    /// Builds the input for one frame.
    pub fn new(
        commands: &'a [PaintCommand],
        arena: &'a TextArena,
        scale: f32,
        glyphs: Option<&'a dyn GlyphSource>,
    ) -> Self {
        Self {
            commands,
            arena,
            scale,
            glyphs,
        }
    }
}

/// One bind group per texture, so a batch only has to swap the group. The texture itself is kept
/// alive for later uploads (the glyph atlas is rewritten every time it grows).
struct TextureSlot {
    texture: Texture,
    bind_group: BindGroup,
}

/// Where a frame is drawn: a window surface, or an offscreen texture (T-10).
enum Target {
    /// The window's swap chain; the frame is presented after rendering.
    Surface {
        surface: Surface<'static>,
        config: SurfaceConfiguration,
    },
    /// A texture nobody presents; [`SurfaceRenderer::read_pixels`] copies it back to the CPU.
    Offscreen { texture: Texture },
}

struct AtlasState {
    width: u32,
    height: u32,
    generation: u32,
}

/// Renders one window's paint commands to its surface.
pub struct SurfaceRenderer {
    target: Target,
    format: TextureFormat,
    physical: [u32; 2],
    pipeline: RenderPipeline,
    globals_buffer: Buffer,
    globals_bind_group: BindGroup,
    texture_layout: BindGroupLayout,
    sampler: Sampler,
    instance_buffer: Buffer,
    textures: HashMap<u32, TextureSlot>,
    atlas: Option<AtlasState>,
    atlas_bytes: Vec<u8>,
    next_texture: u32,
    pending_reconfigure: bool,
}

impl std::fmt::Debug for SurfaceRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceRenderer")
            .field("physical", &self.physical)
            .field("format", &self.format)
            .field(
                "target",
                &match &self.target {
                    Target::Surface { .. } => "surface",
                    Target::Offscreen { .. } => "offscreen",
                },
            )
            .finish()
    }
}

impl SurfaceRenderer {
    /// Creates the renderer for a surface of `width x height` physical pixels.
    pub fn new(
        ctx: &GpuContext,
        surface: Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let Some(config) = surface.get_default_config(ctx.adapter(), width, height) else {
            return Err(UiError::SurfaceCreation(
                "the adapter does not support this surface".to_string(),
            ));
        };
        // Prefer sRGB: widget colors are authored in sRGB and blending happens on the surface.
        let capabilities = surface.get_capabilities(ctx.adapter());
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .unwrap_or(config.format);
        let config = SurfaceConfiguration { format, ..config };
        surface.configure(ctx.device(), &config);
        Ok(Self::build(
            ctx,
            Target::Surface { surface, config },
            width,
            height,
        ))
    }

    /// Creates a renderer that draws into a texture instead of a window - headless rendering,
    /// screenshots and golden image regressions (T-10).
    ///
    /// It picks the sRGB format a window surface normally gets, so a frame looks the same
    /// offscreen as on screen: [`SurfaceRenderer::render`] draws into the texture and
    /// [`SurfaceRenderer::read_pixels`] copies it back to the CPU.
    pub fn new_offscreen(ctx: &GpuContext, width: u32, height: u32) -> Result<Self> {
        let width = width.max(1);
        let height = height.max(1);
        let texture = create_target_texture(ctx, OFFSCREEN_FORMAT, width, height);
        Ok(Self::build(
            ctx,
            Target::Offscreen { texture },
            width,
            height,
        ))
    }

    /// Builds everything that does not depend on where the frame is drawn.
    fn build(ctx: &GpuContext, target: Target, width: u32, height: u32) -> Self {
        let format = match &target {
            Target::Surface { config, .. } => config.format,
            Target::Offscreen { .. } => OFFSCREEN_FORMAT,
        };
        let device = ctx.device();
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rustui-shader"),
            source: ShaderSource::Wgsl(SHADER.into()),
        });

        let globals_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("rustui-globals-layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(wgpu::BufferSize::new(16).expect("16 is valid")),
                },
                count: None,
            }],
        });

        let texture_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("rustui-texture-layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("rustui-pipeline-layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("rustui-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[Some(instance_buffer_layout())],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(premultiplied_blend()),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let globals_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("rustui-globals"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("rustui-globals-bind-group"),
            layout: &globals_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("rustui-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let instance_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("rustui-instances"),
            size: MIN_INSTANCE_BYTES,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut renderer = Self {
            target,
            format,
            physical: [width, height],
            pipeline,
            globals_buffer,
            globals_bind_group,
            texture_layout,
            sampler,
            instance_buffer,
            textures: HashMap::new(),
            atlas: None,
            atlas_bytes: Vec::new(),
            next_texture: SLOT_GLYPH_ATLAS + 1,
            pending_reconfigure: false,
        };
        renderer.write_globals(ctx);
        renderer.create_white_texture(ctx);
        renderer
    }

    /// Drawable size in physical pixels.
    pub fn physical_size(&self) -> [u32; 2] {
        self.physical
    }

    /// Pixel format actually used (surface mode and offscreen mode agree: sRGB).
    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// Called when the window was resized; a size of zero (minimized) is ignored.
    ///
    /// Offscreen renderers recreate their target texture instead of reconfiguring a surface.
    pub fn resize(&mut self, ctx: &GpuContext, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.physical == [width, height] && !self.pending_reconfigure {
            return;
        }
        self.physical = [width, height];
        match &mut self.target {
            Target::Surface { surface, config } => {
                config.width = width;
                config.height = height;
                surface.configure(ctx.device(), config);
            }
            Target::Offscreen { texture } => {
                *texture = create_target_texture(ctx, self.format, width, height);
            }
        }
        self.pending_reconfigure = false;
        self.write_globals(ctx);
    }

    /// Uploads an RGBA8 image and returns the handle widgets paint with.
    pub fn register_texture(
        &mut self,
        ctx: &GpuContext,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> TextureId {
        let width = width.max(1);
        let height = height.max(1);
        let id = self.next_texture;
        self.next_texture = self
            .next_texture
            .saturating_add(1)
            .max(SLOT_GLYPH_ATLAS + 2);
        let data = if rgba8.len() as u32 >= width * height * 4 {
            Some(rgba8)
        } else {
            None
        };
        let (texture, bind_group) = self.create_texture(
            ctx,
            "rustui-image",
            TextureFormat::Rgba8Unorm,
            width,
            height,
            data,
        );
        self.textures.insert(
            id,
            TextureSlot {
                texture,
                bind_group,
            },
        );
        TextureId(id)
    }

    /// Draws one frame and presents it.
    ///
    /// Returns `Ok(())` when the frame was skipped (minimized window, timeout, occluded) - those
    /// are normal and should not stop the application. An offscreen renderer keeps the frame in
    /// its target texture until [`SurfaceRenderer::read_pixels`] asks for the pixels.
    pub fn render(&mut self, ctx: &GpuContext, input: &RenderInput<'_>) -> Result<()> {
        if self.physical[0] == 0 || self.physical[1] == 0 {
            return Ok(());
        }
        if self.pending_reconfigure {
            if let Target::Surface { surface, config } = &self.target {
                surface.configure(ctx.device(), config);
            }
            self.pending_reconfigure = false;
        }
        // Only a window surface has to acquire a frame; the offscreen texture is always there.
        let frame = if matches!(self.target, Target::Surface { .. }) {
            let Some(frame) = self.acquire() else {
                return Ok(());
            };
            Some(frame)
        } else {
            None
        };

        // Rasterising happens here: `tessellate` pulls every glyph quad out of the source, which
        // fills the atlas on first use. The atlas is therefore uploaded *after* tessellation, or
        // the first frame of a window would sample an empty atlas and draw no text at all.
        let tess = tessellate(
            input.commands,
            input.arena,
            input.scale,
            self.physical,
            input.glyphs,
        );
        self.sync_atlas(ctx, input.glyphs);
        if !tess.instances.is_empty() {
            self.upload_instances(ctx, &tess.instances);
        }

        let view = match (&self.target, &frame) {
            (_, Some(frame)) => frame.texture.create_view(&TextureViewDescriptor::default()),
            (Target::Offscreen { texture }, None) => {
                texture.create_view(&TextureViewDescriptor::default())
            }
            (Target::Surface { .. }, None) => return Ok(()),
        };
        let mut encoder = ctx
            .device()
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rustui-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("rustui-pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(GpuColor {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.encode(&mut pass, &tess);
        }
        ctx.queue().submit([encoder.finish()]);
        if let Some(frame) = frame {
            ctx.queue().present(frame);
        }
        Ok(())
    }

    /// Copies the current offscreen frame back to the CPU as tightly packed RGBA8 rows.
    ///
    /// Only a renderer built by [`SurfaceRenderer::new_offscreen`] has pixels to read; a window
    /// surface owns its frame and presents it. Rows are returned top to bottom, `width * 4` bytes
    /// each (T-10).
    pub fn read_pixels(&self, ctx: &GpuContext) -> Result<Vec<u8>> {
        let Target::Offscreen { texture } = &self.target else {
            return Err(UiError::Other(
                "read_pixels needs a renderer created with `new_offscreen`".to_string(),
            ));
        };
        let [width, height] = self.physical;
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        let unpadded = width as usize * 4;
        let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded = unpadded.div_ceil(stride) * stride;
        let buffer = ctx.device().create_buffer(&BufferDescriptor {
            label: Some("rustui-readback"),
            size: (padded * height as usize) as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx
            .device()
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rustui-readback"),
            });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        ctx.queue().submit([encoder.finish()]);

        // Map the buffer: the callback only fires while the device is polled, so block on it.
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        ctx.wait_idle();
        receiver
            .recv()
            .map_err(|_| UiError::Other("the readback mapping never completed".to_string()))?
            .map_err(|error| UiError::Other(format!("mapping the readback buffer: {error}")))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|error| UiError::Other(format!("reading the readback buffer: {error:?}")))?;
        let mut pixels = Vec::with_capacity(unpadded * height as usize);
        for row in 0..height as usize {
            let start = row * padded;
            pixels.extend_from_slice(&mapped[start..start + unpadded]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }

    fn encode(&self, pass: &mut wgpu::RenderPass<'_>, tess: &Tessellation) {
        if tess.batches.is_empty() {
            return;
        }
        let fallback = self.textures.get(&SLOT_WHITE);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        for batch in &tess.batches {
            let slot = self.textures.get(&batch.texture).or(fallback);
            let Some(slot) = slot else { continue };
            let [x, y, width, height] = batch.scissor;
            if width == 0 || height == 0 {
                continue;
            }
            pass.set_scissor_rect(x, y, width, height);
            pass.set_bind_group(1, &slot.bind_group, &[]);
            let offset = batch.start as u64 * INSTANCE_STRIDE;
            pass.set_vertex_buffer(0, self.instance_buffer.slice(offset..));
            pass.draw(0..6, 0..batch.count as u32);
        }
    }

    fn acquire(&mut self) -> Option<SurfaceTexture> {
        let Target::Surface { surface, .. } = &mut self.target else {
            return None;
        };
        for _ in 0..2 {
            match surface.get_current_texture() {
                CurrentSurfaceTexture::Success(texture) => return Some(texture),
                CurrentSurfaceTexture::Suboptimal(texture) => {
                    // Usable; reconfigure before the next frame.
                    self.pending_reconfigure = true;
                    return Some(texture);
                }
                CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                    // Retry with the current configuration; `resize` fixes the size.
                    continue;
                }
                CurrentSurfaceTexture::Timeout
                | CurrentSurfaceTexture::Occluded
                | CurrentSurfaceTexture::Validation => return None,
            }
        }
        None
    }

    fn upload_instances(&mut self, ctx: &GpuContext, instances: &[Instance]) {
        let bytes = (instances.len() as u64) * INSTANCE_STRIDE;
        if bytes > self.instance_buffer.size() {
            let size = bytes.next_power_of_two().max(MIN_INSTANCE_BYTES);
            self.instance_buffer = ctx.device().create_buffer(&BufferDescriptor {
                label: Some("rustui-instances"),
                size,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        ctx.queue()
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    fn write_globals(&self, ctx: &GpuContext) {
        let viewport = [self.physical[0] as f32, self.physical[1] as f32, 0.0, 0.0];
        ctx.queue()
            .write_buffer(&self.globals_buffer, 0, bytemuck::cast_slice(&viewport));
    }

    /// Creates the 1x1 white texture used for every untextured quad.
    fn create_white_texture(&mut self, ctx: &GpuContext) {
        let (texture, bind_group) = self.create_texture(
            ctx,
            "rustui-white",
            TextureFormat::Rgba8Unorm,
            1,
            1,
            Some(&[255, 255, 255, 255]),
        );
        self.textures.insert(
            SLOT_WHITE,
            TextureSlot {
                texture,
                bind_group,
            },
        );
    }

    fn create_texture(
        &self,
        ctx: &GpuContext,
        label: &str,
        format: TextureFormat,
        width: u32,
        height: u32,
        data: Option<&[u8]>,
    ) -> (Texture, BindGroup) {
        let texture = ctx.device().create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if let Some(data) = data {
            ctx.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                data,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * format_bytes_per_pixel(format)),
                    rows_per_image: Some(height),
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&TextureViewDescriptor::default());
        let bind_group = ctx.device().create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &self.texture_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        (texture, bind_group)
    }

    /// Uploads the glyph atlas when the text engine says it changed (§25).
    fn sync_atlas(&mut self, ctx: &GpuContext, glyphs: Option<&dyn GlyphSource>) {
        let Some(source) = glyphs else {
            return;
        };
        let info = source.atlas_info();
        if info.width == 0 || info.height == 0 {
            return;
        }
        let slot = crate::tess::texture_slot(TextureId(source.atlas_texture_index()));
        let mut recreated = false;
        let outdated = match &self.atlas {
            Some(atlas) => {
                atlas.width != info.width
                    || atlas.height != info.height
                    || atlas.generation != info.generation
            }
            None => true,
        };
        if outdated {
            // A fresh texture starts empty, so a full upload follows either way.
            let (texture, bind_group) = self.create_texture(
                ctx,
                "rustui-glyph-atlas",
                TextureFormat::R8Unorm,
                info.width,
                info.height,
                None,
            );
            self.textures.insert(
                slot,
                TextureSlot {
                    texture,
                    bind_group,
                },
            );
            self.atlas = Some(AtlasState {
                width: info.width,
                height: info.height,
                generation: info.generation,
            });
            self.next_texture = self.next_texture.max(slot + 1);
            recreated = true;
        }
        if info.dirty || recreated {
            self.atlas_bytes.clear();
            source.copy_atlas_into(&mut self.atlas_bytes);
            let expected = (info.width as usize) * (info.height as usize);
            if self.atlas_bytes.len() == expected {
                if let (Some(atlas), Some(slot)) = (self.atlas.as_ref(), self.textures.get(&slot)) {
                    ctx.queue().write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &slot.texture,
                            mip_level: 0,
                            origin: Origin3d::ZERO,
                            aspect: TextureAspect::All,
                        },
                        &self.atlas_bytes,
                        TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(info.width),
                            rows_per_image: Some(info.height),
                        },
                        Extent3d {
                            width: atlas.width,
                            height: atlas.height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            } else {
                log::warn!(
                    "glyph atlas size mismatch: {} bytes for {}x{}",
                    self.atlas_bytes.len(),
                    info.width,
                    info.height
                );
            }
            source.mark_atlas_uploaded();
        }
    }
}

fn format_bytes_per_pixel(format: TextureFormat) -> u32 {
    match format {
        TextureFormat::R8Unorm | TextureFormat::R8Uint | TextureFormat::R8Sint => 1,
        TextureFormat::Rg8Unorm | TextureFormat::R16Float => 2,
        _ => 4,
    }
}

/// Vertex layout of [`Instance`], matching `VsIn` in `shader.wgs`.
fn instance_buffer_layout() -> VertexBufferLayout<'static> {
    VertexBufferLayout {
        array_stride: INSTANCE_STRIDE,
        step_mode: VertexStepMode::Instance,
        attributes: &INSTANCE_ATTRIBUTES,
    }
}

/// Attribute offsets derived from the [`Instance`] layout, asserted by a unit test.
pub const INSTANCE_ATTRIBUTES: [VertexAttribute; 13] = [
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 16,
        shader_location: 2,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 24,
        shader_location: 3,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 32,
        shader_location: 4,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 48,
        shader_location: 5,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 64,
        shader_location: 6,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 80,
        shader_location: 7,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 96,
        shader_location: 8,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 112,
        shader_location: 9,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 128,
        shader_location: 10,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 144,
        shader_location: 11,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 160,
        shader_location: 12,
    },
];

/// Premultiplied alpha blending: `src + dst * (1 - src.a)`.
fn premultiplied_blend() -> BlendState {
    BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_attributes_match_the_instance_layout() {
        assert_eq!(INSTANCE_STRIDE, 176);
        for (index, attribute) in INSTANCE_ATTRIBUTES.iter().enumerate() {
            assert_eq!(attribute.shader_location as usize, index);
        }
        assert_eq!(
            INSTANCE_ATTRIBUTES[0].offset,
            std::mem::offset_of!(Instance, corners) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[4].offset,
            std::mem::offset_of!(Instance, uv) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[5].offset,
            std::mem::offset_of!(Instance, colors) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[9].offset,
            std::mem::offset_of!(Instance, inset) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[10].offset,
            std::mem::offset_of!(Instance, params) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[11].offset,
            std::mem::offset_of!(Instance, radii) as u64
        );
        assert_eq!(
            INSTANCE_ATTRIBUTES[12].offset,
            std::mem::offset_of!(Instance, clip) as u64
        );
    }

    #[test]
    fn blending_is_premultiplied() {
        let blend = premultiplied_blend();
        assert_eq!(blend.color.src_factor, BlendFactor::One);
        assert_eq!(blend.color.dst_factor, BlendFactor::OneMinusSrcAlpha);
    }

    /// The WGSL program is only compiled when a pipeline is created, which needs a GPU. Parsing
    /// and validating it here keeps shader mistakes out of runtime code even on headless CI.
    #[test]
    fn shader_parses_and_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).expect("WGSL parses");
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        );
        validator.validate(&module).expect("WGSL validates");

        // Entry points are found by name from the pipeline descriptors.
        let names: Vec<&str> = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert!(names.contains(&"vs"), "vertex entry point: {names:?}");
        assert!(names.contains(&"fs"), "fragment entry point: {names:?}");
    }
}
