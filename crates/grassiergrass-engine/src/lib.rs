//! Grassier Grass for Horizon, CPU side: a deterministic mesher that bakes the blades of the
//! grass blocks around a camera into camera-anchored triangles.
//!
//! Ported from Grassier Grass 1.4.5 (`GrassSectionBuilder` / `GrassGeometry`): same blade
//! dimensions, same two stacked quads, and the same per-vertex packing the shader decodes
//! (`color.a` = side bit + 2 height-class bits + 5 angle bits, `normal.xy` = world-locked noise
//! coordinates, `normal.z` = height fraction). The LOD rings, clumps, trails, snow, flowers and
//! the plant pass of the original are not here.
//!
//! This crate has no game dependencies so it can be built and tested on its own.

pub use bytemuck;

/// Two stacked quads per blade, as the original bakes them (`BLADE_SPLIT_FRACTION`).
pub const BLADE_SEGMENTS: usize = 2;
/// Quads per blade (upper and lower half).
pub const QUADS_PER_BLADE: usize = BLADE_SEGMENTS;
/// Vertices per blade.
pub const VERTS_PER_BLADE: usize = QUADS_PER_BLADE * 4;
/// Indices per blade.
pub const INDICES_PER_BLADE: usize = QUADS_PER_BLADE * 6;

const BLADE_SPLIT: f32 = 2.0 / 3.0;
const BASE_WIDTH: f32 = 0.04;
const MIDDLE_WIDTH: f32 = 0.05;
const TIP_WIDTH: f32 = 0.013;
const WIDTH_SCALE: f32 = 0.8947368;
const TALL_HEIGHT_MULTIPLIER: f32 = 1.45;
const NOISE_TILE_BLOCKS: f32 = 32.0;

/// One vertex, laid out exactly as the render pipeline declares it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Camera-anchor-relative position, in blocks.
    pub pos: [f32; 3],
    /// `rgb` blade tint, `a` the packed side/height-class/angle bits.
    pub color: [u8; 4],
    /// `xy` world-locked noise coordinates in `-1..1`, `z` the height fraction.
    pub normal: [f32; 3],
}

/// What the caller knows about a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Air,
    Grass,
    Other,
}

/// Knobs for the slice. The original exposes these in its config screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Horizontal radius in blocks.
    pub radius: i32,
    pub blades_per_block: u32,
    /// Configured blade height, before the 1.45 visual multiplier (default 0.3).
    pub blade_height: f32,
    /// Width multiplier (default 0.95).
    pub blade_width: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            radius: 20,
            blades_per_block: 24,
            blade_height: 0.3,
            blade_width: 0.95,
        }
    }
}

/// A baked batch, positioned relative to `anchor` (the camera's block position at build time).
pub struct Mesh {
    pub anchor: [i32; 3],
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub blades: usize,
}

impl Mesh {
    pub fn empty(anchor: [i32; 3]) -> Self {
        Self {
            anchor,
            vertices: Vec::new(),
            indices: Vec::new(),
            blades: 0,
        }
    }
}

/// Bakes every grass block with air above it within `config.radius` of `anchor`, using `block`
/// to read the world. `anchor.y + 4` down to `anchor.y - 16` is scanned, so a camera far above
/// the ground finds nothing.
pub fn build(anchor: [i32; 3], config: &Config, mut block: impl FnMut([i32; 3]) -> Block) -> Mesh {
    let mut mesh = Mesh::empty(anchor);
    let r2 = config.radius * config.radius;
    let top = anchor[1] + 4;
    let bottom = anchor[1] - 16;

    for dz in -config.radius..=config.radius {
        for dx in -config.radius..=config.radius {
            if dx * dx + dz * dz > r2 {
                continue;
            }
            let x = anchor[0] + dx;
            let z = anchor[2] + dz;
            // As the original: blades only on a grass block whose block above is air.
            let mut above = block([x, top + 1, z]);
            for y in (bottom..=top).rev() {
                let here = block([x, y, z]);
                if here == Block::Grass && above == Block::Air {
                    emit_block(&mut mesh, anchor, config, [x, y + 1, z]);
                }
                above = here;
            }
        }
    }
    mesh
}

