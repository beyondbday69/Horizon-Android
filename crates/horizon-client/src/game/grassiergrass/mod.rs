//! Grassier Grass for Horizon: bakes blades around the camera and draws them after the
//! world pass with the game's own wgpu device, as the `grassiergrass` feature.
//!
//! A native Rust port of Grassier Grass 1.4.5 (leonardoinc22), made with the author's
//! permission. Both the original and this port are All Rights Reserved — see
//! `THIRD_PARTY_NOTICES.md`. Only this clean rewrite is distributed; the original jar,
//! its decompiled Java and its assets are not.
//!
//! Known differences from the original (upstream's README lists the same): no LOD rings,
//! entity trails, clumps, snow, flowers, particles, config screen, biome tint or
//! Iris/Sodium integration. Blades draw on `minecraft:grass_block` only, in a 20-block
//! disk around the camera.
//!
//! Three things are adapted from upstream, because this tree's mod API differs from the
//! revision the bundle pinned:
//!
//! - Upstream reads `host.client_state()` for the eye position and world id. This tree's
//!   [`Host`] has neither, and world reads (`Host::block`) are only available in `frame`,
//!   so the render pass records [`WorldDrawn::camera`] and `frame` bakes around it. The
//!   anchor therefore lags the camera by one frame, which the two-block trigger hides.
//! - The dimension is `Host::dimension()`, a string, rather than a `NativeWorldId`.
//! - `RenderNeeds`, the depth attachment and the blend state are this tree's.

use std::sync::Mutex;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use grassiergrass_engine as engine;
use crate::game::Game;
use horizon_client::mod_api::{
    Host, RenderHooks, RenderNeedsEvent, WorldDrawn, WorldDrawnEvent, wgpu,
};

mod shader;

/// The slice's fixed settings. `blades_per_block` is lower than the original's 64 so a
/// rebuild costs a fraction of a frame; the original's 0.3 height and 0.95 width are kept.
const CONFIG: engine::Config = engine::Config {
    radius: 20,
    blades_per_block: 24,
    blade_height: 0.3,
    blade_width: 0.95,
};

/// The original's default wind: speed 100/100 with a slow direction wander.
const WIND_INTENSITY: f32 = 1.0;

/// The installed mod, built once. A `&'static` because the renderer keeps the trait object
/// for the life of the process.
static MOD: std::sync::OnceLock<Mod> = std::sync::OnceLock::new();

/// The renderer that draws the blades. Installed once, before settings are read, so the
/// feature gate is checked per frame rather than at install time.
pub(super) fn render_hooks() -> &'static dyn RenderHooks {
    MOD.get_or_init(Mod::new)
}

