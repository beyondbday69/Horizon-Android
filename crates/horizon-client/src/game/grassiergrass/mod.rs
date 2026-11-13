//! Grassier Grass for Horizon: bakes blades around the camera and draws them after the
//! world pass with the game's own wgpu device, as the `grassiergrass` feature.
//!
//! A native Rust port of Grassier Grass 1.4.5 (leonardoinc22), made with the author's
//! permission. Both the original and this port are All Rights Reserved — see
//! `THIRD_PARTY_NOTICES.md`. Only this clean rewrite is distributed; the original jar,
//! its decompiled Java and its assets are not.
//!
//! Known differences from the original (upstream's README lists the same): no LOD rings,
//! entity trails, clumps, snow, flowers, particles or Iris/Sodium integration. Blades draw
//! on `minecraft:grass_block` only. Biome tint is here: each column blends the biomes of
//! the quart cells around it with the game's own gaussian kernel, as terrain does.
//!
//! Three things are adapted from upstream, because this tree's mod API differs from the
//! revision the bundle pinned:
//!
//! - Upstream reads `host.client_state()` for the eye position and world id. This tree has
//!   neither: world reads are only available in `frame`, so the render pass records
//!   [`WorldDrawn::camera`] and `frame` bakes around it. The anchor therefore lags the
//!   camera by one frame, which the two-block trigger hides.
//! - The dimension is a string (`NativeDimension`), rather than a `NativeWorldId`.
//! - `RenderNeeds`, the depth attachment and the blend state are this tree's.

use std::sync::Mutex;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use grassiergrass_engine as engine;
use horizon_client::mod_api::{RenderHooks, RenderNeedsEvent, WorldDrawn, WorldDrawnEvent, wgpu};

use crate::game::Game;

mod settings;
mod shader;

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

/// The colour blades take on the column at `surface`, as Minecraft blends it there: the
/// biomes of the six quart cells around the position, weighted by the game's own gaussian
/// kernel, and each one's tinted grass colour. Falls back to untinted when the mod turns
/// biome tint off or the column has no biomes loaded.
fn biome_tint(game: &Game<'_>, surface: [f64; 3]) -> [f32; 3] {
    // The colour tables live on the game's resources, as does the world's own list when a
    // pack supplies biomes, so this reads through the same accessor the terrain does.
    let at = |x: i32, y: i32, z: i32| -> u16 {
        game.biome_id_at([f64::from(x), f64::from(y), f64::from(z)])
    };
    let weights = horizon_client_render::biome::gaussian_weights(surface, at);
    if weights.is_empty() {
        return [1.0, 1.0, 1.0];
    }
    // Read after the closure above released its borrow of `game`: the macro borrows a
    // field, so it cannot coexist with the `&self` method the closure calls.
    let biomes = world_biomes!(game);
    let blended =
        horizon_client_render::biome::blend_rgb(&weights, |id| {
            horizon_client_render::biome::rgb(biomes.grass(id, surface[0], surface[2]))
        });
    // The blades are baked in the original's own greens; the multiplier lands them on the
    // biome colour without going black where that colour is dark.
    let base = [0.30 + 0.46, 0.52 + 0.67, 0.17 + 0.26].map(|c| c * 0.5);
    [
        (blended[0] / base[0]).clamp(0.35, 2.0),
        (blended[1] / base[1]).clamp(0.35, 2.0),
        (blended[2] / base[2]).clamp(0.35, 2.0),
    ]
}

struct Mod {
    /// A freshly baked batch waiting for the render thread to upload, if any.
    mesh: Mutex<Option<engine::Mesh>>,
    /// Height and width that mesh was baked with, handed to the GPU with it.
    params: Mutex<[f32; 2]>,
    gpu: Mutex<Option<Gpu>>,
    /// The dimension the mesh was baked in and its anchor: a change forces a rebuild.
    last_anchor: Mutex<Option<(String, [i32; 3])>>,
    /// The camera the last render pass drew from. World reads only happen in `frame`, so
    /// the rebuild is anchored here rather than on the render thread.
    camera: Mutex<Option<[f64; 3]>>,
    /// Feet positions of the players blades should part for, recorded by `frame`: the
    /// render thread has no `Game` to ask, the same reason `camera` is recorded there.
    pushers: Mutex<Vec<[f64; 3]>>,
    clock: Instant,
}

impl Mod {
    fn new() -> Self {
        Self {
            mesh: Mutex::new(None),
            params: Mutex::new([
                settings::Settings::default().blade_height,
                settings::Settings::default().blade_width,
            ]),
            gpu: Mutex::new(None),
            last_anchor: Mutex::new(None),
            camera: Mutex::new(None),
            pushers: Mutex::new(Vec::new()),
            clock: Instant::now(),
        }
    }

