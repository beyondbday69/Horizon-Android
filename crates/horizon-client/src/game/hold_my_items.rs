//! Hold My Items (HMI 5.1.1) adapter: the bridge between `crate::game::Game<'_>` and the
//! animation engine in `hold-my-items-core`.
//!
//! Ported from iuaititi's Horizon port <https://github.com/iuaiti/Hold-my-items-horizon>
//! (MIT), which drives HMI's `runtime.rs` from Horizon's `Game`. See
//! `THIRD_PARTY_NOTICES.md`. The engine is a built-in feature (`hold_my_items`) here
//! rather than a mixin, so this file is a module of `crate::game` and the hook points
//! in `android_bundled.rs` call into it.

use hold_my_items_core::{
    self as hmi, Config, HmiParticle, HmiState, ItemFlags, LoadedConfig, Mat4f, ModelPartAnimator,
    ParticleBlend, ParticleSpace, PlayerFrameInput, SettingsEffect, UseKind,
};
use super::maps;
use super::native_hooks::{NativeColoredLight, NativeShake};
use super::{Dim, Game, Screen};
use horizon_assets::item_model::{DisplayContext, FoilType};
use horizon_client::block_entity_render::{diffuse_lights, vertex_light};
use horizon_client::first_person::{self, Hand};
use horizon_client::hud::HudBatch;
use horizon_client::item_render::{
    self, ItemQuad, ItemRenderState, ItemSink, LayerGeometry, WorldSink, transform_normal,
};
use horizon_client::items::ItemStack;
use horizon_client::mesher::Vertex;
use horizon_client::spear::Kinetic;
use horizon_world::consumable::UseAnimation;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
struct State {
    config: Option<LoadedConfig>,
    scripts_loaded: bool,
    inspect_down: bool,
    left_continued: bool,
    right_continued: bool,
    last_partial: f32,
    hmi: HmiState,
    /// Pre-baked local-space map vertices (`Mat4::IDENTITY`) for mainhand/offhand maps,
    /// refreshed each frame in `hmi_frame(&mut self)` so `hmi_render_hands(&self)`
    /// can pose and emit them without needing `&mut self`.
    cached_maps: [Option<(i32, Vec<Vertex>)>; 2],
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn with_state<T>(run: impl FnOnce(&mut State) -> T) -> T {
    let mutex = STATE.get_or_init(|| Mutex::new(State::default()));
    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
    run(&mut guard)
}

fn read_config(game: &Game<'_>) -> LoadedConfig {
    match game.native_setting(hmi::ID, hmi::CONFIG_FILE) {
        Some(text) => Config::from_json(&text).map_err(|e| format!("{}: {e}", hmi::CONFIG_FILE)),
        None => Ok(Config::default()),
    }
}

fn apply_config(loaded: LoadedConfig) {
    if let Err(error) = &loaded {
        eprintln!("Hold My Items config error: {error}");
    }
    with_state(|s| s.config = Some(loaded));
}

fn loaded_config(game: &Game<'_>) -> LoadedConfig {
    if let Some(loaded) = with_state(|s| s.config.clone()) {
        return loaded;
    }
    let read = read_config(game);
    with_state(|s| s.config.get_or_insert(read).clone())
}

fn active_config(game: &Game<'_>) -> Option<Config> {
    loaded_config(game).ok()
}

fn enabled(game: &Game<'_>) -> bool {
    game.native_window() && game.native_on(hmi::ID)
}

fn load_bundled_scripts(game: &Game<'_>, state: &mut State) {
    if state.scripts_loaded {
        return;
    }
    state.scripts_loaded = true;
    state.hmi.overrides = hmi::LuaOverrides::default();
    if let Some(built_dir) = game.native_built_data(hmi::ID) {
        for file in [
            "hand_pose.lua",
            "hand_relative_pose.lua",
            "item_pose.lua",
            "item_model.lua",
        ] {
            if let Ok(script) = std::fs::read_to_string(built_dir.join(file)) {
                state.hmi.overrides.parse_script(&script);
            }
        }
    }
    let data_dir = game.native_data_dir(hmi::ID);
    for file in [
        "hand_pose.lua",
        "hand_relative_pose.lua",
        "item_pose.lua",
        "item_model.lua",
    ] {
        if let Ok(script) = std::fs::read_to_string(data_dir.join(file)) {
            state.hmi.overrides.parse_script(&script);
        }
    }
}

#[inline]
fn to_glam(m: Mat4f) -> glam::Mat4 {
    glam::Mat4::from_cols_array(&m.0)
}

/// Custom `ItemSink` that applies `ModelPartAnimator` (`item_model.lua`) per-quad transforms
/// in local model space `[0..1]^3` before delegating to Horizon's `WorldSink`.
struct HmiAnimatedSink<'a, 'b> {
    world: WorldSink<'a>,
    animator: &'b ModelPartAnimator,
    layer_pose: glam::Mat4,
    layer_pose_inv: glam::Mat4,
    quad_index: usize,
}

impl ItemSink for HmiAnimatedSink<'_, '_> {
    fn quad(&mut self, mut quad: ItemQuad) {
        let idx = self.quad_index;
        self.quad_index += 1;
        if let Some(part_mat) = self.animator.matrix_for_quad(idx) {
            let world_delta = self.layer_pose * to_glam(part_mat) * self.layer_pose_inv;
            quad.pos = quad.pos.map(|p| world_delta.transform_point3(p));
            quad.normal = transform_normal(world_delta, quad.normal);
        }
        self.world.quad(quad);
    }
}

impl Game<'_> {
    fn inspect_key_name(&self) -> String {
        self.native_mod_key(hmi::ID, hmi::INSPECT_KEY, hmi::INSPECT_KEY_DEFAULT)
    }

