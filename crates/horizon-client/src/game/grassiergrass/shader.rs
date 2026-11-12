//! The blade shader, embedded because a bundle carries only `.rs` and `.toml` sources.

pub const BLADES_WGSL: &str = r#"
// Grassier Grass for Horizon: blade wind shader.
// Ported from Grassier Grass 1.4.5 grass_blades.vsh/.fsh (steps 1-9); the entity trail, snow,
// lightmap, fog and texture-shape passes are not here. Noise textures are replaced by
// procedural value noise so the mod ships without assets.

struct Globals {
    view_proj: mat4x4<f32>,
    // xyz = mesh anchor minus camera, so vertices stay camera-relative.
    offset: vec4<f32>,
    // xy = wind direction, z = flutter phase, w = wind intensity.
    wind: vec4<f32>,
    // x = baked blade length in blocks, y = width scale, z = brightness, w = noise scroll.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) tint: vec3<f32>,
    @location(1) @interpolate(flat) wave: f32,
    @location(2) height: f32,
};

struct FragmentIn {
    @location(0) tint: vec3<f32>,
    @location(1) @interpolate(flat) wave: f32,
    @location(2) height: f32,
};

const PI2: f32 = 6.2831853;
const MAX_ROTATION_RADIANS: f32 = 1.05;
const BEND_MIN_HEIGHT_NOISE: f32 = 0.0;
const BEND_FULL_HEIGHT_NOISE: f32 = 0.1;
const BEND_AMOUNT: f32 = 0.1;
const BEND_LOWER_CONTROL_HEIGHT: f32 = 0.45;
const BEND_UPPER_CONTROL_HEIGHT: f32 = 0.82;
const BEND_UPPER_CONTROL_FRACTION: f32 = 0.22;
const BEND_TIP_FRACTION: f32 = 1.0;
const WIND_CURVE_ROTATION: f32 = 0.4;
const WIND_CURVE_PEAK_ROTATION: f32 = 0.52;
const WIND_CURVE_LOWER_ROTATION_FRACTION: f32 = 0.30;
const WIND_CURVE_UPPER_ROTATION_FRACTION: f32 = 0.72;
const REST_CURVE_ROTATION_MIN: f32 = -0.52;
const REST_CURVE_ROTATION_MAX: f32 = 0.52;
const REST_CURVE_FADE_END: f32 = 0.35;
const HEIGHT_NOISE_CONTRAST: f32 = 1.35;
const HEIGHT_NOISE_BIAS: f32 = 0.5;
const HEIGHT_NOISE_SHORT_MULTIPLIER: f32 = 0.1;
const HEIGHT_NOISE_TALL_MULTIPLIER: f32 = 1.28;
const REGIONAL_LENGTH_FRACTION: f32 = 0.59;
const DEFAULT_ANIMATION_BLADE_LENGTH: f32 = 0.5075;
const TALL_PLANT_LENGTH_MULTIPLIER: f32 = 3.0;
const SEGMENT_MAX_COUNT: f32 = 4.0;
const SEGMENT_MIN_BAND_HEIGHT: f32 = 0.14;
const GEO_MAX_SEGMENTS: f32 = 5.0;
const GEO_MIN_QUAD_HEIGHT: f32 = 0.115;
const LEAN_RESPONSE_MIN: f32 = 0.3;
const LEAN_RESPONSE_MAX: f32 = 0.58;
const FLUTTER_SPEED_STEPS: f32 = 64.0;
const FLUTTER_SPEED_MIN_STEP: f32 = 58.0;
const FLUTTER_SPEED_MAX_STEP: f32 = 70.0;
const MIN_BLADE_LENGTH: f32 = 0.02;
const PER_BLADE_HEIGHT_MIN: f32 = 0.88;
const PER_BLADE_HEIGHT_MAX: f32 = 1.14;
const PER_BLADE_WIDTH_COMPENSATION: f32 = 0.12;
const CLUMP_COORD_SCALE: f32 = 2.0;
const CLUMP_WIDTH_MIN: f32 = 0.92;
const CLUMP_WIDTH_MAX: f32 = 1.08;
const CLUMP_LEAN_COHESION: f32 = 0.42;
const CLUMP_REST_COHESION: f32 = 0.55;
const CAMERA_FACE_STRENGTH: f32 = 0.38;
const CAMERA_FACE_START_DISTANCE: f32 = 2.0;
const CAMERA_FACE_END_DISTANCE: f32 = 42.0;
const CAMERA_FACE_MIN_HEIGHT: f32 = 0.08;
const BAKED_BASE_WIDTH: f32 = 0.040;
const BAKED_MIDDLE_WIDTH: f32 = 0.050;
const BAKED_TIP_WIDTH: f32 = 0.013;
const BAKED_SPLIT_FRACTION: f32 = 2.0 / 3.0;
const TAPERED_WIDTH_SCALE: f32 = 0.8947368;
// The original drains blades through its LOD rings and fog; the slice fades them at the disk edge.
const FADE_START: f32 = 15.0;
const FADE_END: f32 = 19.0;

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(cell);
    let b = hash21(cell + vec2<f32>(1.0, 0.0));
    let c = hash21(cell + vec2<f32>(0.0, 1.0));
    let d = hash21(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn stationary_height_noise(coord: vec2<f32>) -> f32 {
    var noise = value_noise(coord);
    noise = clamp((noise - HEIGHT_NOISE_BIAS) * HEIGHT_NOISE_CONTRAST + HEIGHT_NOISE_BIAS, 0.0, 1.0);
    return smoothstep(0.0, 1.0, noise);
}

fn blade_width(h: f32, tip_width: f32, regional_width: f32) -> f32 {
    let tapered = globals.params.y * TAPERED_WIDTH_SCALE * regional_width;
    if h <= BAKED_SPLIT_FRACTION {
        return mix(BAKED_BASE_WIDTH, BAKED_MIDDLE_WIDTH, h / BAKED_SPLIT_FRACTION) * tapered;
    }
    return mix(BAKED_MIDDLE_WIDTH, tip_width, (h - BAKED_SPLIT_FRACTION) / (1.0 - BAKED_SPLIT_FRACTION)) * tapered;
}

fn cubic_bezier(p0: vec3<f32>, p1: vec3<f32>, p2: vec3<f32>, p3: vec3<f32>, t: f32) -> vec3<f32> {
    let inv = 1.0 - t;
    return inv * inv * inv * p0
        + 3.0 * inv * inv * t * p1
        + 3.0 * inv * t * t * p2
        + t * t * t * p3;
}

fn rotated_spine_point(
    base: vec3<f32>,
    dir: vec2<f32>,
    visual_length: f32,
    animation_length: f32,
    height: f32,
    rotation: f32,
) -> vec3<f32> {
    var point = base;
    let sweep = sin(rotation) * animation_length * height;
    point.x = point.x + dir.x * sweep;
    point.z = point.z + dir.y * sweep;
    point.y = point.y + cos(rotation) * visual_length * height;
    return point;
}

fn wind_curve_response(rotation: f32) -> f32 {
    let rise = smoothstep(0.0, WIND_CURVE_PEAK_ROTATION, rotation);
    let fall = 1.0 - smoothstep(WIND_CURVE_PEAK_ROTATION, MAX_ROTATION_RADIANS, rotation);
    return rise * fall;
}

fn camera_side(center: vec3<f32>, original: vec2<f32>, height: f32) -> vec2<f32> {
    let to_camera = -center.xz;
    let dist = length(to_camera);
    if dist < 0.001 || height < CAMERA_FACE_MIN_HEIGHT {
        return original;
    }
    let dir = to_camera / dist;
    var side = vec2<f32>(-dir.y, dir.x);
    if dot(side, original) < 0.0 {
        side = -side;
    }
    let edge_on = 1.0 - abs(dot(original, side));
    let fade = 1.0 - smoothstep(CAMERA_FACE_START_DISTANCE, CAMERA_FACE_END_DISTANCE, dist);
    return normalize(mix(original, side, CAMERA_FACE_STRENGTH * edge_on * fade));
}

@vertex
fn vs_blade(input: VertexIn) -> VertexOut {
    // STEP 1: wind inputs.
    let wind_amount = clamp(globals.wind.w, 0.0, 1.0);
    let wind_dir = normalize(globals.wind.xy);
    let time = globals.wind.z;

    // STEP 2: decode the baked vertex.
    let packed = floor(input.color.a * 255.0 + 0.5);
    let right = step(128.0, packed);
    let side_sign = mix(-1.0, 1.0, right);
    let data = packed - right * 128.0;
    let height_class = floor(data / 32.0);
    let bucket = data - height_class * 32.0;
    let side_angle = bucket / 31.0 * PI2;
    let original_side = vec2<f32>(cos(side_angle), sin(side_angle));
    let noise_coord = input.normal.xy * 0.5 + 0.5;
    let height = clamp(input.normal.z, 0.0, 1.0);

    // STEP 3: regional height/width field. The clump Voronoi texture becomes value noise.
    let height_noise = stationary_height_noise(noise_coord);
    let clump_coord = noise_coord * CLUMP_COORD_SCALE;
    let clump = vec3<f32>(
        value_noise(clump_coord + vec2<f32>(11.3, 5.1)),
        value_noise(clump_coord + vec2<f32>(41.7, 23.9)),
        value_noise(clump_coord + vec2<f32>(73.1, 91.7)),
    );
    let clump_influence = 0.6 + 0.4 * (1.0 - smoothstep(0.25, 0.85, clump.z));
    let clump_width = mix(1.0, mix(CLUMP_WIDTH_MIN, CLUMP_WIDTH_MAX, smoothstep(0.0, 1.0, clump.y)), clump_influence);
    let height_multiplier = mix(HEIGHT_NOISE_SHORT_MULTIPLIER, HEIGHT_NOISE_TALL_MULTIPLIER, height_noise);
    let class_multiplier = select(1.0, TALL_PLANT_LENGTH_MULTIPLIER, height_class > 1.5);
    let base_length = globals.params.x * class_multiplier;

    let blade_random = hash21(noise_coord + vec2<f32>(bucket * 0.071, height_class * 13.37));
    let shaped_random = smoothstep(0.0, 1.0, blade_random);
    let per_height = mix(PER_BLADE_HEIGHT_MIN, PER_BLADE_HEIGHT_MAX, shaped_random);
    let per_width = mix(1.0 + PER_BLADE_WIDTH_COMPENSATION, 1.0 - PER_BLADE_WIDTH_COMPENSATION, shaped_random);
    let regional_width = clump_width * per_width;

    let regional_extra = base_length * (height_multiplier - 1.0) * REGIONAL_LENGTH_FRACTION;
    let per_extra = base_length * (per_height - 1.0);
    var blade_length = max(base_length + (regional_extra + per_extra) * 1.35, MIN_BLADE_LENGTH);
    let extra_length = blade_length - base_length;

    let geo_segments = clamp(floor(blade_length / GEO_MIN_QUAD_HEIGHT), 1.0, GEO_MAX_SEGMENTS);
    let geo_height = floor(height * geo_segments + 0.5) / geo_segments;

    var pos = input.pos + globals.offset.xyz;
    pos.y = pos.y + extra_length * height;

    // STEP 4: recover the baked centerline.
    let baked_width = blade_width(height, BAKED_TIP_WIDTH, 1.0);
    let drawn_width = BAKED_BASE_WIDTH * globals.params.y * TAPERED_WIDTH_SCALE;
    let final_width = drawn_width * regional_width;
    var source_center = pos;
    source_center.x = source_center.x - original_side.x * side_sign * baked_width;
    source_center.z = source_center.z - original_side.y * side_sign * baked_width;

    // STEP 5: scrolling wind field.
    let noise_uv = noise_coord * 0.8 - wind_dir * globals.params.w;
    let wind_noise = value_noise(noise_uv);
    let shaped_wave = clamp((wind_noise - 0.5) + 0.5, 0.0, 1.0);
    let color_wave = smoothstep(0.0, 1.0, shaped_wave);
    let local_wind = mix(wind_noise, 1.0, clamp(globals.wind.w / 5.0, 0.0, 1.0));
    let lean_random = mix(hash21(noise_coord + vec2<f32>(43.17, 11.91)), clump.y, CLUMP_LEAN_COHESION * clump_influence);
    let lean_response = mix(LEAN_RESPONSE_MIN, LEAN_RESPONSE_MAX, lean_random);
    let resisted = local_wind * wind_amount * lean_response;
    let wind_rotation = clamp(resisted, 0.0, 1.0) * MAX_ROTATION_RADIANS;
    let rotation = wind_rotation;

    // Rest posture: no trails, and the clump inward lean needs the baked distance field.
    let rest_random = mix(hash21(noise_coord + vec2<f32>(7.13, 91.77)), clump.x, CLUMP_REST_COHESION * clump_influence);
    let wind_force = clamp(resisted, 0.0, 1.0);
    let rest_fade = 1.0 - smoothstep(0.0, REST_CURVE_FADE_END, wind_force);
    let bend_eligibility = max(smoothstep(BEND_MIN_HEIGHT_NOISE, BEND_FULL_HEIGHT_NOISE, height_noise), select(0.0, 1.0, height_class > 0.5));

    // STEP 6: Bezier spine.
    let animation_length = max(blade_length, DEFAULT_ANIMATION_BLADE_LENGTH * class_multiplier);
    var base_point = source_center;
    base_point.y -= blade_length * height;

    var rest_curve = mix(REST_CURVE_ROTATION_MIN, REST_CURVE_ROTATION_MAX, rest_random) * rest_fade * (bend_eligibility + 0.25);
    let wind_curve = WIND_CURVE_ROTATION * wind_curve_response(rotation) * bend_eligibility;
    let tip_rotation = rotation + wind_curve + rest_curve;

    var tip = rotated_spine_point(base_point, wind_dir, blade_length, animation_length, 1.0, tip_rotation);
    var lower = rotated_spine_point(
        base_point,
        wind_dir,
        blade_length,
        animation_length,
        BEND_LOWER_CONTROL_HEIGHT,
        rotation + wind_curve * WIND_CURVE_LOWER_ROTATION_FRACTION + rest_curve * WIND_CURVE_LOWER_ROTATION_FRACTION,
    );
    var upper = rotated_spine_point(
        base_point,
        wind_dir,
        blade_length,
        animation_length,
        BEND_UPPER_CONTROL_HEIGHT,
        rotation + wind_curve * WIND_CURVE_UPPER_ROTATION_FRACTION + rest_curve * WIND_CURVE_UPPER_ROTATION_FRACTION,
    );

    let calmness = 1.0 - smoothstep(0.25, 0.85, wind_force);
    let bend_phase = dot(noise_coord, vec2<f32>(97.0, 151.0));
    let speed_random = hash21(noise_coord + vec2<f32>(bucket * 1.913 + 73.17, height_class * 11.71 + 29.43));
    let speed_step = floor(mix(FLUTTER_SPEED_MIN_STEP, FLUTTER_SPEED_MAX_STEP + 1.0, speed_random));
    let bend_wave = sin(time * (speed_step / FLUTTER_SPEED_STEPS) + bend_phase);
    let bend = bend_wave * BEND_AMOUNT * animation_length * bend_eligibility * calmness;
    let cross_lean = vec2<f32>(-wind_dir.y, wind_dir.x);
    let bend_offset = cross_lean * bend;
    upper.x = upper.x + bend_offset.x * BEND_UPPER_CONTROL_FRACTION;
    upper.z = upper.z + bend_offset.y * BEND_UPPER_CONTROL_FRACTION;
    tip.x = tip.x + bend_offset.x * BEND_TIP_FRACTION;
    tip.z = tip.z + bend_offset.y * BEND_TIP_FRACTION;

    let blade_center = cubic_bezier(base_point, lower, upper, tip, geo_height);

    // STEP 7: restore the final width around the curved spine.
    let visual_side = camera_side(source_center, original_side, height);
    var curved = blade_center;
    curved.x = curved.x + visual_side.x * (side_sign * final_width);
    curved.z = curved.z + visual_side.y * (side_sign * final_width);

    // STEP 8: fade into the disk edge instead of the original's LOD rings.
    let dist = length(curved.xz);
    let fade = 1.0 - smoothstep(FADE_START, FADE_END, dist);
    let final_pos = base_point + (curved - base_point) * fade;

    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(final_pos, 1.0);
    out.tint = input.color.rgb * globals.params.z;
    out.wave = color_wave;
    out.height = geo_height;
    return out;
}

@fragment
fn fs_blade(input: FragmentIn) -> @location(0) vec4<f32> {
    let shaped = pow(clamp(input.height, 0.0, 1.0), 1.9);
    let gradient = mix(0.8 * 0.75, 1.0, shaped);
    let wave = mix(0.85, 1.0, clamp(input.wave, 0.0, 1.0));
    let color = clamp(input.tint * gradient * wave, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(color, 1.0);
}
"#;
