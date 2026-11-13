// Hold My Items (HMI 5.1.1) runtime animation engine for Horizon.
// Self-contained module mounted at `crate::game::holdmyitems` and testable standalone.

use std::collections::HashMap;

pub const ID: &str = "holdmyitems";
pub const TITLE: &str = "Hold My Items 5.1.1";
pub const DESCRIPTION: &str =
    "First-person hand & item physics, 3D block/item poses, per-quad articulation, inspection, and hand lighting.";
pub const CONFIG_FILE: &str = "config.json";
pub const INSPECT_KEY: &str = "inspect";
pub const INSPECT_KEY_LABEL: &str = "Inspect Held Item";
pub const INSPECT_KEY_DEFAULT: &str = "key.keyboard.c";

pub const COMMAND_HMI: &str = "hmi";
pub const COMMAND_HOLDMYITEMS: &str = "holdmyitems";

/// Roll (degrees, around Z) applied to a trident while it is raised for a throw.
/// The original Hold My Items script uses `170.0`, which assumes the vanilla Java in-hand model
/// orientation. Horizon's trident model appears to be oriented 180 degrees opposite (head shown
/// upside down when drawn), so `-10.0` (= 170 - 180) compensates. This is an in-game-unverified
/// fix: set back to `170.0` to restore the original behavior if it looks worse.
const TRIDENT_DRAW_ROLL_DEG: f32 = -10.0;

/// Shift applied to held beds, in the item base frame *before* the bed's own rotations, so the
/// bed model does not clip into the arm. X is multiplied by the hand sign `l` (negative = away
/// from the arm), Z is forward (-Z). Tune these if the bed still intersects the arm.
const BED_ARM_CLEARANCE_X: f32 = -0.15;
const BED_ARM_CLEARANCE_Z: f32 = -0.10;

/// Longest one main-hand swing may take while a block is being broken, in ticks (vanilla's swing
/// length). The item's normal swing (10 ticks by default) is made for attacks; mining with it
/// finishes the block long before one swing ends, so the hand looks stuck on one slow swing.
const MINING_SWING_TICKS: u32 = 6;

// ---------------------------------------------------------------------------
// Easing functions (1:1 port of com.holdmylua.source.scripting.script_wrappers.Easings)
// ---------------------------------------------------------------------------

pub mod easings {
    use std::f32::consts::PI;

    #[inline]
    pub fn ease_custom(t: f32) -> f32 {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        3.0 * t * mt * mt * 0.44 + 3.0 * t2 * mt * 1.0 + t3
    }

    #[inline]
    pub fn ease_custom_sec(t: f32) -> f32 {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        3.0 * t * mt * mt * 0.44 + 3.0 * t2 * mt * 0.94 + t3
    }

    #[inline]
    pub fn ease_in_out_back(x: f32) -> f32 {
        let c1 = 1.70158_f32;
        let c2 = c1 * 1.525_f32;
        if x < 0.5 {
            ((2.0 * x).powi(2) * ((c2 + 1.0) * 2.0 * x - c2)) / 2.0
        } else {
            (((2.0 * x - 2.0).powi(2) * ((c2 + 1.0) * (x * 2.0 - 2.0) + c2)) + 2.0) / 2.0
        }
    }

    #[inline]
    pub fn ease_in_back(x: f32) -> f32 {
        let c1 = 1.70158_f32;
        let c3 = c1 + 1.0;
        c3 * x * x * x - c1 * x * x
    }

    #[inline]
    pub fn ease_out_back(x: f32) -> f32 {
        let c1 = 1.70158_f32;
        let c3 = c1 + 1.0;
        1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
    }

    #[inline]
    pub fn ease_in_sine(x: f32) -> f32 {
        1.0 - (x * PI / 2.0).cos()
    }

    #[inline]
    pub fn ease_out_sine(x: f32) -> f32 {
        (x * PI / 2.0).sin()
    }

    #[inline]
    pub fn ease_in_out_sine(x: f32) -> f32 {
        -((PI * x).cos() - 1.0) / 2.0
    }

    #[inline]
    pub fn ease_in_quart(x: f32) -> f32 {
        x * x * x * x
    }

    #[inline]
    pub fn ease_out_quart(x: f32) -> f32 {
        1.0 - (1.0 - x).powi(4)
    }

    #[inline]
    pub fn ease_in_out_expo(x: f32) -> f32 {
        if x <= 0.0 {
            0.0
        } else if x >= 1.0 {
            1.0
        } else if x < 0.5 {
            2.0_f32.powf(20.0 * x - 10.0) / 2.0
        } else {
            (2.0 - 2.0_f32.powf(-20.0 * x + 10.0)) / 2.0
        }
    }

    #[inline]
    pub fn cubic_ease(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }
}

// ---------------------------------------------------------------------------
// 4x4 Column-Major Matrix (`Mat4f`, JOML / glam compatible)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4f(pub [f32; 16]);

impl Default for Mat4f {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4f {
    pub const IDENTITY: Self = Self([
        1.0, 0.0, 0.0, 0.0, // col 0
        0.0, 1.0, 0.0, 0.0, // col 1
        0.0, 0.0, 1.0, 0.0, // col 2
        0.0, 0.0, 0.0, 1.0, // col 3
    ]);

    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        let a = self.0;
        let b = rhs.0;
        let mut out = [0.0_f32; 16];
        let mut c = 0;
        while c < 4 {
            let mut r = 0;
            while r < 4 {
                out[c * 4 + r] = a[r] * b[c * 4]
                    + a[4 + r] * b[c * 4 + 1]
                    + a[8 + r] * b[c * 4 + 2]
                    + a[12 + r] * b[c * 4 + 3];
                r += 1;
            }
            c += 1;
        }
        Self(out)
    }

    #[inline]
    pub fn from_translation(x: f32, y: f32, z: f32) -> Self {
        Self([
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            x,   y,   z,   1.0,
        ])
    }

    #[inline]
    pub fn from_scale(x: f32, y: f32, z: f32) -> Self {
        Self([
            x,   0.0, 0.0, 0.0,
            0.0, y,   0.0, 0.0,
            0.0, 0.0, z,   0.0,
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    #[inline]
    pub fn from_rotation_x_deg(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self([
            1.0, 0.0, 0.0, 0.0,
            0.0, c,   s,   0.0,
            0.0, -s,  c,   0.0,
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    #[inline]
    pub fn from_rotation_y_deg(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self([
            c,   0.0, -s,  0.0,
            0.0, 1.0, 0.0, 0.0,
            s,   0.0, c,   0.0,
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    #[inline]
    pub fn from_rotation_z_deg(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self([
            c,   s,   0.0, 0.0,
            -s,  c,   0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    #[inline]
    pub fn translate(&mut self, x: f32, y: f32, z: f32) {
        *self = self.mul(Self::from_translation(x, y, z));
    }

    #[inline]
    pub fn move_x(&mut self, x: f32) {
        self.translate(x, 0.0, 0.0);
    }

    #[inline]
    pub fn move_y(&mut self, y: f32) {
        self.translate(0.0, y, 0.0);
    }

    #[inline]
    pub fn move_z(&mut self, z: f32) {
        self.translate(0.0, 0.0, z);
    }

    #[inline]
    pub fn scale(&mut self, x: f32, y: f32, z: f32) {
        *self = self.mul(Self::from_scale(x, y, z));
    }

    #[inline]
    pub fn rotate_x(&mut self, deg: f32) {
        *self = self.mul(Self::from_rotation_x_deg(deg));
    }

    #[inline]
    pub fn rotate_y(&mut self, deg: f32) {
        *self = self.mul(Self::from_rotation_y_deg(deg));
    }

    #[inline]
    pub fn rotate_z(&mut self, deg: f32) {
        *self = self.mul(Self::from_rotation_z_deg(deg));
    }

    /// `PoseStack.rotateAround(RotationAxis.POSITIVE_X.rotationDegrees(deg), px, py, pz)`
    #[inline]
    pub fn rotate_x_at(&mut self, deg: f32, px: f32, py: f32, pz: f32) {
        self.translate(px, py, pz);
        self.rotate_x(deg);
        self.translate(-px, -py, -pz);
    }

    #[inline]
    pub fn rotate_y_at(&mut self, deg: f32, px: f32, py: f32, pz: f32) {
        self.translate(px, py, pz);
        self.rotate_y(deg);
        self.translate(-px, -py, -pz);
    }

    #[inline]
    pub fn rotate_z_at(&mut self, deg: f32, px: f32, py: f32, pz: f32) {
        self.translate(px, py, pz);
        self.rotate_z(deg);
        self.translate(-px, -py, -pz);
    }

    /// `M:shear(matrices, sx, sy, sz)`
    #[inline]
    pub fn shear(&mut self, sx: f32, sy: f32, sz: f32) {
        let shear = Self([
            1.0, sx,  sx,  0.0,
            sy,  1.0, sy,  0.0,
            sz,  sz,  1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ]);
        *self = self.mul(shear);
    }

    #[inline]
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = self.0;
        [
            m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
            m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
            m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        ]
    }
}

// ---------------------------------------------------------------------------
// Per-Quad Model Part Animator (`ModelPartAnimator.java` + `item_model.lua`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum PoseKind {
    MoveX(f32),
    MoveY(f32),
    MoveZ(f32),
    RotateX(f32, f32, f32, f32),
    RotateY(f32, f32, f32, f32),
    RotateZ(f32, f32, f32, f32),
    Scale(f32, f32, f32),
}

#[derive(Clone, Copy, Debug)]
struct QuadPoseOp {
    start: usize,
    end: usize,
    kind: PoseKind,
}

#[derive(Clone, Debug, Default)]
pub struct ModelPartAnimator {
    ops: Vec<QuadPoseOp>,
}

impl ModelPartAnimator {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn move_x(&mut self, start: usize, end: usize, amount: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::MoveX(amount),
        });
    }

    pub fn move_y(&mut self, start: usize, end: usize, amount: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::MoveY(amount),
        });
    }

    pub fn move_z(&mut self, start: usize, end: usize, amount: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::MoveZ(amount),
        });
    }

    pub fn rotate_x_at(&mut self, start: usize, end: usize, deg: f32, px: f32, py: f32, pz: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::RotateX(deg, px, py, pz),
        });
    }

    pub fn rotate_y_at(&mut self, start: usize, end: usize, deg: f32, px: f32, py: f32, pz: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::RotateY(deg, px, py, pz),
        });
    }

    pub fn rotate_z_at(&mut self, start: usize, end: usize, deg: f32, px: f32, py: f32, pz: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::RotateZ(deg, px, py, pz),
        });
    }

    pub fn scale(&mut self, start: usize, end: usize, x: f32, y: f32, z: f32) {
        self.ops.push(QuadPoseOp {
            start,
            end,
            kind: PoseKind::Scale(x, y, z),
        });
    }

    /// Replicates `ModelPartAnimator.applyPoses(index, matrixStack)` (applied in reverse order).
    pub fn matrix_for_quad(&self, index: usize) -> Option<Mat4f> {
        let mut mat = Mat4f::IDENTITY;
        let mut matched = false;
        let mut i = self.ops.len();
        while i > 0 {
            i -= 1;
            let op = self.ops[i];
            if index >= op.start && index <= op.end {
                matched = true;
                match op.kind {
                    PoseKind::MoveX(v) => mat.move_x(v),
                    PoseKind::MoveY(v) => mat.move_y(v),
                    PoseKind::MoveZ(v) => mat.move_z(v),
                    PoseKind::RotateX(deg, x, y, z) => mat.rotate_x_at(deg, x, y, z),
                    PoseKind::RotateY(deg, x, y, z) => mat.rotate_y_at(deg, x, y, z),
                    PoseKind::RotateZ(deg, x, y, z) => mat.rotate_z_at(deg, x, y, z),
                    PoseKind::Scale(x, y, z) => mat.scale(x, y, z),
                }
            }
        }
        if matched { Some(mat) } else { None }
    }
}

// ---------------------------------------------------------------------------
// Configuration & Options > Mods Settings UI
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub x_offset: f32,
    pub y_offset: f32,
    pub z_offset: f32,
    pub swing_speed: u32,
    pub block_render_3d: bool,
    pub hand_particles: bool,
    pub dynamic_lights: bool,
    pub camera_shake: bool,
    pub switch_sounds: bool,
    pub sword_counter_attack: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            x_offset: 0.0,
            y_offset: 0.0,
            z_offset: 0.0,
            swing_speed: 10,
            block_render_3d: true,
            hand_particles: true,
            dynamic_lights: true,
            camera_shake: true,
            switch_sounds: true,
            sword_counter_attack: true,
        }
    }
}

pub type LoadedConfig = Result<Config, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsEffect {
    Save(Config),
    Reload,
}

impl Config {
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"x_offset\": {:.3},\n",
                "  \"y_offset\": {:.3},\n",
                "  \"z_offset\": {:.3},\n",
                "  \"swing_speed\": {},\n",
                "  \"block_render_3d\": {},\n",
                "  \"hand_particles\": {},\n",
                "  \"dynamic_lights\": {},\n",
                "  \"camera_shake\": {},\n",
                "  \"switch_sounds\": {},\n",
                "  \"sword_counter_attack\": {}\n",
                "}}\n"
            ),
            self.x_offset,
            self.y_offset,
            self.z_offset,
            self.swing_speed,
            self.block_render_3d,
            self.hand_particles,
            self.dynamic_lights,
            self.camera_shake,
            self.switch_sounds,
            self.sword_counter_attack,
        )
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut cfg = Self::default();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(cfg);
        }
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return Err("expected JSON object".to_owned());
        }
        let body = &trimmed[1..trimmed.len() - 1];
        for entry in body.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let Some((raw_k, raw_v)) = entry.split_once(':') else {
                return Err(format!("invalid entry: {entry}"));
            };
            let k = raw_k.trim().trim_matches('"');
            let v = raw_v.trim().trim_matches('"');
            match k {
                "x_offset" | "xOffset" => {
                    cfg.x_offset = v
                        .parse::<f32>()
                        .map_err(|_| format!("invalid x_offset: {v}"))?
                        .clamp(-0.5, 0.5);
                }
                "y_offset" | "yOffset" => {
                    cfg.y_offset = v
                        .parse::<f32>()
                        .map_err(|_| format!("invalid y_offset: {v}"))?
                        .clamp(-0.5, 0.5);
                }
                "z_offset" | "zOffset" => {
                    cfg.z_offset = v
                        .parse::<f32>()
                        .map_err(|_| format!("invalid z_offset: {v}"))?
                        .clamp(-0.5, 0.5);
                }
                "swing_speed" | "swingSpeed" => {
                    cfg.swing_speed = v
                        .parse::<u32>()
                        .map_err(|_| format!("invalid swing_speed: {v}"))?
                        .clamp(4, 24);
                }
                "block_render_3d" | "blockRender" => {
                    cfg.block_render_3d = parse_bool(v)?;
                }
                "hand_particles" | "handParticles" => {
                    cfg.hand_particles = parse_bool(v)?;
                }
                "dynamic_lights" | "dynamicLights" => {
                    cfg.dynamic_lights = parse_bool(v)?;
                }
                "camera_shake" | "cameraShake" => {
                    cfg.camera_shake = parse_bool(v)?;
                }
                "switch_sounds" | "switchSounds" => {
                    cfg.switch_sounds = parse_bool(v)?;
                }
                "sword_counter_attack" | "swordCounterAttack" => {
                    cfg.sword_counter_attack = parse_bool(v)?;
                }
                _ => {}
            }
        }
        Ok(cfg)
    }
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "true" | "1" | "on" => Ok(true),
        "false" | "0" | "off" => Ok(false),
        _ => Err(format!("expected boolean, got {v}")),
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "ON" } else { "OFF" }
}

pub fn settings_labels(loaded: &LoadedConfig) -> Vec<String> {
    let Ok(c) = loaded else {
        return vec!["Reload config.json (fix syntax error)".to_owned()];
    };
    vec![
        format!("3D Held Blocks: {}", on_off(c.block_render_3d)),
        format!("Hand Glow & Petal Particles: {}", on_off(c.hand_particles)),
        format!("Dynamic Hand Lighting: {}", on_off(c.dynamic_lights)),
        format!("Eat/Drink/Walk Camera Shake: {}", on_off(c.camera_shake)),
        format!("Hand Switch & Bell Sounds: {}", on_off(c.switch_sounds)),
        format!("Alternating Sword Combos: {}", on_off(c.sword_counter_attack)),
        format!("Base Swing Speed: {} ticks", c.swing_speed),
        format!(
            "Viewmodel Offset: X {:+.2} / Y {:+.2} / Z {:+.2}",
            c.x_offset, c.y_offset, c.z_offset
        ),
        "Reset Hold My Items Defaults".to_owned(),
    ]
}

pub fn settings_click(loaded: &LoadedConfig, button: usize) -> Option<SettingsEffect> {
    let Ok(mut c) = loaded.clone() else {
        return (button == 0).then_some(SettingsEffect::Reload);
    };
    match button {
        0 => c.block_render_3d = !c.block_render_3d,
        1 => c.hand_particles = !c.hand_particles,
        2 => c.dynamic_lights = !c.dynamic_lights,
        3 => c.camera_shake = !c.camera_shake,
        4 => c.switch_sounds = !c.switch_sounds,
        5 => c.sword_counter_attack = !c.sword_counter_attack,
        6 => {
            c.swing_speed = match c.swing_speed {
                6 => 8,
                8 => 10,
                10 => 12,
                12 => 15,
                _ => 6,
            };
        }
        7 => {
            if c.x_offset == 0.0 && c.y_offset == 0.0 && c.z_offset == 0.0 {
                c.x_offset = 0.05;
                c.y_offset = -0.03;
                c.z_offset = -0.05;
            } else if c.x_offset > 0.0 {
                c.x_offset = -0.05;
                c.y_offset = 0.03;
                c.z_offset = 0.05;
            } else {
                c.x_offset = 0.0;
                c.y_offset = 0.0;
                c.z_offset = 0.0;
            }
        }
        8 => c = Config::default(),
        _ => return None,
    }
    Some(SettingsEffect::Save(c))
}

// ---------------------------------------------------------------------------
// Lua Override Table Parser (`mixins/holdmyitems/data/*.lua`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct LuaOverrides {
    pub render_as_block: HashMap<String, bool>,
    pub apply_block_rotation: HashMap<String, bool>,
    pub translate_item: HashMap<String, bool>,
    pub item_swing_speed: HashMap<String, u32>,
}

impl LuaOverrides {
    pub fn parse_script(&mut self, script: &str) {
        for raw_line in script.lines() {
            let line = raw_line.split("--").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, val)) = parse_put_call(line, "renderAsBlock:put(") {
                if let Ok(b) = parse_bool(&val) {
                    self.render_as_block.insert(qualify_id(&key), b);
                }
            } else if let Some((key, val)) = parse_put_call(line, "applyBlockRotation:put(") {
                if let Ok(b) = parse_bool(&val) {
                    self.apply_block_rotation.insert(qualify_id(&key), b);
                }
            } else if let Some((key, val)) = parse_put_call(line, "translateItem:put(") {
                if let Ok(b) = parse_bool(&val) {
                    self.translate_item.insert(qualify_id(&key), b);
                }
            } else if let Some((key, val)) = parse_put_call(line, "itemSwingSpeed:put(") {
                if let Ok(speed) = val.parse::<u32>() {
                    self.item_swing_speed.insert(qualify_id(&key), speed.clamp(2, 40));
                }
            }
        }
    }
}

fn qualify_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

fn parse_put_call(line: &str, prefix: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(prefix)?;
    let inside = rest.strip_suffix(')')?;
    let (k, v) = inside.split_once(',')?;
    let key = k.trim().trim_matches('"').trim_matches('\'');
    let val = v.trim().trim_matches('"').trim_matches('\'');
    if key.is_empty() || key.contains('(') {
        return None;
    }
    Some((key.to_owned(), val.to_owned()))
}

// ---------------------------------------------------------------------------
// Item Classification & Vanilla Tags (HMI 5.1.1 `I.java` & `item_pose.lua`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseKind {
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Crossbow,
    Trident,
    Spear,
    Brush,
    TootHorn,
    Spyglass,
}

#[derive(Clone, Debug)]
pub struct ItemFlags {
    pub id: String,
    pub is_empty: bool,
    pub is_block: bool,
    pub use_kind: UseKind,
    pub is_charged_crossbow: bool,
    pub is_cooling_down: bool,
    pub spear_can_damage: bool,
    pub spear_can_dismount: bool,
    pub spear_can_knockback: bool,
    pub spear_hit_impact: bool,
}