/// Whether the mod is installed and on, refreshed once a frame from the game side: the
/// render thread has no `Game` to ask.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called once a frame by the game, which is the only place that can resolve the gate.
pub(super) fn set_enabled(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

struct Mod {
    /// A freshly baked batch waiting for the render thread to upload, if any.
    mesh: Mutex<Option<engine::Mesh>>,
    gpu: Mutex<Option<Gpu>>,
    /// The dimension the mesh was baked in and its anchor: a change forces a rebuild.
    last_anchor: Mutex<Option<(String, [i32; 3])>>,
    /// The camera the last render pass drew from. World reads only happen in `frame`, so
    /// the rebuild is anchored here rather than on the render thread.
    camera: Mutex<Option<[f64; 3]>>,
    clock: Instant,
}

impl Mod {
    fn new() -> Self {
        Self {
            mesh: Mutex::new(None),
            gpu: Mutex::new(None),
            last_anchor: Mutex::new(None),
            camera: Mutex::new(None),
            clock: Instant::now(),
        }
    }

    /// Rebuilds when the camera moves two blocks horizontally or one vertically. Block
    /// edits are not watched: the mesh refreshes on the next move.
    fn rebuild(&self, host: &mut dyn Host, anchor: [i32; 3]) {
        let dimension = host.dimension().to_owned();
        {
            let mut last = self.last_anchor.lock().unwrap();
            let moved = match last.as_ref() {
                Some((prev_dim, prev)) => {
                    let dx = anchor[0] - prev[0];
                    let dz = anchor[2] - prev[2];
                    let dy = anchor[1] - prev[1];
                    *prev_dim != dimension || dx * dx + dz * dz >= 4 || dy.abs() >= 2
                }
                None => true,
            };
            if !moved {
                return;
            }
            *last = Some((dimension, anchor));
        }

        // `engine::build` borrows its reader for the whole mesh, and `Host::block` needs
        // `host` mutably, so the mesher runs against a snapshot of the disk it will ask
        // for. That is one column per block column in radius, read top down.
        let (min_y, world_height) = host.world_height();
        let top = anchor[1] + 4;
        let bottom = (anchor[1] - 16).max(min_y);
        let reach = CONFIG.radius;
        let mut snapshot: std::collections::HashMap<[i32; 3], engine::Block> =
            std::collections::HashMap::new();
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                if dx * dx + dz * dz > reach * reach {
                    continue;
                }
                let (x, z) = (anchor[0] + dx, anchor[2] + dz);
                for y in bottom..=top + 1 {
                    let pos = [x, y, z];
                    let block = classify(host, pos, min_y, world_height);
                    snapshot.insert(pos, block);
                }
            }
        }
        let mesh = engine::build(anchor, &CONFIG, |pos| {
            snapshot.get(&pos).copied().unwrap_or(engine::Block::Air)
        });
        *self.mesh.lock().unwrap() = Some(mesh);
    }

    /// Bakes a new mesh when the camera moved far enough. Called once a frame by the game,
    /// which supplies itself as the `Host` for world reads.
    fn rebuild_if_moved(&self, host: &mut dyn Host) {
        if !enabled() {
            self.mesh.lock().unwrap().take();
            self.last_anchor.lock().unwrap().take();
            return;
        }
        let Some(camera) = *self.camera.lock().unwrap() else {
            return;
        };
        let anchor = [
            camera[0].floor() as i32,
            camera[1].floor() as i32,
            camera[2].floor() as i32,
        ];
        self.rebuild(host, anchor);
    }
}

/// The mesher's view of one block.
fn classify(host: &dyn Host, pos: [i32; 3], min_y: i32, height: i32) -> engine::Block {
    if pos[1] < min_y || pos[1] >= min_y + height {
        return engine::Block::Air;
    }
    match host.block(pos).as_str() {
        "minecraft:grass_block" => engine::Block::Grass,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" => engine::Block::Air,
        _ => engine::Block::Other,
    }
}

/// Bakes a new blade mesh for `game` if the camera moved far enough.
pub(super) fn frame(game: &mut Game<'_>) {
    MOD.get_or_init(Mod::new).rebuild_if_moved(game);
}

impl RenderHooks for Mod {
    fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
        if enabled() {
            event.needs.world_depth = true;
        }
    }

    fn world_drawn(&self, event: &mut WorldDrawnEvent<'_, '_>) {
        if !enabled() {
            return;
        }
        let world = &mut *event.world;
        // Where the blades should be baked from next frame.
        *self.camera.lock().unwrap() = Some(world.camera.to_array());
        let mut slot = self.gpu.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|gpu| gpu.format != world.color_format)
        {
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(Gpu::new(world.device, world.color_format));
        }
        let gpu = slot.as_mut().unwrap();

        if let Some(mesh) = self.mesh.lock().unwrap().take() {
            gpu.upload(world.device, world.queue, &mesh);
        }
        if gpu.index_count == 0 {
            return;
        }

        let elapsed = self.clock.elapsed().as_secs_f32();
        gpu.write_globals(world.queue, world, elapsed);
        gpu.draw(world);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [f32; 16],
    offset: [f32; 4],
    wind: [f32; 4],
    params: [f32; 4],
}

