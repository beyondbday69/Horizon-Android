use crate::math::lerp;

#[derive(Clone, Debug)]
pub struct General {
    pub enabled: bool,
    pub enable_in_third_person: bool,
    pub context_transition_smoothing: f64,
    pub turning_roll_accumulation: f64,
    pub turning_roll_intensity: f64,
    pub turning_roll_smoothing: f64,
    pub camera_sway_intensity: f64,
    pub camera_sway_frequency: f64,
    pub camera_sway_fade_in_delay: f64,
    pub camera_sway_fade_in_length: f64,
    pub camera_sway_fade_out_length: f64,
    pub screen_shakes_max_intensity: f64,
    pub screen_shakes_max_frequency: f64,
    pub explosion_trauma: f64,
    pub thunder_trauma: f64,
    pub hand_swing_trauma: f64,
}

impl Default for General {
    fn default() -> Self {
        Self {
            enabled: true,
            enable_in_third_person: true,
            context_transition_smoothing: 0.1,
            turning_roll_accumulation: 1.0,
            turning_roll_intensity: 1.25,
            turning_roll_smoothing: 1.0,
            camera_sway_intensity: 0.60,
            camera_sway_frequency: 0.16,
            camera_sway_fade_in_delay: 0.15,
            camera_sway_fade_in_length: 5.0,
            camera_sway_fade_out_length: 0.75,
            screen_shakes_max_intensity: 2.5,
            screen_shakes_max_frequency: 6.0,
            explosion_trauma: 1.00,
            thunder_trauma: 0.05,
            hand_swing_trauma: 0.03,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Contextual {
    pub strafing_roll_factor: f64,
    pub forward_velocity_pitch_factor: f64,
    pub vertical_velocity_pitch_factor: f64,
    pub horizontal_velocity_smoothing_factor: f64,
    pub vertical_velocity_smoothing_factor: f64,
    pub camera_smoothing: f64,
}

impl Default for Contextual {
    fn default() -> Self {
        Self {
            strafing_roll_factor: 10.0,
            forward_velocity_pitch_factor: 7.0,
            vertical_velocity_pitch_factor: 2.5,
            horizontal_velocity_smoothing_factor: 1.0,
            vertical_velocity_smoothing_factor: 1.0,
            camera_smoothing: 0.0,
        }
    }
}

impl Contextual {
    pub fn lerp_to(&mut self, b: &Contextual, step: f64) {
        let a = *self;
        self.strafing_roll_factor = lerp(a.strafing_roll_factor, b.strafing_roll_factor, step);
        self.forward_velocity_pitch_factor =
            lerp(a.forward_velocity_pitch_factor, b.forward_velocity_pitch_factor, step);
        self.vertical_velocity_pitch_factor =
            lerp(a.vertical_velocity_pitch_factor, b.vertical_velocity_pitch_factor, step);
        self.horizontal_velocity_smoothing_factor = lerp(
            a.horizontal_velocity_smoothing_factor,
            b.horizontal_velocity_smoothing_factor,
            step,
        );
        self.vertical_velocity_smoothing_factor = lerp(
            a.vertical_velocity_smoothing_factor,
            b.vertical_velocity_smoothing_factor,
            step,
        );
        self.camera_smoothing = lerp(a.camera_smoothing, b.camera_smoothing, step);
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub general: General,
    pub walking: Contextual,
    pub sprinting: Contextual,
    pub swimming: Contextual,
    pub flying: Contextual,
    pub mounts: Contextual,
    pub vehicles: Contextual,
}

impl Default for Config {
    fn default() -> Self {
        let mut c = Config {
            general: General::default(),
            walking: Contextual::default(),
            sprinting: Contextual::default(),
            swimming: Contextual::default(),
            flying: Contextual::default(),
            mounts: Contextual::default(),
            vehicles: Contextual::default(),
        };
        c.sprinting.strafing_roll_factor = 12.5;
        c.sprinting.forward_velocity_pitch_factor = 9.5;
        c.flying.strafing_roll_factor *= -1.0;
        c.swimming.strafing_roll_factor *= -3.0;
        c.swimming.forward_velocity_pitch_factor *= 3.0;
        c.swimming.vertical_velocity_pitch_factor *= 3.0;
        c.mounts.strafing_roll_factor *= 2.0;
        c.mounts.forward_velocity_pitch_factor *= 0.5;
        c.vehicles.strafing_roll_factor *= 0.5;
        c.vehicles.forward_velocity_pitch_factor *= 0.5;
        c.vehicles.vertical_velocity_pitch_factor *= 2.0;
        c.sprinting.camera_smoothing = 0.6;
        c.swimming.camera_smoothing = 1.5;
        c.flying.camera_smoothing = 1.0;
        c.mounts.camera_smoothing = 1.0;
        c.vehicles.camera_smoothing = 0.0;
        c
    }
}