    fn classify_stack(&self, stack: Option<&ItemStack>, partial: f32) -> ItemFlags {
        let Some(stack) = stack else {
            return ItemFlags::empty();
        };
        let id = stack.item.to_string();
        if id == "minecraft:air" || stack.count == 0 {
            return ItemFlags::empty();
        }
        let registry = self.pack.assets.registry;
        let is_block = registry.default_state(&id).is_some();
        let view = item_render::stack_view(stack);
        let components = self.pack.item_renderer.models.components(&view);
        let is_charged_crossbow = stack.data().is_some_and(|d| !d.charged.is_empty());

        let short = id.strip_prefix("minecraft:").unwrap_or(&id);
        let use_kind = if Kinetic::of(&id).is_some()
            || components.get("minecraft:kinetic_weapon").is_some()
        {
            UseKind::Spear
        } else if short == "bow" {
            UseKind::Bow
        } else if short == "crossbow" {
            UseKind::Crossbow
        } else if short == "trident" {
            UseKind::Trident
        } else if short == "brush" {
            UseKind::Brush
        } else if short == "spyglass" {
            UseKind::Spyglass
        } else if short == "goat_horn" {
            UseKind::TootHorn
        } else if components.get("minecraft:blocks_attacks").is_some() || short == "shield" {
            UseKind::Block
        } else if let Some(consumable) = components.get("minecraft:consumable") {
            match first_person::consumable_animation(consumable) {
                UseAnimation::Drink => UseKind::Drink,
                UseAnimation::TootHorn => UseKind::TootHorn,
                _ => UseKind::Eat,
            }
        } else {
            UseKind::None
        };

        let is_cooling_down = use_kind == UseKind::Block && self.shield_cooldown > 0;

        let mut spear_can_damage = true;
        let mut spear_can_dismount = true;
        let mut spear_can_knockback = true;
        let mut spear_hit_impact = false;
        if let Some(k) = Kinetic::of(&id) {
            if let Some((_, ticks_used)) = self.using {
                let t = ticks_used as i32;
                spear_can_damage = t <= k.delay_ticks + k.damage.max_duration_ticks;
                spear_can_dismount = t <= k.delay_ticks + k.dismount.max_duration_ticks;
                spear_can_knockback = t <= k.delay_ticks + k.knockback.max_duration_ticks;
            }
            spear_hit_impact = self.kinetic_hit_ticks(partial) < 10.0;
        }

        ItemFlags {
            id,
            is_empty: false,
            is_block,
            use_kind,
            is_charged_crossbow,
            is_cooling_down,
            spear_can_damage,
            spear_can_dismount,
            spear_can_knockback,
            spear_hit_impact,
        }
    }

    fn build_frame_input(&self, partial: f32, inspect_down: bool) -> PlayerFrameInput {
        let Some(p) = self.player.as_ref() else {
            return PlayerFrameInput::default();
        };
        let vx = p.velocity.x as f32;
        let vy = p.velocity.y as f32;
        let vz = p.velocity.z as f32;
        let speed = (vx * vx + vz * vz).sqrt();
        let crawling =
            p.pose == horizon_client::pose::Pose::Swimming && !p.fluids.was_touching_water;
        let heights = self.heights.interpolated(partial);
        let vis = self.visible_hand_items();
        let main_scale = vis[0].as_ref().map_or(1.0, |s| {
            self.pack
                .item_renderer
                .models
                .swap_animation_scale(&item_render::stack_view(s))
        });
        let off_scale = vis[1].as_ref().map_or(1.0, |s| {
            self.pack
                .item_renderer
                .models
                .swap_animation_scale(&item_render::stack_view(s))
        });
        let main_equip = ((1.0 - heights[0]) * main_scale).clamp(0.0, 1.0);
        let off_equip = ((1.0 - heights[1]) * off_scale).clamp(0.0, 1.0);
        let actively_playing = self.is_hmi_actively_playing();

        PlayerFrameInput {
            pitch: self.pitch,
            yaw: self.yaw,
            speed,
            y_speed: vy,
            on_ground: p.ground.on_ground,
            sneaking: p.shift_key_down || p.local_crouching,
            swimming: p.swimming,
            crawling,
            climbing: p.last_climbable.is_some(),
            touching_water: p.fluids.was_touching_water,
            using_riptide: p.auto_spin_attack_ticks > 0,
            using_item: self.using.is_some(),
            active_main_hand: self.used_hand() != Some(Hand::Off),
            inspect_held: inspect_down && actively_playing,
            block_breaking: self.is_hmi_breaking_block(),
            age_ticks: self.ticks as f32 + partial,
            main_equip_progress: main_equip,
            off_equip_progress: off_equip,
        }
    }

    #[inline]
    fn is_hmi_actively_playing(&self) -> bool {
        self.native_playing()
            && self.focused
            && self.captured
            && !self.pause
            && self.screen == Screen::Playing
            && self.title.is_none()
            && !self.dialog_open()
    }