impl Default for ItemFlags {
    fn default() -> Self {
        Self::empty()
    }
}

impl ItemFlags {
    pub fn empty() -> Self {
        Self {
            id: "minecraft:air".to_owned(),
            is_empty: true,
            is_block: false,
            use_kind: UseKind::None,
            is_charged_crossbow: false,
            is_cooling_down: false,
            spear_can_damage: false,
            spear_can_dismount: false,
            spear_can_knockback: false,
            spear_hit_impact: false,
        }
    }

    pub fn short_name(&self) -> &str {
        self.id.strip_prefix("minecraft:").unwrap_or(&self.id)
    }

    pub fn is_pickaxe(&self) -> bool {
        self.short_name().ends_with("_pickaxe")
    }

    pub fn is_sword(&self) -> bool {
        self.short_name().ends_with("_sword")
    }

    pub fn is_axe(&self) -> bool {
        self.short_name().ends_with("_axe")
    }

    pub fn is_shovel(&self) -> bool {
        self.short_name().ends_with("_shovel")
    }

    pub fn is_hoe(&self) -> bool {
        self.short_name().ends_with("_hoe")
    }

    pub fn is_spear(&self) -> bool {
        self.use_kind == UseKind::Spear || self.short_name().ends_with("_spear")
    }

    pub fn is_trident(&self) -> bool {
        self.use_kind == UseKind::Trident || self.short_name() == "trident"
    }

    pub fn is_mace(&self) -> bool {
        self.short_name() == "mace"
    }

    pub fn is_shears(&self) -> bool {
        self.short_name() == "shears"
    }

    pub fn is_filled_map(&self) -> bool {
        self.short_name() == "filled_map"
    }

    pub fn is_lantern(&self) -> bool {
        let s = self.short_name();
        s == "lantern" || s == "soul_lantern" || s.ends_with("_copper_lantern") || s == "copper_lantern"
    }

    pub fn is_bell(&self) -> bool {
        self.short_name() == "bell"
    }

    pub fn is_end_crystal(&self) -> bool {
        self.short_name() == "end_crystal"
    }

    pub fn is_hanging_sign(&self) -> bool {
        self.short_name().ends_with("_hanging_sign")
    }

    pub fn is_sign(&self) -> bool {
        let s = self.short_name();
        s.ends_with("_sign") && !s.ends_with("_hanging_sign")
    }

    pub fn is_door(&self) -> bool {
        self.short_name().ends_with("_door")
    }

    pub fn is_bed(&self) -> bool {
        self.short_name().ends_with("_bed")
    }

    pub fn is_totem(&self) -> bool {
        self.short_name() == "totem_of_undying"
    }

    pub fn is_banner(&self) -> bool {
        self.short_name().ends_with("_banner")
    }

    pub fn is_skull(&self) -> bool {
        let s = self.short_name();
        s.ends_with("_skull") || s.ends_with("_head")
    }

    pub fn is_dragon_head(&self) -> bool {
        self.short_name() == "dragon_head"
    }

    pub fn is_flower_litter(&self) -> bool {
        matches!(
            self.short_name(),
            "pink_petals" | "leaf_litter" | "wildflowers"
        )
    }

    pub fn is_painting_or_frame(&self) -> bool {
        matches!(
            self.short_name(),
            "painting" | "item_frame" | "glow_item_frame"
        )
    }

    pub fn is_bundle(&self) -> bool {
        let s = self.short_name();
        s == "bundle" || s.ends_with("_bundle")
    }

    pub fn is_music_disc(&self) -> bool {
        self.short_name().starts_with("music_disc_")
    }

    pub fn is_nugget(&self) -> bool {
        self.short_name().ends_with("_nugget")
    }

    pub fn is_throwable(&self) -> bool {
        matches!(
            self.short_name(),
            "snowball"
                | "egg"
                | "brown_egg"
                | "blue_egg"
                | "ender_pearl"
                | "ender_eye"
                | "experience_bottle"
                | "splash_potion"
                | "lingering_potion"
                | "wind_charge"
                | "fire_charge"
        )
    }

    pub fn has_switch_flip(&self) -> bool {
        (self.is_bundle()
            || self.is_throwable()
            || self.is_music_disc()
            || self.is_nugget()
            || self.is_skull())
            && self.use_kind != UseKind::Trident
    }

    pub fn is_bucket(&self) -> bool {
        let s = self.short_name();
        matches!(
            s,
            "bucket"
                | "water_bucket"
                | "lava_bucket"
                | "powder_snow_bucket"
                | "axolotl_bucket"
                | "pufferfish_bucket"
                | "tadpole_bucket"
                | "salmon_bucket"
                | "cod_bucket"
                | "tropical_fish_bucket"
        ) || s.starts_with("frog_bucket_")
    }

    pub fn is_milk_bucket(&self) -> bool {
        self.short_name() == "milk_bucket"
    }

    pub fn is_chest_boat(&self) -> bool {
        let s = self.short_name();
        s.ends_with("_chest_boat") || s == "bamboo_chest_raft"
    }

    pub fn is_inspectable_weapon(&self) -> bool {
        self.is_sword() || self.is_pickaxe() || self.is_axe() || self.is_trident()
    }

    /// `BlockTags.INSIDE_STEP_SOUND_BLOCKS` (`field_28040`) and `BlockTags.FLOWERS` (`field_20341`),
    /// which `HeldItemRendererMixin.java` lines 492-493 exclude from 3D block rendering so
    /// `wildflowers`, `pink_petals`, `leaf_litter` (3D flower baskets) and flowers use their item models.
    pub fn is_flower_or_step_block(&self) -> bool {
        if self.is_flower_litter() {
            return true;
        }
        matches!(
            self.short_name(),
            "frogspawn"
                | "lily_pad"
                | "small_amethyst_bud"
                | "glow_lichen"
                | "sculk_vein"
                | "resin_clump"
                | "dandelion"
                | "golden_dandelion"
                | "poppy"
                | "blue_orchid"
                | "allium"
                | "azure_bluet"
                | "red_tulip"
                | "orange_tulip"
                | "white_tulip"
                | "pink_tulip"
                | "oxeye_daisy"
                | "cornflower"
                | "lily_of_the_valley"
                | "wither_rose"
                | "torchflower"
                | "open_eyeblossom"
                | "closed_eyeblossom"
                | "sunflower"
                | "lilac"
                | "peony"
                | "rose_bush"
                | "pitcher_plant"
                | "mangrove_propagule"
                | "chorus_flower"
                | "spore_blossom"
                | "cactus_flower"
        )
    }

    pub fn should_render_as_block(&self, overrides: &LuaOverrides) -> bool {
        if !self.is_block
            || self.is_flower_or_step_block()
            || self.use_kind == UseKind::Eat
            || self.is_banner()
            || self.is_bed()
            || self.is_skull()
            || self.is_sword()
            || self.is_trident()
            || self.is_spear()
        {
            return false;
        }
        if let Some(&v) = overrides.render_as_block.get(&self.id) {
            return v;
        }
        let s = self.short_name();
        if s.ends_with("_stained_glass_pane")
            || s.ends_with("_candle")
            || s.ends_with("_sign")
            || s.contains("copper_golem_statue")
        {
            return false;
        }
        !matches!(
            s,
            "string"
                | "resin_clump"
                | "vine"
                | "bamboo"
                | "kelp"
                | "seagrass"
                | "iron_bars"
                | "glass_pane"
                | "ladder"
                | "tripwire_hook"
                | "hopper"
                | "cauldron"
                | "rail"
                | "powered_rail"
                | "detector_rail"
                | "activator_rail"
                | "repeater"
                | "comparator"
                | "twisting_vines"
                | "weeping_vines"
                | "sniffer_egg"
                | "candle"
                | "frogspawn"
                | "light"
                | "structure_void"
                | "barrier"
                | "carrot"
                | "powder_snow_bucket"
                | "glow_berries"
                | "potato"
                | "sweet_berries"
                | "redstone"
                | "painting"
                | "item_frame"
                | "glow_item_frame"
        )
    }

    pub fn should_apply_block_rotation(&self, overrides: &LuaOverrides) -> bool {
        if let Some(&v) = overrides.apply_block_rotation.get(&self.id) {
            return v;
        }
        !(self.is_hanging_sign() || self.is_door() || self.is_skull() || self.is_sign())
    }

    pub fn is_rod(&self) -> bool {
        matches!(
            self.short_name(),
            "fishing_rod"
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
                | "blaze_rod"
                | "breeze_rod"
                | "stick"
        )
    }

    pub fn is_tool_or_weapon(&self) -> bool {
        self.is_sword()
            || self.is_pickaxe()
            || self.is_axe()
            || self.is_shovel()
            || self.is_hoe()
            || self.is_mace()
            || self.is_shears()
            || self.is_rod()
            || self.use_kind == UseKind::Brush
            || self.short_name() == "flint_and_steel"
    }

    pub fn is_custom_translate(&self, overrides: &LuaOverrides) -> bool {
        if let Some(&v) = overrides.translate_item.get(&self.id) {
            return v;
        }
        let s = self.short_name();
        if s.ends_with("_stained_glass_pane") || s.ends_with("_candle") || self.is_sign() {
            return true;
        }
        matches!(
            s,
            "string"
                | "resin_clump"
                | "vine"
                | "kelp"
                | "seagrass"
                | "iron_bars"
                | "glass_pane"
                | "ladder"
                | "tripwire_hook"
                | "hopper"
                | "cauldron"
                | "rail"
                | "powered_rail"
                | "detector_rail"
                | "activator_rail"
                | "repeater"
                | "comparator"
                | "twisting_vines"
                | "weeping_vines"
                | "sniffer_egg"
                | "candle"
                | "frogspawn"
                | "light"
                | "structure_void"
                | "barrier"
                | "carrot"
                | "powder_snow_bucket"
                | "glow_berries"
                | "potato"
                | "sweet_berries"
                | "redstone"
        )
    }

    /// Replicates `I.java` `shouldTranslateItem` (lines 106-119) + `item_pose.lua` lines 571-583 (`shouldTranslateItem` and `isCustomTranslate`).
    /// Note: in `I.java`, `item.getUseAction() != UseAction.BLOCK` (shield returns `false`),
    /// weapons with `getAttackDamage(item) > 0` (including trident) return `false`, and
    /// `UseAction.EAT`, `UseAction.DRINK`, and `UseAction.SPYGLASS` return `true`.
    pub fn should_translate_item(&self, overrides: &LuaOverrides) -> bool {
        if self.is_custom_translate(overrides) {
            return true;
        }
        if self.is_block {
            return false;
        }
        let s = self.short_name();
        if s == "bone" || self.use_kind == UseKind::Bow || self.is_spear() {
            return false;
        }
        if matches!(
            self.use_kind,
            UseKind::Eat | UseKind::Drink | UseKind::Spyglass
        ) {
            return true;
        }
        if self.is_tool_or_weapon()
            || self.is_trident()
            || matches!(self.use_kind, UseKind::Crossbow | UseKind::Block)
        {
            return false;
        }
        true
    }

    pub fn swing_duration(&self, base_speed: u32, overrides: &LuaOverrides) -> u32 {
        if let Some(&custom) = overrides.item_swing_speed.get(&self.id) {
            return custom;
        }
        if self.is_spear() {
            15
        } else if self.is_shovel() {
            14
        } else if self.is_trident() || self.is_mace() {
            12
        } else {
            base_speed.clamp(4, 24)
        }
    }
}

