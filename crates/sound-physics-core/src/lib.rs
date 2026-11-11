//! An engine-independent port of [Sound Physics Remastered]'s occlusion and
//! reverberation model, for Horizon's built-in `sound_physics` feature.
//!
//! Based on Sound Physics Remastered by Sonic Ether and henkelmax
//! <https://github.com/henkelmax/sound-physics-remastered> (GPL-3.0, see `LICENSE`),
//! which is based on Sound Physics by Sonic Ether.
//!
//! What it does: for one sound event it ray-casts the world to work out how much
//! the blocks between the source and the listener absorb (occlusion), and shines
//! rays around the source to work out the room's reverberation. The result is a
//! direct gain and low-pass cutoff, four reverb sends with their own cutoffs, and
//! (optionally) the position the sound should be heard from.
//!
//! The world itself is the host's: [`SoundWorld`] is the block lookups and
//! ray-casts, so the maths here stays free of any engine.
//!
//! ```no_run
//! # use sound_physics_core::{Config, evaluate, SoundWorld};
//! # fn go(world: &dyn SoundWorld, sound: [f64; 3], listener: [f64; 3]) {
//! let env = evaluate(world, sound, listener, false, &Config::default(), 1.0, 16.0, false);
//! let _ = (env.direct_gain, env.direct_cutoff, env.sends);
//! # }
//! ```
//!
//! [Sound Physics Remastered]: https://github.com/henkelmax/sound-physics-remastered

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::f64::consts::TAU;

/// The golden ratio the ray directions' longitude spiral uses (`SoundPhysics.PHI`).
const PHI: f64 = 1.618_033_988;

/// How many reverb sends the model feeds.
pub const SENDS: usize = 4;

/// The settings, with Sound Physics Remastered's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `attenuation_factor`: divides the source's maximum distance.
    pub attenuation_factor: f64,
    /// `reverb_attenuation_distance`: the ray distance at which reverb starts.
    pub reverb_attenuation_distance: f64,
    /// `reverb_gain`.
    pub reverb_gain: f64,
    /// `reverb_brightness`.
    pub reverb_brightness: f64,
    /// `reverb_distance`: how far reverb carries, relative to the source's.
    pub reverb_distance: f64,
    /// `block_absorption`: how much a block wall absorbs.
    pub block_absorption: f64,
    /// `occlusion_variation`: how far the occlusion probe rays are offset.
    pub occlusion_variation: f64,
    /// `default_block_reflectivity`.
    pub default_block_reflectivity: f64,
    /// `default_block_occlusion_factor`.
    pub default_block_occlusion_factor: f64,
    /// `air_absorption`.
    pub air_absorption: f64,
    /// `underwater_filter`: how much the low-pass closes underwater.
    pub underwater_filter: f64,
    /// `evaluate_ambient_sounds`.
    pub evaluate_ambient_sounds: bool,
    /// `environment_evaluation_ray_count`.
    pub environment_ray_count: u32,
    /// `environment_evaluation_ray_bounces`.
    pub environment_ray_bounces: u32,
    /// `non_full_block_occlusion_factor`: scales a hit's occlusion when the face
    /// hit is not a full square.
    pub non_full_block_occlusion_factor: f64,
    /// `max_occlusion_rays`.
    pub max_occlusion_rays: u32,
    /// `max_occlusion`: the cap the occlusion accumulation stops at.
    pub max_occlusion: f64,
    /// `strict_occlusion`: one occlusion path instead of nine.
    pub strict_occlusion: bool,
    /// `sound_direction_evaluation`.
    pub sound_direction_evaluation: bool,
    /// `redirect_non_occluded_sounds`.
    pub redirect_non_occluded_sounds: bool,
    /// `max_sound_processing_distance`.
    pub max_sound_processing_distance: f64,
    /// Sounds with the same name a tick can be processed (`SoundRateConfig`).
    pub max_sounds_per_tick: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            attenuation_factor: 1.0,
            reverb_attenuation_distance: 0.0,
            reverb_gain: 1.0,
            reverb_brightness: 1.0,
            reverb_distance: 1.5,
            block_absorption: 1.0,
            occlusion_variation: 0.35,
            default_block_reflectivity: 0.5,
            default_block_occlusion_factor: 1.0,
            air_absorption: 1.0,
            underwater_filter: 0.9,
            evaluate_ambient_sounds: false,
            environment_ray_count: 32,
            environment_ray_bounces: 4,
            non_full_block_occlusion_factor: 0.25,
            max_occlusion_rays: 16,
            max_occlusion: 64.0,
            strict_occlusion: false,
            sound_direction_evaluation: true,
            redirect_non_occluded_sounds: true,
            max_sound_processing_distance: 512.0,
            max_sounds_per_tick: 8,
        }
    }
}