    #[inline]
    fn is_hmi_breaking_block(&self) -> bool {
        if !self.is_hmi_actively_playing() || self.using.is_some() {
            return false;
        }
        if self.creative && !self.res.items().can_destroy_block(self.held(), true) {
            return false;
        }
        // `Minecraft.continueAttack` calls `swing` on every destroy tick, so a held break
        // reaches `hmi_swing_hand` many times a second. HMI only keeps one clean repeating
        // swing when it is told the hold is continuous; otherwise each call restarts the
        // animation mid-arc and the tool looks like it plays several animations at once.
        // On touch `self.left` stays false while a break is in progress, so the destroy
        // state itself has to count: `ticks`/`progress` rule out a one-off sword hit on a
        // block, which starts a `Destroying` that never advances.
        self.mining.is_some()
            || (self.clicks.destroying.active
                && (self.left
                    || self.clicks.destroying.ticks > 0.0
                    || self.clicks.destroying.progress > 0.0))
    }

    /// Sanitizes stuck mouse button (`self.left`, `self.right`) and block-destroying states
    /// (`self.clicks.destroying.active`, `self.mining`) when screens, dialogs, pause, or
    /// window unfocus/uncapture occur, or when holding non-mining weapons in Creative mode.
    fn sanitize_hmi_mouse_state(&mut self) {
        if !self.is_hmi_actively_playing() {
            self.left = false;
            self.right = false;
            self.clicks.destroying.active = false;
            if self.mining.is_some() {
                self.stop_remote_mining();
                self.mining = None;
            }
            with_state(|s| {
                s.left_continued = false;
                s.right_continued = false;
            });
            return;
        }

        // In Creative mode, weapons with `can_destroy_blocks_in_creative = false` (swords, tridents, maces)
        // abort `self.mining` in Horizon without clearing `self.clicks.destroying.active`, which otherwise
        // leaves continuous destroy active and forces block-breaking animations.
        let cannot_mine_creative =
            self.creative && !self.res.items().can_destroy_block(self.held(), true);
        if cannot_mine_creative || self.using.is_some() {
            if self.clicks.destroying.active {
                self.clicks.destroying.active = false;
            }
            if self.mining.is_some() {
                self.stop_remote_mining();
                self.mining = None;
            }
        } else if !self.left && self.mining.is_none() && self.clicks.destroying.active {
            self.clicks.destroying.active = false;
        }
    }

    /// `horizon_client::game::hand::Game<'_>::swing_hand` HEAD hook.
    pub(super) fn hmi_swing_hand(&mut self, hand: Hand) {
        if !enabled(self) {
            return;
        }
        let Some(cfg) = active_config(self) else {
            return;
        };
        let stack = match hand {
            Hand::Main => self.held(),
            Hand::Off => self.offhand.as_ref(),
        };
        let flags = self.classify_stack(stack, 0.0);
        let is_main = hand == Hand::Main;
        let breaking = self.is_hmi_breaking_block();
        let left_now = self.left;
        let right_now = self.right;
        with_state(|s| {
            let continuous_hold = if is_main {
                let cont = breaking || (left_now && s.left_continued);
                s.left_continued = left_now;
                cont
            } else {
                let cont = right_now && s.right_continued;
                s.right_continued = right_now;
                cont
            };
            s.hmi.on_swing_ex(is_main, &flags, &cfg, continuous_hold);
        });
    }

    /// `native_tick` (20 TPS): syncs haste/fatigue amplifiers and sanitizes input state.
    /// Swing progression itself is stepped in `hmi_frame` via `dt` so it never
    /// desynchronizes or double-steps across threads.
    pub(super) fn hmi_tick(&mut self) {
        if !enabled(self) || self.player.is_none() {
            return;
        }
        self.sanitize_hmi_mouse_state();
        let left_now = self.left;
        let right_now = self.right;
        let effects = &self.potions.effects;
        let haste = effects
            .amplifier("haste")
            .max(effects.amplifier("conduit_power"));
        let fatigue = effects.amplifier("mining_fatigue");
        with_state(|s| {
            if !left_now {
                s.left_continued = false;
            }
            if !right_now {
                s.right_continued = false;
            }
            s.hmi.last_haste = haste;
            s.hmi.last_fatigue = fatigue;
        });
    }

