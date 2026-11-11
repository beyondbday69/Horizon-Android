pub fn clamp(v: f64, min: f64, max: f64) -> f64 {
    if v < min { min } else if v > max { max } else { v }
}
pub fn clamp01(v: f64) -> f64 {
    clamp(v, 0.0, 1.0)
}
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * clamp01(t)
}
/// Frame-rate independent smoothing step.
pub fn damp_step(smoothing: f64, dt: f64) -> f64 {
    1.0 - (smoothing * smoothing).powf(dt)
}
pub fn damp(src: f64, dst: f64, smoothing: f64, dt: f64) -> f64 {
    lerp(src, dst, damp_step(smoothing, dt))
}
pub fn step_towards(current: f64, target: f64, step: f64) -> f64 {
    if current < target {
        (current + step).min(target)
    } else if current > target {
        (current - step).max(target)
    } else {
        current
    }
}
/// Wrap an angle difference into (-180, 180].
pub fn wrap_diff(d: f64) -> f64 {
    let mut d = d % 360.0;
    if d <= -180.0 {
        d += 360.0;
    } else if d > 180.0 {
        d -= 360.0;
    }
    d
}
pub fn unwrap_step(d: f64) -> f64 {
    wrap_diff(d)
}