    /// Rebuilds when the camera moves two blocks horizontally or one vertically. Block
    /// edits are not watched: the mesh refreshes on the next move.
    fn rebuild(&self, game: &mut Game<'_>, anchor: [i32; 3]) {
        let Some(player) = game.native_player() else {
            return;
        };
        let dimension = player.dimension.to_owned();
        let settings = settings::load(|key| game.native_setting(settings::ID, key));
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

        // `engine::build` borrows its reader for the whole mesh, and reading a block goes
        // through `game` mutably, so the mesher runs against a snapshot of the blocks it
        // will ask for: one column per block column in radius, read top down.
        let (min_y, world_height) = game.native_world_height();
        let top = anchor[1] + 4;
        let bottom = (anchor[1] - 16).max(min_y);
        let reach = settings.radius;
        let mut snapshot: std::collections::HashMap<[i32; 3], engine::Block> =
            std::collections::HashMap::new();
        // One tint per column, blended from the biomes the column's surface sits in.
        let mut tints: std::collections::HashMap<[i32; 3], [f32; 3]> =
            std::collections::HashMap::new();
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                if dx * dx + dz * dz > reach * reach {
                    continue;
                }
                let (x, z) = (anchor[0] + dx, anchor[2] + dz);
                for y in bottom..=top + 1 {
                    let pos = [x, y, z];
                    let block = classify(game, pos, min_y, world_height);
                    snapshot.insert(pos, block);
                }
                if settings.biome_tint {
                    let surface = [f64::from(x) + 0.5, f64::from(anchor[1]), f64::from(z) + 0.5];
                    tints.insert([x, anchor[1], z], biome_tint(game, surface));
                }
            }
        }
        // One build for the whole area: the engine asks for a column's tint only where it
        // actually grows blades, so the biomes are sampled per column, not per block.
        let config = settings.config();
        let tinted = settings.biome_tint;
        let tint_of = |column: [i32; 3]| -> [f32; 3] {
            if !tinted {
                return engine::Config::default().tint;
            }
            tints
                .get(&column)
                .copied()
                .unwrap_or(engine::Config::default().tint)
        };
        let mesh = engine::build_tinted(
            anchor,
            &config,
            |pos| snapshot.get(&pos).copied().unwrap_or(engine::Block::Air),
            tint_of,
        );
        *self.params.lock().unwrap() = [settings.blade_height, settings.blade_width];
        *self.mesh.lock().unwrap() = Some(mesh);
    }

    /// Bakes a new mesh when the camera moved far enough. Called once a frame by the game,
    /// which supplies itself for world reads.
    fn rebuild_if_moved(&self, game: &mut Game<'_>) {
        if !enabled() {
            self.mesh.lock().unwrap().take();
            self.last_anchor.lock().unwrap().take();
            return;
        }
        // Recorded here because `frame` is the only place with a `Game`; the render pass
        // reads them back through `pushers`.
        *self.pushers.lock().unwrap() = game
            .native_player()
            .map(|p| vec![[p.x, p.y, p.z]])
            .unwrap_or_default();
        let Some(camera) = *self.camera.lock().unwrap() else {
            return;
        };
        let anchor = [
            camera[0].floor() as i32,
            camera[1].floor() as i32,
            camera[2].floor() as i32,
        ];
        self.rebuild(game, anchor);
    }
}

/// The mesher's view of one block.
fn classify(game: &Game<'_>, pos: [i32; 3], min_y: i32, height: i32) -> engine::Block {
    if pos[1] < min_y || pos[1] >= min_y + height {
        return engine::Block::Air;
    }
    match game.native_block(pos).as_str() {
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
            gpu.upload(world.device, world.queue, &mesh, self.params.lock().unwrap().to_owned());
        }
        if gpu.index_count == 0 {
            return;
        }

        let elapsed = self.clock.elapsed().as_secs_f32();
        let players = self.pushers.lock().map(|p| p.clone()).unwrap_or_default();
        gpu.write_globals(world.queue, world, elapsed, &players);
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
    /// The players blades part for, anchor-relative: `xyz` position, `w` reach in blocks.
    /// `w = 0` marks an unused slot.
    pushers: [[f32; 4]; PUSHERS],
}

/// How many players a frame can part the grass for.
const PUSHERS: usize = 4;

/// How far from a player blades lean aside, in blocks.
const PUSH_RADIUS: f32 = 1.6;

struct Gpu {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    /// Sub-100% render scale keeps a smaller depth than the frame target, so the pass
    /// cannot attach it; this pipeline samples it instead and discards hidden fragments.
    pipeline_no_depth: wgpu::RenderPipeline,
    /// Layout of `bind_sampled`, kept to rebuild the group when the depth view changes.
    layout_sampled: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// The sampled-depth bind group with the size of the depth view it was built for. A
    /// resize replaces that view, so a size change is enough to know the group is stale.
    bind_sampled: Mutex<Option<(u32, wgpu::BindGroup)>>,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
    index_count: u32,
    anchor: [i32; 3],
    /// Blade height and width the current mesh was baked with, for the shader's params.
    params: [f32; 2],
}