    /// `native_frame`: steps HMI spring-damper physics, plays switch/bell/flower sounds,
    /// and pre-caches local map vertices when a filled map is held.
    pub(super) fn hmi_frame(&mut self, elapsed: Duration) {
        if !enabled(self) {
            return;
        }
        let Some(cfg) = active_config(self) else {
            return;
        };
        with_state(|s| load_bundled_scripts(self, s));
        if self.player.is_none() {
            with_state(|s| {
                s.inspect_down = false;
                s.left_continued = false;
                s.right_continued = false;
                s.hmi.clear_active_swings();
                s.hmi.particles.clear();
                s.cached_maps = [None, None];
            });
            return;
        }

        self.sanitize_hmi_mouse_state();

        let actively_playing = self.is_hmi_actively_playing();
        let left_now = self.left;
        let right_now = self.right;
        let dt = if self.pause {
            0.0
        } else {
            elapsed.as_secs_f32().clamp(0.0, 0.05)
        };
        let partial = self.clock.fraction() as f32;
        let effects = &self.potions.effects;
        let haste = effects
            .amplifier("haste")
            .max(effects.amplifier("conduit_power"));
        let fatigue = effects.amplifier("mining_fatigue");
        let inspect_down = with_state(|s| {
            if !actively_playing {
                s.inspect_down = false;
            }
            if !left_now {
                s.left_continued = false;
            }
            if !right_now {
                s.right_continued = false;
            }
            s.last_partial = partial;
            s.hmi.last_haste = haste;
            s.hmi.last_fatigue = fatigue;
            s.inspect_down
        });
        let main_flags = self.classify_stack(self.held(), partial);
        let off_flags = self.classify_stack(self.offhand.as_ref(), partial);
        let input = self.build_frame_input(partial, inspect_down);

        let sounds = with_state(|s| {
            s.hmi
                .step_frame(dt, partial, &main_flags, &off_flags, &input, &cfg)
        });
        for snd in sounds {
            self.audio
                .play_unshared(snd.sound, None, snd.volume, snd.pitch);
        }

        // Pre-cache filled map geometry in local map space (`Mat4::IDENTITY`) while we have `&mut self`.
        let vis = self.visible_hand_items();
        let mut map_caches: [Option<(i32, Vec<Vertex>)>; 2] = [None, None];
        for (slot_idx, maybe_stack) in vis.iter().enumerate() {
            if let Some(stack) = maybe_stack
                && let Some(map_id) = maps::map_id(stack)
            {
                let mut verts = Vec::new();
                self.held_map(glam::Mat4::IDENTITY, map_id, 0xF0F0, &mut verts);
                map_caches[slot_idx] = Some((map_id, verts));
            }
        }
        with_state(|s| {
            s.cached_maps = map_caches;
        });
    }

