//! # rustui-renderer
//!
//! The GPU side of RustUI (§17-§21 of the design documents): it turns [`PaintCommand`]s into
//! instanced draw calls on top of `wgpu`.
//!
//! ```text
//! Painter ──> PaintCommand[] ──> tessellate() ──> Instance[] + Batch[] ──> wgpu
//! ```
//!
//! Design choices, all visible in the [`tess`] module and in `shader.wgs`:
//!
//! * **One pipeline, many instances.** Each quad carries its four corners in physical pixels, its
//!   UVs, four per-corner colors and a rounding radius. Consecutive quads that share a texture and
//!   a clip rectangle become one [`Batch`] = one draw call with a scissor rectangle.
//! * **Premultiplied alpha.** The fragment shader outputs `rgb * a`, and the pipeline blends with
//!   `src + dst * (1 - src.a)`, which composites correctly over the window and over other quads.
//! * **Rounded corners in the shader.** A signed distance field rounds the quad and, when a border
//!   inset is present, subtracts an inner rounded rectangle so borders follow the radius exactly.
//! * **Gradients on the CPU.** A linear gradient is linear in screen space, so sampling it at the
//!   four corners and letting the rasterizer interpolate is exact for any number of stops.
//! * **No `winit` here.** [`SurfaceRenderer::new`] takes a `wgpu` surface, so the crate depends on
//!   `rustui-core` + `wgpu` only (principle P1).
//!
//! # Usage
//!
//! ```ignore
//! let gpu = GpuContext::new()?;
//! let surface = gpu.create_surface(window)?;
//! let mut renderer = SurfaceRenderer::new(&gpu, surface, width, height)?;
//! // ... every frame:
//! renderer.render(&gpu, &RenderInput::new(painter.commands(), painter.arena(), scale, Some(&text)))?;
//! ```
//!
//! [`PaintCommand`]: rustui_core::PaintCommand

mod gpu;
mod surface;
mod tess;

pub use gpu::GpuContext;
pub use surface::{RenderInput, SurfaceRenderer, INSTANCE_ATTRIBUTES, INSTANCE_STRIDE};
pub use tess::{
    group_batches, tessellate, texture_slot, Batch, DrawItem, Instance, Tessellation, MODE_COLOR,
    MODE_COVERAGE, SLOT_GLYPH_ATLAS, SLOT_WHITE,
};

/// Result type of this crate.
pub type Result<T> = rustui_core::Result<T>;
