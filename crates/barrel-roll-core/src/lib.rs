//! An engine-independent port of the client-side flight rotation of **Do a
//! Barrel Roll** by enjarai, for Horizon's built-in `barrel_roll` feature.
//!
//! Based on Do a Barrel Roll by enjarai <https://github.com/enjarai/do-a-barrel-roll>,
//! which is based on Cool Elytra Roll by Jorbon. GPL-3.0 (see `LICENSE.md`).
//!
//! Ported: the free orientation while gliding — the look input rolls and pitches,
//! the strafe input yaws — with banking, smoothing and automatic righting, and the
//! way the player's real yaw and pitch are derived from the forward vector.
//!
//! Not ported: thrusting, the momentum-based mouse, the config screen, the
//! horizon widget and every server-side or sync feature.
//!
//! ```no_run
//! # let (mut player_yaw, mut player_pitch) = (0.0_f64, 0.0_f64);
//! # let (dy, dx, strafe, delta_seconds) = (0.0_f64, 0.0, 0.0, 1.0 / 60.0);
//! let mut roll = barrel_roll_core::BarrelRoll::new(Default::default());
//! roll.set_orientation(player_yaw, player_pitch); // when gliding starts
//! let o = roll.update(barrel_roll_core::Input { pitch: dy, roll: dx, yaw: strafe }, delta_seconds);
//! player_yaw = o.yaw;   // the engine flies where you look
//! player_pitch = o.pitch;
//! let camera_roll = -o.roll; // the view banks (CameraMixin: tempRoll = -entity roll)
//! ```

#![forbid(unsafe_code)]

/// Degrees to radians, and back (`MagicNumbers`).
pub const TORAD: f64 = std::f64::consts::PI / 180.0;
/// Radians to degrees.
pub const TODEG: f64 = 180.0 / std::f64::consts::PI;

/// One input unit moves the view this far, matching `Entity.turn`'s 0.15 degrees
/// per cursor unit (`ClientPlayerEntityMixin.doABarrelRoll$changeElytraLook`).
const UNIT: f64 = 0.15;
/// `RotationModifiers.ROLL_REORIENT_CUTOFF` = `sqrt(10 / 3)`.
const ROLL_REORIENT_CUTOFF: f64 = 1.825_741_858_350_553_8;

/// Per-axis sensitivity (`nl.enjarai.doabarrelroll.config.Sensitivity`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sensitivity {
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

impl Default for Sensitivity {
    fn default() -> Self {
        // Sensitivity's field defaults.
        Self {
            pitch: 1.0,
            yaw: 0.4,
            roll: 1.0,
        }
    }
}

/// The mod's settings (`ModConfig`), minus everything not ported.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `controls.invert_pitch`.
    pub invert_pitch: bool,
    /// `sensitivity.desktop`.
    pub sensitivity: Sensitivity,
    /// `sensitivity.smoothing`.
    pub smoothing: Sensitivity,
    pub smoothing_enabled: bool,
    /// `banking.enable_banking`.
    pub banking: bool,
    /// `banking.banking_strength`.
    pub banking_strength: f64,
    /// `banking.automatic_righting`.
    pub automatic_righting: bool,
    /// `banking.righting_strength`.
    pub righting_strength: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            invert_pitch: false,
            sensitivity: Sensitivity::default(),
            smoothing: Sensitivity {
                pitch: 1.0,
                yaw: 0.4,
                roll: 1.0,
            },
            smoothing_enabled: true,
            banking: true,
            banking_strength: 20.0,
            automatic_righting: false,
            righting_strength: 50.0,
        }
    }
}

/// A camera or player orientation in degrees, and (internally) a rotation delta.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Orientation {
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
}

impl Orientation {
    /// Any field that came out NaN becomes 0 (`RotationModifiers.fixNaN`).
    fn fixed(self) -> Self {
        Self {
            yaw: if self.yaw.is_finite() { self.yaw } else { 0.0 },
            pitch: if self.pitch.is_finite() { self.pitch } else { 0.0 },
            roll: if self.roll.is_finite() { self.roll } else { 0.0 },
        }
    }

    /// `RotationInstantImpl.addAbsolute`: an upright (pitch, yaw) nudge turned into
    /// the frame the camera is currently rolled to. `roll_rad` is in radians.
    fn add_absolute(self, x: f64, y: f64, roll_rad: f64) -> Self {
        let (sin, cos) = roll_rad.sin_cos();
        Self {
            pitch: self.pitch - y * cos - x * sin,
            yaw: self.yaw - y * sin + x * cos,
            roll: self.roll,
        }
    }
}