    /// `horizon_client::game::totem::Game<'_>::item_activation_geometry` HEAD hook:
    /// replaces `out` (the first-person hand vertex buffer from `hand_geometry`) with
    /// the complete Hold My Items 5.1.1 first-person arms, 3D blocks, items, per-quad
    /// animations, maps, and first-person particles.
    pub(super) fn hmi_render_hands(&self, partial: f32, out: &mut Vec<Vertex>) {
        if !enabled(self) {
            return;
        }
        let Some(cfg) = active_config(self) else {
            return;
        };
        if self.spectator()
            || self.ui.hide_gui
            || self.sleeping.is_some()
            || !self.camera_type.first_person()
            || self.scoping()
            || self
                .player
                .as_ref()
                .is_none_or(|p| p.health.dead_or_dying())
            || self.screen == Screen::Dead
        {
            return;
        }

        // Replace vanilla's `hand_geometry` vertices with HMI's first-person overhaul.
        out.clear();

        let to_world = self.sway.to_world(partial, &self.view);
        let eye = self.eye_position().unwrap_or_default();
        let raw_light = self.world.light(
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        );
        let base_sky = u32::from(raw_light >> 4);
        let base_block = u32::from(raw_light & 15);

        let inspect_down = with_state(|s| s.inspect_down);
        let input = self.build_frame_input(partial, inspect_down);
        let items = self.visible_hand_items();
        let main_flags = self.classify_stack(items[0].as_ref(), partial);
        let off_flags = self.classify_stack(items[1].as_ref(), partial);
        let used_hand = self.used_hand();

        for (i, (hand, item_opt)) in Hand::BOTH.into_iter().zip(items.iter()).enumerate() {
            let main_hand = hand == Hand::Main;
            let left = self.settings.main_hand_left != (hand == Hand::Off);
            let right_arm = !left;
            let l = if right_arm { 1.0_f32 } else { -1.0_f32 };

            let (this_flags, other_flags) = if main_hand {
                (&main_flags, &off_flags)
            } else {
                (&off_flags, &main_flags)
            };

            let eval = with_state(|s| {
                s.hmi.evaluate_hand(
                    main_hand,
                    right_arm,
                    this_flags,
                    other_flags,
                    partial,
                    &input,
                    &cfg,
                )
            });

            // Combined block emission light (HeldItemRendererMixin line 482)
            let emission =
                if let Some(state_id) = self.pack.assets.registry.default_state(&this_flags.id) {
                    u32::from(self.res.facts().get(state_id).emission)
                } else if hmi::hand_light_for_item(&this_flags.id).is_some() {
                    14
                } else {
                    0
                };
            let eff_block = base_block.max(emission).min(15);
            let combined_raw = ((base_sky as u8) << 4) | (eff_block as u8);
            let packed_light = vertex_light(combined_raw);

            // 1. Render the player arm (`scenePoseMain` + `mainHandPose` + `player_arm_pose(left, 0, 0)`)
            // In HMI 5.1.1, the player arm is ALWAYS rendered gripping the held item (or empty hand),
            // unless the player has Invisibility active.
            if !self.own_invisible() {
                let arm_world = to_world
                    * to_glam(eval.arm_pose)
                    * first_person::player_arm_pose(left, 0.0, 0.0);
                self.first_person_arm(left, arm_world, packed_light, out);
            }

            // 2. Render the held item, 3D block, or map if non-empty
            if eval.effective_empty {
                continue;
            }
            let Some(stack) = item_opt.as_ref() else {
                continue;
            };

            let item_view_mat = to_glam(eval.item_pose);

            if eval.render_as_3d_block
                && let Some(mut state_id) = self.pack.assets.registry.default_state(&stack.item)
            {
                // HeldItemRendererMixin lines 500-584: 3D Block in hand
                let mut block_mat = item_view_mat
                    * glam::Mat4::from_translation(glam::Vec3::new(0.22 * l, 0.25, 0.2));
                let s_name = this_flags.short_name();
                if s_name.contains("torch") || this_flags.is_lantern() || s_name == "frogspawn" {
                    block_mat = block_mat
                        * glam::Mat4::from_translation(glam::Vec3::new(-0.05 * l, 0.0, 0.0))
                        * glam::Mat4::from_scale(glam::Vec3::splat(1.75));
                } else {
                    block_mat = block_mat
                        * glam::Mat4::from_translation(glam::Vec3::new(-0.25 * l, -0.05, 0.0));
                }
                if !right_arm {
                    block_mat =
                        block_mat * glam::Mat4::from_translation(glam::Vec3::new(-0.3, 0.0, 0.0));
                }
                block_mat = block_mat
                    * glam::Mat4::from_scale(glam::Vec3::splat(0.3))
                    * glam::Mat4::from_translation(glam::Vec3::new(-0.9 * l, -0.45, -0.7));

                let world_block_mat = to_world * block_mat;

                // HeldItemRendererMixin lines 544-557: Bell renders bell block entity + SINGLE_WALL frame
                if this_flags.is_bell() {
                    if let Some(wall_id) =
                        self.state_with_prop_opt(state_id, "attachment", "single_wall")
                    {
                        state_id = wall_id;
                    }
                    self.special_block_model(state_id, world_block_mat, combined_raw, out);
                    self.block_model_vertices(
                        state_id,
                        42,
                        world_block_mat,
                        packed_light,
                        true,
                        out,
                    );
                } else {
                    // HeldItemRendererMixin lines 521-523: Stairs face north in HMI (`HORIZONTAL_FACING = NORTH`)
                    if s_name.ends_with("_stairs")
                        && let Some(north_id) =
                            self.state_with_prop_opt(state_id, "facing", "north")
                    {
                        state_id = north_id;
                    }

                    // HeldItemRendererMixin lines 559-572: Double-tall doors render upper half at +1.0 Y
                    if this_flags.is_door()
                        && let Some(upper_id) = self.state_with_prop_opt(state_id, "half", "upper")
                    {
                        let upper_mat = block_mat
                            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.0, 0.0));
                        self.block_model_vertices(
                            upper_id,
                            42,
                            to_world * upper_mat,
                            packed_light,
                            true,
                            out,
                        );
                    }

                    let before_block_verts = out.len();
                    self.block_model_vertices(
                        state_id,
                        42,
                        world_block_mat,
                        packed_light,
                        true,
                        out,
                    );

                    // BlockRenderManagerMixin.java line 69:
                    // `LoadedBlockEntityModels.renderByBlock(blockState.getBlock(), ItemDisplayContext.NONE, poseStack, ...)`
                    // Renders 3D block-entity models (`chest`, `trapped_chest`, `ender_chest`, `copper_chest`,
                    // `shulker_box`, `conduit`, `decorated_pot`, etc.) directly in `[0, 1]^3` block space.
                    let owner = self.player_owner(Some(stack));
                    let special_state =
                        self.resolve_item(stack, DisplayContext::None, Some(&owner), 0);
                    let lights = diffuse_lights(self.dim == Dim::Nether);
                    for layer in &special_state.layers {
                        if let LayerGeometry::Special { model, argument } = &layer.geometry {
                            let mut sink = WorldSink {
                                out,
                                light: packed_light,
                                lights: Some(lights),
                                glint: self.glint(),
                            };
                            self.pack.item_renderer.submit_special(
                                model,
                                argument,
                                layer.foil != FoilType::None,
                                world_block_mat,
                                &self.gui.atlas.sprites,
                                &self.item_host(),
                                &mut sink,
                            );
                        }
                    }

                    if out.len() == before_block_verts {
                        self.special_block_model(state_id, world_block_mat, combined_raw, out);
                    }
                }

                let mut particle_mat = block_mat;
                if !right_arm {
                    particle_mat = particle_mat
                        * glam::Mat4::from_translation(glam::Vec3::new(1.0, 0.0, 0.0));
                }
                self.emit_hmi_particles(
                    main_hand,
                    ParticleSpace::Item,
                    to_world,
                    particle_mat,
                    packed_light,
                    out,
                );
            } else if maps::map_id(stack).is_some() {
                // HeldItemRendererMixin lines 607-613: Filled Map in hand
                let map_mat = item_view_mat
                    * glam::Mat4::from_translation(glam::Vec3::new(-0.05 * l, 0.2, 0.1))
                    * glam::Mat4::from_rotation_y((-12.0 * l).to_radians());
                let world_map = to_world * map_mat;
                with_state(|s| {
                    if let Some((_, ref cached)) = s.cached_maps[i] {
                        for v in cached {
                            let mut tv = *v;
                            tv.pos = world_map
                                .transform_point3(glam::Vec3::from_array(v.pos))
                                .to_array();
                            tv.light = packed_light;
                            out.push(tv);
                        }
                    }
                });
            } else {
                // Standard / Special Item with HMI `item_model.lua` per-quad animations
                let mut owner = self.player_owner(Some(stack));
                owner.using_this_item &= used_hand == Some(hand);
                if hand == Hand::Off {
                    owner.is_selected = false;
                    owner.is_carried = false;
                }
                owner.fishing_cast_with_this_item =
                    self.main_hand_rod() && self.cast_rod() == Some(stack);

                // HeldItemRendererMixin.java lines 585 & 673-702:
                // For `UseAction.BLOCK` (shield), `UseAction.BRUSH` (brush), and `UseAction.SPEAR` (spear),
                // HMI passes a cloned `new ItemStack(...)` (`renderStack`) to `renderItem`, which causes
                // vanilla's `UsingItemProperty` (`entity.getActiveItem() == stack`) to evaluate to `false`
                // so `items/shield.json` keeps the base `minecraft:item/shield` model while HMI's own
                // `hand_pose.lua` (`shieldM` / `shieldO`) animates the blocking pose.
                if matches!(
                    this_flags.use_kind,
                    UseKind::Block | UseKind::Brush | UseKind::Spear
                ) {
                    owner.using_this_item = false;
                }

                // Override bow pull duration when HMI smooth bow pull is active
                // (`item_pose.lua` lines 968-974: `usingItem:put("minecraft:bow", bc >= 0.1)`
                // and `useDuration:put("minecraft:bow", Easings:cubicEase(bc) * 20)`).
                if this_flags.use_kind == UseKind::Bow {
                    let bc = if main_hand {
                        with_state(|s| hmi::easings::ease_out_back(s.hmi.bow_count_sec))
                    } else {
                        with_state(|s| hmi::easings::ease_out_back(s.hmi.bow_count_sec_o))
                    };
                    if bc < 0.1 {
                        owner.using_this_item = false;
                    } else {
                        owner.using_this_item = true;
                        let pull_ticks =
                            (hmi::easings::cubic_ease(bc).clamp(0.0, 1.0) * 20.0).round() as i32;
                        owner.use_duration = 72_000;
                        owner.use_item_remaining_ticks = 72_000 - pull_ticks;
                    }
                }

                // HeldItemRendererMixin.java lines 641 & 702:
                // `this.renderItem(player, renderStack, bl2 ? ItemDisplayContext.THIRD_PERSON_RIGHT_HAND : ItemDisplayContext.THIRD_PERSON_LEFT_HAND, ...)`
                // HMI renders all held items in third-person hand display contexts so items align
                // with the 3D player arm and HMI resourcepack 3D item models (buckets, flower baskets, bows, tools).
                let (context, seed) = if left {
                    (DisplayContext::ThirdPersonLeftHand, 1)
                } else {
                    (DisplayContext::ThirdPersonRightHand, 2)
                };
                let state = self.resolve_item(stack, context, Some(&owner), seed);
                let world_pose = to_world * item_view_mat;
                self.submit_item_hmi(&state, world_pose, packed_light, &eval.animator, out);

                self.emit_hmi_particles(
                    main_hand,
                    ParticleSpace::Item,
                    to_world,
                    item_view_mat,
                    packed_light,
                    out,
                );
            }

