//! The mod's block material tables, matched by block name.
//!
//! Sound Physics Remastered keys these on the game's `SoundType`
//! (`SoundPhysics.java`'s `BlockOcclusionConfig` and `ReflectivityConfig`).
//! Horizon has no sound types, so the host passes the block's name
//! (`minecraft:white_wool`) and these tables match that instead. The values are
//! the original's; a name the original's table does not list gets the config's
//! default.

use crate::Config;

/// A block name without its namespace (`minecraft:white_wool` -> `white_wool`).
fn path(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, path)| path)
}

/// `default_block_occlusion_factor`, or the material's own: how much the block
/// named `name` absorbs the sound passing through it.
pub fn occlusion(name: &str, config: &Config) -> f64 {
    let name = path(name);
    if absorbs_nothing(name) {
        return 0.0;
    }
    if name.contains("glass")
        || name.contains("snow")
        || matches!(name, "bamboo" | "bamboo_sapling" | "wet_grass" | "moss_carpet")
    {
        return 0.1;
    }
    if matches!(name, "water" | "bubble_column" | "water_cauldron") {
        return 0.25;
    }
    if name == "honey_block" {
        return 0.5;
    }
    if matches!(name, "lava" | "lava_cauldron" | "moss_block") {
        return 0.75;
    }
    // Wool is the one material that absorbs more than a full block.
    if name.contains("wool") {
        return 1.5;
    }
    config.default_block_occlusion_factor
}

/// The materials that absorb nothing: thin growth, crops, and the blocks you can
/// see and walk through (`SoundType.VINE`, `CROP`, `FUNGUS`, the amethyst buds).
fn absorbs_nothing(name: &str) -> bool {
    name.contains("vine") // vine, twisting/weeping/cave vines
        || name.contains("roots") // crimson/warped/mangrove/hanging roots
        || name.contains("fungus")
        || name.contains("amethyst_cluster")
        || name.contains("amethyst_bud")
        || matches!(
            name,
            "sweet_berry_bush"
                | "spore_blossom"
                | "small_dripleaf"
                | "pointed_dripstone"
                | "scaffolding"
                | "glow_lichen"
                | "lily_pad"
                | "ladder"
                | "chain"
                | "jukebox"
                // SoundType.CROP's plants.
                | "wheat"
                | "carrots"
                | "potatoes"
                | "beetroots"
                | "nether_wart"
                | "torchflower_crop"
                | "pitcher_crop"
                | "pumpkin_stem"
                | "melon_stem"
                | "attached_pumpkin_stem"
                | "attached_melon_stem"
        )
}