// ---------------------------------------------------------------------------
// First-Person Particles (`Particle.java` & `item_pose.lua`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleSpace {
    Item,
    Screen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlend {
    Additive,
    Translucent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleLifecycle {
    Spawn,
    Opacity,
}

#[derive(Clone, Debug)]
pub struct HmiParticle {
    pub main_hand: bool,
    pub space: ParticleSpace,
    pub blend: ParticleBlend,
    pub lifecycle: ParticleLifecycle,
    pub sprite: &'static str,
    pub tint_rgb: u32,
    pub gravity: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub dx: f32,
    pub dy: f32,
    pub dz: f32,
    pub scale: f32,
    pub max_scale: f32,
    pub alpha: f32,
    pub lifetime: f32,
    pub is_birth: bool,
    pub dead: bool,
    pub drift_wave: bool,
}

impl HmiParticle {
    pub fn tick(&mut self, dt30: f32, age_ticks: f32) {
        if self.dead {
            return;
        }
        self.lifetime = (self.lifetime - 0.1 * dt30).clamp(0.0, 9999.0);
        self.x += self.dx * dt30;
        self.y += self.dy * dt30;
        self.z += self.dz * dt30;
        let damp = 0.9_f32.powf(dt30);
        self.dx *= damp;
        self.dy *= damp;
        self.dz *= damp;
        if self.gravity {
            self.dy -= 0.01 * dt30;
        }
        if self.drift_wave {
            self.dx += 0.005 * (age_ticks * 0.5).sin() * dt30;
        }
        match self.lifecycle {
            ParticleLifecycle::Spawn => {
                if !self.is_birth && self.lifetime <= 0.0 {
                    self.dead = true;
                } else {
                    self.scale = self.max_scale;
                    self.is_birth = false;
                }
            }
            ParticleLifecycle::Opacity => {
                if !self.is_birth && self.lifetime <= 0.0 {
                    self.alpha -= 10.0 * dt30;
                    if self.alpha <= 0.0 {
                        self.alpha = 0.0;
                        self.dead = true;
                    }
                } else {
                    self.scale = self.max_scale;
                    self.is_birth = false;
                    self.alpha = 255.0;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Frame Input, Sound Events, and Evaluation Output
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PlayerFrameInput {
    pub pitch: f32,
    pub yaw: f32,
    pub speed: f32,
    pub y_speed: f32,
    pub on_ground: bool,
    pub sneaking: bool,
    pub swimming: bool,
    pub crawling: bool,
    pub climbing: bool,
    pub touching_water: bool,
    pub using_riptide: bool,
    pub using_item: bool,
    pub active_main_hand: bool,
    pub inspect_held: bool,
    pub block_breaking: bool,
    pub age_ticks: f32,
    pub main_equip_progress: f32,
    pub off_equip_progress: f32,
}

#[derive(Clone, Debug)]
pub struct QueuedSound {
    pub sound: &'static str,
    pub volume: f32,
    pub pitch: f32,
}

#[derive(Clone, Debug)]
pub struct HandEvaluation {
    pub arm_pose: Mat4f,
    pub item_pose: Mat4f,
    pub render_as_3d_block: bool,
    pub effective_empty: bool,
    pub animator: ModelPartAnimator,
}

// ---------------------------------------------------------------------------
// Complete HMI 5.1.1 State Machine (`hand_pose.lua`, `hand_relative_pose.lua`, `item_pose.lua`, `item_model.lua`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct HmiState {
    pub bow_count: f32,
    pub bow_count_sec: f32,
    pub bow_wiggle: f32,
    pub bow_count_o: f32,
    pub bow_count_sec_o: f32,
    pub bow_wiggle_o: f32,
    pub cross_bow_m: f32,
    pub cross_bow_sec_m: f32,
    pub cross_bow_o: f32,
    pub cross_bow_sec_o: f32,
    pub trident_m: f32,
    pub trident: f32,
    pub trident_j: f32,
    pub trident_m_o: f32,
    pub trident_o: f32,
    pub trident_j_o: f32,
    pub riptide_counter: f32,
    pub riptide_counter_o: f32,
    pub shield_m: f32,
    pub shield_o: f32,
    pub shield_disable: f32,
    pub food_count: f32,
    pub food_count_sec: f32,
    pub food_count_o: f32,
    pub food_count_sec_o: f32,
    pub drink_count: f32,
    pub drink_count_o: f32,
    pub food_speed: f32,
    pub brush_counter: f32,
    pub brush_counter_o: f32,
    pub brush_speed_m: f32,
    pub brush_angle_m: f32,
    pub brush_speed_o: f32,
    pub brush_angle_o: f32,
    pub spear_counter_m: f32,
    pub spear_counter_o: f32,
    pub can_dismount_counter: f32,
    pub can_knockback_counter: f32,
    pub can_dismount_counter_o: f32,
    pub can_knockback_counter_o: f32,
    pub hit_impact_counter: f32,
    pub hit_impact_counter_o: f32,
    pub walk: f32,
    pub walk_smoother: f32,
    pub water_walk: f32,
    pub in_water_count: f32,
    pub fall: f32,
    pub fall_speed: f32,
    pub fall_f: f32,
    pub jiggle_i: f32,
    pub sneak: f32,
    pub a: f32,
    pub smoothing: f32,
    pub smoothing_crawl: f32,
    pub crawl_default_pos: f32,
    pub crawler: f32,
    pub crwl: f32,
    pub swim_counter: f32,
    pub swim_smoother: f32,
    pub offhand: f32,
    pub main_hand_switch: f32,
    pub off_hand_switch: f32,
    pub map_smoother: f32,
    pub map_zoomer: f32,
    pub inspection_counter: f32,
    pub inspection_spin: f32,
    pub tilting: f32,
    pub pitch_angle: f32,
    pub pitch_speed: f32,
    pub pitch_angle_o: f32,
    pub pitch_speed_o: f32,
    pub yaw_angle: f32,
    pub yaw_speed: f32,
    pub yaw_angle_o: f32,
    pub yaw_speed_o: f32,
    pub prev_pitch: f32,
    pub prev_yaw: f32,
    pub sword_attack: bool,
    pub sword_attack2: bool,
    pub prev_swing_m: bool,
    pub swing_m_hand_prev: bool,
    pub swing_o_hand_prev: bool,
    pub swing_count_prev: u32,
    pub prev_main_item: String,
    pub prev_off_item: String,
    pub main_switch_event: bool,
    pub off_switch_event: bool,

    // LivingEntityMixin per-hand 20 TPS swing state
    pub main_hand_swinging: bool,
    pub main_hand_swing_ticks: u32,
    pub main_hand_swing_progress: f32,
    pub last_main_hand_swing_progress: f32,
    pub off_hand_swinging: bool,
    pub off_hand_swing_ticks: u32,
    pub off_hand_swing_progress: f32,
    pub last_off_hand_swing_progress: f32,
    pub swing_m_hand: bool,
    pub swing_o_hand: bool,
    pub main_swing_count: u32,
    pub main_swing_ticked_20tps: bool,
    pub off_swing_ticked_20tps: bool,
    pub last_haste: Option<i32>,
    pub last_fatigue: Option<i32>,
    /// Whether a block is being broken, as of the last swing start or `step_frame`. While true,
    /// main-hand swings are capped at `MINING_SWING_TICKS` so the animation keeps pace with mining.
    pub block_breaking: bool,
    pub angles_initialized: bool,

    pub particles: Vec<HmiParticle>,
    pub rng_state: u64,
    pub debug_lines: Vec<String>,
    pub overrides: LuaOverrides,
}

impl Default for HmiState {
    fn default() -> Self {
        Self {
            bow_count: 0.0,
            bow_count_sec: 0.0,
            bow_wiggle: 0.0,
            bow_count_o: 0.0,
            bow_count_sec_o: 0.0,
            bow_wiggle_o: 0.0,
            cross_bow_m: 0.0,
            cross_bow_sec_m: 0.0,
            cross_bow_o: 0.0,
            cross_bow_sec_o: 0.0,
            trident_m: 0.0,
            trident: 0.0,
            trident_j: 0.0,
            trident_m_o: 0.0,
            trident_o: 0.0,
            trident_j_o: 0.0,
            riptide_counter: 0.0,
            riptide_counter_o: 0.0,
            shield_m: 0.0,
            shield_o: 0.0,
            shield_disable: 0.0,
            food_count: 0.0,
            food_count_sec: 0.0,
            food_count_o: 0.0,
            food_count_sec_o: 0.0,
            drink_count: 0.0,
            drink_count_o: 0.0,
            food_speed: 0.0,
            brush_counter: 0.0,
            brush_counter_o: 0.0,
            brush_speed_m: 0.0,
            brush_angle_m: 0.0,
            brush_speed_o: 0.0,
            brush_angle_o: 0.0,
            spear_counter_m: 0.0,
            spear_counter_o: 0.0,
            can_dismount_counter: 0.0,
            can_knockback_counter: 0.0,
            can_dismount_counter_o: 0.0,
            can_knockback_counter_o: 0.0,
            hit_impact_counter: 0.0,
            hit_impact_counter_o: 0.0,
            walk: 0.0,
            walk_smoother: 0.0,
            water_walk: 0.0,
            in_water_count: 0.0,
            fall: 0.0,
            fall_speed: 0.0,
            fall_f: 0.0,
            jiggle_i: 0.0,
            sneak: 0.0,
            a: 0.0,
            smoothing: 0.0,
            smoothing_crawl: 0.0,
            crawl_default_pos: 0.0,
            crawler: 0.0,
            crwl: 0.0,
            swim_counter: 0.0,
            swim_smoother: 0.0,
            offhand: 0.0,
            main_hand_switch: 1.0,
            off_hand_switch: 1.0,
            map_smoother: 0.0,
            map_zoomer: 0.0,
            inspection_counter: 0.0,
            inspection_spin: 0.0,
            tilting: 0.0,
            pitch_angle: 0.0,
            pitch_speed: 0.0,
            pitch_angle_o: 0.0,
            pitch_speed_o: 0.0,
            yaw_angle: 0.0,
            yaw_speed: 0.0,
            yaw_angle_o: 0.0,
            yaw_speed_o: 0.0,
            prev_pitch: 0.0,
            prev_yaw: 0.0,
            sword_attack: true,
            sword_attack2: false,
            prev_swing_m: false,
            swing_m_hand_prev: false,
            swing_o_hand_prev: false,
            swing_count_prev: 0,
            prev_main_item: "minecraft:air".to_owned(),
            prev_off_item: "minecraft:air".to_owned(),
            main_switch_event: false,
            off_switch_event: false,
            main_hand_swinging: false,
            main_hand_swing_ticks: 0,
            main_hand_swing_progress: 0.0,
            last_main_hand_swing_progress: 0.0,
            off_hand_swinging: false,
            off_hand_swing_ticks: 0,
            off_hand_swing_progress: 0.0,
            last_off_hand_swing_progress: 0.0,
            swing_m_hand: false,
            swing_o_hand: false,
            main_swing_count: 0,
            main_swing_ticked_20tps: false,
            off_swing_ticked_20tps: false,
            last_haste: None,
            last_fatigue: None,
            block_breaking: false,
            angles_initialized: false,
            particles: Vec::new(),
            rng_state: 0x9E37_79B9_7F4A_7C15,
            debug_lines: Vec::new(),
            overrides: LuaOverrides::default(),
        }
    }
}

impl HmiState {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        ((x >> 40) as u32) as f32 / ((1u32 << 24) as f32)
    }

    #[inline]
    pub fn modify_swing_max(base: u32, dig_speed: Option<i32>, fatigue: Option<i32>) -> u32 {
        let b = base as i32;
        let modified = if let Some(amp) = dig_speed {
            b - (1 + amp)
        } else if let Some(amp) = fatigue {
            b + (1 + amp) * 2
        } else {
            b
        };
        modified.max(2) as u32
    }

    /// One swing's length in ticks before haste/fatigue: the item's own speed, capped at
    /// `MINING_SWING_TICKS` for the main hand while a block is being broken so the swing
    /// repeats as fast as the block progresses.
    fn swing_base(&self, item: &ItemFlags, config: &Config, main_hand: bool) -> u32 {
        let ticks = item.swing_duration(config.swing_speed, &self.overrides);
        if main_hand && self.block_breaking {
            ticks.min(MINING_SWING_TICKS)
        } else {
            ticks
        }
    }

    pub fn clear_active_swings(&mut self) {
        self.main_hand_swinging = false;
        self.main_hand_swing_ticks = 0;
        self.main_hand_swing_progress = 0.0;
        self.last_main_hand_swing_progress = 0.0;
        self.off_hand_swinging = false;
        self.off_hand_swing_ticks = 0;
        self.off_hand_swing_progress = 0.0;
        self.last_off_hand_swing_progress = 0.0;
        self.main_swing_ticked_20tps = false;
        self.off_swing_ticked_20tps = false;
    }

    pub fn main_swing(&self, partial: f32) -> f32 {
        if !self.main_hand_swinging {
            return 0.0;
        }
        let d = (self.main_hand_swing_progress - self.last_main_hand_swing_progress).max(0.0);
        (self.last_main_hand_swing_progress + d * partial.clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }

    pub fn off_swing(&self, partial: f32) -> f32 {
        if !self.off_hand_swinging {
            return 0.0;
        }
        let d = (self.off_hand_swing_progress - self.last_off_hand_swing_progress).max(0.0);
        (self.last_off_hand_swing_progress + d * partial.clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }

    /// Replicates `LivingEntityMixin.swingHand(Hand)` for discrete swings.
    pub fn on_swing(&mut self, main_hand: bool, item: &ItemFlags, config: &Config) {
        self.on_swing_ex(main_hand, item, config, false);
    }

    /// Swing start handler that distinguishes a held button / continuous block-mining repeat
    /// (`continuous_hold = true`, where `swing_hand` is invoked every tick while the button stays
    /// down) from a discrete mouse click (`continuous_hold = false`).
    pub fn on_swing_ex(
        &mut self,
        main_hand: bool,
        item: &ItemFlags,
        config: &Config,
        continuous_hold: bool,
    ) {
        let max_ticks = Self::modify_swing_max(
            self.swing_base(item, config, main_hand),
            self.last_haste,
            self.last_fatigue,
        );
        // When holding the mouse button down continuously (`continuous_hold`), let the current
        // swing complete all the way to 1.0 (`!self.main_hand_swinging`) before starting the next
        // cycle so mining animations never cut off mid-swing or jitter.
        // For discrete mouse clicks (`!continuous_hold`), allow immediate re-swing after a brief
        // debounce (`progress >= 0.08`) so fast clicks are always registered without double-firing
        // within the same frame.
        let min_reclick_progress = (1.0 / (max_ticks as f32)).min(0.08);
        if main_hand {
            let can_start = if continuous_hold {
                !self.main_hand_swinging
            } else {
                !self.main_hand_swinging
                    || self.main_hand_swing_progress >= min_reclick_progress
                    || self.main_hand_swing_progress == 0.0
            };
            if can_start {
                let already_just_started =
                    self.main_hand_swinging && self.main_hand_swing_progress == 0.0;
                self.main_hand_swing_ticks = 0;
                self.main_hand_swing_progress = 0.0;
                self.last_main_hand_swing_progress = 0.0;
                self.main_hand_swinging = true;
                self.main_swing_ticked_20tps = false;
                if !already_just_started {
                    self.swing_m_hand = !self.swing_m_hand;
                    self.main_swing_count = self.main_swing_count.wrapping_add(1);
                }
            }
        } else {
            let can_start = if continuous_hold {
                !self.off_hand_swinging
            } else {
                !self.off_hand_swinging
                    || self.off_hand_swing_progress >= min_reclick_progress
                    || self.off_hand_swing_progress == 0.0
            };
            if can_start {
                let already_just_started =
                    self.off_hand_swinging && self.off_hand_swing_progress == 0.0;
                self.off_hand_swing_ticks = 0;
                self.off_hand_swing_progress = 0.0;
                self.last_off_hand_swing_progress = 0.0;
                self.off_hand_swinging = true;
                self.off_swing_ticked_20tps = false;
                if !already_just_started {
                    self.swing_o_hand = !self.swing_o_hand;
                }
            }
        }
    }

    /// Replicates `LivingEntityMixin.tick` (20 TPS per-hand swing progress update for tests/callers
    /// that step swings by integer ticks).
    pub fn tick_swings(
        &mut self,
        main_item: &ItemFlags,
        off_item: &ItemFlags,
        dig_speed: Option<i32>,
        fatigue: Option<i32>,
        config: &Config,
    ) {
        self.last_haste = dig_speed;
        self.last_fatigue = fatigue;

        let max_main = Self::modify_swing_max(
            self.swing_base(main_item, config, true),
            dig_speed,
            fatigue,
        );
        self.last_main_hand_swing_progress = self.main_hand_swing_progress;
        if self.main_hand_swinging {
            self.main_hand_swing_ticks += 1;
            if self.main_hand_swing_ticks >= max_main {
                self.main_hand_swing_ticks = 0;
                self.main_hand_swinging = false;
                self.main_hand_swing_progress = 0.0;
                self.last_main_hand_swing_progress = 0.0;
            } else {
                self.main_hand_swing_progress =
                    self.main_hand_swing_ticks as f32 / max_main as f32;
            }
            self.main_swing_ticked_20tps = true;
        } else {
            self.main_hand_swing_ticks = 0;
            self.main_hand_swing_progress = 0.0;
            self.last_main_hand_swing_progress = 0.0;
            self.main_swing_ticked_20tps = false;
        }

        let max_off = Self::modify_swing_max(
            off_item.swing_duration(config.swing_speed, &self.overrides),
            dig_speed,
            fatigue,
        );
        self.last_off_hand_swing_progress = self.off_hand_swing_progress;
        if self.off_hand_swinging {
            self.off_hand_swing_ticks += 1;
            if self.off_hand_swing_ticks >= max_off {
                self.off_hand_swing_ticks = 0;
                self.off_hand_swinging = false;
                self.off_hand_swing_progress = 0.0;
                self.last_off_hand_swing_progress = 0.0;
            } else {
                self.off_hand_swing_progress =
                    self.off_hand_swing_ticks as f32 / max_off as f32;
            }
            self.off_swing_ticked_20tps = true;
        } else {
            self.off_hand_swing_ticks = 0;
            self.off_hand_swing_progress = 0.0;
            self.last_off_hand_swing_progress = 0.0;
            self.off_swing_ticked_20tps = false;
        }
    }

    /// Steps frame-rate independent physics and counters once per rendered frame.
    pub fn step_frame(
        &mut self,
        dt: f32,
        partial: f32,
        main_item: &ItemFlags,
        off_item: &ItemFlags,
        input: &PlayerFrameInput,
        config: &Config,
    ) -> Vec<QueuedSound> {
        let mut sounds = Vec::new();
        self.block_breaking = input.block_breaking;
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.05) } else { 0.0 };
        let dt30 = dt * 30.0;
        let dt20 = dt * 20.0;

        // Advance per-hand swing progress directly with frame delta-time (`dt * 20.0` ticks/sec)
        // unless `tick_swings` already stepped this frame explicitly.
        if self.main_swing_ticked_20tps {
            self.main_swing_ticked_20tps = false;
        } else if dt > 0.0 {
            let max_main = Self::modify_swing_max(
                self.swing_base(main_item, config, true),
                self.last_haste,
                self.last_fatigue,
            ) as f32;
            if self.main_hand_swinging {
                self.main_hand_swing_progress += dt20 / max_main;
                if self.main_hand_swing_progress >= 1.0 {
                    self.main_hand_swinging = false;
                    self.main_hand_swing_ticks = 0;
                    self.main_hand_swing_progress = 0.0;
                    self.last_main_hand_swing_progress = 0.0;
                } else {
                    self.main_hand_swing_ticks =
                        (self.main_hand_swing_progress * max_main).floor() as u32;
                    self.last_main_hand_swing_progress = self.main_hand_swing_progress;
                }
            } else {
                self.main_hand_swing_ticks = 0;
                self.main_hand_swing_progress = 0.0;
                self.last_main_hand_swing_progress = 0.0;
            }
        }

        if self.off_swing_ticked_20tps {
            self.off_swing_ticked_20tps = false;
        } else if dt > 0.0 {
            let max_off = Self::modify_swing_max(
                off_item.swing_duration(config.swing_speed, &self.overrides),
                self.last_haste,
                self.last_fatigue,
            ) as f32;
            if self.off_hand_swinging {
                self.off_hand_swing_progress += dt20 / max_off;
                if self.off_hand_swing_progress >= 1.0 {
                    self.off_hand_swinging = false;
                    self.off_hand_swing_ticks = 0;
                    self.off_hand_swing_progress = 0.0;
                    self.last_off_hand_swing_progress = 0.0;
                } else {
                    self.off_hand_swing_ticks =
                        (self.off_hand_swing_progress * max_off).floor() as u32;
                    self.last_off_hand_swing_progress = self.off_hand_swing_progress;
                }
            } else {
                self.off_hand_swing_ticks = 0;
                self.off_hand_swing_progress = 0.0;
                self.last_off_hand_swing_progress = 0.0;
            }
        }

        let main_swing_prog = self.main_swing(partial);
        let off_swing_prog = self.off_swing(partial);

        // Detect hand item switches
        self.main_switch_event = main_item.id != self.prev_main_item;
        self.off_switch_event = off_item.id != self.prev_off_item;
        self.prev_main_item = main_item.id.clone();
        self.prev_off_item = off_item.id.clone();

        if self.main_switch_event && self.drink_count == 0.0 {
            self.main_hand_switch = 0.0;
            if config.switch_sounds && !main_item.is_empty {
                sounds.push(QueuedSound {
                    sound: "minecraft:item.armor.equip_leather",
                    volume: 0.2,
                    pitch: 1.0,
                });
                if main_item.is_nugget() {
                    sounds.push(QueuedSound {
                        sound: "minecraft:entity.experience_orb.pickup",
                        volume: 0.3,
                        pitch: 1.0,
                    });
                } else if main_item.has_switch_flip() {
                    sounds.push(QueuedSound {
                        sound: "minecraft:entity.player.attack.weak",
                        volume: 0.3,
                        pitch: 1.0,
                    });
                }
            }
        }
        self.main_hand_switch = (self.main_hand_switch + 0.015 * dt30).clamp(0.0, 1.0);

        if self.off_switch_event {
            self.off_hand_switch = 0.0;
            if config.switch_sounds && !off_item.is_empty {
                sounds.push(QueuedSound {
                    sound: "minecraft:item.armor.equip_leather",
                    volume: 0.2,
                    pitch: 1.0,
                });
            }
        }
        self.off_hand_switch = (self.off_hand_switch + 0.015 * dt30).clamp(0.0, 1.0);

        // Bell swing sound
        if self.swing_count_prev != self.main_swing_count {
            if config.switch_sounds && main_item.is_bell() {
                sounds.push(QueuedSound {
                    sound: "minecraft:block.bell.use",
                    volume: 0.3,
                    pitch: 1.0,
                });
            }
            self.swing_count_prev = self.main_swing_count;
        }

        // Sword alternating attack state
        if !input.on_ground && main_swing_prog == 0.0 {
            self.sword_attack = false;
        } else if main_swing_prog == 0.0 {
            self.sword_attack = true;
        }
        if self.prev_swing_m != self.swing_m_hand {
            if config.sword_counter_attack {
                self.sword_attack2 = !self.sword_attack2;
            } else {
                self.sword_attack2 = false;
            }
            self.prev_swing_m = self.swing_m_hand;
        }

        // Bow counters (main & offhand)
        if input.using_item && input.active_main_hand && main_item.use_kind == UseKind::Bow {
            self.bow_count = (self.bow_count + 0.075 * dt30).clamp(0.0, 1.0);
        } else {
            self.bow_count = (self.bow_count - 0.07 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && input.active_main_hand
            && main_item.use_kind == UseKind::Bow
            && self.bow_count >= 1.0
        {
            self.bow_count_sec = (self.bow_count_sec + 0.025 * dt30).clamp(0.0, 1.0);
            self.bow_wiggle += 0.07 * dt30;
        } else {
            self.bow_count_sec = 0.0;
        }

        if input.using_item && !input.active_main_hand && off_item.use_kind == UseKind::Bow {
            self.bow_count_o = (self.bow_count_o + 0.075 * dt30).clamp(0.0, 1.0);
        } else {
            self.bow_count_o = (self.bow_count_o - 0.07 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && !input.active_main_hand
            && off_item.use_kind == UseKind::Bow
            && self.bow_count_o >= 1.0
        {
            self.bow_count_sec_o = (self.bow_count_sec_o + 0.025 * dt30).clamp(0.0, 1.0);
            self.bow_wiggle_o += 0.07 * dt30;
        } else {
            self.bow_count_sec_o = (self.bow_count_sec_o - 0.11 * dt30).clamp(0.0, 1.0);
        }

        // Crossbow counters
        if input.using_item
            && input.active_main_hand
            && main_item.use_kind == UseKind::Crossbow
            && !main_item.is_charged_crossbow
        {
            self.cross_bow_m = (self.cross_bow_m + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.cross_bow_m = (self.cross_bow_m - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && input.active_main_hand
            && main_item.use_kind == UseKind::Crossbow
            && self.cross_bow_m >= 1.0
            && !main_item.is_charged_crossbow
        {
            self.cross_bow_sec_m = (self.cross_bow_sec_m + 0.02 * dt30).clamp(0.0, 1.0);
        } else {
            self.cross_bow_sec_m = (self.cross_bow_sec_m - 0.1 * dt30).clamp(0.0, 1.0);
        }

        if input.using_item && !input.active_main_hand && off_item.use_kind == UseKind::Crossbow {
            self.cross_bow_o = (self.cross_bow_o + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.cross_bow_o = (self.cross_bow_o - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && !input.active_main_hand
            && off_item.use_kind == UseKind::Crossbow
            && self.cross_bow_o >= 1.0
        {
            self.cross_bow_sec_o = (self.cross_bow_sec_o + 0.02 * dt30).clamp(0.0, 1.0);
        } else {
            self.cross_bow_sec_o = (self.cross_bow_sec_o - 0.1 * dt30).clamp(0.0, 1.0);
        }

        // Eat / Drink / Horn / Brush counters
        let main_eating = input.using_item
            && input.active_main_hand
            && matches!(
                main_item.use_kind,
                UseKind::Eat | UseKind::Drink | UseKind::TootHorn
            );
        if main_eating {
            self.food_count = (self.food_count + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.food_count = (self.food_count - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && input.active_main_hand
            && matches!(
                main_item.use_kind,
                UseKind::Eat | UseKind::Drink | UseKind::TootHorn | UseKind::Brush
            )
            && (self.food_count >= 1.0 || self.brush_counter > 0.4)
        {
            self.food_count_sec += 0.1 * dt30;
        }
        if input.using_item
            && input.active_main_hand
            && main_item.use_kind == UseKind::Drink
            && self.food_count >= 1.0
        {
            self.drink_count = (self.drink_count + 0.04 * dt30).clamp(0.0, 1.0);
        } else {
            self.drink_count = (self.drink_count - 0.1 * dt30).clamp(0.0, 1.0);
        }

        let off_eating = input.using_item
            && !input.active_main_hand
            && matches!(
                off_item.use_kind,
                UseKind::Eat | UseKind::Drink | UseKind::TootHorn
            );
        if off_eating {
            self.food_count_o = (self.food_count_o + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.food_count_o = (self.food_count_o - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item
            && !input.active_main_hand
            && matches!(
                off_item.use_kind,
                UseKind::Eat | UseKind::Drink | UseKind::TootHorn | UseKind::Brush
            )
            && (self.food_count_o >= 1.0 || self.brush_counter_o > 0.4)
        {
            self.food_count_sec_o += 0.1 * dt30;
        }
        if input.using_item
            && !input.active_main_hand
            && off_item.use_kind == UseKind::Drink
            && self.food_count_o >= 1.0
        {
            self.drink_count_o = (self.drink_count_o + 0.04 * dt30).clamp(0.0, 1.0);
        } else {
            self.drink_count_o = (self.drink_count_o - 0.1 * dt30).clamp(0.0, 1.0);
        }

        // Brush counters & spring physics
        if input.using_item && input.active_main_hand && main_item.use_kind == UseKind::Brush {
            self.brush_counter = (self.brush_counter + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.brush_counter = (self.brush_counter - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.using_item && !input.active_main_hand && off_item.use_kind == UseKind::Brush {
            self.brush_counter_o = (self.brush_counter_o + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.brush_counter_o = (self.brush_counter_o - 0.1 * dt30).clamp(0.0, 1.0);
        }

        self.brush_speed_m += ((self.food_count_sec * 4.14).sin() * self.brush_counter) * dt30;
        self.brush_speed_m -= 0.04 * self.brush_angle_m * dt30;
        self.brush_speed_m *= 0.85_f32.powf(dt30);
        self.brush_angle_m += self.brush_speed_m * dt30;

        self.brush_speed_o += ((self.food_count_sec_o * 4.14).sin() * self.brush_counter_o) * dt30;
        self.brush_speed_o -= 0.04 * self.brush_angle_o * dt30;
        self.brush_speed_o *= 0.85_f32.powf(dt30);
        self.brush_angle_o += self.brush_speed_o * dt30;

        // Trident & Riptide counters
        if input.using_item && input.active_main_hand && main_item.use_kind == UseKind::Trident {
            self.trident_m = (self.trident_m + 0.07 * dt30).clamp(0.0, 1.0);
            self.trident = (self.trident + 0.05 * dt30).clamp(0.0, 1.0);
            self.trident_j += 0.1 * dt30;
        } else {
            if input.using_riptide {
                self.trident_m -= 0.57 * dt30;
                self.trident -= 0.53 * dt30;
            } else {
                self.trident_m -= 0.1 * dt30;
                self.trident -= 0.07 * dt30;
            }
            self.trident_j = (self.trident_j - 0.1 * dt30).max(0.0);
        }
        self.trident *= 0.95_f32.powf(dt30);
        self.trident_m = self.trident_m.clamp(0.0, 1.0);
        self.trident = self.trident.clamp(0.0, 1.0);

        if input.using_item && !input.active_main_hand && off_item.use_kind == UseKind::Trident {
            self.trident_m_o = (self.trident_m_o + 0.07 * dt30).clamp(0.0, 1.0);
            self.trident_o = (self.trident_o + 0.045 * dt30).clamp(0.0, 1.0);
            self.trident_j_o += 0.1 * dt30;
        } else {
            if input.using_riptide {
                self.trident_m_o -= 0.57 * dt30;
                self.trident_o -= 0.53 * dt30;
            } else {
                self.trident_m_o -= 0.1 * dt30;
                self.trident_o -= 0.07 * dt30;
            }
            self.trident_j_o = (self.trident_j_o - 0.1 * dt30).max(0.0);
        }
        self.trident_m_o = self.trident_m_o.clamp(0.0, 1.0);
        self.trident_o = self.trident_o.clamp(0.0, 1.0);

        if input.using_riptide && main_item.use_kind == UseKind::Trident && input.active_main_hand {
            self.riptide_counter += 0.08 * dt30;
        } else {
            self.riptide_counter -= 0.025 * dt30;
        }
        if input.using_riptide && off_item.use_kind == UseKind::Trident && !input.active_main_hand {
            self.riptide_counter_o += 0.08 * dt30;
        } else {
            self.riptide_counter_o -= 0.025 * dt30;
        }
        self.riptide_counter = self.riptide_counter.clamp(0.0, 1.0)
            * 0.95_f32.powf(dt30)
            * (1.0 - (main_swing_prog * 4.0).clamp(0.0, 1.0))
            * (1.0 - (self.trident_m * 6.0).clamp(0.0, 1.0));
        self.riptide_counter_o = self.riptide_counter_o.clamp(0.0, 1.0)
            * 0.95_f32.powf(dt30)
            * (1.0 - (off_swing_prog * 4.0).clamp(0.0, 1.0))
            * (1.0 - (self.trident_m_o * 6.0).clamp(0.0, 1.0));

        // Spear counters
        let swing_main_sin = ((easings::ease_custom_sec(main_swing_prog) * 4.78).sin()).clamp(0.0, 1.0);
        let swing_off_sin = ((easings::ease_custom_sec(off_swing_prog) * 4.78).sin()).clamp(0.0, 1.0);
        let prev_dismount = self.can_dismount_counter;
        let prev_knockback = self.can_knockback_counter;
        if input.using_item
            && input.active_main_hand
            && main_item.is_spear()
            && main_item.spear_can_damage
        {
            self.spear_counter_m += 0.08 * dt30;
            if !main_item.spear_can_dismount {
                self.can_dismount_counter += 0.035 * dt30;
            }
            if !main_item.spear_can_knockback {
                self.can_knockback_counter += 0.055 * dt30;
            }
            if main_item.spear_hit_impact || self.hit_impact_counter > 0.0 {
                self.hit_impact_counter += 0.08 * dt30;
            }
        } else {
            self.spear_counter_m -= 0.08 * dt30;
            self.can_dismount_counter = 0.0;
            self.can_knockback_counter = 0.0;
            self.hit_impact_counter = 0.0;
        }
        if self.hit_impact_counter >= 1.0 {
            self.hit_impact_counter = 0.0;
        }
        self.spear_counter_m = self.spear_counter_m.clamp(0.0, 1.0)
            * (1.0 - (swing_main_sin * 8.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        self.can_dismount_counter = self.can_dismount_counter.clamp(0.0, 1.0);
        self.can_knockback_counter = self.can_knockback_counter.clamp(0.0, 1.0);
        if self.can_dismount_counter == 0.0 {
            self.can_dismount_counter =
                prev_dismount + (0.0 - prev_dismount) * (0.5 * dt30).clamp(0.0, 1.0);
        }
        if self.can_knockback_counter == 0.0 {
            self.can_knockback_counter =
                prev_knockback + (0.0 - prev_knockback) * (0.5 * dt30).clamp(0.0, 1.0);
        }

        let prev_dismount_o = self.can_dismount_counter_o;
        let prev_knockback_o = self.can_knockback_counter_o;
        if input.using_item
            && !input.active_main_hand
            && off_item.is_spear()
            && off_item.spear_can_damage
        {
            self.spear_counter_o += 0.08 * dt30;
            if !off_item.spear_can_dismount {
                self.can_dismount_counter_o += 0.035 * dt30;
            }
            if !off_item.spear_can_knockback {
                self.can_knockback_counter_o += 0.055 * dt30;
            }
        } else {
            self.spear_counter_o -= 0.08 * dt30;
            self.can_dismount_counter_o = 0.0;
            self.can_knockback_counter_o = 0.0;
        }
        self.spear_counter_o = self.spear_counter_o.clamp(0.0, 1.0)
            * (1.0 - (swing_off_sin * 8.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        self.can_dismount_counter_o = self.can_dismount_counter_o.clamp(0.0, 1.0);
        self.can_knockback_counter_o = self.can_knockback_counter_o.clamp(0.0, 1.0);
        if self.can_dismount_counter_o == 0.0 {
            self.can_dismount_counter_o =
                prev_dismount_o + (0.0 - prev_dismount_o) * (0.5 * dt30).clamp(0.0, 1.0);
        }
        if self.can_knockback_counter_o == 0.0 {
            self.can_knockback_counter_o =
                prev_knockback_o + (0.0 - prev_knockback_o) * (0.5 * dt30).clamp(0.0, 1.0);
        }

        // Shield counters
        if (main_item.use_kind == UseKind::Block && main_item.is_cooling_down)
            || (off_item.use_kind == UseKind::Block && off_item.is_cooling_down)
        {
            self.shield_disable = (self.shield_disable + 0.04 * dt30).clamp(0.0, 1.0);
        } else {
            self.shield_disable = (self.shield_disable - 0.06 * dt30).clamp(0.0, 1.0);
        }

        let main_shield_step = if main_item.is_sword() { 0.12 } else { 0.07 };
        if input.using_item && input.active_main_hand && main_item.use_kind == UseKind::Block {
            self.shield_m += main_shield_step * dt30;
        } else {
            self.shield_m -= main_shield_step * dt30;
        }
        self.shield_m = (self.shield_m - self.shield_disable * 0.04 * dt30).clamp(0.0, 1.0);

        if input.using_item && !input.active_main_hand && off_item.use_kind == UseKind::Block {
            self.shield_o += 0.07 * dt30;
        } else {
            self.shield_o -= 0.07 * dt30;
        }
        self.shield_o = (self.shield_o - self.shield_disable * 0.04 * dt30).clamp(0.0, 1.0);

        // Offhand auto-hide counter
        let main_two_handed_use = input.using_item
            && !main_item.is_charged_crossbow
            && !matches!(
                main_item.use_kind,
                UseKind::Block
                    | UseKind::Eat
                    | UseKind::Drink
                    | UseKind::TootHorn
                    | UseKind::Brush
                    | UseKind::Spear
            );
        if off_item.is_empty
            && !main_two_handed_use
            && !input.climbing
            && !input.swimming
            && !input.crawling
        {
            self.offhand = (self.offhand + 0.08 * dt30).clamp(0.0, 1.0);
        } else {
            self.offhand = (self.offhand - 0.08 * dt30).clamp(0.0, 1.0);
        }
        if self.bow_count > 0.0 {
            let eased_bow = easings::ease_in_out_back(self.bow_count);
            self.offhand = if (eased_bow - 1.0).abs() < 1e-4 {
                0.0
            } else {
                (1.0 - eased_bow).clamp(0.0, 1.0)
            };
        }

        // Walk, Fall, Sneak, Climb, Crawl, Swim
        if input.speed > 0.08 && input.y_speed.abs() < 0.08 && input.on_ground {
            self.walk += input.speed * dt30;
            self.walk_smoother = (self.walk_smoother + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.walk_smoother = (self.walk_smoother - 0.1 * dt30).clamp(0.0, 1.0);
        }

        if input.speed > 0.05 {
            self.water_walk += input.speed * 2.0 * dt30;
        }
        if input.touching_water {
            self.in_water_count = (self.in_water_count + 0.07 * dt30).clamp(0.0, 1.0);
        } else {
            self.in_water_count = (self.in_water_count - 0.07 * dt30).clamp(0.0, 1.0);
        }

        self.fall_speed += (-input.y_speed
            + (self.sneak * 3.14).sin() * 0.14
            + (self.bow_count * 3.14).sin() * 0.12
            + (self.bow_count_o * 3.14).sin() * 0.12)
            * 0.27
            * dt30;
        self.fall_speed -= 0.1 * self.fall * dt30;
        self.fall_speed *= 0.82_f32.powf(dt30);
        self.fall += self.fall_speed * dt30;

        self.jiggle_i += input.y_speed * dt30;
        if !input.on_ground && -input.y_speed > 0.55 {
            self.fall_f = (self.fall_f + 0.045 * dt30).clamp(0.0, 1.0);
        } else {
            self.fall_f = (self.fall_f - 0.07 * dt30).clamp(0.0, 1.0);
        }

        if input.sneaking {
            self.sneak = (self.sneak + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.sneak = (self.sneak - 0.1 * dt30).clamp(0.0, 1.0);
        }

        self.a += 0.04 * dt30;

        if input.climbing {
            self.smoothing = (self.smoothing + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.smoothing = (self.smoothing - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.climbing && input.y_speed.abs() > 0.08 {
            if input.y_speed > 0.0 {
                self.crawler += input.y_speed * dt30;
            } else {
                self.crawler += (input.y_speed * 0.5) * dt30;
            }
        }

        if input.crawling {
            self.smoothing_crawl = (self.smoothing_crawl + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.smoothing_crawl = (self.smoothing_crawl - 0.1 * dt30).clamp(0.0, 1.0);
        }
        if input.crawling && input.speed > 0.08 {
            self.crawl_default_pos = (self.crawl_default_pos + 0.1 * dt30).clamp(0.0, 1.0);
            self.crwl += input.speed * dt30;
        } else {
            self.crawl_default_pos = (self.crawl_default_pos - 0.06 * dt30).clamp(0.0, 1.0);
        }

        if input.swimming && !input.using_item {
            self.swim_counter += input.speed * dt30;
            self.swim_smoother = (self.swim_smoother + 0.1 * dt30).clamp(0.0, 1.0);
        } else {
            self.swim_smoother = (self.swim_smoother - 0.1 * dt30).clamp(0.0, 1.0);
        }

        // Map smoother & zoomer
        if main_item.is_filled_map() && off_item.is_empty {
            self.map_smoother = (self.map_smoother + 0.07 * dt30).clamp(0.0, 1.0);
        } else if main_item.is_filled_map() || off_item.is_filled_map() {
            self.map_smoother = (self.map_smoother - 0.07 * dt30).clamp(0.0, 1.0);
        }
        if input.pitch > 40.0 {
            self.map_zoomer = (self.map_zoomer + 0.05 * dt30).clamp(0.0, 1.0);
        } else {
            self.map_zoomer = (self.map_zoomer - 0.095 * dt30).clamp(0.0, 1.0);
        }

        // Inspection keybind counter & spin
        if input.inspect_held {
            self.inspection_counter = (self.inspection_counter + 0.04 * dt30).clamp(0.0, 1.0);
            self.inspection_spin = (self.inspection_spin + 0.025 * dt30).clamp(0.0, 1.0);
        } else {
            self.inspection_counter = (self.inspection_counter - 0.04 * dt30).clamp(0.0, 1.0);
            self.inspection_spin = 0.0;
        }

        self.tilting = (self.tilting + main_swing_prog * dt * 3.0).rem_euclid(628.31853);
        self.a = self.a.rem_euclid(628.31853);
        self.walk = self.walk.rem_euclid(628.31853);
        self.water_walk = self.water_walk.rem_euclid(628.31853);
        self.swim_counter = self.swim_counter.rem_euclid(628.31853);
        self.crawler = self.crawler.rem_euclid(628.31853);
        self.crwl = self.crwl.rem_euclid(628.31853);
        self.bow_wiggle = self.bow_wiggle.rem_euclid(628.31853);
        self.bow_wiggle_o = self.bow_wiggle_o.rem_euclid(628.31853);
        self.food_count_sec = self.food_count_sec.rem_euclid(628.31853);
        self.food_count_sec_o = self.food_count_sec_o.rem_euclid(628.31853);
        self.jiggle_i = self.jiggle_i.rem_euclid(628.31853);

        // Food speed camera feedback (only active when food_count > 0.80041)
        let sinal_food_speed = if self.food_count > 0.80041 {
            (self.food_count.clamp(0.80041, 1.0) * 3.14 * 5.0).sin() * 0.45
        } else {
            0.0
        };
        self.food_speed += sinal_food_speed * dt30;
        self.food_speed *= 0.8_f32.powf(dt30);

        // Pitch & Yaw spring-damper inertia (item_pose.lua lines 163-201)
        let sp = if main_item.is_spear() { 1.0 } else { 0.0 };
        let spo = if off_item.is_spear() { 1.0 } else { 0.0 };
        if !self.angles_initialized {
            self.prev_pitch = input.pitch;
            self.prev_yaw = input.yaw;
            self.angles_initialized = true;
        }
        let mut d_pitch = if input.pitch.is_finite() && self.prev_pitch.is_finite() {
            input.pitch - self.prev_pitch
        } else {
            0.0
        };
        let mut d_yaw = if input.yaw.is_finite() && self.prev_yaw.is_finite() {
            (input.yaw - self.prev_yaw + 180.0).rem_euclid(360.0) - 180.0
        } else {
            0.0
        };
        d_pitch = d_pitch.clamp(-45.0, 45.0);
        d_yaw = d_yaw.clamp(-45.0, 45.0);
        self.prev_pitch = input.pitch;
        self.prev_yaw = input.yaw;

        self.pitch_speed += ((-input.speed * 22.0 * self.walk_smoother)
            - (main_swing_prog * 3.14).sin() * 8.0
            + self.fall * 3.0
            + (self.sneak * 3.14).sin() * 0.3
            + d_pitch)
            * 0.15
            * dt30;
        if main_item.use_kind == UseKind::Block && !main_item.is_sword() {
            self.pitch_speed += 10.0 * (self.shield_disable * 3.14).sin() * 0.15 * dt30;
            self.pitch_speed += 12.0 * (self.shield_m * 3.14).sin() * 0.15 * dt30;
        }
        self.pitch_speed += ((-20.0 * (self.can_dismount_counter * 3.14).sin() * self.spear_counter_m)
            + (20.0 * (self.can_knockback_counter * 3.14).sin() * self.spear_counter_m)
            + (12.0 * (self.inspection_counter * 3.14).sin())
            + (15.0 * (self.spear_counter_m * 3.14).sin())
            + (-10.0 * ((easings::ease_in_back(self.hit_impact_counter) * 6.28).sin()).clamp(0.0, 1.0))
            + (40.0 * (((self.main_hand_switch * 1.5 * sp).clamp(0.0, 1.0) * 6.28).sin()).clamp(0.0, 1.0)))
            * 0.15
            * dt30;
        self.pitch_speed -= 0.04 * self.pitch_angle * dt30;
        self.pitch_speed *= 0.85_f32.powf(dt30);
        self.pitch_angle += self.pitch_speed * dt30;

        self.yaw_speed += (self.walk.sin() * 3.0 * self.walk_smoother
            + (main_swing_prog * 3.14).sin() * 8.0
            + (self.swim_counter * self.swim_smoother).sin() * 3.0
            + (self.main_hand_switch * 6.28).sin() * 3.0
            + d_yaw)
            * 0.15
            * dt30;
        self.yaw_speed -= 0.04 * self.yaw_angle * dt30;
        self.yaw_speed *= 0.85_f32.powf(dt30);
        self.yaw_angle += self.yaw_speed * dt30;

        self.pitch_speed_o += ((-input.speed * 22.0 * self.walk_smoother)
            - (off_swing_prog * 3.14).sin() * 8.0
            + self.fall * 3.0
            + (self.sneak * 3.14).sin() * 0.3
            + d_pitch)
            * 0.15
            * dt30;
        if off_item.use_kind == UseKind::Block && !off_item.is_sword() {
            self.pitch_speed_o += 10.0 * (self.shield_disable * 3.14).sin() * 0.15 * dt30;
            self.pitch_speed_o += 12.0 * (self.shield_o * 3.14).sin() * 0.15 * dt30;
        }
        self.pitch_speed_o += ((-20.0 * (self.can_dismount_counter_o * 3.14).sin() * self.spear_counter_o)
            + (20.0 * (self.can_knockback_counter_o * 3.14).sin() * self.spear_counter_o)
            + (15.0 * (self.spear_counter_o * 3.14).sin())
            + (40.0 * (((self.off_hand_switch * 1.5 * spo).clamp(0.0, 1.0) * 6.28).sin()).clamp(0.0, 1.0)))
            * 0.15
            * dt30;
        self.pitch_speed_o -= 0.04 * self.pitch_angle_o * dt30;
        self.pitch_speed_o *= 0.85_f32.powf(dt30);
        self.pitch_angle_o += self.pitch_speed_o * dt30;

        self.yaw_speed_o += (self.walk.sin() * 3.0 * self.walk_smoother
            + (off_swing_prog * 3.14).sin() * 8.0
            + (self.swim_counter * self.swim_smoother).sin() * 3.0
            + (self.off_hand_switch * 6.28).sin() * 3.0
            + d_yaw)
            * 0.15
            * dt30;
        self.yaw_speed_o -= 0.04 * self.yaw_angle_o * dt30;
        self.yaw_speed_o *= 0.85_f32.powf(dt30);
        self.yaw_angle_o += self.yaw_speed_o * dt30;

        // Tick existing particles & spawn new ones if enabled
        for p in &mut self.particles {
            p.tick(dt30, input.age_ticks);
        }
        self.particles.retain(|p| !p.dead);

        if config.hand_particles {
            for (is_main, item) in [(true, main_item), (false, off_item)] {
                let l = if is_main { 1.0 } else { -1.0 };
                self.spawn_item_particles(is_main, l, item, input.age_ticks, &mut sounds);
            }
        } else {
            self.particles.clear();
        }

        self.swing_m_hand_prev = self.swing_m_hand;
        self.swing_o_hand_prev = self.swing_o_hand;

        sounds
    }

    fn spawn_item_particles(
        &mut self,
        main_hand: bool,
        l: f32,
        item: &ItemFlags,
        age_ticks: f32,
        sounds: &mut Vec<QueuedSound>,
    ) {
        let s = item.short_name();
        let pulse_200 = 200.0 + 20.0 * (age_ticks * 0.2).sin();
        let pulse_110 = 110.0 + 10.0 * (age_ticks * 0.2).sin();

        // Continuous SPAWN glow particles on light-emitting held items (`item_pose.lua` lines 614-628 & 874-899)
        let glow: Option<(f32, f32, f32, f32, &'static str, f32)> =
            if s == "brewing_stand" || s == "torch" {
                Some((0.5 * l, 0.6, 0.5, 1.3, "particle/orange_glow", pulse_200))
            } else if s == "lantern" || (item.is_lantern() && s.contains("copper")) {
                Some((0.45 * l, 0.15, 0.5, 1.3, "particle/orange_glow", pulse_200))
            } else if s == "soul_torch" {
                Some((0.5 * l, 0.6, 0.5, 1.3, "particle/blue_glow", pulse_110))
            } else if s == "soul_lantern" {
                Some((0.45 * l, 0.15, 0.5, 1.3, "particle/blue_glow", pulse_110))
            } else if s == "redstone_torch" {
                Some((0.5 * l, 0.6, 0.5, 1.3, "particle/red_glow", pulse_110))
            } else if s == "lava_bucket" {
                Some((
                    -0.05 * l,
                    0.0,
                    0.0,
                    2.0,
                    "particle/orange_glow",
                    150.0 + 20.0 * (age_ticks * 0.2).sin(),
                ))
            } else {
                None
            };

        if let Some((x, y, z, max_scale, sprite, alpha)) = glow {
            self.particles.push(HmiParticle {
                main_hand,
                space: ParticleSpace::Item,
                blend: ParticleBlend::Additive,
                lifecycle: ParticleLifecycle::Spawn,
                sprite,
                tint_rgb: 0xFFFFFF,
                gravity: false,
                x,
                y,
                z,
                dx: 0.0,
                dy: 0.0,
                dz: 0.0,
                scale: max_scale,
                max_scale,
                alpha,
                lifetime: 0.0,
                is_birth: true,
                dead: false,
                drift_wave: false,
            });
        }

        // Flower / petal / leaf litter burst particles on swing (`item_pose.lua` lines 649-698)
        let swung = if main_hand {
            self.swing_m_hand_prev != self.swing_m_hand
        } else {
            self.swing_o_hand_prev != self.swing_o_hand
        };
        if swung && item.is_flower_litter() {
            sounds.push(QueuedSound {
                sound: "minecraft:block.leaf_litter.place",
                volume: 0.7,
                pitch: 1.0,
            });
            let (s1, s2, s4) = match s {
                "pink_petals" => (
                    "particle/pink_petals_1",
                    "particle/pink_petals_2",
                    "particle/pink_petals_4",
                ),
                "wildflowers" => (
                    "particle/wild_flowers_1",
                    "particle/wild_flowers_2",
                    "particle/wild_flowers_4",
                ),
                _ => (
                    "particle/leaf_litter_1",
                    "particle/leaf_litter_2",
                    "particle/leaf_litter_4",
                ),
            };
            let specs: [(&'static str, f32, f32, f32); 8] = [
                ("particle/firefly", 0.4, 0.75 * l, -0.2),
                ("particle/firefly", 0.4, 0.75 * l, -0.2),
                (s1, 0.3, 0.65 * l, -0.25),
                (s1, 0.3, 0.65 * l, -0.25),
                (s2, 0.3, 0.65 * l, -0.25),
                (s2, 0.3, 0.65 * l, -0.25),
                (s4, 0.2, 0.65 * l, -0.25),
                (s4, 0.2, 0.65 * l, -0.25),
            ];
            for (sprite, max_scale, px, py) in specs {
                let rx = (self.next_f32() * 0.12 - 0.06) * l;
                let ry = self.next_f32() * 0.12;
                self.particles.push(HmiParticle {
                    main_hand,
                    space: ParticleSpace::Screen,
                    blend: ParticleBlend::Translucent,
                    lifecycle: ParticleLifecycle::Opacity,
                    sprite,
                    tint_rgb: 0xFFFFFF,
                    gravity: true,
                    x: px,
                    y: py,
                    z: -0.9,
                    dx: rx,
                    dy: ry,
                    dz: 0.0,
                    scale: max_scale,
                    max_scale,
                    alpha: 255.0,
                    lifetime: 1.0,
                    is_birth: true,
                    dead: false,
                    drift_wave: true,
                });
            }
        }
    }

    /// Evaluates the full HMI 5.1.1 matrix hierarchy (`scenePoseMain`, `mainHandPose`, `itemPose`, and `item_model.lua`)
    /// for one hand (`main_hand`: true for main hand, false for offhand; `right_arm`: true for right arm).
    pub fn evaluate_hand(
        &self,
        main_hand: bool,
        right_arm: bool,
        raw_item: &ItemFlags,
        other_item: &ItemFlags,
        partial: f32,
        input: &PlayerFrameInput,
        config: &Config,
    ) -> HandEvaluation {
        let l = if right_arm { 1.0_f32 } else { -1.0_f32 };

        // HeldItemRendererMixin lines 360-380: hide non-active hand's item when drawing bow/crossbow/spyglass
        let active_this_hand = input.using_item && (input.active_main_hand == main_hand);
        let active_other_hand = input.using_item && (input.active_main_hand != main_hand);
        let empty_override = active_other_hand
            && (other_item.use_kind == UseKind::Spyglass
                || other_item.use_kind == UseKind::Bow
                || (other_item.use_kind == UseKind::Crossbow && !other_item.is_charged_crossbow));
        let empty_flags = ItemFlags::empty();
        let item = if empty_override { &empty_flags } else { raw_item };

        let raw_swing = if main_hand {
            self.main_swing(partial)
        } else {
            self.off_swing(partial)
        };
        let mut equip_progress = if main_hand {
            input.main_equip_progress
        } else {
            input.off_equip_progress
        };
        let hand_switch_val = if main_hand {
            self.main_hand_switch
        } else {
            self.off_hand_switch
        };
        // Suppress vanilla's attack-strength / durability re-equip dip when a swing or mining is active,
        // or once HMI's own hand-switch animation (`main_hand_switch`) has finished, preventing swords
        // and held items from repeatedly chopping down after swings, durability updates, or slot syncs.
        if item.is_spear() || input.block_breaking || raw_swing > 0.0 || hand_switch_val >= 1.0 {
            equip_progress = 0.0;
        } else {
            equip_progress *= (1.0 - hand_switch_val).clamp(0.0, 1.0);
        }

        // 1. `scenePoseMain`: runs `hand_pose.lua`, then translates (0, -0.35, 0.2) if `!item.is_empty`
        let mut scene_mat = Mat4f::IDENTITY;
        self.apply_hand_pose(
            &mut scene_mat,
            main_hand,
            right_arm,
            l,
            item,
            raw_swing,
            equip_progress,
            active_this_hand,
            input,
            config,
        );
        if !item.is_empty {
            scene_mat.translate(0.0, -0.35, 0.2);
        }

        // 2. `mainHandPose`: starts from `scene_mat`, runs `hand_relative_pose.lua`, then grips item if `!item.is_empty`
        let mut arm_pose = scene_mat;
        self.apply_hand_relative_pose(&mut arm_pose, main_hand, l, item);
        if !item.is_empty {
            arm_pose.translate(1.5 * l, -0.3, -0.6);
            arm_pose.rotate_x_at(15.0, 0.5 * l, 0.5, 0.5);
            arm_pose.rotate_y_at(35.0 * l, 0.5 * l, 0.5, 0.5);
            arm_pose.rotate_z_at(-65.0 * l, 0.5 * l, 0.5, 0.5);
            arm_pose.scale(0.9, 0.9, 0.9);
        }

        // 3. `itemPose`: starts from `scene_mat`, applies base item frame, then runs `item_pose.lua`
        let mut item_pose = scene_mat;
        let render_as_3d_block =
            config.block_render_3d && item.should_render_as_block(&self.overrides);
        let item_swing = if render_as_3d_block || item.is_painting_or_frame() {
            0.0
        } else {
            raw_swing
        };

        item_pose.translate(0.5 * l, -0.15, -0.85);
        item_pose.rotate_x_at(15.0, 0.5, 0.5, 0.5);
        item_pose.scale(0.9, 0.9, 0.9);
        self.apply_item_pose(
            &mut item_pose,
            main_hand,
            right_arm,
            l,
            item,
            item_swing,
            equip_progress,
            input,
        );

        // 4. `item_model.lua`: per-quad animations for buckets, fish buckets, chest boats, etc.
        let mut animator = ModelPartAnimator::default();
        self.populate_item_model_animator(&mut animator, main_hand, right_arm, item);

        HandEvaluation {
            arm_pose,
            item_pose,
            render_as_3d_block,
            effective_empty: item.is_empty,
            animator,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_hand_pose(
        &self,
        mat: &mut Mat4f,
        main_hand: bool,
        right_arm: bool,
        l: f32,
        item: &ItemFlags,
        raw_swing: f32,
        equip_progress: f32,
        active_this_hand: bool,
        input: &PlayerFrameInput,
        config: &Config,
    ) {
        let swing_progress = if item.is_pickaxe() {
            easings::ease_custom(raw_swing)
        } else {
            easings::ease_custom_sec(raw_swing)
        };

        mat.move_x(0.2 * l);

        let swing_rot = if swing_progress < 0.70016 {
            (swing_progress.clamp(0.0, 0.308) * 5.1).sin()
        } else {
            (swing_progress.clamp(0.70016, 1.0) * 5.1 - 2.0).sin()
        };
        let swing_rot = swing_rot * swing_rot * swing_rot;

        let swing_sword_tilt = if swing_progress < 0.65245 {
            (swing_progress.clamp(0.0, 0.16675) * 3.14 * 3.0).sin()
        } else {
            (swing_progress.clamp(0.65245, 1.0) * 4.4 - 1.2584).sin()
        };

        let swing = (swing_progress * 4.78).sin().clamp(0.0, 1.0);
        let swing_hit = (swing_progress.clamp(0.16561, 0.49422) * 4.78 * 2.0 + 4.7).sin();
        let swing_hit_second = if swing_progress < 0.65594 {
            (swing_progress.clamp(0.16561, 0.32991) * 4.78 * 2.0 + 4.7).sin()
        } else {
            (swing_progress.clamp(0.65594, 0.82025) * 4.78 * 2.0 - 4.7).sin()
        };

        let pt_angle = if main_hand {
            self.pitch_angle
        } else {
            self.pitch_angle_o
        };

        let x_offset = config.x_offset;
        mat.translate(x_offset * l, config.y_offset, config.z_offset);

        if !item.is_empty {
            mat.move_z(-0.16);
        } else {
            mat.move_z(-0.08);
        }

        // Hand switch animation
        if main_hand {
            let switch_items = easings::ease_in_out_back((self.main_hand_switch.clamp(0.0, 0.5) * 3.14).sin());
            let switch_fast = (self.main_hand_switch.clamp(0.0, 0.125) * 12.56).sin();
            if item.use_kind == UseKind::Trident && self.trident_m > 0.9 {
                mat.translate(0.0, -0.15 * switch_fast, -0.3 * switch_fast);
                mat.rotate_x_at(75.0 * switch_fast, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-75.0 * switch_items, 0.3 * l, -0.4, 0.0);
                mat.translate(0.0, 0.15 * switch_fast, 0.3 * switch_fast);
            } else {
                if item.use_kind == UseKind::Crossbow {
                    mat.move_y(-0.25 * switch_fast);
                }
                mat.translate(0.0, 0.0, -0.2 * switch_fast);
                mat.rotate_y_at(25.0 * l * switch_fast, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-55.0 * switch_fast, 0.3 * l, -0.4, 0.0);
                mat.rotate_z_at(40.0 * l * switch_fast, 0.3 * l, -0.4, 0.0);

                mat.rotate_z_at(-40.0 * l * switch_items, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(55.0 * switch_items, 0.3 * l, -0.4, 0.0);
                mat.rotate_y_at(-25.0 * l * switch_items, 0.3 * l, -0.4, 0.0);
                if item.use_kind == UseKind::Crossbow {
                    mat.move_y(0.25 * switch_items);
                }
                mat.translate(0.0, 0.0, 0.2 * switch_items);
            }
        } else if self.food_count_o == 0.0 {
            let switch_items = easings::ease_in_out_back((self.off_hand_switch.clamp(0.0, 0.5) * 3.14).sin());
            let switch_fast = (self.off_hand_switch.clamp(0.0, 0.125) * 12.56).sin();
            if item.use_kind == UseKind::Crossbow {
                mat.move_y(-0.25 * switch_fast);
            }
            mat.translate(0.0, 0.0, -0.2 * switch_fast);
            mat.rotate_y_at(25.0 * switch_fast * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-55.0 * switch_fast, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(40.0 * switch_fast * l, 0.3 * l, -0.4, 0.0);

            mat.rotate_z_at(-40.0 * switch_items * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(55.0 * switch_items, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(-25.0 * switch_items * l, 0.3 * l, -0.4, 0.0);
            if item.use_kind == UseKind::Crossbow {
                mat.move_y(0.25 * switch_fast);
            }
            mat.translate(0.0, 0.0, 0.2 * switch_items);
        }

        // Sneak & Swim
        mat.move_y(-0.08 * self.sneak);
        mat.rotate_x_at(4.0 * (self.sneak * 3.14).sin(), 0.0, -0.4, 0.0);
        mat.move_z(0.3 * (self.swim_counter * 0.55).sin() * self.swim_smoother);
        mat.rotate_x_at(input.pitch * -0.03 + pt_angle * 0.1, 0.0, -0.4, 0.0);

        if item.is_empty {
            mat.translate(
                0.0,
                -0.15 * -(self.swim_counter * 0.55).cos() * self.swim_smoother
                    + 0.25 * self.swim_smoother,
                0.0,
            );
            mat.rotate_y_at(
                -15.0 * l * (self.swim_counter * 0.55).cos() * self.swim_smoother,
                0.3 * l,
                -0.4,
                0.0,
            );
            mat.rotate_x_at(
                -10.0 * (self.swim_counter * 0.55).cos() * self.swim_smoother,
                0.3 * l,
                -0.4,
                0.0,
            );
            mat.rotate_x_at(-30.0 * self.swim_smoother, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(
                -20.0 * l * (self.swim_counter * 0.55).sin() * self.swim_smoother,
                0.3 * l,
                -0.4,
                0.0,
            );
        } else {
            mat.rotate_y_at(
                -15.0 * l * (self.swim_counter * 0.55).cos() * self.swim_smoother,
                0.3 * l,
                -0.4,
                0.0,
            );
            mat.rotate_x_at(
                -10.0 * (self.swim_counter * 0.55).cos() * self.swim_smoother,
                0.3 * l,
                -0.4,
                0.0,
            );
        }

        // Base hand orientation for special items vs standard items
        if item.is_bell()
            || item.is_lantern()
            || item.is_hanging_sign()
            || item.is_flower_litter()
            || item.is_end_crystal()
            || item.is_painting_or_frame()
        {
            if item.is_flower_litter() {
                mat.translate(0.0, 0.25, -0.05);
            } else if item.is_end_crystal() {
                mat.move_z(-0.12);
                mat.rotate_x(-10.0);
            } else {
                mat.translate(0.0, -0.1, 0.05);
                mat.rotate_x(25.0);
            }
        } else if !item.is_empty && item.use_kind != UseKind::Crossbow {
            mat.move_y(-0.12);
            mat.rotate_z(-6.0 * l);
            mat.rotate_x(6.0);
        }

        // Idle breathing
        mat.move_y(0.01 * self.a.sin());
        mat.rotate_x_at(1.1 * l * self.a.cos(), 0.3 * l, -0.4, 0.0);
        mat.rotate_y_at(0.5 * self.a.sin(), 0.3 * l, -0.4, 0.0);
        mat.rotate_z_at(2.0 * (self.a * 0.3).sin(), 0.3 * l, -0.4, 0.0);

        if item.is_empty && !main_hand {
            let e = easings::ease_in_out_expo(self.offhand);
            mat.translate(0.0, -1.0 * e, 0.5 * e);
        }

        // Fall & Walk bobbing
        let fall_mul = if item.is_empty || item.is_block { 0.7 } else { 1.0 };
        if item.is_empty {
            mat.move_z(0.06 * self.fall * fall_mul);
        }
        mat.rotate_x_at(2.0 * self.fall * fall_mul, 0.0, -0.4, 0.0);
        mat.move_y(0.06 * self.fall * fall_mul);

        let walk_val = if right_arm {
            self.walk
        } else {
            self.walk - 0.75
        };
        mat.rotate_x_at(1.5 * walk_val.sin() * self.walk_smoother, 0.0, -0.4, 0.0);
        mat.rotate_y_at(
            -0.5 * (self.walk * 1.5).cos() * self.walk_smoother * l,
            0.0,
            -0.4,
            0.0,
        );
        mat.rotate_z_at(
            1.0 * (self.walk * 1.5).cos() * self.walk_smoother * l,
            0.0,
            -0.4,
            0.0,
        );

        // Shield / Sword blocking
        if item.use_kind == UseKind::Block && !item.is_sword() {
            mat.translate(0.2 * l, 0.0, 0.1);
            mat.rotate_y_at(20.0 * l, 0.3 * l, -0.4, 0.0);
        }
        if item.use_kind == UseKind::Block {
            let sh = if main_hand { self.shield_m } else { self.shield_o };
            let eb = easings::ease_in_out_back(sh);
            mat.translate((-x_offset) * l * eb + 0.1 * l * eb, 0.0, 0.0);
            let rot_y = if item.is_sword() { 50.0 } else { 70.0 };
            mat.rotate_y_at(rot_y * eb * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(13.0 * (sh * 4.14).sin().clamp(0.0, 1.0), 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-15.0 * eb, 0.3 * l, -0.4, 0.0);
        }

        // Trident draw
        let trident_draw_s = easings::ease_out_back(self.trident_m);
        let trident_draw_s_o = easings::ease_out_back(self.trident_m_o);
        if item.use_kind == UseKind::Trident && main_hand {
            mat.translate(0.0, -0.15 * trident_draw_s, -0.3 * trident_draw_s);
            mat.rotate_x_at(65.0 * trident_draw_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(55.0 * self.trident, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(10.0 * l * trident_draw_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(
                0.3 * (self.trident_j * trident_draw_s * 9.14).sin(),
                0.3 * l,
                -0.4,
                0.0,
            );
        }
        if !main_hand {
            mat.move_y(if item.is_empty { 0.6 } else { 0.2 } * trident_draw_s);
            mat.translate(0.25 * l * trident_draw_s, 0.0, -0.15 * trident_draw_s);
            mat.rotate_y_at(25.0 * l * trident_draw_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-30.0 * trident_draw_s, 0.3 * l, -0.4, 0.0);
        }
        if item.use_kind == UseKind::Trident && !main_hand {
            mat.translate(0.0, -0.15 * trident_draw_s_o, -0.3 * trident_draw_s_o);
            mat.rotate_x_at(65.0 * trident_draw_s_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(10.0 * l * trident_draw_s_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(
                0.3 * (self.trident_j_o * trident_draw_s_o * 9.14).sin(),
                0.3 * l,
                -0.4,
                0.0,
            );
        }
        if main_hand {
            mat.move_y(if item.is_empty { 0.6 } else { 0.2 } * trident_draw_s_o);
            mat.translate(0.25 * l * trident_draw_s_o, 0.0, -0.15 * trident_draw_s_o);
            mat.rotate_y_at(25.0 * l * trident_draw_s_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-30.0 * trident_draw_s_o, 0.3 * l, -0.4, 0.0);
        }

        // Per-weapon / tool swing choreography
        let swing_overall = (swing_progress * 3.14).sin();
        let swing_rise = (swing_progress * 6.28).sin().clamp(0.0, 1.0);
        let swing_rise_s = (swing_progress * 6.28).sin();

        if item.is_empty {
            mat.translate(
                -0.15 * l * swing,
                0.1 * swing_rise_s + 0.33 * swing + 0.05 * swing_rot + 0.14 * swing_rise,
                -0.1 * swing_rise_s - 0.4 * swing_hit - 0.2 * swing,
            );
            mat.rotate_x(-10.0 * swing_rise);
            mat.move_z(0.15 * swing_rot);
            mat.rotate_x_at(-20.0 * swing, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-7.0 * swing_hit, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(10.0 * swing_rot, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(20.0 * l * swing, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(5.0 * l * swing, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-5.0 * swing_rise_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(10.0 * l * swing_rise_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(5.0 * l * swing_rise_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l * swing_hit, 0.3 * l, -0.4, 0.0);
        } else if item.is_pickaxe() {
            mat.move_z(-0.1 * swing_sword_tilt - 0.3 * swing_overall);
            mat.move_y(-0.1 * swing_rise_s + 0.2 * swing_overall);
            mat.move_x(
                -0.15 * l * swing_hit + 0.15 * l * swing_hit_second - 0.15 * l * swing_overall,
            );
            mat.rotate_x_at(60.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(10.0 * swing_rise, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(10.0 * swing_rot, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-45.0 * swing_hit, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-10.0 * swing_hit_second, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-95.0 * swing_overall, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(20.0 * swing_overall * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(
                (20.0 * l * self.tilting.sin() * swing).clamp(-60.0, 60.0),
                0.5 * l,
                -0.5,
                0.0,
            );
        } else if item.is_sword()
            || item.is_mace()
            || item.is_trident()
            || item.is_spear()
            || item.is_axe()
        {
            if self.sword_attack
                && self.sword_attack2
                && !input.block_breaking
                && item.is_sword()
            {
                // Sword combo slash 2
                mat.move_z(0.2 * swing_sword_tilt - 0.2 * swing_hit - 0.3 * swing_overall);
                mat.move_x(
                    -0.5 * l * swing_sword_tilt - 0.15 * l * swing_hit
                        + 0.15 * l * swing_hit_second
                        + 0.15 * l * swing_overall,
                );
                mat.move_y(-0.5 * swing_sword_tilt - 0.1 * swing_rise_s + 0.2 * swing_overall);
                mat.rotate_x_at(20.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                mat.rotate_z_at(70.0 * l * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(30.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(10.0 * swing_rise, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(10.0 * swing_rot, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-25.0 * swing_hit, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-10.0 * swing_hit_second, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-75.0 * swing_overall, 0.3 * l, -0.4, 0.0);
            } else if (self.sword_attack && !input.block_breaking && item.is_sword())
                || item.is_trident()
                || item.is_spear()
            {
                if item.is_sword() {
                    // Sword combo slash 1
                    mat.move_z(
                        -0.2 * swing_sword_tilt
                            - 0.15 * swing_hit
                            - 0.25 * swing_hit_second
                            - 0.45 * swing_overall,
                    );
                    mat.move_y(0.2 * swing_hit - 0.1 * swing_rise_s + 0.2 * swing_overall);
                    mat.move_x(
                        -0.15 * l * swing_hit + 0.15 * l * swing_hit_second
                            - 0.15 * l * swing_overall,
                    );
                    mat.rotate_x_at(20.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                    mat.rotate_z_at(-70.0 * l * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(30.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(10.0 * swing_rise, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(10.0 * swing_rot, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(-55.0 * swing_hit, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(-15.0 * swing_hit_second, 0.3 * l, -0.4, 0.0);
                    mat.rotate_x_at(-95.0 * swing_overall, 0.3 * l, -0.4, 0.0);
                } else {
                    // Trident / Spear jab
                    if item.is_trident() {
                        mat.move_y(-0.5 * swing_sword_tilt);
                        mat.move_z(0.1 * swing_sword_tilt - 0.25 * swing_hit_second);
                    } else {
                        mat.move_z(
                            0.37 * swing_sword_tilt
                                - 0.35 * swing_hit_second
                                - 0.25 * easings::ease_out_quart(swing),
                        );
                    }
                    mat.move_x(-0.15 * l * swing_sword_tilt);
                    mat.move_z(0.14 * swing_overall);
                    mat.move_y(-0.2 * swing_overall);
                    mat.move_x(
                        -0.1 * l * swing_hit - 0.1 * l * swing_overall - 0.1 * l * swing_hit_second,
                    );
                    if item.is_trident() {
                        mat.rotate_z(80.0 * swing_sword_tilt * l);
                    } else {
                        mat.rotate_z(20.0 * swing_sword_tilt * l);
                        mat.rotate_y(6.0 * swing_sword_tilt * l);
                    }
                    mat.rotate_z(5.0 * swing_overall * l);
                    mat.rotate_x(-40.0 * swing_sword_tilt);
                    mat.rotate_x(15.0 * swing_hit_second + 15.0 * swing_hit + 30.0 * swing_overall);
                    mat.rotate_x(-10.0 * swing_rise);
                    mat.rotate_z((10.0 * l * (self.tilting * 2.0).sin() * swing).clamp(0.0, 30.0));
                    mat.move_y(-0.1 * (self.tilting * 2.0).sin() * swing);
                }
            } else {
                // Axe / Mace / Air-crit / Block-breaking chop
                mat.move_z(-0.2 * swing_sword_tilt - 0.3 * swing_overall);
                mat.move_y(-0.1 * swing_rise_s + 0.2 * swing_overall);
                mat.move_x(
                    -0.15 * l * swing_hit + 0.15 * l * swing_hit_second - 0.15 * l * swing_overall,
                );
                mat.rotate_x_at(20.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                let z_chop = if item.is_axe() && input.block_breaking {
                    -60.0
                } else {
                    -40.0
                };
                mat.rotate_z_at(z_chop * l * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(30.0 * swing_sword_tilt, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(10.0 * swing_rise, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(10.0 * swing_rot, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-45.0 * swing_hit, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-10.0 * swing_hit_second, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(-95.0 * swing_overall, 0.3 * l, -0.4, 0.0);
                mat.rotate_z((30.0 * l * (self.tilting * 2.0).sin() * swing).clamp(0.0, 30.0));
                mat.move_y(-0.2 * (self.tilting * 2.0).sin() * swing);
            }
        } else if item.is_shovel() {
            mat.move_y(
                0.6 * swing_sword_tilt
                    - 0.3 * swing_overall
                    - 0.4 * swing_rot
                    - 0.1 * swing_hit_second,
            );
            mat.move_z(-0.5 * swing_sword_tilt - 0.3 * swing_overall + 0.1 * swing_rot);
            mat.rotate_x(
                -130.0 * swing_sword_tilt
                    + 70.0 * swing_overall
                    + 40.0 * swing_rot
                    + 20.0 * swing_hit_second
                    + 20.0 * swing_rise
                    - 10.0 * swing_rise_s,
            );
        } else {
            // General item / block swing
            mat.move_z(-0.1 * swing - 0.1 * swing_rise - 0.05 * swing_hit);
            mat.move_x(-0.1 * l * swing);
            mat.move_y(0.25 * swing_hit);
            mat.rotate_x_at(5.0 * swing_rot, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-25.0 * swing_hit, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(5.0 * swing_rise, 0.3 * l, -0.4, 0.0);
            mat.rotate_z(-5.0 * l * swing_overall);
            mat.rotate_y(15.0 * l * swing_overall);
            mat.rotate_x_at(-2.0 * swing_rise_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_z_at(5.0 * l * swing_rise_s, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(5.0 * l * swing_rise_s, 0.3 * l, -0.4, 0.0);
        }

        // Equip progress & shield disable recoil
        let eq_sin = (equip_progress * equip_progress * equip_progress).sin();
        let sh_dis_sin = (self.shield_disable * self.shield_disable * self.shield_disable * 3.14).sin();
        if active_this_hand && item.use_kind == UseKind::Block {
            let sh = if main_hand { self.shield_m } else { self.shield_o };
            mat.move_x(-0.25 * (eq_sin + 4.0 * sh_dis_sin) * l * sh);
            mat.rotate_z_at(10.0 * l * (eq_sin + 4.0 * sh_dis_sin) * sh, 0.3 * l, -0.4, 0.0);
        }
        if item.use_kind == UseKind::Crossbow && (self.cross_bow_m + self.cross_bow_o) == 0.0 {
            mat.move_z(0.25 * eq_sin);
            mat.rotate_x_at(20.0 * eq_sin, 0.3 * l, -0.4, 0.0);
        } else if self.food_count == 0.0
            && item.use_kind != UseKind::Bow
            && item.use_kind != UseKind::Block
            && self.trident_m == 0.0
            && self.trident_m_o == 0.0
        {
            mat.move_z(-0.25 * eq_sin);
            mat.rotate_x_at(-20.0 * eq_sin, 0.3 * l, -0.4, 0.0);
            if item.is_mace() {
                mat.move_z(-0.25 * eq_sin);
                mat.rotate_z(6.0 * l * eq_sin);
                mat.rotate_x_at(-10.0 * eq_sin, 0.3 * l, -0.4, 0.0);
            }
            if !item.is_block {
                mat.rotate_x(self.pitch_angle * 0.35 * swing);
            }
        }

        // Climbing & Crawling
        let al = if input.pitch != 0.0 {
            let v = 90.0 / input.pitch / 2.5;
            if !(0.0..=1.0).contains(&v) { 1.0 } else { v }
        } else {
            1.0
        };
        let mult = if item.is_lantern() { 0.2 } else { 1.0 };
        mat.move_z(0.2 * self.smoothing);
        mat.move_y(-0.2 * self.crawler.cos() * l * al * self.smoothing * mult);
        mat.rotate_x(-30.0 * l * self.crawler.sin() * al * self.smoothing * mult);
        mat.rotate_x(input.pitch * self.smoothing);
        mat.move_z(0.01 * input.pitch * self.smoothing);
        mat.move_y(0.003 * input.pitch * self.smoothing);
        mat.move_x(-0.0025 * l * input.pitch * self.smoothing);
        if !item.is_empty {
            mat.translate(-0.05 * l * self.smoothing, -0.1 * self.smoothing, -0.2 * self.smoothing);
        }

        mat.move_z(0.2 * self.smoothing_crawl);
        mat.move_z(
            -0.2 * l * self.crwl.sin() * self.smoothing_crawl * al * mult * self.crawl_default_pos,
        );
        mat.rotate_y(10.0 * self.crwl.sin() * self.smoothing_crawl * mult * self.crawl_default_pos);
        mat.rotate_x(
            (20.0 * l * self.crwl.cos() * self.smoothing_crawl * mult * self.crawl_default_pos)
                .clamp(0.0, 20.0),
        );
        if item.is_empty {
            mat.translate(0.0, 0.3 * self.smoothing_crawl, -0.55 * self.smoothing_crawl);
            mat.rotate_x(-45.0 * self.smoothing_crawl);
            mat.rotate_z_at(
                (16.0 * self.crwl.sin() * self.smoothing_crawl * mult * self.crawl_default_pos)
                    .clamp(0.0, 20.0),
                0.3,
                -0.4,
                0.0,
            );
        }
        mat.rotate_x((input.pitch - 7.0) * self.smoothing_crawl);
        mat.move_z(0.01 * input.pitch * self.smoothing_crawl);
        if item.is_empty {
            mat.move_z(0.005 * input.pitch * self.smoothing_crawl);
        }
        mat.move_y(0.003 * input.pitch * self.smoothing_crawl);
        mat.move_x(-0.0025 * l * input.pitch * self.smoothing_crawl);
        if !item.is_empty {
            mat.translate(
                -0.1 * l * self.smoothing_crawl,
                -0.1 * self.smoothing_crawl,
                -0.2 * self.smoothing_crawl,
            );
        }

        // Bow two-handed draw choreography
        let eased_bow = easings::ease_in_out_back(self.bow_count);
        let eased_bow_o = easings::ease_in_out_back(self.bow_count_o);
        if item.use_kind == UseKind::Bow && main_hand {
            mat.translate(
                0.15 * l - 0.1 * l * eased_bow,
                0.15 * eased_bow,
                -0.085 - 0.015 * eased_bow,
            );
            mat.rotate_x_at(5.0 * (eased_bow * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l, 0.3 * l, -0.4, 0.0);
            mat.move_z(0.0015 * (self.bow_wiggle * 15.14).sin() * self.bow_count_sec);
        }
        if item.use_kind == UseKind::Bow && !main_hand {
            mat.translate(
                0.15 * l - 0.1 * l * eased_bow_o,
                0.15 * eased_bow_o,
                -0.085 - 0.015 * eased_bow_o,
            );
            mat.rotate_x_at(5.0 * (eased_bow_o * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l, 0.3 * l, -0.4, 0.0);
            mat.move_z(0.0015 * (self.bow_wiggle_o * 15.14).sin() * self.bow_count_sec_o);
        }
        if (!main_hand && input.using_item) || (!main_hand && item.is_empty) {
            let eased_bow_sec = easings::ease_out_back(self.bow_count_sec);
            mat.move_x((-x_offset - (x_offset / 1.5)) * l * eased_bow + 0.27 * l * eased_bow);
            if !item.is_empty {
                mat.move_y((eased_bow * 1.56 + 3.14).sin());
            }
            mat.move_z(-0.65 * eased_bow);
            mat.rotate_x_at(10.0 * (eased_bow * 3.14).sin() * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(70.0 * eased_bow * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(25.0 * eased_bow_sec * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y(0.25 * l * (self.bow_wiggle * 15.14).sin() * eased_bow_sec);
            mat.rotate_y_at(25.0 * eased_bow * l, 0.3 * l, -0.4, 0.0);
            mat.move_y(-0.5 * (eased_bow * 3.14).sin());
        }
        if (main_hand && input.using_item) || (main_hand && item.is_empty) {
            let eased_bow_sec_o = easings::ease_out_back(self.bow_count_sec_o);
            mat.move_x((-x_offset - (x_offset / 1.5)) * l * eased_bow_o + 0.27 * l * eased_bow_o);
            if !item.is_empty {
                mat.move_y((eased_bow_o * 1.56 + 3.14).sin());
            }
            mat.move_z(-0.65 * eased_bow_o);
            mat.rotate_x_at(10.0 * (eased_bow_o * 3.14).sin() * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(70.0 * eased_bow_o * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(25.0 * eased_bow_sec_o * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_y(0.25 * l * (self.bow_wiggle_o * 15.14).sin() * eased_bow_sec_o);
            mat.rotate_y_at(25.0 * eased_bow_o * l, 0.3 * l, -0.4, 0.0);
            mat.move_y(-0.5 * (eased_bow_o * 3.14).sin());
        }

        // Crossbow two-handed loading choreography
        let e_cb_m = easings::ease_out_back(self.cross_bow_m);
        let e_cb_sec_m = easings::ease_out_back(self.cross_bow_sec_m);
        let e_cb_o = easings::ease_out_back(self.cross_bow_o);
        let e_cb_sec_o = easings::ease_out_back(self.cross_bow_sec_o);
        if item.use_kind == UseKind::Crossbow && main_hand {
            mat.translate(0.0, -0.15 * e_cb_m, 0.3 * e_cb_m);
            mat.rotate_z_at(20.0 * l * e_cb_m, -0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l * e_cb_m, -0.3 * l, -0.4, 0.0);
        }
        if (!main_hand && input.using_item) || (!main_hand && item.is_empty) {
            mat.move_x((-x_offset - (x_offset / 1.5)) * l * e_cb_m + 0.25 * l * e_cb_m);
            mat.translate(0.0, 0.55 * e_cb_m, -0.1 * e_cb_m);
            if !item.is_empty {
                mat.move_y((e_cb_m * 1.56 + 3.14).sin());
            }
            mat.rotate_z_at(15.0 * l * e_cb_m, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(80.0 * l * e_cb_m, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l * e_cb_sec_m, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-7.0 * e_cb_sec_m, 0.3 * l, -0.4, 0.0);
        }
        if item.use_kind == UseKind::Crossbow && !main_hand {
            mat.translate(0.0, -0.15 * e_cb_o, 0.3 * e_cb_o);
            mat.rotate_z_at(20.0 * l * e_cb_o, -0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l * e_cb_o, -0.3 * l, -0.4, 0.0);
        }
        if (main_hand && input.using_item) || (main_hand && item.is_empty) {
            mat.move_x((-x_offset - (x_offset / 1.5)) * l * e_cb_o + 0.25 * l * e_cb_o);
            mat.translate(0.0, 0.55 * e_cb_o, -0.1 * e_cb_o);
            if !item.is_empty {
                mat.move_y((e_cb_o * 1.56 + 3.14).sin());
            }
            mat.rotate_z_at(15.0 * l * e_cb_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(80.0 * l * e_cb_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(15.0 * l * e_cb_sec_o, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(-7.0 * e_cb_o, 0.3 * l, -0.4, 0.0);
        }

        // Eat / Drink / Horn / Brush hand pose
        if main_hand {
            let ef = self.food_count * self.food_count;
            if matches!(item.use_kind, UseKind::Eat | UseKind::TootHorn) {
                mat.translate(0.135 * l * ef, -0.27 * ef, 0.155 * ef);
                mat.rotate_x(30.0 * ef + 20.0 * self.drink_count);
                if item.use_kind == UseKind::Eat {
                    mat.rotate_x(3.0 * easings::ease_in_out_back((self.food_count_sec * 3.0).sin().abs()) * ef);
                    mat.rotate_y(4.0 * l * easings::ease_in_out_back((self.food_count_sec * 2.0).sin().abs()) * ef);
                    mat.rotate_z(6.0 * l * easings::ease_in_out_back((self.food_count_sec * 2.0).sin().abs()) * ef);
                } else {
                    mat.rotate_x(2.0 * easings::ease_in_out_sine((self.food_count_sec * 2.0).sin()) * ef);
                    mat.rotate_y(3.0 * l * easings::ease_in_out_sine(self.food_count_sec.sin()) * ef);
                    mat.rotate_z(5.0 * l * easings::ease_in_out_sine(self.food_count_sec.sin()) * ef);
                }
                mat.rotate_y_at(60.0 * ef * l, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(25.0 * (ef * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            }
            if item.use_kind == UseKind::Drink
                || item.is_empty
                || item.short_name() == "glass_bottle"
            {
                mat.translate(0.11 * l * ef, -0.5 * ef, 0.1 * ef);
                mat.rotate_x(
                    50.0 * ef
                        + 20.0 * self.drink_count
                        + 2.0 * (self.food_count_sec * 6.0).sin() * self.drink_count,
                );
                mat.rotate_y_at(60.0 * l * ef, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(25.0 * (ef * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            }
        } else {
            let ef = self.food_count_o * self.food_count_o;
            if matches!(item.use_kind, UseKind::Eat | UseKind::TootHorn) {
                mat.translate(0.135 * l * ef, -0.27 * ef, 0.155 * ef);
                mat.rotate_x(30.0 * ef + 20.0 * self.drink_count_o);
                if item.use_kind == UseKind::Eat {
                    mat.rotate_x(3.0 * easings::ease_in_out_back((self.food_count_sec_o * 3.0).sin().abs()) * ef);
                    mat.rotate_y(4.0 * l * easings::ease_in_out_back((self.food_count_sec_o * 2.0).sin().abs()) * ef);
                    mat.rotate_z(6.0 * l * easings::ease_in_out_back((self.food_count_sec_o * 2.0).sin().abs()) * ef);
                } else {
                    mat.rotate_x(2.0 * easings::ease_in_out_sine((self.food_count_sec_o * 2.0).sin()) * ef);
                    mat.rotate_y(3.0 * l * easings::ease_in_out_sine(self.food_count_sec_o.sin()) * ef);
                    mat.rotate_z(5.0 * l * easings::ease_in_out_sine(self.food_count_sec_o.sin()) * ef);
                }
                mat.rotate_y_at(60.0 * l * ef, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(25.0 * (ef * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            }
            if item.use_kind == UseKind::Drink {
                mat.translate(0.11 * l * ef, -0.5 * ef, 0.1 * ef);
                mat.rotate_x(
                    50.0 * ef
                        + 20.0 * self.drink_count_o
                        + 2.0 * (self.food_count_sec_o * 6.0).sin() * self.drink_count_o,
                );
                mat.rotate_y_at(60.0 * l * ef, 0.3 * l, -0.4, 0.0);
                mat.rotate_x_at(25.0 * (ef * 3.14).sin(), 0.3 * l, -0.4, 0.0);
            }
        }

        // Brush hand pose
        if item.use_kind == UseKind::Brush {
            let (bc, fc_sec) = if main_hand {
                (easings::ease_in_out_back(self.brush_counter), self.food_count_sec)
            } else {
                (easings::ease_in_out_back(self.brush_counter_o), self.food_count_sec_o)
            };
            let s = (fc_sec * 4.14).sin();
            mat.translate(-0.2 * l * bc * (1.0 + s), -0.3 * bc * (1.0 + s), -0.2 * bc);
            mat.rotate_x(10.0 * (bc * 3.14).sin());
            mat.rotate_y(l * bc * (20.0 + 10.0 * s));
            mat.rotate_z(30.0 * l * bc * (1.0 + s));
        }

        if item.is_door() {
            mat.move_x(0.2 * l);
            mat.rotate_x_at(6.0, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(20.0 * l, 0.3 * l, -0.4, 0.0);
        }

        if item.use_kind == UseKind::Block {
            let ed = self.shield_disable * self.shield_disable;
            mat.translate(-0.1 * l * ed, 0.15 * ed, -0.4 * ed);
            mat.rotate_x(-30.0 * ed - 10.0 * (ed * 3.14).sin());
            mat.rotate_y(-20.0 * l * ed);
            mat.rotate_z(-6.0 * l * ed);
        }

        if item.is_empty && self.drink_count > 0.0 {
            mat.rotate_z(-6.0 * l);
            mat.move_y(-0.35);
        }

        // Filled map hand pose
        if item.is_filled_map() {
            let ems = easings::ease_in_out_back(self.map_smoother);
            let emz = easings::ease_out_back(self.map_zoomer);
            mat.translate(
                (0.3 - 0.1 * emz) * l * ems,
                0.18 * ems,
                0.12 * emz * ems,
            );
            mat.rotate_x(input.pitch.clamp(0.0, 50.0) * ems - 40.0 * ems);
            mat.rotate_y_at((40.0 + 30.0 * emz) * l * ems, 0.3 * l, -0.4, 0.0);

            let sm = 1.0 - ems;
            mat.translate(0.1 * l * sm, -0.35 * sm, 0.22 * sm);
            mat.rotate_x(24.0 * sm);
            mat.rotate_y(10.0 * l * sm);
        }

        if item.use_kind == UseKind::Crossbow {
            mat.translate(0.1 * l, 0.0, 0.2);
            mat.rotate_x_at(-5.0, 0.3 * l, -0.4, 0.0);
            mat.rotate_y_at(20.0 * l, 0.3 * l, -0.4, 0.0);
        }

        // Inspection pose
        if item.is_inspectable_weapon() && main_hand {
            let eic = easings::ease_in_out_back(self.inspection_counter);
            mat.translate(0.35 * l * eic, 0.0, -0.15 * eic);
            mat.rotate_y_at(40.0 * eic * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(
                13.0 * (self.inspection_counter * 4.14).sin().clamp(0.0, 1.0),
                0.3 * l,
                -0.4,
                0.0,
            );
            mat.rotate_x(10.0 * (easings::ease_in_out_back(self.inspection_spin) * 6.28).sin());
        } else if item.is_mace() && main_hand {
            let eic = easings::ease_in_out_back(self.inspection_counter);
            mat.translate(0.35 * l * eic, 0.0, -0.15 * eic);
            mat.rotate_y_at(40.0 * eic * l, 0.3 * l, -0.4, 0.0);
            mat.rotate_x_at(
                17.0 * (self.inspection_counter * 3.14).sin().clamp(0.0, 1.0),
                0.3 * l,
                -0.4,
                0.0,
            );
        }

        // Riptide hand pose
        if item.use_kind == UseKind::Trident {
            let rc = if main_hand {
                self.riptide_counter
            } else {
                self.riptide_counter_o
            };
            let er = easings::ease_out_back(rc);
            let rp = (rc * 3.0).clamp(0.0, 1.0);
            let sin_rc = easings::ease_out_back((rc * 3.14).sin());
            mat.translate(-1.25 * l * er * rp, 0.0, 0.2 * er * rp);
            mat.rotate_y(5.0 * l * er * rp);
            mat.rotate_x(-30.0 * sin_rc * rp);
            mat.rotate_z_at(70.0 * sin_rc * rp * l, 0.7 * l, 0.7, 0.0);
        }

        // Spear hand pose
        if item.is_spear() {
            let (sc, scd, sck, sw, hic) = if main_hand {
                (
                    self.spear_counter_m,
                    self.can_dismount_counter,
                    self.can_knockback_counter,
                    self.main_hand_switch,
                    easings::ease_in_out_sine(self.hit_impact_counter),
                )
            } else {
                (
                    self.spear_counter_o,
                    self.can_dismount_counter_o,
                    self.can_knockback_counter_o,
                    self.off_hand_switch,
                    self.hit_impact_counter_o,
                )
            };
            mat.translate(0.0, 0.1, -0.25);
            mat.rotate_x(-20.0);
            mat.move_z(
                0.75 * (easings::ease_in_out_sine(hic) * 3.14).sin()
                    - 0.25 * easings::ease_in_out_back(sc)
                    + 0.25 * easings::ease_out_back(sck) * sck,
            );
            mat.rotate_y(8.0 * easings::ease_in_out_back(sc) * l);
            mat.rotate_x_at(40.0 * (sc * 3.14).sin(), 0.0, -0.2, -0.35);
            let escd = easings::ease_in_out_back(scd);
            mat.rotate_x_at(-8.0 * escd, 0.5 * l, -0.5, -0.35);
            mat.rotate_y_at(-1.5 * (self.a * 1.5).sin() * escd * l, 0.5 * l, -0.5, 0.0);
            mat.rotate_x_at(-1.5 * (self.a * 3.0).sin() * escd, 0.5 * l, -0.5, 0.0);
            mat.rotate_x(5.0 * (hic * 3.14).sin());
            mat.rotate_z_at(
                -30.0 * easings::ease_out_back(sck) * sck * l,
                0.5 * l,
                -0.5,
                -0.35,
            );
            let sw2 = (sw * 2.0).clamp(0.0, 1.0);
            mat.rotate_z_at(
                -8.0 * (sw2 * 6.28).sin().powi(2) * l,
                0.5 * l,
                -0.5,
                0.0,
            );
            mat.rotate_x_at(20.0 * (sw2 * 3.14).sin().powi(2), 0.5 * l, -0.5, 0.0);
        }
    }

    fn apply_hand_relative_pose(
        &self,
        mat: &mut Mat4f,
        main_hand: bool,
        l: f32,
        item: &ItemFlags,
    ) {
        if item.is_empty && self.drink_count > 0.0 {
            mat.translate(1.5 * l, -0.3, -0.47);
            mat.rotate_x_at(15.0, 0.5 * l, 0.5, 0.5);
            mat.rotate_y_at(35.0 * l, 0.5 * l, 0.5, 0.5);
            mat.rotate_z_at(-65.0 * l, 0.5 * l, 0.5, 0.5);
            mat.scale(0.9, 0.9, 0.9);
        }
        let switch_val = if main_hand {
            self.main_hand_switch
        } else {
            self.off_hand_switch
        };
        let switch_anim = easings::ease_in_back(
            (switch_val.clamp(0.09723, 0.60632) * 3.24 * 1.65 - 0.1).sin(),
        );
        if item.has_switch_flip() {
            mat.rotate_x(10.0 * switch_anim);
            mat.rotate_z(6.0 * switch_anim);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_item_pose(
        &self,
        mat: &mut Mat4f,
        main_hand: bool,
        right_arm: bool,
        l: f32,
        item: &ItemFlags,
        raw_swing: f32,
        equip_progress: f32,
        input: &PlayerFrameInput,
    ) {
        let yw_angle = if main_hand {
            self.yaw_angle
        } else {
            self.yaw_angle_o
        };
        let pt_angle = if main_hand {
            self.pitch_angle
        } else {
            self.pitch_angle_o
        };

        let swing_progress = if item.is_pickaxe() {
            easings::ease_custom(raw_swing)
        } else {
            easings::ease_custom_sec(raw_swing)
        };

        let swing_rot = if swing_progress < 0.70016 {
            (swing_progress.clamp(0.0, 0.308) * 5.1).sin()
        } else {
            (swing_progress.clamp(0.70016, 1.0) * 5.1 - 2.0).sin()
        };
        let swing_rot = swing_rot * swing_rot * swing_rot;

        let swing_sword_tilt = if swing_progress < 0.65245 {
            (swing_progress.clamp(0.0, 0.16675) * 3.14 * 3.0).sin()
        } else {
            (swing_progress.clamp(0.65245, 1.0) * 4.4 - 1.3).sin()
        };

        let swing_hit = (swing_progress.clamp(0.16561, 0.49422) * 4.78 * 2.0 + 4.7).sin();
        let swing_overall = (swing_progress * 3.14).sin();
        let swing_hit_second = if swing_progress < 0.65594 {
            (swing_progress.clamp(0.16561, 0.32991) * 4.78 * 2.0 + 4.7).sin()
        } else {
            (swing_progress.clamp(0.65594, 0.82025) * 4.78 * 2.0 - 4.7).sin()
        };

        // Spear item pose
        if item.is_spear() {
            let (sc, scd, sck, sw, hic) = if main_hand {
                (
                    self.spear_counter_m,
                    self.can_dismount_counter,
                    self.can_knockback_counter,
                    self.main_hand_switch,
                    easings::ease_in_out_sine(self.hit_impact_counter),
                )
            } else {
                (
                    self.spear_counter_o,
                    self.can_dismount_counter_o,
                    self.can_knockback_counter_o,
                    self.off_hand_switch,
                    self.hit_impact_counter_o,
                )
            };
            let esc = easings::ease_in_out_back(sc * 0.8);
            let escd = easings::ease_in_out_back(scd);
            mat.rotate_z(180.0 * l - 180.0 * easings::ease_in_out_back((sw * 2.0).clamp(0.0, 1.0)) * l);
            mat.move_z(-0.2 * easings::ease_in_out_sine(esc));
            mat.move_y(-0.05 * escd);
            mat.rotate_x(-70.0 * esc - 8.0 * escd);
            mat.rotate_y(
                60.0 * esc * l
                    - 30.0 * escd * l
                    - 60.0 * easings::ease_out_back(sck) * sck * l,
            );
            mat.move_y(-0.25 * (easings::ease_in_out_sine(hic) * 6.28).sin().clamp(0.0, 1.0));
        }

        if (item.use_kind != UseKind::Block && item.use_kind != UseKind::Crossbow)
            || item.is_sword()
        {
            mat.translate(0.0, -0.05 * swing_rot, -0.05 * swing_rot);
            mat.rotate_x(-20.0 * swing_rot - 10.0 * swing_hit);

            if !item.is_sword() {
                if item.is_trident() || item.is_spear() {
                    mat.translate(0.0, -0.05 * swing_rot, -0.1 * swing_rot);
                    if item.is_spear() {
                        mat.move_y(-0.15 * swing_hit);
                        mat.rotate_x(-5.0 * swing_hit);
                    }
                    mat.rotate_x(-10.0 * swing_rot - 15.0 * swing_hit);
                    if item.is_trident() {
                        mat.rotate_x(-45.0 * swing_overall);
                    } else {
                        mat.rotate_x(-45.0 * swing_sword_tilt);
                    }
                    mat.move_y(0.05 * swing_hit + 0.3 * swing_overall);
                } else {
                    mat.translate(0.0, -0.05 * swing_rot, -0.05 * swing_rot);
                    mat.rotate_x(-10.0 * swing_rot - 25.0 * swing_hit);
                }
            }
            if item.is_shovel() {
                mat.translate(0.0, 0.12 * swing_sword_tilt, 0.05 * swing_sword_tilt);
                mat.rotate_x(
                    10.0 * swing_sword_tilt
                        - 30.0 * swing_overall
                        + 20.0 * swing_rot
                        + 10.0 * swing_hit_second,
                );
            }
            if item.is_sword() {
                let sw = easings::ease_in_out_back(swing_overall);
                mat.move_y(-0.1 * sw);
                mat.rotate_x(-60.0 * sw);
            }
            if item.use_kind == UseKind::Bow {
                mat.move_x(-0.065 * l);
            }
        }

        if item.is_bed() {
            // Keep the bed clear of the arm geometry. Applied here, before any rotation, so the
            // offset stays in the arm's frame (a later translate would be rotated by ~145 degrees).
            mat.translate(BED_ARM_CLEARANCE_X * l, 0.0, BED_ARM_CLEARANCE_Z);
            mat.move_z(0.2);
            mat.rotate_y_at(180.0 * l, -0.15 * l, -0.4, 0.0);
        }

        if item.is_bell()
            || item.is_lantern()
            || item.is_end_crystal()
            || item.is_hanging_sign()
            || item.is_flower_litter()
        {
            if !item.is_end_crystal() {
                mat.move_y(-0.62);
            }
            if item.is_hanging_sign() {
                mat.move_y(-0.07);
            }
            if item.is_flower_litter() {
                mat.move_y(0.4);
                mat.rotate_x(-70.0);
                mat.rotate_x_at(
                    (input.pitch / 2.5).clamp(-20.0, 90.0) + pt_angle + yw_angle * 0.5,
                    0.0,
                    -0.13,
                    0.0,
                );
            } else {
                mat.move_z(0.2);
                mat.rotate_x(-25.0);
            }
            if item.is_end_crystal() {
                mat.scale(
                    1.0 + 0.01 * (self.a * 15.0).sin(),
                    1.0 + 0.01 * (self.a * 15.0).sin(),
                    1.0 + 0.01 * (self.a * 8.0).sin(),
                );
                mat.translate(
                    yw_angle / 150.0 * -l,
                    0.03 * (self.a * 2.0).sin() + 0.25 + pt_angle / 150.0,
                    0.0,
                );
                mat.rotate_z(5.0 * self.a.sin());
                mat.scale(0.7, 0.7, 0.7);
            } else if item.is_bell() {
                mat.translate(0.15 * l, -0.05, -0.1);
                mat.scale(1.2, 1.2, 1.2);
                mat.rotate_x_at(
                    (input.pitch / 2.5).clamp(-20.0, 90.0) + pt_angle,
                    -0.1 * l,
                    0.4,
                    0.1,
                );
                mat.rotate_z_at(-yw_angle, -0.1 * l, 0.4, 0.1);
            } else if item.is_lantern() {
                mat.rotate_x_at((input.pitch / 2.5).clamp(-20.0, 90.0) + pt_angle, 0.0, 0.4, 0.0);
                mat.rotate_z_at(-yw_angle, 0.0, 0.4, 0.0);
            }
            if item.is_hanging_sign() {
                mat.rotate_x_at((input.pitch / 2.5).clamp(-35.0, 90.0) + pt_angle, 0.0, 0.55, 0.0);
                mat.rotate_z_at(-yw_angle, 0.0, 0.55, 0.0);
            }
        } else if item.is_painting_or_frame() {
            mat.rotate_x(-25.0);
            mat.move_y(-0.65);
            mat.rotate_x_at((input.pitch / 2.5).clamp(-25.0, 90.0) + pt_angle, 0.0, 0.45, 0.0);
            mat.rotate_z_at(-yw_angle, 0.0, 0.55, 0.0);
        } else if item.is_block {
            mat.translate(0.0, -0.025, -0.025);
            mat.rotate_x(-5.0);
        } else {
            let eq_sin = (equip_progress * equip_progress * equip_progress).sin();
            if !item.is_empty && item.use_kind == UseKind::None {
                if item.is_axe() || item.is_mace() {
                    let mult = if item.is_mace() { 0.2 } else { 0.15 };
                    mat.rotate_x_at(-20.0 * eq_sin + pt_angle * mult, 0.3 * l, -0.3, 0.0);
                } else {
                    mat.rotate_x_at(-20.0 * eq_sin + pt_angle * 0.05, 0.3 * l, -0.4, 0.0);
                }
            }
            if (item.is_axe() || item.is_mace()) && item.use_kind != UseKind::Crossbow {
                mat.rotate_x_at(input.pitch * -0.05 + pt_angle * 0.2, 0.0, -0.2, 0.0);
            } else if item.use_kind != UseKind::Crossbow {
                mat.rotate_x_at(input.pitch * -0.025 + pt_angle * 0.1, 0.0, -0.2, 0.0);
            }
        }

        // Eat / Drink / Horn item tilt
        let fc = if main_hand {
            self.food_count
        } else {
            self.food_count_o
        };
        if matches!(
            item.use_kind,
            UseKind::Drink | UseKind::Eat | UseKind::TootHorn
        ) {
            mat.translate(0.02 * l * fc, 0.0, -0.05 * fc);
            if matches!(item.use_kind, UseKind::Eat | UseKind::TootHorn) {
                mat.rotate_x(-23.0 * fc * fc);
                mat.rotate_z(-12.0 * l * fc * fc);
            }
            mat.rotate_y(-50.0 * l * fc * fc);
            if item.use_kind == UseKind::Drink {
                mat.rotate_x(15.0 * fc * fc);
            }
        }

        // Brush item tilt
        if item.use_kind == UseKind::Brush {
            let (bc, fc_sec, angle) = if main_hand {
                (
                    easings::ease_in_out_back(self.brush_counter),
                    self.food_count_sec,
                    self.brush_angle_m,
                )
            } else {
                (
                    easings::ease_in_out_back(self.brush_counter_o),
                    self.food_count_sec_o,
                    self.brush_angle_o,
                )
            };
            mat.move_z(-0.03 * bc);
            mat.rotate_x(-30.0 * bc);
            mat.rotate_z(15.0 * l * ((fc_sec - 0.5) * 4.14).sin() * bc + l * angle);
        }

        if item.is_door() {
            mat.translate(0.1 * l, -0.35, 0.25);
            mat.rotate_z(-10.0 * l);
            mat.rotate_y(-90.0 * l);
        } else if item.is_bed() {
            mat.move_z(0.17);
            mat.rotate_y_at(-35.0 * l, 0.3 * l, -0.4, 0.0);
            mat.scale(0.9, 0.9, 0.9);
        }

        // Slime / Honey / Magma Cream squish & shear
        let s = item.short_name();
        if s == "slime_ball" {
            let sy = if self.fall < 0.0 {
                self.fall * 0.06
            } else {
                self.fall * 0.12
            };
            mat.move_y(-0.1);
            mat.scale(1.0, 1.0 + sy, 1.0);
            mat.move_y(0.1);
        } else if s == "slime_block" || s == "honey_block" {
            let sxz = if self.fall < 0.0 {
                self.fall * 0.05
            } else {
                self.fall * 0.1
            };
            let sy = if self.fall < 0.0 {
                self.fall * 0.1
            } else {
                self.fall * 0.3
            };
            mat.scale(1.0 - sxz, 1.0 + sy, 1.0 - sxz);
            let sh = if right_arm {
                -yw_angle * 0.006
            } else {
                yw_angle * 0.006
            };
            mat.shear(0.0, sh, 0.0);
        } else if s == "magma_cream" {
            mat.scale(1.0 - self.fall / 5.0, 1.0 + self.fall / 5.0, 1.0);
        }

        if item.is_shovel() {
            mat.move_x(-0.09 * l);
            mat.rotate_y(80.0 * l);
        }

        // Switch flip for nuggets, music discs, ender pearls, bundles, skulls
        let switch_val = if main_hand {
            self.main_hand_switch
        } else {
            self.off_hand_switch
        };
        let switch_anim = easings::ease_in_back(
            (switch_val.clamp(0.09723, 0.60632) * 3.24 * 1.65 - 0.1).sin(),
        );
        if item.has_switch_flip() {
            mat.rotate_x(-10.0 * switch_anim);
            mat.move_y(0.62 * switch_anim + (0.1 * self.fall).clamp(0.0, 255.0));
            let spin_t = easings::ease_in_out_back((switch_val * 1.65).clamp(0.0, 1.0));
            if item.is_nugget() {
                mat.move_y(-0.07);
                mat.rotate_x_at(360.0 * spin_t, 0.0, 0.1, 0.0);
            } else if item.is_music_disc() {
                mat.rotate_z_at(360.0 * spin_t, -0.1 * l, 0.25, 0.0);
            } else {
                let cs = (switch_val * 1.2).clamp(0.0, 1.0);
                mat.rotate_z(-7.0 * l * (cs.clamp(0.0943, 0.66791) * 7.07 * 1.5 - 0.8).sin());
            }
        }

        // Filled map item orientation
        if item.is_filled_map() {
            let ems = easings::ease_in_out_back(self.map_smoother);
            let emz = easings::ease_in_out_back(self.map_zoomer);
            mat.rotate_z(5.0 * l * ems);
            mat.rotate_y((-40.0 - 20.0 * emz) * l * ems);
            mat.rotate_z(15.0 * l * ems);
            mat.rotate_x(-10.0 * emz * ems);

            let sm = 1.0 - ems;
            mat.translate(0.0, -0.05 * sm, -0.05 * sm);
            mat.rotate_x(-40.0 * sm);
            mat.rotate_y(-10.0 * l * sm);
            mat.rotate_z(5.0 * l * sm);
        } else if item.is_totem() {
            // Totem of Undying: hold the flat sprite upright and facing the camera, without the
            // generic flat-item tilt (`rotate_x(-8)`, `rotate_y(-10)`, `rotate_z(6)`).
            mat.translate(-0.02 * l, 0.04, 0.0);
        } else if item.should_translate_item(&self.overrides) {
            mat.move_x(-0.05 * l);
            mat.rotate_x(-8.0);
            mat.rotate_y(-10.0 * l);
            mat.rotate_z(6.0 * l);
        }

        if item.is_shears() {
            if !right_arm {
                mat.move_z(0.1);
                mat.rotate_y(180.0);
            }
            mat.rotate_z(45.0);
        }

        if item.is_skull() && !item.is_dragon_head() {
            mat.translate(-0.1 * l, 0.11, 0.0);
            mat.rotate_z(15.0 * l);
            mat.rotate_y(-85.0 * l);
            mat.rotate_x(-55.0);
        } else if item.is_dragon_head() {
            mat.move_y(0.25);
            mat.rotate_z(6.0 * l);
            mat.rotate_y(160.0 * l);
        }

        // Inspection 360 spin
        if item.is_inspectable_weapon() && main_hand {
            mat.move_x(-0.2 * l * self.inspection_counter);
            mat.rotate_x_at(
                -360.0 * easings::ease_in_out_back(self.inspection_spin),
                0.0,
                0.0,
                0.15,
            );
        }

        // Buckets & Milk Bucket
        if item.is_bucket() {
            mat.translate(0.0, 0.025, -0.1);
            mat.rotate_y(180.0);
            mat.rotate_x(-82.5);
            mat.rotate_z(-20.0 * l);
        } else if item.is_milk_bucket() {
            let efc = easings::ease_in_quart(fc);
            mat.translate(0.0, 0.025, -0.1);
            mat.rotate_y(180.0);
            mat.rotate_x(-82.5);
            mat.rotate_z(-20.0 * l + 30.0 * l * efc);
            mat.translate(0.0, 0.1 * efc, 0.02 * efc);
        }

        if item.use_kind == UseKind::Trident {
            let tm = if main_hand {
                self.trident_m
            } else {
                self.trident_m_o * 1.5
            };
            mat.rotate_z(TRIDENT_DRAW_ROLL_DEG * l * easings::ease_out_back(tm.clamp(0.0, 1.0)));
            mat.move_z(-0.1);
            mat.rotate_y(40.0 * l);

            let rc = if main_hand {
                self.riptide_counter
            } else {
                self.riptide_counter_o
            };
            let sin_rc = easings::ease_out_back((rc * 3.14).sin());
            mat.rotate_x(-90.0 * sin_rc);
            mat.rotate_z(-45.0 * l * sin_rc);
        }

        if item.is_spear() {
            mat.move_z(-0.1);
            mat.rotate_y(10.0 * l);
        }

        // 3D Block orientation in hand
        if item.is_block
            && item.should_apply_block_rotation(&self.overrides)
            && item.should_render_as_block(&self.overrides)
            && !item.is_flower_litter()
            && item.short_name() != "redstone"
            && !item.is_bell()
        {
            mat.move_z(-0.05);
            if !item.is_lantern() {
                mat.move_y(-0.15);
                mat.rotate_z(6.0 * l);
                mat.rotate_x(-8.0);
            }
            mat.rotate_y(25.0 * l);
            mat.scale(1.1, 1.1, 1.1);
        }
    }

    /// Replicates `item_model.lua` per-quad animations (`water_bucket`, `lava_bucket`, `milk_bucket`,
    /// `axolotl_bucket`, `tropical_fish_bucket`, `cod_bucket`, `salmon_bucket`, `tadpole_bucket`,
    /// `pufferfish_bucket`, and `chest_boats`).
    fn populate_item_model_animator(
        &self,
        animator: &mut ModelPartAnimator,
        main_hand: bool,
        right_arm: bool,
        item: &ItemFlags,
    ) {
        let l = if right_arm { 1.0_f32 } else { -1.0_f32 };
        let yw_angle = if main_hand {
            self.yaw_angle
        } else {
            self.yaw_angle_o
        };
        let pt_angle = if main_hand {
            self.pitch_angle
        } else {
            self.pitch_angle_o
        };
        let wf = (self.fall * 0.4).clamp(0.0, 2.0) * self.fall_f.powi(3);
        let walk_bob = 0.02 * (self.a * -1.5).sin() * (1.0 - self.walk_smoother)
            + 0.08 * easings::ease_in_sine(self.walk.sin().abs()) * self.walk_smoother;

        let s = item.short_name();
        if matches!(s, "water_bucket" | "lava_bucket" | "milk_bucket") {
            animator.rotate_x_at(0, 1, (-pt_angle * 0.6).clamp(-5.0, 5.0), 0.5, 0.7, 0.5);
            animator.rotate_z_at(0, 1, (yw_angle * 0.6).clamp(-5.0, 5.0), 0.5, 0.7, 0.5);
        } else if s == "axolotl_bucket" {
            animator.move_y(0, 11, self.fall * 0.02 + walk_bob);
            animator.rotate_x_at(0, 11, (20.0 * self.fall).clamp(-20.0, 20.0), 0.5, 0.7, 0.5);
            animator.rotate_z_at(0, 11, 2.0 * self.a.sin(), 0.5, 0.7, 0.5);
            animator.rotate_x_at(0, 11, pt_angle * 0.3, 0.5, 0.7, 0.5);
            animator.rotate_z_at(0, 11, -yw_angle * 0.3, 0.5, 0.7, 0.5);

            animator.move_y(12, 19, walk_bob);
            animator.rotate_x_at(12, 19, (15.0 * self.fall).clamp(-20.0, 20.0), 0.5, 0.7, 0.5);
            animator.rotate_z_at(12, 19, self.a.sin(), 0.5, 0.7, 0.5);
            animator.rotate_x_at(12, 19, pt_angle * 0.15, 0.5, 0.7, 0.5);
            animator.rotate_z_at(12, 19, -yw_angle * 0.15, 0.5, 0.7, 0.5);

            let flutter = 10.0 * (self.a * 10.0).sin() * (self.a * 0.4).sin().powi(30);
            animator.rotate_y_at(8, 9, flutter, 0.8, 0.5, 0.4);
            animator.rotate_y_at(10, 11, -flutter, 0.3, 0.5, 0.4);
            animator.rotate_x_at(20, 21, 80.0 * wf, 0.5, 0.85, 0.3);
            animator.rotate_z_at(20, 21, 15.0 * wf, 0.5, 0.85, 0.3);
            animator.rotate_x_at(22, 23, 80.0 * wf, 0.5, 0.85, 0.3);
            animator.rotate_z_at(22, 23, -15.0 * wf, 0.5, 0.85, 0.3);

            animator.move_x(0, 23, self.fall * 0.02);
            animator.move_z(0, 23, self.fall * 0.025);
            animator.scale(
                0,
                23,
                1.0 - 0.03 * self.fall,
                1.0 + 0.03 * self.fall,
                1.0 - 0.03 * self.fall,
            );
        } else if s == "tropical_fish_bucket" {
            animator.move_y(0, 12, self.fall * 0.1 + walk_bob - 0.03 - yw_angle * 0.002);
            animator.move_z(0, 12, 0.1);
            animator.rotate_x_at(0, 12, -yw_angle, 0.5, 0.5, 0.5);
            animator.rotate_z_at(0, 12, -pt_angle, 0.5, 0.5, 0.5);
            animator.rotate_x_at(
                0,
                12,
                3.0 * (self.a * 3.0).sin() * (1.0 - self.walk_smoother),
                0.5,
                0.5,
                0.5,
            );
            animator.rotate_z_at(6, 9, 15.0 * (self.a * 3.0).sin(), 0.5, 0.5, 0.5);
        } else if s == "cod_bucket" {
            animator.scale(0, 25, 0.9, 0.9, 0.9);
            animator.move_y(0, 25, self.fall * 0.1 + 0.02 + walk_bob - yw_angle * 0.002);
            animator.move_z(0, 25, 0.1);
            animator.rotate_x_at(0, 25, -yw_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_z_at(0, 25, -pt_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_x_at(
                0,
                25,
                3.0 * (self.a * 1.5).sin() * (1.0 - self.walk_smoother) - self.fall * 10.0,
                0.5,
                0.5,
                0.5,
            );
            animator.move_x(10, 21, -0.05 * l);
            animator.rotate_x_at(10, 21, -yw_angle * 0.2 - self.fall * 5.0, 0.0, 0.6, 0.2);
        } else if s == "salmon_bucket" {
            animator.scale(0, 52, 0.85, 0.85, 0.85);
            animator.move_y(0, 52, self.fall * 0.1 + 0.04 + walk_bob - yw_angle * 0.002);
            animator.move_z(0, 52, 0.1);
            animator.rotate_x_at(0, 52, -yw_angle * 0.6, 1.0, 0.5, 0.5);
            animator.rotate_z_at(0, 52, -pt_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_x_at(
                0,
                52,
                3.0 * (self.a * 1.5).sin() * (1.0 - self.walk_smoother) - self.fall * 10.0,
                0.5,
                0.5,
                0.5,
            );
            animator.rotate_x_at(0, 5, -yw_angle * 0.2 - self.fall * 5.0, 0.0, 0.6, 0.25);
        } else if s == "tadpole_bucket" {
            animator.move_y(0, 7, self.fall * 0.1 + walk_bob - 0.03 - yw_angle * 0.002);
            animator.move_z(0, 7, 0.1);
            animator.rotate_y_at(0, 7, -yw_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_x_at(0, 7, -pt_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_z_at(0, 7, -yw_angle * 0.6, 0.5, 0.5, 0.5);
            animator.rotate_x_at(
                0,
                7,
                3.0 * (self.a * 3.0).sin() * (1.0 - self.walk_smoother),
                0.5,
                0.5,
                0.5,
            );
        } else if s == "pufferfish_bucket" {
            animator.move_y(0, 29, self.fall * 0.1 + walk_bob - 0.03 - yw_angle * 0.002);
            animator.move_z(0, 29, 0.1);
            animator.rotate_x_at(
                0,
                29,
                yw_angle.abs() * 0.2 - pt_angle * 0.2
                    + (self.a * 3.0).sin() * (1.0 - self.walk_smoother),
                0.5,
                0.5,
                0.5,
            );
            animator.rotate_z_at(0, 29, -yw_angle * 0.2, 1.0, 0.5, 0.5);
            let fin = 10.0 * (self.a * 10.0).sin() * (self.a * 0.4).sin().powi(30);
            animator.rotate_z_at(6, 7, fin, 0.5, 0.5, 0.5);
            animator.rotate_z_at(8, 9, -fin, 0.5, 0.5, 0.5);
        } else if item.is_chest_boat() {
            animator.move_y(0, 15, (self.fall * 0.1).clamp(0.0, 1.0));
            animator.rotate_x_at(0, 11, (pt_angle * 1.35).clamp(0.0, 999.0), 0.5, 0.5, 0.8);
            animator.rotate_z_at(0, 15, -yw_angle * 0.12, 0.5, 0.0, 0.5);
        }
    }

    /// Computes camera pitch, yaw, roll, and offset feedback (`hand_pose.lua` lines 1245-1248).
    pub fn camera_shake(&self, config: &Config) -> (f32, f32, f32, [f32; 3]) {
        if !config.camera_shake {
            return (0.0, 0.0, 0.0, [0.0; 3]);
        }
        let food_cam = 0.25 * (self.food_count_sec * 3.0).sin() * self.food_count
            + 0.25 * (self.food_count_sec_o * 3.0).sin() * self.food_count_o
            + self.food_speed * 1.5;
        let drink_cam = (0.25 * (self.food_count_sec * 3.0).sin() * self.drink_count
            + self.drink_count * self.drink_count * 2.75)
            + (0.25 * (self.food_count_sec_o * 3.0).sin() * self.drink_count_o
                + self.drink_count_o * self.drink_count_o * 2.75);
        let pitch = (food_cam + drink_cam + self.fall * 0.35 + self.pitch_angle * 0.015)
            .clamp(-6.0, 6.0);
        let roll = (self.yaw_angle * 0.04
            + 0.15 * (self.food_count_sec * 4.0).cos() * self.drink_count
            + 0.15 * (self.food_count_sec_o * 4.0).cos() * self.drink_count_o)
            .clamp(-4.0, 4.0);
        let ox = (0.0015 * self.yaw_angle * self.walk_smoother).clamp(-0.08, 0.08);
        let oy = (0.025 * (self.walk * 0.8).sin().powi(3).abs() * self.walk_smoother)
            .clamp(0.0, 0.08);
        (pitch, 0.0, roll, [ox, oy, 0.0])
    }
}

/// Dynamic hand light colour `(rgb, radius, intensity)` for light-emitting held items.
pub fn hand_light_for_item(item_id: &str) -> Option<(u32, f32, f32)> {
    let s = item_id.strip_prefix("minecraft:").unwrap_or(item_id);
    match s {
        "torch" | "lantern" | "jack_o_lantern" | "campfire" | "glowstone" | "shroomlight"
        | "ochre_froglight" | "pearlescent_froglight" | "verdant_froglight" => {
            Some((0xFFB45A, 11.0, 0.85))
        }
        "soul_torch" | "soul_lantern" | "soul_campfire" => Some((0x4AD8FF, 9.0, 0.75)),
        "redstone_torch" => Some((0xFF3828, 7.0, 0.55)),
        "lava_bucket" => Some((0xFF7A24, 12.0, 0.95)),
        "brewing_stand" | "blaze_rod" | "magma_cream" => Some((0xFF9436, 7.5, 0.55)),
        "sea_lantern" | "conduit" | "end_rod" => Some((0xD8F6FF, 11.0, 0.8)),
        _ if s.ends_with("copper_lantern") => Some((0xFFA854, 10.5, 0.8)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (`cargo test`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_match_hmi_reference_points() {
        assert!((easings::ease_custom(0.0) - 0.0).abs() < 1e-6);
        assert!((easings::ease_custom(1.0) - 1.0).abs() < 1e-6);
        assert!((easings::ease_custom_sec(0.0) - 0.0).abs() < 1e-6);
        assert!((easings::ease_custom_sec(1.0) - 1.0).abs() < 1e-6);
        assert!((easings::ease_in_out_back(0.0) - 0.0).abs() < 1e-5);
        assert!((easings::ease_in_out_back(1.0) - 1.0).abs() < 1e-5);
        assert!((easings::ease_in_out_expo(0.0) - 0.0).abs() < 1e-6);
        assert!((easings::ease_in_out_expo(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn config_json_roundtrip_and_buttons() {
        let mut cfg = Config::default();
        cfg.x_offset = 0.05;
        cfg.swing_speed = 12;
        cfg.dynamic_lights = false;
        let json = cfg.to_json();
        let parsed = Config::from_json(&json).expect("valid config json");
        assert_eq!(parsed, cfg);

        let labels = settings_labels(&Ok(cfg.clone()));
        assert_eq!(labels.len(), 9);
        if let Some(SettingsEffect::Save(updated)) = settings_click(&Ok(cfg), 2) {
            assert!(updated.dynamic_lights);
        } else {
            panic!("expected Save effect");
        }
    }

    #[test]
    fn lua_overrides_parse_bundled_hmi_scripts() {
        let mut ov = LuaOverrides::default();
        ov.parse_script(
            r#"
            renderAsBlock:put("minecraft:string", false)
            itemSwingSpeed:put('minecraft:trident', 12)
            itemSwingSpeed:put('minecraft:iron_spear', 15)
            applyBlockRotation:put("minecraft:oak_door", false)
            "#,
        );
        assert_eq!(ov.render_as_block.get("minecraft:string"), Some(&false));
        assert_eq!(ov.item_swing_speed.get("minecraft:trident"), Some(&12));
        assert_eq!(ov.item_swing_speed.get("minecraft:iron_spear"), Some(&15));
        assert_eq!(
            ov.apply_block_rotation.get("minecraft:oak_door"),
            Some(&false)
        );
    }

    #[test]
    fn hmi_state_evaluates_hand_and_item_poses_and_part_animator() {
        let mut state = HmiState::default();
        let cfg = Config::default();
        let main_item = ItemFlags {
            id: "minecraft:axolotl_bucket".to_owned(),
            is_empty: false,
            is_block: false,
            use_kind: UseKind::None,
            is_charged_crossbow: false,
            is_cooling_down: false,
            spear_can_damage: false,
            spear_can_dismount: false,
            spear_can_knockback: false,
            spear_hit_impact: false,
        };
        let off_item = ItemFlags::empty();
        let input = PlayerFrameInput {
            pitch: 15.0,
            yaw: 45.0,
            speed: 0.15,
            y_speed: -0.02,
            on_ground: true,
            inspect_held: true,
            ..Default::default()
        };
        state.on_swing(true, &main_item, &cfg);
        state.tick_swings(&main_item, &off_item, None, None, &cfg);
        let _sounds = state.step_frame(0.016, 0.5, &main_item, &off_item, &input, &cfg);
        let eval = state.evaluate_hand(true, true, &main_item, &off_item, 0.5, &input, &cfg);
        assert_ne!(eval.arm_pose, Mat4f::IDENTITY);
        assert_ne!(eval.item_pose, Mat4f::IDENTITY);
        assert!(!eval.animator.is_empty());
        assert!(eval.animator.matrix_for_quad(0).is_some());
    }

    #[test]
    fn swing_completes_cleanly_without_looping_and_accepts_fast_clicks() {
        let mut state = HmiState::default();
        let cfg = Config::default();
        let sword = ItemFlags {
            id: "minecraft:diamond_sword".to_owned(),
            is_empty: false,
            is_block: false,
            use_kind: UseKind::None,
            is_charged_crossbow: false,
            is_cooling_down: false,
            spear_can_damage: false,
            spear_can_dismount: false,
            spear_can_knockback: false,
            spear_hit_impact: false,
        };
        let off = ItemFlags::empty();
        let input = PlayerFrameInput {
            on_ground: true,
            ..Default::default()
        };

        // Start a sword swing and advance via step_frame alone.
        state.on_swing_ex(true, &sword, &cfg, false);
        assert!(state.main_hand_swinging);
        for _ in 0..40 {
            state.step_frame(0.016, 0.75, &sword, &off, &input, &cfg);
        }
        // Swing must finish and stay strictly at 0.0 regardless of partial tick.
        assert!(!state.main_hand_swinging);
        assert_eq!(state.main_swing(0.0), 0.0);
        assert_eq!(state.main_swing(0.5), 0.0);
        assert_eq!(state.main_swing(0.99), 0.0);

        // Fast discrete re-click after 2 frames (~64ms) must register immediately without wrap-around jump.
        state.on_swing_ex(true, &sword, &cfg, false);
        let first_combo = state.swing_m_hand;
        state.step_frame(0.032, 0.2, &sword, &off, &input, &cfg);
        state.step_frame(0.032, 0.4, &sword, &off, &input, &cfg);
        assert!(state.main_swing(0.4) > 0.08 && state.main_swing(0.4) < 0.35);

        state.on_swing_ex(true, &sword, &cfg, false);
        assert_ne!(state.swing_m_hand, first_combo);
        assert_eq!(state.main_swing(0.9), 0.0);

        // Continuous hold (mining / held button) must NOT reset mid-swing or flip sword combo prematurely.
        state.step_frame(0.032, 0.5, &sword, &off, &input, &cfg);
        let combo_before_hold = state.swing_m_hand;
        let prog_before_hold = state.main_swing(0.5);
        state.on_swing_ex(true, &sword, &cfg, true);
        assert_eq!(state.swing_m_hand, combo_before_hold);
        assert_eq!(state.main_swing(0.5), prog_before_hold);
    }

    #[test]
    fn pickaxe_swing_keeps_pace_with_mining() {
        let cfg = Config::default();
        let pickaxe = ItemFlags {
            id: "minecraft:iron_pickaxe".to_owned(),
            is_empty: false,
            is_block: false,
            use_kind: UseKind::None,
            is_charged_crossbow: false,
            is_cooling_down: false,
            spear_can_damage: false,
            spear_can_dismount: false,
            spear_can_knockback: false,
            spear_hit_impact: false,
        };
        let off = ItemFlags::empty();

        // Frames (50 ms each, i.e. one tick) one swing takes to finish.
        let frames_for_one_swing = |block_breaking: bool| -> u32 {
            let mut state = HmiState::default();
            state.block_breaking = block_breaking;
            let input = PlayerFrameInput {
                on_ground: true,
                block_breaking,
                ..Default::default()
            };
            state.on_swing_ex(true, &pickaxe, &cfg, block_breaking);
            let mut frames = 0;
            while state.main_hand_swinging && frames < 100 {
                state.step_frame(0.05, 0.5, &pickaxe, &off, &input, &cfg);
                frames += 1;
            }
            frames
        };

        let attack = frames_for_one_swing(false);
        let mining = frames_for_one_swing(true);
        assert_eq!(attack, cfg.swing_speed);
        assert_eq!(mining, MINING_SWING_TICKS);

        // A swing that started as a normal click speeds up once mining begins,
        // without the progress jumping backwards or past the end.
        let mut state = HmiState::default();
        let idle = PlayerFrameInput { on_ground: true, ..Default::default() };
        let digging = PlayerFrameInput { on_ground: true, block_breaking: true, ..Default::default() };
        state.on_swing_ex(true, &pickaxe, &cfg, false);
        state.step_frame(0.05, 0.5, &pickaxe, &off, &idle, &cfg);
        let before = state.main_hand_swing_progress;
        state.step_frame(0.05, 0.5, &pickaxe, &off, &digging, &cfg);
        assert!(state.block_breaking);
        assert!(state.main_hand_swing_progress > before);
        assert!(state.main_hand_swing_progress < 1.0);
    }
}