            // 3. Emit SCREEN-space particles (petal/firefly bursts) for this hand
            self.emit_hmi_particles(
                main_hand,
                ParticleSpace::Screen,
                to_world,
                glam::Mat4::IDENTITY,
                packed_light,
                out,
            );
        }
    }

    fn state_with_prop_opt(
        &self,
        state: horizon_world::registry::BlockStateId,
        key: &str,
        val: &str,
    ) -> Option<horizon_world::registry::BlockStateId> {
        let reg = self.pack.assets.registry;
        let mut s = reg.state(state)?.clone();
        s.properties.insert(key.to_owned(), val.to_owned());
        reg.id(&s)
    }

    fn submit_item_hmi(
        &self,
        state: &ItemRenderState,
        pose: glam::Mat4,
        light: u32,
        animator: &ModelPartAnimator,
        out: &mut Vec<Vertex>,
    ) {
        if animator.is_empty() {
            self.submit_item_world(state, pose, light, out);
            return;
        }
        let lights = diffuse_lights(self.dim == Dim::Nether);
        let first_layer_mat = state
            .layers
            .first()
            .map_or(glam::Mat4::IDENTITY, |l| l.matrix);
        let layer_pose = pose * first_layer_mat;
        let layer_pose_inv = if layer_pose.determinant().abs() > 1e-6 {
            layer_pose.inverse()
        } else {
            glam::Mat4::IDENTITY
        };
        let mut sink = HmiAnimatedSink {
            world: WorldSink {
                out,
                light,
                lights: Some(lights),
                glint: self.glint(),
            },
            animator,
            layer_pose,
            layer_pose_inv,
            quad_index: 0,
        };
        state.submit(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            &self.item_host(),
            pose,
            &mut sink,
        );
    }

    fn emit_hmi_particles(
        &self,
        main_hand: bool,
        space: ParticleSpace,
        to_world: glam::Mat4,
        view_space_parent: glam::Mat4,
        packed_light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let particles: Vec<HmiParticle> = with_state(|s| {
            s.hmi
                .particles
                .iter()
                .filter(|p| !p.dead && p.main_hand == main_hand && p.space == space)
                .cloned()
                .collect()
        });
        if particles.is_empty() {
            return;
        }
        let sprites = &self.gui.atlas.sprites;
        for p in particles {
            let Some(rect) = sprites.get(p.sprite).copied() else {
                continue;
            };
            let local = view_space_parent
                * glam::Mat4::from_translation(glam::Vec3::new(p.x, p.y, p.z))
                * glam::Mat4::from_scale(glam::Vec3::splat(p.scale));
            let (scale_vec, _, center_view) = local.to_scale_rotation_translation();
            let half = match p.blend {
                ParticleBlend::Additive => 0.12 * scale_vec.x.abs(),
                ParticleBlend::Translucent => 0.14 * scale_vec.x.abs(),
            };
            let a_byte = match p.blend {
                ParticleBlend::Additive => ((p.alpha * 0.35).clamp(0.0, 255.0) as u32) & 0xFF,
                ParticleBlend::Translucent => (p.alpha.clamp(0.0, 255.0) as u32) & 0xFF,
            };
            let r = (p.tint_rgb >> 16) & 0xFF;
            let g = (p.tint_rgb >> 8) & 0xFF;
            let b = p.tint_rgb & 0xFF;
            let color = (a_byte << 24) | (b << 16) | (g << 8) | r;
            let light = if p.blend == ParticleBlend::Additive || p.sprite == "particle/firefly" {
                vertex_light(0xFF)
            } else {
                packed_light
            };
            let corners = [
                ([half, half, 0.0], [rect.u1, rect.v0]),
                ([-half, half, 0.0], [rect.u0, rect.v0]),
                ([-half, -half, 0.0], [rect.u0, rect.v1]),
                ([half, -half, 0.0], [rect.u1, rect.v1]),
            ];
            for (offset, uv) in corners {
                let pos_view = center_view + glam::Vec3::from_array(offset);
                let pos_world = to_world.transform_point3(pos_view).to_array();
                out.push(Vertex {
                    normal: [0, 0, 127, 0],
                    pos: pos_world,
                    uv,
                    layer: horizon_client::gfx::TRANSLUCENT_ITEM_OVERLAY,
                    color,
                    light,
                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }

    /// `native_camera_shake`: applies HMI camera pitch/roll/offset feedback during eating, drinking, walking, and landing.
    pub(super) fn hmi_shake(&self, _partial: f32, shake: &mut NativeShake) {
        if !enabled(self) || !self.native_playing() {
            return;
        }
        let Some(cfg) = active_config(self) else {
            return;
        };
        let (pitch, yaw, roll, offset) = with_state(|s| s.hmi.camera_shake(&cfg));
        shake.pitch += pitch;
        shake.yaw += yaw;
        shake.roll += roll;
        shake.offset[0] += offset[0];
        shake.offset[1] += offset[1];
        shake.offset[2] += offset[2];
    }

    /// `native_visuals`: emits dynamic camera-local colored light when holding a light source.
    pub(super) fn hmi_visuals(&self, lights: &mut Vec<NativeColoredLight>) {
        if !enabled(self) {
            return;
        }
        let Some(_cfg) = active_config(self).filter(|c| c.dynamic_lights) else {
            return;
        };
        let Some(eye) = self.eye_position() else {
            return;
        };
        let main_light = self
            .held()
            .and_then(|s| hmi::hand_light_for_item(&s.item));
        let off_light = self
            .offhand
            .as_ref()
            .and_then(|s| hmi::hand_light_for_item(&s.item));
        if let Some((rgb, radius, intensity)) = main_light.or(off_light) {
            lights.push(NativeColoredLight {
                at: eye,
                rgb,
                radius,
                intensity,
            });
        }
    }

    /// `native_key`: tracks the Inspect keybind (`key.holdmyitems.inspect`, default C).
    pub(super) fn hmi_key(&mut self, key: &str, pressed: bool) {
        if !enabled(self) {
            with_state(|s| s.inspect_down = false);
            return;
        }
        if key == self.inspect_key_name() {
            with_state(|s| s.inspect_down = pressed && self.native_playing());
        }
    }

    /// `native_hud`: renders any queued HMI debug lines.
    pub(super) fn hmi_hud(&mut self, batch: &mut HudBatch) {
        if !enabled(self) {
            return;
        }
        let lines = with_state(|s| std::mem::take(&mut s.hmi.debug_lines));
        for (idx, line) in lines.iter().enumerate() {
            self.native_text(batch, line, [10.0, 10.0 + (idx as f32) * 10.0], 0xFFFFFF, true);
        }
    }

    /// `native_command`: handles `/hmi` and `/holdmyitems` commands.
    pub(super) fn hmi_command(&mut self, command: &str, ran: &mut bool) {
        if *ran {
            return;
        }
        let mut parts = command.split_whitespace();
        let Some(cmd) = parts.next() else {
            return;
        };
        if cmd != hmi::COMMAND_HMI && cmd != hmi::COMMAND_HOLDMYITEMS {
            return;
        }
        *ran = true;
        let sub = parts.next().unwrap_or("status");
        match sub {
            "reload" => {
                with_state(|s| s.scripts_loaded = false);
                let fresh = read_config(self);
                apply_config(fresh);
                self.native_chat("§a[Hold My Items] Reloaded config.json and Lua overrides.");
            }
            "inspect" => {
                let now = with_state(|s| {
                    s.inspect_down = !s.inspect_down;
                    s.inspect_down
                });
                self.native_chat(&format!(
                    "§e[Hold My Items] Weapon inspection toggled {}.",
                    if now { "ON" } else { "OFF" }
                ));
            }
            "reset" => {
                let def = Config::default();
                self.native_set_setting(hmi::ID, hmi::CONFIG_FILE, &def.to_json());
                apply_config(Ok(def));
                self.native_chat("§a[Hold My Items] Reset settings to defaults.");
            }
            "speed" => {
                if let Some(ticks) = parts.next().and_then(|v| v.parse::<u32>().ok()) {
                    let mut cfg = active_config(self).unwrap_or_default();
                    cfg.swing_speed = ticks.clamp(4, 24);
                    self.native_set_setting(hmi::ID, hmi::CONFIG_FILE, &cfg.to_json());
                    apply_config(Ok(cfg.clone()));
                    self.native_chat(&format!(
                        "§a[Hold My Items] Set base swing speed to {} ticks.",
                        cfg.swing_speed
                    ));
                } else {
                    self.native_chat("§cUsage: /hmi speed <4..24>");
                }
            }
            "offset" => {
                let x = parts.next().and_then(|v| v.parse::<f32>().ok());
                let y = parts.next().and_then(|v| v.parse::<f32>().ok());
                let z = parts.next().and_then(|v| v.parse::<f32>().ok());
                if let (Some(x), Some(y), Some(z)) = (x, y, z) {
                    let mut cfg = active_config(self).unwrap_or_default();
                    cfg.x_offset = x.clamp(-0.5, 0.5);
                    cfg.y_offset = y.clamp(-0.5, 0.5);
                    cfg.z_offset = z.clamp(-0.5, 0.5);
                    self.native_set_setting(hmi::ID, hmi::CONFIG_FILE, &cfg.to_json());
                    apply_config(Ok(cfg.clone()));
                    self.native_chat(&format!(
                        "§a[Hold My Items] Set offset to X {:+.2} Y {:+.2} Z {:+.2}.",
                        cfg.x_offset, cfg.y_offset, cfg.z_offset
                    ));
                } else {
                    self.native_chat("§cUsage: /hmi offset <x> <y> <z>");
                }
            }
            _ => {
                let cfg = active_config(self).unwrap_or_default();
                self.native_chat(&format!(
                    "§b[Hold My Items 5.1.1] §73D Blocks: {} | Particles: {} | Lights: {} | Speed: {}t | Commands: /hmi reload, /hmi inspect, /hmi speed <ticks>, /hmi offset <x> <y> <z>, /hmi reset",
                    if cfg.block_render_3d { "ON" } else { "OFF" },
                    if cfg.hand_particles { "ON" } else { "OFF" },
                    if cfg.dynamic_lights { "ON" } else { "OFF" },
                    cfg.swing_speed,
                ));
            }
        }
    }

    /// `native_mods`: lists Hold My Items 5.1.1 in `Options > Mods`.
    /// `native_settings`: returns button labels for `Options > Mods > Hold My Items`.
    pub(super) fn hmi_settings_inner(&self) -> Vec<String> {
        hmi::settings_labels(&loaded_config(self))
    }

    /// `native_settings_click`: toggles settings and writes `config.json`.
    pub(super) fn hmi_settings_click_inner(&mut self, button: usize) {
        match hmi::settings_click(&loaded_config(self), button) {
            Some(SettingsEffect::Save(cfg)) => {
                self.native_set_setting(hmi::ID, hmi::CONFIG_FILE, &cfg.to_json());
                apply_config(Ok(cfg));
            }
            Some(SettingsEffect::Reload) => apply_config(read_config(self)),
            None => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Hook-point entry points. `android_bundled.rs` gates each of these on
// `hold_my_items_active`, so they only run when a loaded mod offers the feature.
// ---------------------------------------------------------------------------

pub(super) fn hmi_frame(game: &mut Game<'_>, elapsed: Duration) {
    game.hmi_frame(elapsed);
}

pub(super) fn hmi_tick(game: &mut Game<'_>) {
    game.hmi_tick();
}

pub(super) fn hmi_swing_hand(game: &mut Game<'_>, hand: Hand) {
    game.hmi_swing_hand(hand);
}

pub(super) fn hmi_render_hands(game: &Game<'_>, partial: f32, out: &mut Vec<Vertex>) {
    game.hmi_render_hands(partial, out);
}

pub(super) fn hmi_shake(game: &Game<'_>, shake: &mut NativeShake) {
    game.hmi_shake(f32::NAN, shake);
}

pub(super) fn hmi_visuals(game: &Game<'_>, lights: &mut Vec<NativeColoredLight>) {
    game.hmi_visuals(lights);
}

pub(super) fn hmi_key(game: &mut Game<'_>, key: &str, pressed: bool) {
    game.hmi_key(key, pressed);
}

pub(super) fn hmi_hud(game: &mut Game<'_>, batch: &mut HudBatch) {
    game.hmi_hud(batch);
}

pub(super) fn hmi_command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    game.hmi_command(command, ran);
}

pub(super) fn hmi_settings(game: &Game<'_>) -> Vec<String> {
    game.hmi_settings_inner()
}

pub(super) fn hmi_settings_click(game: &mut Game<'_>, button: usize) {
    game.hmi_settings_click_inner(button);
}