impl Gpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let uniforms = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // Two layouts: the attached-depth path binds the uniform alone, the sampled path
        // also binds the world depth as a filtering-less depth texture.
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grassiergrass blades"),
            entries: &[uniforms],
        });
        let layout_sampled = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grassiergrass blades sampled"),
            entries: &[
                uniforms,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grassiergrass blades"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline_layout_sampled =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grassiergrass blades sampled"),
                bind_group_layouts: &[Some(&layout_sampled)],
                ..Default::default()
            });
        let shader_attached = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grassiergrass blades attached"),
            source: wgpu::ShaderSource::Wgsl(shader::blades_wgsl_attached().into()),
        });
        let shader_sampled = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grassiergrass blades sampled"),
            source: wgpu::ShaderSource::Wgsl(shader::blades_wgsl_sampled().into()),
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Float32x3];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<engine::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let make_pipeline = |depth: bool, shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("grassiergrass blades"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_blade"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                // Reversed depth: nearer is greater. `GreaterEqual`, as the terrain pass
                // uses: a blade's base sits exactly on the grass block's top face, and
                // `Greater` throws those fragments away, which reads as grass you can see
                // the ground through. The bias pulls blades a hair towards the camera so
                // the two surfaces never fight for the same depth.
                depth_stencil: depth.then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: -2,
                        slope_scale: -1.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
        let pipeline = make_pipeline(true, &shader_attached, &pipeline_layout);
        let pipeline_no_depth = make_pipeline(false, &shader_sampled, &pipeline_layout_sampled);
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
            layout_sampled,
            globals,
            bind,
            bind_sampled: Mutex::new(None),
            vertices: None,
            indices: None,
            index_count: 0,
            anchor: [0; 3],
            params: [settings::Settings::default().blade_height, settings::Settings::default().blade_width],
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &engine::Mesh,
        params: [f32; 2],
    ) {
        self.params = params;
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

    fn write_globals(
        &self,
        queue: &wgpu::Queue,
        world: &WorldDrawn<'_>,
        elapsed: f32,
        players: &[[f64; 3]],
    ) {
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
        // Anchor-relative, as the vertices are: the shader never sees world space.
        let mut pushers = [[0.0f32; 4]; PUSHERS];
        for (slot, at) in pushers.iter_mut().zip(players.iter()) {
            *slot = [
                at[0] as f32 - self.anchor[0] as f32,
                at[1] as f32 - self.anchor[1] as f32,
                at[2] as f32 - self.anchor[2] as f32,
                PUSH_RADIUS,
            ];
        }
        let globals = Globals {
            view_proj: world.view_proj.to_cols_array(),
            offset: [offset[0] as f32, offset[1] as f32, offset[2] as f32, 0.0],
            // z is the original's flutter phase, w the intensity.
            wind: [angle.cos(), angle.sin(), flutter, WIND_INTENSITY],
            // w = the original's noise scroll in UV space.
            params: [self.params[0] * 1.45, self.params[1], 1.0, scroll],
            pushers,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
    }

    fn draw(&self, world: &mut WorldDrawn<'_>) {
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return;
        };
        // A pass cannot attach a depth texture of a different size than its colour target,
        // which is the case at any render scale but 100%. Attach it when the sizes match and
        // sample it otherwise, so blades are hidden by terrain either way.
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
        if with_depth {
            pass.set_bind_group(0, &self.bind, &[]);
        } else {
            // The group holds the depth view, which is recreated whenever the target is
            // resized, so it is rebuilt on first use and after every resize.
            let mut cached = self.bind_sampled.lock().unwrap();
            let size = world.depth_size[0];
            if cached.as_ref().is_some_and(|(built, _)| *built != size) {
                *cached = None;
            }
            let cached: &mut (u32, wgpu::BindGroup) = cached.get_or_insert_with(|| {
                let group = world.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("grassiergrass blades sampled"),
                    layout: &self.layout_sampled,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.globals.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(world.depth),
                        },
                    ],
                });
                (size, group)
            });
            // `set_bind_group` takes anything `Option<&BindGroup>` is built from, and the
            // only such impl is the blanket one, so the reference must be explicit.
            let group: Option<&wgpu::BindGroup> = Some(&cached.1);
            pass.set_bind_group(0, group, &[]);
        }
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// `native_settings`: the rows shown under `Options > Mods > Grassier Grass`.
pub(super) fn settings_labels(game: &Game<'_>) -> Vec<String> {
    let settings = settings::load(|key| game.native_setting(settings::ID, key));
    settings::labels(&settings)
}

/// `native_settings_click`: steps the clicked row and forces the next bake to redraw.
pub(super) fn settings_click(game: &mut Game<'_>, row: usize) {
    let settings = settings::load(|key| game.native_setting(settings::ID, key));
    let settings::Effect::Save(knob, value) = settings::click(&settings, row) else {
        return;
    };
    game.native_set_setting(settings::ID, settings::key_of(knob), &value);
    // Drop the remembered anchor so the next frame bakes with the new values even when
    // the player has not moved.
    if let Some(mod_) = MOD.get() {
        mod_.last_anchor.lock().unwrap().take();
    }
}
