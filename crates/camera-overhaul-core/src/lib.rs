//! Rust port of CameraOverhaul's camera logic by Mirsario & Contributors.
//! Original: https://github.com/Mirsario/Minecraft-CameraOverhaul
//! Licensed under GPL-3.0 (see LICENSE.md). This port is GPL-3.0 too.
//!
//! The crate has no engine dependency. Each frame the host fills a
//! `CameraContext`, calls `CameraOverhaul::update`, then ADDS the returned
//! offsets (degrees) to the camera: pitch, yaw, roll.

pub mod config;
pub mod math;
pub mod noise;
pub mod shakes;

use config::{Config, Contextual};
use math::*;
use noise::simplex2;
use shakes::ScreenShakes;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perspective {
    FirstPerson,
    ThirdPerson,
    ThirdPersonReverse,
}

/// Per-frame input from the game. Angles are degrees (Minecraft convention).
#[derive(Clone, Debug)]
pub struct CameraContext {
    pub is_swimming: bool,
    pub is_flying: bool, // elytra
    pub is_sprinting: bool,
    pub is_riding_mount: bool,
    pub is_riding_vehicle: bool,
    /// Player (or vehicle) velocity in blocks/tick, world space.
    pub velocity: [f64; 3],
    pub perspective: Perspective,
    pub position: [f64; 3],
    pub pitch: f64,
    pub yaw: f64,
}

impl CameraContext {
    /// Velocity rotated into the camera's frame: (strafe, vertical, forward).
    fn forward_relative_velocity(&self) -> [f64; 3] {
        let rad = (360.0 - self.yaw).to_radians();
        let (s, c) = rad.sin_cos();
        let (vx, vz) = (self.velocity[0], self.velocity[2]);
        [c * vx - s * vz, self.velocity[1], s * vx + c * vz]
    }
}

/// Offsets to add to the camera, in degrees.
#[derive(Clone, Copy, Default, Debug)]
pub struct CameraOffset {
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

const BASE_VERTICAL_PITCH_SMOOTHING: f64 = 0.00004;
const VERTICAL_PITCH_THRESHOLD: f64 = 0.4;
const BASE_FORWARD_PITCH_SMOOTHING: f64 = 0.008;
const BASE_TURNING_ROLL_ACCUMULATION: f64 = 0.0048;
const BASE_TURNING_ROLL_INTENSITY: f64 = 1.25;
const BASE_TURNING_ROLL_SMOOTHING: f64 = 0.0825;
const BASE_STRAFING_ROLL_SMOOTHING: f64 = 0.008;
const CAMERASWAY_FADING_SMOOTHNESS: f64 = 3.0;
const BASE_MOUSE_SMOOTHING: f64 = 16.0;
const MOUSE_SMOOTHING_THRESHOLD: f64 = 0.001;

pub struct CameraOverhaul {
    pub config: Config,
    pub shakes: ScreenShakes,
    time: f64,
    ctx_cfg: Contextual,
    prev_euler: [f64; 3], // pitch, yaw, roll(0)
    prev_velocity: [f64; 3],
    prev_perspective: Option<Perspective>,
    last_action_time: f64,
    prev_vertical_pitch: f64,
    prev_forward_pitch: f64,
    turning_roll_target: f64,
    prev_strafing_roll: f64,
    sway_factor: f64,
    sway_factor_target: f64,
    ms_init: bool,
    prev_yaw_norm: f64,
    prev_pitch_norm: f64,
    cont_yaw: f64,
    cont_pitch: f64,
    sm_yaw: f64,
    sm_pitch: f64,
}

impl CameraOverhaul {
    pub fn new(config: Config) -> Self {
        let ctx_cfg = config.walking;
        Self {
            config,
            shakes: ScreenShakes::default(),
            time: 0.0,
            ctx_cfg,
            prev_euler: [0.0; 3],
            prev_velocity: [0.0; 3],
            prev_perspective: None,
            last_action_time: 0.0,
            prev_vertical_pitch: 0.0,
            prev_forward_pitch: 0.0,
            turning_roll_target: 0.0,
            prev_strafing_roll: 0.0,
            sway_factor: 0.0,
            sway_factor_target: 0.0,
            ms_init: false,
            prev_yaw_norm: 0.0,
            prev_pitch_norm: 0.0,
            cont_yaw: 0.0,
            cont_pitch: 0.0,
            sm_yaw: 0.0,
            sm_pitch: 0.0,
        }
    }

    /// Call on player actions (hand swing, etc.) to pause the idle sway.
    pub fn notify_player_action(&mut self) {
        self.last_action_time = self.time;
    }

    pub fn on_explosion(&mut self, pos: [f64; 3]) {
        self.shakes.explosion(self.time, pos, &self.config);
    }
    pub fn on_lightning(&mut self, pos: [f64; 3]) {
        self.shakes.lightning(self.time, pos, &self.config);
    }
    pub fn on_hand_swing(&mut self) {
        self.shakes.hand_swing(self.time, &self.config);
        self.notify_player_action();
    }