/// One frame of input.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Look input this frame along -Y (mouse or touch drag, cursor units;
    /// `ClientPlayerEntityMixin.changeElytraLook`'s pitch).
    pub pitch: f64,
    /// Look input this frame along +X, turned into roll (`ModConfig.configureRotation`
    /// swaps yaw and roll by default: mouse X = roll).
    pub roll: f64,
    /// Strafe input, right positive, in [-1, 1], turned into yaw.
    pub yaw: f64,
}

/// A first-order low-pass: `amount` in [0, 1] moves the output this fraction of
/// the way to `value`. Stands in for `net.minecraft.util.math.Smoother`.
#[derive(Clone, Copy, Debug, Default)]
struct Smoother {
    current: f64,
}

impl Smoother {
    fn smooth(&mut self, value: f64, amount: f64) -> f64 {
        if !amount.is_finite() {
            return value;
        }
        self.current += (value - self.current) * amount.clamp(0.0, 1.0);
        self.current
    }

    fn clear(&mut self) {
        self.current = 0.0;
    }
}

/// `RotationModifiers.smoothing`: pass the raw value when smoothing is off or the
/// axis has no smoothness, otherwise run it through the smoother with
/// `1 / smoothness * dt` as the amount.
fn smooth(enabled: bool, smoother: &mut Smoother, value: f64, smoothness: f64, dt: f64) -> f64 {
    if !enabled || smoothness <= 0.0 {
        return value;
    }
    smoother.smooth(value, dt / smoothness)
}

/// A small vector, only for the forward/left rotation (`org.joml.Vector3d`).
#[derive(Clone, Copy, Debug)]
struct Vec3 {
    x: f64,
    y: f64,
    z: f64,
}

impl Vec3 {
    fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    fn scale(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }

    fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    fn normalized(self) -> Self {
        let length = self.dot(self).sqrt();
        if length > 0.0 { self.scale(1.0 / length) } else { self }
    }

    fn rotate_x(self, a: f64) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x, self.y * c - self.z * s, self.y * s + self.z * c)
    }

    fn rotate_y(self, a: f64) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }

    fn rotate_z(self, a: f64) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c, self.z)
    }

    /// `Vector3d.rotateAxis(angle, axis)`, Rodrigues' formula.
    fn rotate_axis(self, angle: f64, axis: Self) -> Self {
        let (s, c) = angle.sin_cos();
        let k = axis.normalized();
        self.scale(c)
            .add(k.cross(self).scale(s))
            .add(k.scale(k.dot(self) * (1.0 - c)))
    }
}

/// `Entity.getRotationVector(pitch, yaw)`: the unit direction the player looks.
fn forward_vector(pitch: f64, yaw: f64) -> Vec3 {
    let (p, y) = (pitch * TORAD, yaw * TORAD);
    Vec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos())
}

/// The camera's "left" for a rolled orientation (`ClientPlayerEntityMixin`).
fn left_vector(pitch: f64, yaw: f64, roll: f64) -> Vec3 {
    Vec3::new(1.0, 0.0, 0.0)
        .rotate_z(-roll * TORAD)
        .rotate_x(-(pitch * TORAD))
        .rotate_y(-(yaw + 180.0) * TORAD)
}

/// The gliding orientation and everything it needs to advance a frame.
#[derive(Clone, Debug)]
pub struct BarrelRoll {
    config: Config,
    yaw: f64,
    pitch: f64,
    roll: f64,
    pitch_smoother: Smoother,
    yaw_smoother: Smoother,
    roll_smoother: Smoother,
}

impl BarrelRoll {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            pitch_smoother: Smoother::default(),
            yaw_smoother: Smoother::default(),
            roll_smoother: Smoother::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn orientation(&self) -> Orientation {
        Orientation {
            yaw: self.yaw,
            pitch: self.pitch,
            roll: self.roll,
        }
    }

    /// Start from the engine's angles when gliding begins, upright.
    pub fn set_orientation(&mut self, yaw: f64, pitch: f64) {
        self.yaw = yaw;
        self.pitch = pitch;
        self.roll = 0.0;
        self.clear_smoothers();
    }

    pub fn reset(&mut self) {
        self.set_orientation(0.0, 0.0);
    }

    fn clear_smoothers(&mut self) {
        self.pitch_smoother.clear();
        self.yaw_smoother.clear();
        self.roll_smoother.clear();
    }

