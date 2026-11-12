//! Sound Physics Remastered's host side: the client's world as the mod reads it,
//! and the per-tick driver that gives every playing sound an environment.
//!
//! The maths and the block material tables live in `sound_physics_core`; what is
//! here is the world ([`SoundWorld`]: a voxel walk of the client's blocks and
//! their `StateFacts`), and the piece that hands the result to the mixer for the
//! sounds worth processing.
//!
//! Based on Sound Physics Remastered by Sonic Ether and henkelmax (GPL-3.0),
//! which is based on Sound Physics by Sonic Ether.

use super::*;
use crate::sound_engine::{Attenuation, SoundInstance};
use sound_physics_core::{
    Config, Environment, RateLimiter, RayHit, Send, SoundWorld, blocks, evaluate, is_ambient_sound,
};
use std::collections::BTreeSet;

/// The crate's face order (`0..5` = x-, x+, y-, y+, z-, z+) as the game's
/// `Direction` ordinals (DOWN, UP, NORTH, SOUTH, WEST, EAST): the bit
/// `StateFacts.sturdy_full` keeps per face.
const FACE: [u32; 6] = [4, 5, 0, 1, 2, 3];

/// The client's world as the mod reads it.
struct GameWorld<'a, 'g> {
    game: &'a Game<'g>,
    config: &'a Config,
}

/// Whether the block at `pos` is water (`SoundPhysics.isWaterAt`: the fluid, or
/// a waterlogged block, as seagrass).
fn water_at(game: &Game<'_>, pos: [i32; 3]) -> bool {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    let facts = game.res.facts().get(state);
    facts.fluid == Fluid::Water || facts.waterlogged
}

/// The face a ray that already starts inside the block it hits would have come
/// through: the dominant axis of its direction, from the far side.
fn entry_normal(direction: [f64; 3]) -> [f64; 3] {
    let axis = (0..3)
        .max_by(|a, b| direction[*a].abs().total_cmp(&direction[*b].abs()))
        .unwrap_or(0);
    let mut normal = [0.0; 3];
    normal[axis] = if direction[axis] > 0.0 { -1.0 } else { 1.0 };
    normal
}

impl SoundWorld for GameWorld<'_, '_> {
    fn block_occlusion(&self, pos: [i32; 3]) -> f64 {
        let state = self.game.world.block(pos[0], pos[1], pos[2]);
        let facts = self.game.res.facts().get(state);
        blocks::occlusion(&facts.name, self.config)
    }

    fn block_reflectivity(&self, pos: [i32; 3]) -> f64 {
        let state = self.game.world.block(pos[0], pos[1], pos[2]);
        let facts = self.game.res.facts().get(state);
        blocks::reflectivity(&facts.name, self.config)
    }

    fn face_is_sturdy(&self, pos: [i32; 3], side: usize) -> bool {
        let state = self.game.world.block(pos[0], pos[1], pos[2]);
        let facts = self.game.res.facts().get(state);
        facts.sturdy_full & (1 << FACE[side.min(5)]) != 0
    }

    fn is_water(&self, pos: [i32; 3]) -> bool {
        water_at(self.game, pos)
    }