/// What a ray-cast hit (`RaycastUtils.rayCast` + `BlockHitResult`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub hit: bool,
    /// Where it hit, or the segment's end on a miss.
    pub at: [f64; 3],
    /// The block hit, or the block at the end on a miss.
    pub block: [i32; 3],
    /// The face hit, as a unit vector along an axis.
    pub normal: [f64; 3],
}

impl RayHit {
    pub fn miss(at: [f64; 3]) -> Self {
        Self {
            hit: false,
            at,
            block: [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32],
            normal: [0.0, 0.0, 0.0],
        }
    }
}

/// The world, as the mod reads it: block lookups and the ray-casts.
///
/// The host implements this on its own thread with its own world. `ignore` is the
/// block a ray starts inside, which the traversal must skip (`RaycastUtils`'s
/// `ignore`, so a ray can leave the block it just hit).
pub trait SoundWorld {
    /// How much the block at `pos` occludes sound (`OcclusionConfig`).
    fn block_occlusion(&self, pos: [i32; 3]) -> f64;
    /// How much the block at `pos` reflects sound (`ReflectivityConfig`).
    fn block_reflectivity(&self, pos: [i32; 3]) -> f64;
    /// Whether the block's face on `side` (0..5, `[x-, x+, y-, y+, z-, z+]`) is
    /// sturdy (`BlockState.isFaceSturdy`).
    fn face_is_sturdy(&self, pos: [i32; 3], side: usize) -> bool;
    /// Whether the block at `pos` is water.
    fn is_water(&self, pos: [i32; 3]) -> bool;
    /// The first block or fluid between `from` and `to`, skipping `ignore`.
    fn raycast(&self, from: [f64; 3], to: [f64; 3], ignore: Option<[i32; 3]>) -> RayHit;
}

/// One reverb send: how loud it is, and the low-pass high-frequency gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Send {
    pub gain: f32,
    pub cutoff: f32,
}

impl Send {
    const NONE: Self = Self {
        gain: 0.0,
        cutoff: 1.0,
    };
}

/// What the mixer applies to one sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    /// The direct path's gain.
    pub direct_gain: f32,
    /// The direct path's low-pass high-frequency gain.
    pub direct_cutoff: f32,
    pub sends: [Send; SENDS],
    /// Where the sound should be heard from, when reflections moved it.
    pub position: Option<[f64; 3]>,
    /// The source's air absorption (`AL_AIR_ABSORPTION_FACTOR`).
    pub air_absorption: f32,
}

/// The environment of an unprocessed sound (`setDefaultEnvironment`).
pub fn default_environment(direct_gain: f32) -> Environment {
    Environment {
        direct_gain,
        direct_cutoff: 1.0,
        sends: [Send::NONE; SENDS],
        position: None,
        air_absorption: 1.0,
    }
}

/// One reverb slot's parameters (`ReverbParams`), which the host turns into its
/// own reverb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    pub decay_time: f32,
    pub density: f32,
    pub diffusion: f32,
    pub gain: f32,
    pub gain_hf: f32,
    pub decay_hf_ratio: f32,
    pub reflections_gain: f32,
    pub reflections_delay: f32,
    pub late_reverb_gain: f32,
    pub late_reverb_delay: f32,
    pub air_absorption_gain_hf: f32,
    pub room_rolloff_factor: f32,
}