    /// Advance one frame: apply the input, then banking and righting, and return
    /// the new absolute orientation in degrees. `dt` is the render delta, in
    /// seconds.
    pub fn update(&mut self, input: Input, dt: f64) -> Orientation {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        let sensitivity = self.config.sensitivity;
        let mut delta = Orientation {
            pitch: input.pitch * sensitivity.pitch,
            yaw: input.yaw * sensitivity.yaw,
            roll: input.roll * sensitivity.roll,
        }
        .fixed();
        if self.config.invert_pitch {
            delta.pitch = -delta.pitch;
        }

        // LATE_CAMERA_MODIFIERS, in the original's order: smoothing, banking, righting.
        let smoothing = self.config.smoothing;
        let enabled = self.config.smoothing_enabled;
        delta.pitch = smooth(enabled, &mut self.pitch_smoother, delta.pitch, smoothing.pitch, dt);
        delta.yaw = smooth(enabled, &mut self.yaw_smoother, delta.yaw, smoothing.yaw, dt);
        delta.roll = smooth(enabled, &mut self.roll_smoother, delta.roll, smoothing.roll, dt);

        if self.config.banking {
            // The default banking formulas, with the current roll and pitch.
            let roll = self.roll * TORAD;
            let pitch = self.pitch * TORAD;
            let x = roll.sin() * pitch.cos() * 10.0 * self.config.banking_strength;
            let y = (-1.0 + roll.cos()) * pitch.cos() * 10.0 * self.config.banking_strength;
            delta = delta.add_absolute(x * dt, y * dt, roll).fixed();
        }

        if self.config.automatic_righting {
            let roll = self.roll * TORAD;
            let righting = if -ROLL_REORIENT_CUTOFF < roll && roll < ROLL_REORIENT_CUTOFF {
                -roll.powi(3) / 3.0 + roll
            } else {
                0.0
            };
            delta.roll += -righting * 10.0 * self.config.righting_strength * dt;
        }

        self.apply(delta);
        self.orientation()
    }

    /// Blend the roll to upright after flight ends, without a snap, and return it
    /// in degrees. `CameraMixin` halves the camera roll every eye-height update;
    /// call this each frame until it rests, then [`BarrelRoll::reset`].
    pub fn settle(&mut self, dt: f64) -> f64 {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        // Half per frame at 60 fps, made independent of the frame rate.
        self.roll *= 0.5_f64.powf(dt * 60.0);
        if self.roll.abs() < 0.01 {
            self.roll = 0.0;
        }
        self.roll
    }

    /// `ClientPlayerEntityMixin.doABarrelRoll$changeElytraLook`: turn the forward
    /// and left vectors by the delta, then read the new angles back out.
    fn apply(&mut self, delta: Orientation) {
        let (current_yaw, current_pitch, current_roll) = (self.yaw, self.pitch, self.roll);
        let mut facing = forward_vector(current_pitch, current_yaw);
        let mut left = left_vector(current_pitch, current_yaw, current_roll);

        // Pitch about the left axis.
        facing = facing.rotate_axis(-UNIT * delta.pitch * TORAD, left);
        // Yaw about up = facing x left.
        let up = facing.cross(left);
        facing = facing.rotate_axis(UNIT * delta.yaw * TORAD, up);
        left = left.rotate_axis(UNIT * delta.yaw * TORAD, up);
        // Roll about the forward axis.
        left = left.rotate_axis(UNIT * delta.roll * TORAD, facing);

        let new_pitch = -facing.y.asin() * TODEG;
        let new_yaw = -facing.x.atan2(facing.z) * TODEG;
        let normal_left = Vec3::new(1.0, 0.0, 0.0).rotate_y(-(new_yaw + 180.0) * TORAD);
        let new_roll =
            -left.cross(normal_left).dot(facing).atan2(left.dot(normal_left)) * TODEG;

        let next = Orientation {
            yaw: new_yaw,
            pitch: new_pitch,
            roll: new_roll,
        }
        .fixed();
        self.yaw = next.yaw;
        self.pitch = next.pitch;
        self.roll = next.roll;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    /// Only the raw input-to-rotation maths: no smoothing, no banking.
    fn plain() -> Config {
        Config {
            smoothing_enabled: false,
            banking: false,
            automatic_righting: false,
            ..Config::default()
        }
    }

    #[test]
    fn the_identity_orientation_stays_put() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let o = roll.update(Input::default(), DT);
        assert_eq!(
            o,
            Orientation {
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0
            }
        );
    }

