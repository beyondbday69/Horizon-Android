use crate::config::Config;
use crate::noise::simplex2;

const MAX_SHAKES: usize = 64;

#[derive(Clone, Debug)]
pub struct Shake {
    pub trauma: f32,
    pub radius: f32,
    pub frequency: f32,
    pub length_secs: f32,
    /// None = positionless (always full strength).
    pub position: Option<[f64; 3]>,
    start_time: f64,
}

impl Shake {
    pub fn new(now: f64) -> Self {
        Self { trauma: 0.5, radius: 10.0, frequency: 1.0, length_secs: 2.0, position: None, start_time: now }
    }
}

#[derive(Default)]
pub struct ScreenShakes {
    shakes: Vec<Shake>,
}

impl ScreenShakes {
    pub fn add(&mut self, shake: Shake) {
        if self.shakes.len() < MAX_SHAKES {
            self.shakes.push(shake);
        }
    }

    /// Explosion at a world position (radius 32 on modern versions, 2s).
    pub fn explosion(&mut self, now: f64, pos: [f64; 3], cfg: &Config) {
        let mut s = Shake::new(now);
        s.position = Some(pos);
        s.radius = 32.0;
        s.trauma = cfg.general.explosion_trauma as f32;
        s.length_secs = 2.0;
        self.add(s);
    }

    /// Lightning: a close hit plus a long, low rumble.
    pub fn lightning(&mut self, now: f64, pos: [f64; 3], cfg: &Config) {
        let mut a = Shake::new(now);
        a.position = Some(pos);
        a.radius = 16.0;
        a.trauma = cfg.general.explosion_trauma as f32;
        a.length_secs = 3.0;
        self.add(a);

        let mut b = Shake::new(now);
        b.position = Some(pos);
        b.radius = 192.0;
        b.trauma = cfg.general.thunder_trauma as f32;
        b.frequency = 0.5;
        b.length_secs = 7.0;
        self.add(b);
    }

    /// Small shake on hand swing. Replaces any previous swing shake.
    pub fn hand_swing(&mut self, now: f64, cfg: &Config) {
        let mut s = Shake::new(now);
        s.trauma = cfg.general.hand_swing_trauma as f32;
        s.frequency = 0.5;
        s.length_secs = 0.5;
        self.add(s);
    }

    /// Returns [pitch, yaw, roll] shake offsets in degrees.
    pub fn sample(&mut self, now: f64, cam_pos: [f64; 3], cfg: &Config) -> [f64; 3] {
        let sample_base = (now * cfg.general.screen_shakes_max_frequency) as f32;
        let mut total = 0.0f32;
        let mut noise = [0.0f32; 3];

        self.shakes.retain(|ss| {
            ss.length_secs <= 0.0 || ((now - ss.start_time) / ss.length_secs as f64) < 1.0
        });

        for ss in &self.shakes {
            let progress = if ss.length_secs > 0.0 {
                (((now - ss.start_time) / ss.length_secs as f64).clamp(0.0, 1.0)) as f32
            } else {
                1.0
            };
            let decay = 1.0 - progress;
            let mut intensity = ss.trauma.clamp(0.0, 1.0) * decay * decay;

            if let Some(p) = ss.position {
                let d = ((cam_pos[0] - p[0]).powi(2)
                    + (cam_pos[1] - p[1]).powi(2)
                    + (cam_pos[2] - p[2]).powi(2))
                .sqrt() as f32;
                let f = 1.0 - (d / ss.radius).min(1.0);
                intensity *= f * f;
            }
            if intensity <= 0.0 || !intensity.is_finite() {
                continue;
            }
            let step = sample_base * ss.frequency;
            noise[0] += simplex2(step, -69.0) * intensity;
            noise[1] += simplex2(step, -420.0) * intensity;
            noise[2] += simplex2(step, -1337.0) * intensity;
            total += intensity;
        }
        if total > 1.0 {
            for n in &mut noise {
                *n /= total;
            }
        }
        let k = cfg.general.screen_shakes_max_intensity;
        [noise[0] as f64 * k, noise[1] as f64 * k, noise[2] as f64 * k]
    }
}