/// `default_block_reflectivity`, or the material's own: how much the block named
/// `name` reflects sound back into the world.
pub fn reflectivity(name: &str, config: &Config) -> f64 {
    let name = path(name);
    // The nether's own materials, ahead of the ores and stone they resemble.
    if name.contains("netherrack")
        || name.contains("nether_quartz_ore")
        || name.contains("nether_gold_ore")
    {
        return 1.1;
    }
    // The hard, polished materials: stone and its family, the deepslate, and the
    // metals' blocks. Checked before wool, moss and wood so that mossy
    // cobblestone reads as stone, not as moss.
    if name.contains("stone")
        || name.contains("deepslate")
        || name.contains("netherite_block")
        || name.contains("tuff")
        || name.contains("amethyst")
        || name.contains("basalt")
        || name.contains("calcite")
        || name.contains("bone_block")
        || name.contains("nether_brick")
        || name.ends_with("_ore")
    {
        return 1.5;
    }
    if name.contains("copper")
        || name.contains("iron")
        || name.contains("gold")
        || name.contains("metal")
    {
        return 1.25;
    }
    if name.contains("wool") || name.contains("honey") || name.contains("moss") {
        return 0.1;
    }
    if name.contains("soul_sand") || name.contains("soul_soil") || name.contains("coral_block") {
        return 0.2;
    }
    if name.contains("sand") {
        return 0.2;
    }
    if name.contains("gravel") || name.contains("grass") {
        return 0.3;
    }
    // Wood and its stems, bamboo's included; a crop's stem is not wood.
    let stem = name.contains("_stem")
        && !name.contains("pumpkin")
        && !name.contains("melon")
        && !name.contains("attached");
    let bamboo_wood = name.contains("bamboo_planks")
        || name.contains("bamboo_mosaic")
        || name.contains("bamboo_block");
    if stem
        || bamboo_wood
        || name.contains("wood")
        || name.contains("planks")
        || name.contains("_log")
        || name.contains("hyphae")
        || name.contains("stripped")
    {
        return 0.4;
    }
    if name.contains("glass") {
        return 0.75;
    }
    if name.contains("snow") && !name.contains("powder") {
        return 0.15;
    }
    config.default_block_reflectivity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occlusion_by_material() {
        let config = Config::default();
        assert_eq!(occlusion("minecraft:wool", &config), 1.5);
        assert_eq!(occlusion("white_wool", &config), 1.5);
        assert_eq!(occlusion("moss_block", &config), 0.75);
        assert_eq!(occlusion("honey_block", &config), 0.5);
        assert_eq!(occlusion("water", &config), 0.25);
        assert_eq!(occlusion("lava", &config), 0.75);
    }

    #[test]
    fn thin_blocks_absorb_nothing_and_stone_absorbs_all() {
        let config = Config::default();
        for name in [
            "vine",
            "cave_vines",
            "weeping_vines",
            "glow_lichen",
            "small_dripleaf",
            "pointed_dripstone",
            "scaffolding",
            "ladder",
            "chain",
            "jukebox",
            "crimson_roots",
            "warped_fungus",
            "small_amethyst_bud",
            "sweet_berry_bush",
            "lily_pad",
            "wheat",
            "attached_melon_stem",
        ] {
            assert_eq!(occlusion(name, &config), 0.0, "{name}");
        }
        for name in ["stone", "dirt", "iron_block", "oak_planks"] {
            assert_eq!(occlusion(name, &config), 1.0, "{name}");
        }
    }

    #[test]
    fn a_crop_stem_is_not_a_wood_stem() {
        let config = Config::default();
        assert_eq!(reflectivity("crimson_stem", &config), 0.4);
        assert_eq!(reflectivity("mushroom_stem", &config), 0.4);
        assert_eq!(reflectivity("melon_stem", &config), 0.5);
        assert_eq!(reflectivity("pumpkin_stem", &config), 0.5);
    }

    #[test]
    fn mossy_stone_reads_as_stone_and_moss_as_moss() {
        let config = Config::default();
        assert_eq!(reflectivity("mossy_cobblestone", &config), 1.5);
        assert_eq!(reflectivity("moss_block", &config), 0.1);
        assert_eq!(reflectivity("moss_carpet", &config), 0.1);
    }

    #[test]
    fn the_materials_of_the_table() {
        let config = Config::default();
        for name in [
            "stone",
            "cobblestone",
            "sandstone",
            "deepslate_bricks",
            "polished_deepslate",
            "netherite_block",
            "tuff",
            "amethyst_block",
            "basalt",
            "calcite",
            "bone_block",
            "nether_bricks",
            "gold_ore",
        ] {
            assert_eq!(reflectivity(name, &config), 1.5, "{name}");
        }
        assert_eq!(reflectivity("copper_block", &config), 1.25);
        assert_eq!(reflectivity("iron_block", &config), 1.25);
        assert_eq!(reflectivity("netherrack", &config), 1.1);
        assert_eq!(reflectivity("nether_gold_ore", &config), 1.1);
        assert_eq!(reflectivity("nether_quartz_ore", &config), 1.1);
        assert_eq!(reflectivity("oak_planks", &config), 0.4);
        assert_eq!(reflectivity("stripped_oak_log", &config), 0.4);
        assert_eq!(reflectivity("white_wool", &config), 0.1);
        assert_eq!(reflectivity("soul_sand", &config), 0.2);
        assert_eq!(reflectivity("tube_coral_block", &config), 0.2);
        assert_eq!(reflectivity("sand", &config), 0.2);
        assert_eq!(reflectivity("gravel", &config), 0.3);
        assert_eq!(reflectivity("grass_block", &config), 0.3);
        assert_eq!(reflectivity("glass", &config), 0.75);
        assert_eq!(reflectivity("snow_block", &config), 0.15);
        // Powder snow is not the snow material.
        assert_eq!(reflectivity("powder_snow", &config), 0.5);
        assert_eq!(reflectivity("dirt", &config), 0.5);
    }
}