fn emit_block(mesh: &mut Mesh, anchor: [i32; 3], config: &Config, base: [i32; 3]) {
    let seed = hash_pos(base);
    let height = config.blade_height * TALL_HEIGHT_MULTIPLIER;
    let [origin_x, origin_y, origin_z] = base;

    for blade in 0..config.blades_per_block {
        let bx = 0.08 + rand01(seed, blade as u64 * 4 + 1);
        let bz = 0.08 + rand01(seed, blade as u64 * 4 + 2);
        // `quantizedBladeAngle`: floor(random * 31), so bucket 31 is only the clamped 2π edge.
        let bucket = (rand01(seed, blade as u64 * 4 + 3) * 31.0) as u32;
        let angle = bucket as f32 / 31.0 * std::f32::consts::TAU;
        let (dir_x, dir_z) = (angle.cos(), angle.sin());

        let local_x = origin_x as f32 + bx - anchor[0] as f32;
        let local_z = origin_z as f32 + bz - anchor[2] as f32;
        let local_y = origin_y as f32 - anchor[1] as f32;

        let noise_x = noise_coord(origin_x as f32 + bx);
        let noise_z = noise_coord(origin_z as f32 + bz);
        let normal_x = noise_x * 2.0 - 1.0;
        let normal_z = noise_z * 2.0 - 1.0;

        let tint = blade_tint(seed, blade as u64);

        for section in 0..BLADE_SEGMENTS {
            // The original's two-quad layout splits at 2/3, not halfway (BLADE_SPLIT).
            let [t0, t1] = if section == 0 {
                [0.0, BLADE_SPLIT]
            } else {
                [BLADE_SPLIT, 1.0]
            };
            let y0 = local_y + height * t0;
            let y1 = local_y + height * t1;
            let w0 = blade_width(t0, config.blade_width);
            let w1 = blade_width(t1, config.blade_width);

            let lower_left = Vertex {
                pos: [local_x - dir_x * w0, y0, local_z - dir_z * w0],
                color: with_alpha(tint, pack_alpha(-1.0, bucket, 0)),
                normal: [normal_x, normal_z, t0],
            };
            let lower_right = Vertex {
                pos: [local_x + dir_x * w0, y0, local_z + dir_z * w0],
                color: with_alpha(tint, pack_alpha(1.0, bucket, 0)),
                normal: [normal_x, normal_z, t0],
            };
            let upper_right = Vertex {
                pos: [local_x + dir_x * w1, y1, local_z + dir_z * w1],
                color: with_alpha(tint, pack_alpha(1.0, bucket, 0)),
                normal: [normal_x, normal_z, t1],
            };
            let upper_left = Vertex {
                pos: [local_x - dir_x * w1, y1, local_z - dir_z * w1],
                color: with_alpha(tint, pack_alpha(-1.0, bucket, 0)),
                normal: [normal_x, normal_z, t1],
            };

            let base = mesh.vertices.len() as u32;
            mesh.vertices
                .extend([lower_left, lower_right, upper_right, upper_left]);
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh.blades += 1;
    }
}

/// The shader's `bladeWidthAt`, halved into a side offset.
fn blade_width(t: f32, width_scale: f32) -> f32 {
    let tapered = width_scale * WIDTH_SCALE;
    if t <= BLADE_SPLIT {
        lerp(BASE_WIDTH, MIDDLE_WIDTH, t / BLADE_SPLIT) * tapered
    } else {
        lerp(
            MIDDLE_WIDTH,
            TIP_WIDTH,
            (t - BLADE_SPLIT) / (1.0 - BLADE_SPLIT),
        ) * tapered
    }
}

/// Must mirror the vertex shader's decode (`packBladeAlpha` in the original).
pub fn pack_alpha(side_sign: f32, angle_bucket: u32, height_class: u32) -> u8 {
    let side = if side_sign < 0.0 { 0 } else { 128 };
    (side + height_class.min(3) * 32 + angle_bucket.min(31)) as u8
}

/// Inverse of [`pack_alpha`], for tests and tools.
pub fn unpack_alpha(packed: u8) -> (f32, u32, u32) {
    let right = u32::from(packed) >= 128;
    let rest = u32::from(packed) - u32::from(right) * 128;
    let height_class = rest / 32;
    let bucket = rest - height_class * 32;
    (if right { 1.0 } else { -1.0 }, bucket, height_class)
}

fn with_alpha(tint: [u8; 3], alpha: u8) -> [u8; 4] {
    [tint[0], tint[1], tint[2], alpha]
}

/// The original tints blades with the block's biome grass colour. The slice has no biome table,
/// so it bakes a two-colour green palette with a per-blade blend.
fn blade_tint(seed: u64, blade: u64) -> [u8; 3] {
    const A: [f32; 3] = [0.30, 0.52, 0.17];
    const B: [f32; 3] = [0.46, 0.67, 0.26];
    let blend = 0.35 + 0.65 * rand01(seed, blade * 4 + 4);
    let mut out = [0u8; 3];
    for (out, (a, b)) in out.iter_mut().zip(A.into_iter().zip(B)) {
        *out = (lerp(a, b, blend) * 255.0).round() as u8;
    }
    out
}

/// `GrassGeometry.worldNoiseCoord`: world position into a repeating 32-block tile, 0..1.
fn noise_coord(world: f32) -> f32 {
    let tiled = world / NOISE_TILE_BLOCKS;
    tiled - tiled.floor()
}

fn hash_pos([x, y, z]: [i32; 3]) -> u64 {
    let x = u64::from(x as u32);
    let y = u64::from(y as u32);
    let z = u64::from(z as u32);
    mix(x | (y << 21) | (z << 42))
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn rand01(seed: u64, salt: u64) -> f32 {
    let bits = mix(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 40;
    bits as f32 * (1.0 / (1u64 << 24) as f32)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grass plane at y = 63, with stone below and air above.
    fn flat_world([x, y, z]: [i32; 3]) -> Block {
        let _ = (x, z);
        if y > 63 {
            Block::Air
        } else if y == 63 {
            Block::Grass
        } else {
            Block::Other
        }
    }

    #[test]
    fn one_quad_pair_per_blade() {
        let config = Config {
            blades_per_block: 8,
            ..Config::default()
        };
        let mesh = build([0, 64, 0], &config, flat_world);
        assert!(mesh.blades > 0);
        assert_eq!(mesh.vertices.len(), mesh.blades * VERTS_PER_BLADE);
        assert_eq!(mesh.indices.len(), mesh.blades * INDICES_PER_BLADE);
    }

    #[test]
    fn deterministic() {
        let config = Config::default();
        let a = build([0, 64, 0], &config, flat_world);
        let b = build([0, 64, 0], &config, flat_world);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
    }

    #[test]
    fn alpha_round_trips() {
        for bucket in 0..32 {
            for class in 0..4 {
                for side in [-1.0, 1.0] {
                    let packed = pack_alpha(side, bucket, class);
                    let (decoded_side, decoded_bucket, decoded_class) = unpack_alpha(packed);
                    assert_eq!(decoded_side, side);
                    assert_eq!(decoded_bucket, bucket);
                    assert_eq!(decoded_class, class);
                }
            }
        }
    }

    #[test]
    fn blades_sit_above_their_block_and_inside_the_radius() {
        let config = Config {
            blades_per_block: 4,
            ..Config::default()
        };
        let anchor = [0, 64, 0];
        let mesh = build(anchor, &config, flat_world);
        let r = config.radius as f32 + 2.0;
        for v in &mesh.vertices {
            let x = v.pos[0] + anchor[0] as f32;
            let z = v.pos[2] + anchor[2] as f32;
            assert!((x as i32 - anchor[0]).abs() <= r as i32);
            assert!((z as i32 - anchor[2]).abs() <= r as i32);
            assert!(v.pos[1] >= 0.0, "blade starts on top of the grass block");
            assert!(v.pos[1] <= config.blade_height * 1.45 + 0.001);
        }
    }

    #[test]
    fn non_grass_columns_get_no_blades() {
        let mesh = build([0, 64, 0], &Config::default(), |_| Block::Other);
        assert_eq!(mesh.blades, 0);
        assert!(mesh.vertices.is_empty());
    }

    /// A tree canopy above the ground doesn't hide the blades under it.
    #[test]
    fn canopy_above_does_not_hide_the_grass() {
        let mut mesh = build([0, 64, 0], &Config::default(), |[_, y, _]| {
            if y == 70 {
                Block::Other
            } else if y > 63 {
                Block::Air
            } else if y == 63 {
                Block::Grass
            } else {
                Block::Other
            }
        });
        assert!(mesh.blades > 0);
    }

    #[test]
    fn blade_width_matches_the_original_shape() {
        assert!((blade_width(0.0, 1.0) - 0.04 * WIDTH_SCALE).abs() < 1e-6);
        assert!((blade_width(BLADE_SPLIT, 1.0) - 0.05 * WIDTH_SCALE).abs() < 1e-6);
        assert!((blade_width(1.0, 1.0) - 0.013 * WIDTH_SCALE).abs() < 1e-6);
    }
}