struct Gpu {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    /// Sub-100% render scale keeps a smaller depth than the frame target; a pass can't mix
    /// the two, so this one draws without depth rather than hide the grass at that setting.
    pipeline_no_depth: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    index_count: u32,
    anchor: [i32; 3],
}

impl Gpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grassiergrass blades"),
            source: wgpu::ShaderSource::Wgsl(shader::BLADES_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grassiergrass blades"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grassiergrass blades"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Float32x3];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<engine::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let make_pipeline = |depth: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("grassiergrass blades"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_blade"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                // Reversed depth: nearer is greater.
                depth_stencil: depth.then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_blade"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline(true);
        let pipeline_no_depth = make_pipeline(false);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grassiergrass globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grassiergrass blades"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        Self {
            format,
            pipeline,
            pipeline_no_depth,
            globals,
            bind,
            vertices: None,
            indices: None,
            index_count: 0,
            anchor: [0; 3],
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &engine::Mesh) {
        if mesh.indices.is_empty() {
            self.index_count = 0;
            return;
        }
        let vertex_bytes = bytemuck::cast_slice::<engine::Vertex, u8>(&mesh.vertices);
        let index_bytes = bytemuck::cast_slice::<u32, u8>(&mesh.indices);
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grassiergrass vertices"),
            size: vertex_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grassiergrass indices"),
            size: index_bytes.len() as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertices, 0, vertex_bytes);
        queue.write_buffer(&indices, 0, index_bytes);
        self.vertices = Some(vertices);
        self.indices = Some(indices);
        self.index_count = mesh.indices.len() as u32;
        self.anchor = mesh.anchor;
    }

    fn write_globals(&self, queue: &wgpu::Queue, world: &WorldDrawn<'_>, elapsed: f32) {
        let offset = [
            self.anchor[0] as f64 - world.camera.x,
            self.anchor[1] as f64 - world.camera.y,
            self.anchor[2] as f64 - world.camera.z,
            0.0,
        ];
        // The original's dynamic wind: ±80° of slow direction wander over a few minutes.
        let ticks = elapsed * 20.0;
        let wander = (ticks * 0.0025).sin() + (ticks * 0.005925 + 1.6).sin() * 0.35;
        let angle = (wander / 1.35 * 80.0).to_radians();
        // Both phases wrap where the original wraps them, so an f32 keeps its precision:
        // flutter at 64 * 2π (the speed steps share that period), scroll at one UV tile.
        let flutter = (ticks * 0.24 * WIND_INTENSITY) % (64.0 * std::f32::consts::TAU);
        let scroll = (ticks * 0.005 * WIND_INTENSITY).rem_euclid(1.0);
        // Shortcut: constant wind speed; the original pulses 70-110% and integrates its
        // scroll. Upgrade when a config exists.
        let globals = Globals {
            view_proj: world.view_proj.to_cols_array(),
            offset: [offset[0] as f32, offset[1] as f32, offset[2] as f32, 0.0],
            // z is the original's flutter phase, w the intensity.
            wind: [angle.cos(), angle.sin(), flutter, WIND_INTENSITY],
            // w = the original's noise scroll in UV space.
            params: [CONFIG.blade_height * 1.45, CONFIG.blade_width, 1.0, scroll],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
    }

    fn draw(&self, world: &mut WorldDrawn<'_>) {
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return;
        };
        // Shortcut: at sub-100% render scale the world depth is smaller than the colour
        // target and can't be attached here, so blades skip depth testing for that frame.
        // Upgrade by sampling the kept depth in the fragment shader.
        let with_depth = world.depth_size == world.size;
        let mut pass = world
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grassiergrass blades"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: world.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: with_depth.then(|| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view: world.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }
                }),
                ..Default::default()
            });
        pass.set_pipeline(if with_depth {
            &self.pipeline
        } else {
            &self.pipeline_no_depth
        });
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