    /// `Level.clip`: the first block between `from` and `to` (air is no block),
    /// skipping the one the ray starts inside (`ignore`, so a ray can leave the
    /// block it just hit). Fluid counts, as water occludes.
    fn raycast(&self, from: [f64; 3], to: [f64; 3], ignore: Option<[i32; 3]>) -> RayHit {
        let direction = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let mut pos = [
            from[0].floor() as i32,
            from[1].floor() as i32,
            from[2].floor() as i32,
        ];
        let mut step = [0i32; 3];
        let mut t_max = [f64::INFINITY; 3];
        let mut t_delta = [f64::INFINITY; 3];
        for axis in 0..3 {
            if direction[axis] == 0.0 {
                continue;
            }
            step[axis] = if direction[axis] > 0.0 { 1 } else { -1 };
            t_delta[axis] = (1.0 / direction[axis]).abs();
            let boundary = if step[axis] > 0 {
                f64::from(pos[axis]) + 1.0
            } else {
                f64::from(pos[axis])
            };
            t_max[axis] = (boundary - from[axis]) / direction[axis];
        }
        // `t_max` and `t` are fractions of the whole segment, so the walk stops
        // at 1.
        let mut t = 0.0;
        let mut normal = [0.0; 3];
        loop {
            if Some(pos) != ignore {
                let state = self.game.world.block(pos[0], pos[1], pos[2]);
                if !self.game.res.facts().is_air(state) {
                    let at = if t == 0.0 && normal == [0.0; 3] {
                        from
                    } else {
                        [
                            from[0] + direction[0] * t,
                            from[1] + direction[1] * t,
                            from[2] + direction[2] * t,
                        ]
                    };
                    return RayHit {
                        hit: true,
                        at,
                        block: pos,
                        normal: if t == 0.0 && normal == [0.0; 3] {
                            entry_normal(direction)
                        } else {
                            normal
                        },
                    };
                }
            }
            let axis = if t_max[0] <= t_max[1] && t_max[0] <= t_max[2] {
                0
            } else if t_max[1] <= t_max[2] {
                1
            } else {
                2
            };
            if t_max[axis] > 1.0 {
                return RayHit::miss(to);
            }
            t = t_max[axis];
            pos[axis] += step[axis];
            t_max[axis] += t_delta[axis];
            normal = [0.0; 3];
            normal[axis] = if step[axis] > 0 { -1.0 } else { 1.0 };
        }
    }
}

/// Whether a sound is worth evaluating: a world sound the listener would hear
/// through the world, near enough to matter (`SoundPhysics.shouldProcessSound`).
fn worth_processing(instance: &SoundInstance, listener: [f64; 3], config: &Config) -> bool {
    if instance.relative
        || instance.attenuation != Attenuation::Linear
        || instance.stopped
        || instance.event.is_none()
        || (!config.evaluate_ambient_sounds && is_ambient_sound(&instance.identifier))
    {
        return false;
    }
    let distance = (0..3)
        .map(|i| (instance.position[i] - listener[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    distance <= config.max_sound_processing_distance
}

/// Give every sound that started since the last tick its environment, as
/// `SoundPhysics.onSoundPlay` + `SoundRateManager.onClientTick` do.
///
/// `evaluated` is the sounds already done, so one is worked out once (the mod's
/// `update_moving_sounds` defaults to false: only a sound that starts is
/// evaluated). `reverb` off zeroes the four sends.
pub(super) fn tick(
    game: &mut Game<'_>,
    config: &Config,
    reverb: bool,
    evaluated: &mut BTreeSet<u64>,
    limiter: &mut RateLimiter,
) {
    let Some(listener) = game.eye_position() else {
        return;
    };
    limiter.tick(true);
    let listener_underwater = water_at(game, [
        listener[0].floor() as i32,
        listener[1].floor() as i32,
        listener[2].floor() as i32,
    ]);
    let updates: Vec<(u64, Environment)> = {
        let world = GameWorld {
            game: &*game,
            config,
        };
        let active = game.audio.engine.active_instances();
        // A sound that ended is evaluated again if its channel is reused.
        let playing: BTreeSet<u64> = active.iter().copied().collect();
        evaluated.retain(|id| playing.contains(id));
        active
            .into_iter()
            .filter_map(|id| {
                if evaluated.contains(&id) {
                    return None;
                }
                let instance = game.audio.engine.instance(id)?;
                if !worth_processing(instance, listener, config) {
                    return None;
                }
                evaluated.insert(id);
                // The mod's own rate limit: so many sounds of one name a tick.
                if limiter.over_limit(&instance.identifier, config) {
                    return None;
                }
                let max_distance = instance
                    .sound
                    .as_ref()
                    .map_or(16.0, |sound| f64::from(sound.attenuation_distance(instance.volume)));
                let mut environment = evaluate(
                    &world,
                    instance.position,
                    listener,
                    listener_underwater,
                    config,
                    1.0,
                    max_distance,
                    false,
                );
                if !reverb {
                    environment.sends = [Send {
                        gain: 0.0,
                        cutoff: 1.0,
                    }; sound_physics_core::SENDS];
                }
                Some((id, environment))
            })
            .collect()
    };
    for (id, environment) in updates {
        game.audio.set_environment(id, Some(environment));
    }
}