    /// Call once per rendered frame (skip while the game is paused).
    pub fn update(&mut self, ctx: &CameraContext, dt: f64) -> CameraOffset {
        self.time += dt;
        self.update_context(ctx, dt);

        let general = self.config.general.clone();
        if !general.enabled
            || (!general.enable_in_third_person && ctx.perspective != Perspective::FirstPerson)
        {
            return CameraOffset::default();
        }

        let mut out = CameraOffset::default();

        // Screen shakes (pitch, yaw, roll).
        let sh = self.shakes.sample(self.time, ctx.position, &self.config);
        let shake = CameraOffset { pitch: sh[0], yaw: sh[1], roll: sh[2] };

        if ctx.velocity != self.prev_velocity || [ctx.pitch, ctx.yaw, 0.0] != self.prev_euler {
            self.notify_player_action();
        }

        self.camera_smoothing(ctx, &mut out, dt);
        self.noise_offset(&mut out, dt);
        self.vertical_velocity_pitch(ctx, &mut out, dt);
        self.forward_velocity_pitch(ctx, &mut out, dt);
        self.turning_roll(ctx, &mut out, dt);
        self.strafing_roll(ctx, &mut out, dt);

        self.prev_velocity = ctx.velocity;
        self.prev_euler = [ctx.pitch, ctx.yaw, 0.0];
        self.prev_perspective = Some(ctx.perspective);

        out.pitch += shake.pitch;
        out.yaw += shake.yaw;
        out.roll += shake.roll;
        out
    }

    fn update_context(&mut self, ctx: &CameraContext, dt: f64) {
        let c = &self.config;
        let target = if ctx.is_riding_vehicle {
            c.vehicles
        } else if ctx.is_riding_mount {
            c.mounts
        } else if ctx.is_swimming {
            c.swimming
        } else if ctx.is_flying {
            c.flying
        } else if ctx.is_sprinting {
            c.sprinting
        } else {
            c.walking
        };
        let smoothing = if c.general.context_transition_smoothing > 0.0 {
            damp_step(c.general.context_transition_smoothing, dt)
        } else {
            1.0
        };
        self.ctx_cfg.lerp_to(&target, damp_step(smoothing, dt));
    }

    fn vertical_velocity_pitch(&mut self, ctx: &CameraContext, out: &mut CameraOffset, dt: f64) {
        let smoothing = BASE_VERTICAL_PITCH_SMOOTHING * self.ctx_cfg.vertical_velocity_smoothing_factor;
        let mut target = ctx.velocity[1] * self.ctx_cfg.vertical_velocity_pitch_factor;
        if target.abs() < VERTICAL_PITCH_THRESHOLD {
            target = 0.0; // fixes slime block shake
        }
        let cur = damp(self.prev_vertical_pitch, target, smoothing, dt);
        out.pitch += cur;
        self.prev_vertical_pitch = cur;
    }

    fn forward_velocity_pitch(&mut self, ctx: &CameraContext, out: &mut CameraOffset, dt: f64) {
        let smoothing = BASE_FORWARD_PITCH_SMOOTHING * self.ctx_cfg.horizontal_velocity_smoothing_factor;
        let target = ctx.forward_relative_velocity()[2] * self.ctx_cfg.forward_velocity_pitch_factor;
        let cur = damp(self.prev_forward_pitch, target, smoothing, dt);
        out.pitch += cur;
        self.prev_forward_pitch = cur;
    }

    fn turning_roll(&mut self, ctx: &CameraContext, out: &mut CameraOffset, dt: f64) {
        let g = &self.config.general;
        let decay = BASE_TURNING_ROLL_SMOOTHING * g.turning_roll_smoothing;
        let intensity = BASE_TURNING_ROLL_INTENSITY * g.turning_roll_intensity;
        let accumulation = BASE_TURNING_ROLL_ACCUMULATION * g.turning_roll_accumulation;
        let mut yaw_delta = self.prev_euler[1] - ctx.yaw;
        if Some(ctx.perspective) != self.prev_perspective {
            yaw_delta = 0.0; // don't spazz when switching perspectives
        }
        self.turning_roll_target = damp(self.turning_roll_target, 0.0, decay, dt);
        self.turning_roll_target =
            clamp(self.turning_roll_target + yaw_delta * accumulation, -1.0, 1.0);
        let t = self.turning_roll_target;
        let roll = clamp01(turning_easing(t.abs())) * intensity * t.signum();
        out.roll += if t == 0.0 { 0.0 } else { roll };
    }