/// The four reverb presets (`ReverbParams.getReverb0..3`).
pub fn reverb_params(slot: usize, config: &Config) -> ReverbParams {
    let global = 0.7 * config.reverb_gain;
    let bright = config.reverb_brightness;
    let rolloff = config.attenuation_factor;
    match slot {
        // A small room: almost no tail.
        0 => ReverbParams {
            decay_time: 0.15,
            density: 0.0,
            diffusion: 1.0,
            gain: (0.2 * global * 0.85) as f32,
            gain_hf: 0.99,
            decay_hf_ratio: (0.6 * bright) as f32,
            reflections_gain: 2.5,
            reflections_delay: 0.001,
            late_reverb_gain: 1.26,
            late_reverb_delay: 0.011,
            air_absorption_gain_hf: 0.994,
            room_rolloff_factor: (0.16 * rolloff) as f32,
        },
        // A room.
        1 => ReverbParams {
            decay_time: 0.55,
            density: 0.0,
            diffusion: 1.0,
            gain: (0.3 * global * 0.85) as f32,
            gain_hf: 0.99,
            decay_hf_ratio: (0.7 * bright) as f32,
            reflections_gain: 0.2,
            reflections_delay: 0.015,
            late_reverb_gain: 1.26,
            late_reverb_delay: 0.011,
            air_absorption_gain_hf: 0.994,
            room_rolloff_factor: (0.15 * rolloff) as f32,
        },
        // A hall.
        2 => ReverbParams {
            decay_time: 1.68,
            density: 0.1,
            diffusion: 1.0,
            gain: (0.5 * global * 0.85) as f32,
            gain_hf: 0.99,
            decay_hf_ratio: (0.7 * bright) as f32,
            reflections_gain: 0.0,
            reflections_delay: 0.021,
            late_reverb_gain: 1.26,
            late_reverb_delay: 0.021,
            air_absorption_gain_hf: 0.994,
            room_rolloff_factor: (0.13 * rolloff) as f32,
        },
        // A cave.
        _ => ReverbParams {
            decay_time: 4.142,
            density: 0.5,
            diffusion: 1.0,
            gain: (0.4 * global * 0.85) as f32,
            gain_hf: 0.89,
            decay_hf_ratio: (0.7 * bright) as f32,
            reflections_gain: 0.0,
            reflections_delay: 0.025,
            late_reverb_gain: 1.26,
            late_reverb_delay: 0.021,
            air_absorption_gain_hf: 0.994,
            room_rolloff_factor: (0.11 * rolloff) as f32,
        },
    }
}

/// `SoundPhysics.isAmbientSound`: `namespace:ambient...`.
pub fn is_ambient_sound(name: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && path.starts_with("ambient."))
}

/// `SoundPhysics.isBlockSound`: the sound names the mod treats as coming from a block.
pub fn is_block_sound(name: &str) -> bool {
    name.contains("block")
}

/// How many sounds of each name a tick may be processed (`SoundRateManager`).
#[derive(Clone, Debug, Default)]
pub struct RateLimiter {
    counts: HashMap<String, u32>,
    world_ready: bool,
}

impl RateLimiter {
    /// `SoundRateManager.onClientTick` and `onLoadLevel`/`onUnloadLevel`.
    pub fn tick(&mut self, world_ready: bool) {
        self.counts.clear();
        self.world_ready = world_ready;
    }

    pub fn world_ready(&self) -> bool {
        self.world_ready
    }

    /// `incrementAndCheckLimit`: true when this sound is over its limit.
    pub fn over_limit(&mut self, name: &str, config: &Config) -> bool {
        let count = self.counts.entry(name.to_owned()).or_insert(0);
        if *count >= config.max_sounds_per_tick {
            return true;
        }
        *count += 1;
        false
    }
}

fn clamp(v: f64, min: f64, max: f64) -> f64 {
    if v < min { min } else if v > max { max } else { v }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    length(sub(a, b))
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let len = length(a);
    if len > 0.0 { scale(a, 1.0 / len) } else { a }
}

/// `SoundPhysics.reflect`.
fn reflect(dir: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    let d = dot(dir, normal) * 2.0;
    [
        dir[0] - d * normal[0],
        dir[1] - d * normal[1],
        dir[2] - d * normal[2],
    ]
}

fn block_pos(p: [f64; 3]) -> [i32; 3] {
    [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ]
}

/// The nearest face of the block at `pos` to `point` (`Direction.getNearest`).
fn nearest_side(pos: [i32; 3], point: [f64; 3]) -> usize {
    let centre = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
    let d = sub(point, centre);
    let (ax, ay, az) = (d[0].abs(), d[1].abs(), d[2].abs());
    if ax >= ay && ax >= az {
        if d[0] >= 0.0 { 1 } else { 0 }
    } else if ay >= az {
        if d[1] >= 0.0 { 3 } else { 2 }
    } else if d[2] >= 0.0 {
        5
    } else {
        4
    }
}

/// `SoundPhysics.getSharedAirspace`: the vector between the two points when
/// nothing blocks the line between them.
fn shared_airspace(
    world: &dyn SoundWorld,
    from: [f64; 3],
    listener: [f64; 3],
) -> Option<[f64; 3]> {
    if world.raycast(from, listener, None).hit {
        return None;
    }
    Some(sub(from, listener))
}