    #[test]
    fn a_pure_roll_only_rolls() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let o = roll.update(
            Input {
                roll: 10.0,
                ..Input::default()
            },
            DT,
        );
        // 10 input units at 0.15 degrees each.
        assert!((o.roll.abs() - 1.5).abs() < 1e-6, "{o:?}");
        assert!(o.pitch.abs() < 1e-9, "{o:?}");
        assert!(o.yaw.abs() < 1e-9, "{o:?}");
    }

    #[test]
    fn a_pure_pitch_only_pitches() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let o = roll.update(
            Input {
                pitch: 10.0,
                ..Input::default()
            },
            DT,
        );
        assert!((o.pitch.abs() - 1.5).abs() < 1e-6, "{o:?}");
        assert!(o.yaw.abs() < 1e-9, "{o:?}");
        assert!(o.roll.abs() < 1e-9, "{o:?}");
    }

    #[test]
    fn yaw_turns_while_rolled() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let banked = roll.update(
            Input {
                roll: 60.0,
                ..Input::default()
            },
            DT,
        );
        assert!(banked.roll.abs() > 5.0, "{banked:?}");
        let o = roll.update(
            Input {
                yaw: 1.0,
                ..Input::default()
            },
            DT,
        );
        assert!(o.yaw.is_finite() && o.pitch.is_finite() && o.roll.is_finite());
        assert!((o.yaw - banked.yaw).abs() > 1e-4, "{o:?}");
    }

    #[test]
    fn a_hundred_thousand_small_updates_stay_bounded() {
        let mut roll = BarrelRoll::new(Config::default());
        roll.set_orientation(0.0, 0.0);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as f64 / f64::from(1u32 << 31) - 0.5
        };
        for _ in 0..100_000 {
            let input = Input {
                pitch: rnd() * 0.2,
                roll: rnd() * 0.2,
                yaw: rnd() * 0.1,
            };
            let o = roll.update(input, DT);
            assert!(
                o.yaw.is_finite() && o.pitch.is_finite() && o.roll.is_finite(),
                "{o:?}"
            );
            assert!(o.pitch.abs() <= 90.0 + 1e-6, "{o:?}");
            assert!(o.roll.abs() <= 180.0 + 1e-6, "{o:?}");
        }

        // A still glide must not drift: banking off, no input, no movement.
        roll.config_mut().banking = false;
        roll.config_mut().smoothing_enabled = false;
        let before = roll.orientation();
        for _ in 0..600 {
            roll.update(Input::default(), DT);
        }
        let after = roll.orientation();
        assert!(
            (after.yaw - before.yaw).abs() < 0.05
                && (after.pitch - before.pitch).abs() < 0.05
                && (after.roll - before.roll).abs() < 0.05,
            "{before:?} -> {after:?}"
        );
    }

    #[test]
    fn banking_turns_the_way_you_lean() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        // Lean right... then let banking act with no further input.
        let banked = roll.update(
            Input {
                roll: 60.0,
                ..Input::default()
            },
            DT,
        );
        assert!(banked.roll > 5.0, "{banked:?}");
        roll.config_mut().banking = true;
        let before = roll.orientation();
        let after = roll.update(Input::default(), DT);
        assert!(after.yaw > before.yaw, "{before:?} -> {after:?}");

        // And the mirror image: leaning the other way turns the other way.
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let banked = roll.update(
            Input {
                roll: -60.0,
                ..Input::default()
            },
            DT,
        );
        assert!(banked.roll < -5.0, "{banked:?}");
        roll.config_mut().banking = true;
        let before = roll.orientation();
        let after = roll.update(Input::default(), DT);
        assert!(after.yaw < before.yaw, "{before:?} -> {after:?}");
    }

    #[test]
    fn settling_blends_the_roll_back_upright() {
        let mut roll = BarrelRoll::new(plain());
        roll.set_orientation(0.0, 0.0);
        let banked = roll.update(
            Input {
                roll: 60.0,
                ..Input::default()
            },
            DT,
        );
        assert!(banked.roll.abs() > 5.0, "{banked:?}");
        let mut previous = banked.roll.abs();
        for _ in 0..120 {
            let r = roll.settle(DT).abs();
            assert!(r <= previous + 1e-9, "{r} > {previous}");
            previous = r;
        }
        assert!(previous < 0.1, "{previous}");
    }
}