    fn strafing_roll(&mut self, ctx: &CameraContext, out: &mut CameraOffset, dt: f64) {
        let smoothing = BASE_STRAFING_ROLL_SMOOTHING * self.ctx_cfg.horizontal_velocity_smoothing_factor;
        let target = -ctx.forward_relative_velocity()[0] * self.ctx_cfg.strafing_roll_factor;
        let off = damp(self.prev_strafing_roll, target, smoothing, dt);
        out.roll += off;
        self.prev_strafing_roll = off;
    }

    fn noise_offset(&mut self, out: &mut CameraOffset, dt: f64) {
        let g = &self.config.general;
        let time = self.time;
        let noise_x = (time * g.camera_sway_frequency) as f32;

        if (time - self.last_action_time) < g.camera_sway_fade_in_delay {
            self.sway_factor_target = 0.0; // fade out
        } else if self.sway_factor == self.sway_factor_target {
            self.sway_factor_target = 1.0; // fade in
        }
        let fade_len = if self.sway_factor_target > 0.0 {
            g.camera_sway_fade_in_length
        } else {
            g.camera_sway_fade_out_length
        };
        let fade_step = if fade_len > 0.0 { dt / fade_len } else { 1.0 };
        self.sway_factor = step_towards(self.sway_factor, self.sway_factor_target, fade_step);

        let intensity = g.camera_sway_intensity * self.sway_factor.powf(CAMERASWAY_FADING_SMOOTHNESS);
        out.pitch += simplex2(noise_x, 420.0) as f64 * intensity;
        out.yaw += simplex2(noise_x, 1337.0) as f64 * intensity;
        // roll (z) target is 0 in the original, so no roll sway.
    }

    fn camera_smoothing(&mut self, ctx: &CameraContext, out: &mut CameraOffset, dt: f64) {
        let value = self.ctx_cfg.camera_smoothing.max(0.0);
        let (yaw_now, pitch_now) = (ctx.yaw, ctx.pitch);

        if !self.ms_init || Some(ctx.perspective) != self.prev_perspective {
            self.prev_yaw_norm = yaw_now;
            self.prev_pitch_norm = pitch_now;
            self.cont_yaw = yaw_now;
            self.cont_pitch = pitch_now;
            self.sm_yaw = yaw_now;
            self.sm_pitch = pitch_now;
            self.ms_init = true;
            return;
        }

        let step_yaw = unwrap_step(yaw_now - self.prev_yaw_norm);
        let step_pitch = unwrap_step(pitch_now - self.prev_pitch_norm);
        self.prev_yaw_norm = yaw_now;
        self.prev_pitch_norm = pitch_now;
        self.cont_yaw += step_yaw;
        self.cont_pitch += step_pitch;

        if value <= MOUSE_SMOOTHING_THRESHOLD {
            self.sm_yaw = self.cont_yaw;
            self.sm_pitch = self.cont_pitch;
            return;
        }

        let k = BASE_MOUSE_SMOOTHING / value;
        let step = 1.0 - (-k * dt.max(0.0)).exp();
        self.sm_yaw += (self.cont_yaw - self.sm_yaw) * step;
        self.sm_pitch += (self.cont_pitch - self.sm_pitch) * step;

        out.yaw += wrap_diff(self.sm_yaw - yaw_now);
        out.pitch += wrap_diff(self.sm_pitch - pitch_now);
    }
}

fn turning_easing(x: f64) -> f64 {
    // easeInOutCubic
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CameraContext {
        CameraContext {
            is_swimming: false,
            is_flying: false,
            is_sprinting: false,
            is_riding_mount: false,
            is_riding_vehicle: false,
            velocity: [0.0; 3],
            perspective: Perspective::FirstPerson,
            position: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn standing_still_is_small() {
        let mut co = CameraOverhaul::new(Config::default());
        let c = ctx();
        let mut o = CameraOffset::default();
        for _ in 0..120 {
            o = co.update(&c, 1.0 / 60.0);
        }
        assert!(o.pitch.abs() < 1.0 && o.roll.abs() < 0.001);
    }

    #[test]
    fn strafing_rolls_camera() {
        let mut co = CameraOverhaul::new(Config::default());
        let mut c = ctx();
        c.velocity = [0.2, 0.0, 0.0];
        let mut o = CameraOffset::default();
        for _ in 0..120 {
            o = co.update(&c, 1.0 / 60.0);
        }
        assert!(o.roll.abs() > 0.1);
    }

    #[test]
    fn explosion_shakes_then_fades() {
        let mut co = CameraOverhaul::new(Config::default());
        let c = ctx();
        co.on_explosion([1.0, 0.0, 0.0]);
        let mut peak = 0.0f64;
        for _ in 0..30 {
            let o = co.update(&c, 1.0 / 60.0);
            peak = peak.max(o.pitch.abs() + o.yaw.abs());
        }
        assert!(peak > 0.0);
        for _ in 0..300 {
            co.update(&c, 1.0 / 60.0);
        }
        assert!(co.shakes.sample(co.time, [0.0; 3], &co.config) == [0.0, 0.0, 0.0]);
    }
}