/// `SoundPhysics.getSharedAirspace` for a hit: start just off the hit face.
fn shared_airspace_at_hit(
    world: &dyn SoundWorld,
    hit: &RayHit,
    listener: [f64; 3],
) -> Option<[f64; 3]> {
    let start = add(hit.at, scale(hit.normal, 0.001));
    shared_airspace(world, start, listener)
}

/// `SoundPhysics.runOcclusion`: walk from the source to the listener, adding up
/// what each block on the way absorbs.
fn run_occlusion(
    world: &dyn SoundWorld,
    sound: [f64; 3],
    listener: [f64; 3],
    config: &Config,
) -> f64 {
    let mut accumulation = 0.0;
    let mut origin = sound;
    let mut last_block = block_pos(sound);

    for _ in 0..config.max_occlusion_rays {
        let hit = world.raycast(origin, listener, Some(last_block));
        last_block = hit.block;
        if !hit.hit {
            break;
        }
        let mut occlusion = world.block_occlusion(hit.block);
        // Whether the face was hit from inside or outside, the nearest face decides.
        let side = nearest_side(hit.block, origin);
        if !world.face_is_sturdy(hit.block, side) {
            occlusion *= config.non_full_block_occlusion_factor;
        }
        accumulation += occlusion;
        origin = hit.at;
        if accumulation > config.max_occlusion {
            break;
        }
    }
    accumulation
}

/// `SoundPhysics.calculateOcclusion`: the strict path, or the least occluded of
/// nine paths offset by the variation factor.
fn calculate_occlusion(
    world: &dyn SoundWorld,
    sound: [f64; 3],
    listener: [f64; 3],
    is_block_sound: bool,
    config: &Config,
) -> f64 {
    if config.strict_occlusion {
        return run_occlusion(world, sound, listener, config).min(config.max_occlusion);
    }

    let mut variation = config.occlusion_variation;
    if is_block_sound {
        variation = variation.max(0.49);
    }

    let mut least = run_occlusion(world, sound, listener, config);
    if variation > 0.0 {
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    let offset = [x * variation, y * variation, z * variation];
                    least = least.min(run_occlusion(
                        world,
                        add(sound, offset),
                        add(listener, offset),
                        config,
                    ));
                }
            }
        }
    }
    least.min(config.max_occlusion)
}

/// Where the sound is heard from, from the reflections that reached the listener
/// (`ReflectedAudio`).
#[derive(Default)]
struct Reflections {
    /// Direction and travel distance of each shared-airspace path.
    directions: Vec<([f64; 3], f64)>,
    /// The direct path's vector, when the source shares air with the listener.
    direct: Option<[f64; 3]>,
    occlusion: f64,
    shared: u32,
}

impl Reflections {
    fn should_evaluate(&self, config: &Config) -> bool {
        config.sound_direction_evaluation
            && (self.occlusion > 0.0 || !config.redirect_non_occluded_sounds)
    }

    fn add_shared(&mut self, vector: [f64; 3], total_ray_distance: f64, config: &Config) {
        let travel = total_ray_distance + length(vector);
        let limit = config.max_sound_processing_distance / config.attenuation_factor;
        if travel <= 0.0 || travel > limit {
            return;
        }
        self.shared += 1;
        if self.should_evaluate(config) {
            self.directions.push((vector, travel));
        }
    }

    /// `evaluateSoundPosition`: the weighted average of where the reflections came
    /// from, at the source's distance.
    fn position(
        &self,
        sound: [f64; 3],
        listener: [f64; 3],
        config: &Config,
    ) -> Option<[f64; 3]> {
        if !self.should_evaluate(config) || self.directions.is_empty() {
            return None;
        }
        let mut sum = self.direct.map_or([0.0, 0.0, 0.0], normalize);
        for (direction, travel) in &self.directions {
            if *travel <= 0.0 {
                return None;
            }
            let weight = 1.0 / (travel * travel);
            sum = add(sum, scale(normalize(*direction), weight));
        }
        let normalized = normalize(sum);
        if length(normalized) < 0.5 {
            return None;
        }
        Some(add(
            scale(normalized, distance(sound, listener)),
            listener,
        ))
    }
}

/// Evaluate one sound's environment, as `SoundPhysics.evaluateEnvironment` does.
///
/// `listener_underwater` is whether the listener is in water, `direct_gain_scale`
/// scales the direct path (0 for a reverb-only pass), and `max_distance` is the
/// source's own maximum distance, which reverb is attenuated against.
pub fn evaluate(
    world: &dyn SoundWorld,
    sound: [f64; 3],
    listener: [f64; 3],
    listener_underwater: bool,
    config: &Config,
    direct_gain_scale: f32,
    max_distance: f64,
    rate_limited: bool,
) -> Environment {
    if rate_limited || distance(listener, sound) > config.max_sound_processing_distance {
        return default_environment(direct_gain_scale);
    }

    let absorption = config.block_absorption * 3.0;
    let source_underwater = world.is_water(block_pos(sound));

    let occlusion = calculate_occlusion(world, sound, listener, false, config);
    let mut direct_cutoff = (-occlusion * absorption).exp();
    let mut direct_gain = f64::from(direct_gain_scale) * direct_cutoff.powf(0.1);

    if listener_underwater || source_underwater {
        direct_cutoff *= 1.0 - config.underwater_filter;
    }

    let const_distance = 256.0;
    let num_rays = config.environment_ray_count.max(1);
    let bounces = config.environment_ray_bounces.max(1);
    let rcp_total_rays = 1.0 / (f64::from(num_rays) * f64::from(bounces));

    let mut reflections = Reflections {
        occlusion,
        ..Reflections::default()
    };
    if let Some(vector) = shared_airspace(world, sound, listener) {
        reflections.direct = Some(vector);
    }

    let mut send_gains = [0.0f64; SENDS];
    let mut bounce_reflectivity = vec![0.0f64; bounces as usize];
    let sound_block = block_pos(sound);
    let angle = PHI * TAU;

    for i in 0..num_rays {
        let fraction = f64::from(i) / f64::from(num_rays);
        let longitude = angle * f64::from(i);
        let latitude = (fraction * 2.0 - 1.0).asin();
        let ray_dir = [
            latitude.cos() * longitude.cos(),
            latitude.cos() * longitude.sin(),
            latitude.sin(),
        ];
        let ray_end = add(sound, scale(ray_dir, const_distance));
        let hit = world.raycast(sound, ray_end, Some(sound_block));
        if !hit.hit {
            continue;
        }

        let mut total_ray_distance = distance(sound, hit.at);
        let mut last_pos = hit.at;
        let mut last_normal = hit.normal;
        let mut last_dir = ray_dir;
        let mut last_block = hit.block;

        if let Some(vector) = shared_airspace_at_hit(world, &hit, listener) {
            reflections.add_shared(vector, total_ray_distance, config);
        }

        for ratio in bounce_reflectivity.iter_mut() {
            let new_dir = reflect(last_dir, last_normal);
            let new_start = last_pos;
            let new_end = add(new_start, scale(new_dir, const_distance));
            let new_hit = world.raycast(new_start, new_end, Some(last_block));

            let block_reflectivity = world.block_reflectivity(last_block);
            let energy_towards_player = 0.25 * (block_reflectivity * 0.75 + 0.25);

            if new_hit.hit {
                *ratio += block_reflectivity;
                total_ray_distance += distance(last_pos, new_hit.at);
                last_pos = new_hit.at;
                last_normal = new_hit.normal;
                last_dir = new_dir;
                last_block = new_hit.block;
                if let Some(vector) = shared_airspace_at_hit(world, &new_hit, listener) {
                    reflections.add_shared(vector, total_ray_distance, config);
                }
            } else {
                // Nothing to bounce off: the path runs on to the listener.
                total_ray_distance += distance(last_pos, listener);
            }

            // Bandaid solution for distance based attenuation.
            if total_ray_distance < config.reverb_attenuation_distance {
                continue;
            }

            let reflection_delay = total_ray_distance.max(0.0) * 0.12 * block_reflectivity;
            let cross = [
                1.0 - clamp((reflection_delay - 0.0).abs(), 0.0, 1.0),
                1.0 - clamp((reflection_delay - 1.0).abs(), 0.0, 1.0),
                1.0 - clamp((reflection_delay - 2.0).abs(), 0.0, 1.0),
                clamp(reflection_delay - 2.0, 0.0, 1.0),
            ];
            let energy = [
                cross[0] * energy_towards_player * 6.4 * rcp_total_rays,
                cross[1] * energy_towards_player * 12.8 * rcp_total_rays,
                cross[2] * energy_towards_player * 12.8 * rcp_total_rays,
                cross[3] * energy_towards_player * 12.8 * rcp_total_rays,
            ];
            for (gain, add) in send_gains.iter_mut().zip(energy) {
                *gain += add;
            }

            if !new_hit.hit {
                break;
            }
        }
    }

    for ratio in bounce_reflectivity.iter_mut() {
        *ratio /= f64::from(num_rays);
    }

    let new_position = reflections.position(sound, listener, config);
    let heard_from = new_position.unwrap_or(sound);

    let shared_airspace = f64::from(reflections.shared) * 64.0 * rcp_total_rays;
    let weights = [
        clamp(shared_airspace / 20.0, 0.0, 1.0),
        clamp(shared_airspace / 15.0, 0.0, 1.0),
        clamp(shared_airspace / 10.0, 0.0, 1.0),
        clamp(shared_airspace / 10.0, 0.0, 1.0),
    ];
    let mut cutoffs = [1.0f64; SENDS];
    let occluded_cutoff = (-occlusion * absorption).exp();
    for (cutoff, weight) in cutoffs.iter_mut().zip(weights) {
        *cutoff = occluded_cutoff * (1.0 - weight) + weight;
    }

    // Preserve some directionality when airspace is shared: let a filtered part of
    // the dry signal through.
    let average_shared =
        (weights[0] + weights[1] + weights[2] + weights[3]) * 0.25;
    direct_cutoff = (average_shared.powf(0.5) * 0.2).max(direct_cutoff);
    direct_gain = f64::from(direct_gain_scale) * direct_cutoff.powf(0.1);

    // The original weights each send by how reflective the paths that fed it were.
    send_gains[1] *= bounce_reflectivity.get(1).copied().unwrap_or(0.0);
    if let Some(ratio) = bounce_reflectivity.get(2) {
        send_gains[2] *= ratio.powf(3.0);
    }
    if let Some(ratio) = bounce_reflectivity.get(3) {
        send_gains[3] *= ratio.powf(4.0);
    }

    send_gains[0] = clamp(send_gains[0], 0.0, 1.0);
    send_gains[1] = clamp(send_gains[1], 0.0, 1.0);
    send_gains[2] = clamp(send_gains[2] * 1.05 - 0.05, 0.0, 1.0);
    send_gains[3] = clamp(send_gains[3] * 1.05 - 0.05, 0.0, 1.0);

    for i in 0..SENDS {
        send_gains[i] *= cutoffs[i].powf(0.1);
    }

    // Reverb is not attenuated by distance on its own.
    let sound_distance = distance(listener, heard_from);
    let send_multiplier = 1.0 - (sound_distance / (max_distance * config.reverb_distance)).min(1.0);
    for gain in send_gains.iter_mut() {
        *gain *= send_multiplier;
    }

    if listener_underwater || source_underwater {
        for cutoff in cutoffs.iter_mut() {
            *cutoff *= 0.4;
        }
    }

    let mut sends = [Send::NONE; SENDS];
    for ((send, gain), cutoff) in sends.iter_mut().zip(send_gains).zip(cutoffs) {
        *send = Send {
            gain: gain as f32,
            cutoff: cutoff as f32,
        };
    }

    Environment {
        direct_gain: direct_gain as f32,
        direct_cutoff: direct_cutoff as f32,
        sends,
        position: new_position,
        air_absorption: config.air_absorption as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world of solid blocks with a voxel walk, enough to test the maths.
    struct Grid {
        solid: fn([i32; 3]) -> bool,
        occlusion: f64,
        reflectivity: f64,
    }

    impl Grid {
        fn new(solid: fn([i32; 3]) -> bool) -> Self {
            Self {
                solid,
                occlusion: 1.0,
                reflectivity: 0.5,
            }
        }
    }

    impl SoundWorld for Grid {
        fn block_occlusion(&self, pos: [i32; 3]) -> f64 {
            if (self.solid)(pos) { self.occlusion } else { 0.0 }
        }

        fn block_reflectivity(&self, pos: [i32; 3]) -> f64 {
            if (self.solid)(pos) { self.reflectivity } else { 0.0 }
        }

        fn face_is_sturdy(&self, _pos: [i32; 3], _side: usize) -> bool {
            true
        }

        fn is_water(&self, _pos: [i32; 3]) -> bool {
            false
        }

        /// A plain voxel walk (Amanatides and Woo), the same shape the host does.
        fn raycast(&self, from: [f64; 3], to: [f64; 3], ignore: Option<[i32; 3]>) -> RayHit {
            let dir = sub(to, from);
            let mut pos = block_pos(from);
            let step = [
                if dir[0] > 0.0 { 1 } else { -1 },
                if dir[1] > 0.0 { 1 } else { -1 },
                if dir[2] > 0.0 { 1 } else { -1 },
            ];
            let mut t_max = [0.0f64; 3];
            let mut t_delta = [f64::INFINITY; 3];
            let mut normal = [0.0f64; 3];
            for axis in 0..3 {
                if dir[axis] == 0.0 {
                    t_max[axis] = f64::INFINITY;
                    continue;
                }
                t_delta[axis] = (1.0 / dir[axis]).abs();
                let boundary = if step[axis] > 0 {
                    pos[axis] as f64 + 1.0
                } else {
                    pos[axis] as f64
                };
                t_max[axis] = (boundary - from[axis]) / dir[axis];
            }
            let mut t = 0.0f64;
            for _ in 0..4096 {
                if (self.solid)(pos) && Some(pos) != ignore {
                    return RayHit {
                        hit: true,
                        at: [from[0] + dir[0] * t, from[1] + dir[1] * t, from[2] + dir[2] * t],
                        block: pos,
                        normal,
                    };
                }
                let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
                    0
                } else if t_max[1] <= t_max[2] {
                    1
                } else {
                    2
                };
                if t_max[axis] > 1.0 {
                    break;
                }
                t = t_max[axis];
                pos[axis] += step[axis];
                normal = [0.0, 0.0, 0.0];
                normal[axis] = -step[axis] as f64;
                t_max[axis] += t_delta[axis];
            }
            RayHit::miss(to)
        }
    }

    fn empty(_pos: [i32; 3]) -> bool {
        false
    }

    /// A slab from x = 8 to 11, floor at y = -1.
    fn room(pos: [i32; 3]) -> bool {
        pos[1] < 0 || (8..=11).contains(&pos[0])
    }

    fn open(config: &Config) -> Config {
        // Deterministic and cheap: no variation, rays from one direction only.
        Config {
            occlusion_variation: 0.0,
            environment_ray_count: 8,
            environment_ray_bounces: 2,
            max_occlusion: 64.0,
            ..config.clone()
        }
    }

    #[test]
    fn open_air_leaves_the_sound_alone() {
        let world = Grid::new(empty);
        let config = open(&Config::default());
        let env = evaluate(
            &world,
            [0.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!((env.direct_gain - 1.0).abs() < 1e-6, "{env:?}");
        assert!((env.direct_cutoff - 1.0).abs() < 1e-6, "{env:?}");
        assert_eq!(env.position, None, "{env:?}");
        for send in env.sends {
            assert!((0.0..=1.0).contains(&send.gain), "{env:?}");
            assert!((0.0..=1.0).contains(&send.cutoff), "{env:?}");
        }
    }

    #[test]
    fn a_wall_between_muffles_the_sound() {
        let world = Grid::new(room);
        let config = open(&Config::default());
        // The listener at x = 0, the source behind a three block wall.
        let env = evaluate(
            &world,
            [13.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!(env.direct_gain < 0.9, "{env:?}");
        assert!(env.direct_cutoff < 0.9, "{env:?}");
        let near = evaluate(
            &world,
            [7.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!(
            near.direct_gain > env.direct_gain,
            "closer: {near:?} behind: {env:?}"
        );
    }

    #[test]
    fn more_blocks_muffle_more() {
        let config = open(&Config::default());
        let one = evaluate(
            &Grid::new(|p| p[1] < 0 || p[0] == 8),
            [13.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        let four = evaluate(
            &Grid::new(|p| p[1] < 0 || (8..=11).contains(&p[0])),
            [13.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!(four.direct_gain < one.direct_gain, "{four:?} vs {one:?}");
        assert!(four.direct_cutoff < one.direct_cutoff, "{four:?} vs {one:?}");
    }

    #[test]
    fn underwater_closes_the_low_pass() {
        let world = Grid::new(room);
        let config = open(&Config::default());
        let dry = evaluate(
            &world,
            [7.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        let wet = evaluate(
            &world,
            [7.5, 0.0, 0.5],
            [0.0, 0.0, 0.0],
            true,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!(wet.direct_cutoff < dry.direct_cutoff, "{wet:?} vs {dry:?}");
    }

    #[test]
    fn an_enclosed_source_sends_reverb() {
        // A hollow room: solid outside a 6x4x6 box, so rays bounce off its walls.
        let world = Grid::new(|p| {
            !((0..=5).contains(&p[0]) && (0..=3).contains(&p[1]) && (0..=5).contains(&p[2]))
        });
        let config = Config {
            occlusion_variation: 0.0,
            environment_ray_count: 32,
            environment_ray_bounces: 4,
            ..Config::default()
        };
        let env = evaluate(
            &world,
            [2.5, 1.5, 2.5],
            [1.5, 1.5, 1.5],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert!(
            env.sends.iter().any(|s| s.gain > 0.0),
            "no reverb in an enclosed space: {env:?}"
        );
        assert!((env.direct_gain - 1.0).abs() < 1e-6, "no wall in the way: {env:?}");
    }

    #[test]
    fn far_or_limited_sounds_rest() {
        let world = Grid::new(empty);
        let config = Config::default();
        let far = evaluate(
            &world,
            [10_000.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            false,
        );
        assert_eq!(far, default_environment(1.0));

        let limited = evaluate(
            &world,
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            false,
            &config,
            1.0,
            16.0,
            true,
        );
        assert_eq!(limited, default_environment(1.0));
    }

    #[test]
    fn ambient_and_block_sounds_are_recognised() {
        assert!(is_ambient_sound("minecraft:ambient.cave"));
        assert!(!is_ambient_sound("minecraft:block.stone.break"));
        assert!(!is_ambient_sound("ambient.cave"));
        assert!(is_block_sound("minecraft:block.stone.break"));
        assert!(!is_block_sound("minecraft:entity.cow.moo"));
    }

    #[test]
    fn the_rate_limiter_counts_per_name() {
        let config = Config {
            max_sounds_per_tick: 2,
            ..Config::default()
        };
        let mut limiter = RateLimiter::default();
        limiter.tick(true);
        assert!(limiter.world_ready());
        assert!(!limiter.over_limit("a", &config));
        assert!(!limiter.over_limit("a", &config));
        assert!(limiter.over_limit("a", &config));
        assert!(!limiter.over_limit("b", &config));
        limiter.tick(true);
        assert!(!limiter.over_limit("a", &config));
    }

    #[test]
    fn the_reverb_presets_grow_with_their_slot() {
        let config = Config::default();
        let decay: Vec<f32> = (0..4).map(|i| reverb_params(i, &config).decay_time).collect();
        assert!(decay.windows(2).all(|w| w[0] < w[1]), "{decay:?}");
        for i in 0..4 {
            let params = reverb_params(i, &config);
            assert!((0.1..=10.0).contains(&params.decay_time));
            assert!((0.0..=1.0).contains(&params.gain));
            assert!((0.0..=1.0).contains(&params.density));
            assert!((0.0..=1.0).contains(&params.diffusion));
        }
    }

    #[test]
    fn a_scanned_sound_never_produces_a_nan() {
        let world = Grid::new(room);
        // Small ray counts: this scans thousands of positions.
        let config = Config {
            occlusion_variation: 0.0,
            environment_ray_count: 4,
            environment_ray_bounces: 2,
            max_occlusion_rays: 4,
            ..Config::default()
        };
        let mut cases = 0;
        for x in -2..14 {
            for y in -1..5 {
                for z in 0..2 {
                    let from = [x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5];
                    for to in [[0.5, 0.5, 0.5], [9.5, 1.5, 2.5]] {
                        let env = evaluate(&world, from, to, false, &config, 1.0, 16.0, false);
                        assert!(env.direct_gain.is_finite(), "{from:?} {to:?}");
                        assert!(env.direct_cutoff.is_finite(), "{from:?} {to:?}");
                        assert!((0.0..=1.0).contains(&env.direct_cutoff), "{env:?}");
                        for send in env.sends {
                            assert!(send.gain.is_finite() && (0.0..=1.0).contains(&send.gain));
                            assert!(send.cutoff.is_finite() && (0.0..=1.0).contains(&send.cutoff));
                        }
                        if let Some(p) = env.position {
                            assert!(p.iter().all(|v| v.is_finite()));
                        }
                        cases += 1;
                    }
                }
            }
        }
        assert!(cases > 300, "{cases}");
    }
}
