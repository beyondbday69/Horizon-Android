//! The blade settings players change in `Options > Mods > Grassier Grass`.
//!
//! The original keeps these in its config screen. Here they live in the game's own mod
//! settings (`Host::setting` / `set_setting`), so they survive a restart and need no file
//! of their own. Every value is read once per bake, which is why a change is applied on
//! the next rebuild rather than mid-frame.

use grassiergrass_engine as engine;

/// The mod id the `NativeMod` row and `Host::setting` use.
pub(crate) const ID: &str = "grassiergrass";

/// One knob a player can change, with the range the panel walks through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Knob {
    /// Horizontal radius in blocks, 8..=48.
    Radius,
    /// Blades per grass block, 8..=64, rounded to a multiple of 4.
    Density,
    /// Blade height, 0.1..=0.6 in 0.05 steps.
    Height,
    /// Blade width, 0.5..=1.6 in 0.1 steps.
    Width,
    /// Tint blades with the biome's grass colour.
    BiomeTint,
    /// Draw blades at all.
    Enabled,
}

/// The settings panel's rows, in order. `Knob::from_row` maps back.
const ROWS: [Knob; 6] = [
    Knob::Enabled,
    Knob::Radius,
    Knob::Density,
    Knob::Height,
    Knob::Width,
    Knob::BiomeTint,
];

impl Knob {
    fn from_row(row: usize) -> Option<Self> {
        ROWS.get(row).copied()
    }
}

/// The live values, as read for one bake.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) enabled: bool,
    pub(crate) radius: i32,
    pub(crate) blades_per_block: u32,
    pub(crate) blade_height: f32,
    pub(crate) blade_width: f32,
    pub(crate) biome_tint: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            // Smaller than the original's 20-block, 64-blade default so an Android rebuild
            // fits in a frame; both are raised in the panel.
            radius: 20,
            blades_per_block: 24,
            blade_height: 0.3,
            blade_width: 0.95,
            biome_tint: true,
        }
    }
}

impl Settings {
    /// The engine's config for this bake. `tint` is filled in per rebuild, because it
    /// depends on where the camera is.
    pub(crate) fn config(&self) -> engine::Config {
        engine::Config {
            radius: self.radius,
            blades_per_block: self.blades_per_block,
            blade_height: self.blade_height,
            blade_width: self.blade_width,
            tint: [1.0, 1.0, 1.0],
        }
    }
}

/// Reads a knob's stored value, falling back to `default` when the setting is absent or
/// unparseable, so a hand-edited file cannot crash the renderer.
pub(crate) fn read(store: impl Fn(&str) -> Option<String>, knob: Knob, default: bool) -> String {
    let key = key_of(knob);
    store(key).unwrap_or_else(|| default_of(knob, default).to_owned())
}

/// The setting key a knob is stored under.
pub(crate) fn key_of(knob: Knob) -> &'static str {
    match knob {
        Knob::Radius => "radius",
        Knob::Density => "blades_per_block",
        Knob::Height => "blade_height",
        Knob::Width => "blade_width",
        Knob::BiomeTint => "biome_tint",
        Knob::Enabled => "enabled",
    }
}

/// The value a knob shows before anything is stored.
fn default_of(knob: Knob, enabled: bool) -> &'static str {
    match knob {
        Knob::Radius => "20",
        Knob::Density => "24",
        Knob::Height => "0.30",
        Knob::Width => "0.95",
        Knob::BiomeTint => "on",
        Knob::Enabled => {
            if enabled {
                "on"
            } else {
                "off"
            }
        }
    }
}

/// Parses the whole set from the game's mod settings.
pub(crate) fn load(store: impl Fn(&str) -> Option<String>) -> Settings {
    let d = Settings::default();
    let number = |knob: Knob| store(key_of(knob)).and_then(|v| v.parse::<f32>().ok());
    let flag = |knob: Knob| match store(key_of(knob)).as_deref() {
        Some("off") | Some("false") | Some("0") => false,
        Some("on") | Some("true") | Some("1") => true,
        _ => true,
    };
    Settings {
        enabled: flag(Knob::Enabled),
        radius: number(Knob::Radius).map_or(d.radius, |v| v.round().clamp(8.0, 48.0) as i32),
        blades_per_block: number(Knob::Density).map_or(d.blades_per_block, |v| {
            ((v.round() as i32).clamp(8, 64) as u32) / 4 * 4
        }),
        blade_height: number(Knob::Height).map_or(d.blade_height, |v| v.clamp(0.1, 0.6)),
        blade_width: number(Knob::Width).map_or(d.blade_width, |v| v.clamp(0.5, 1.6)),
        biome_tint: flag(Knob::BiomeTint),
    }
}

/// What a click on `row` did, so the caller knows whether to rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Nothing changed.
    None,
    /// The value changed; write it, then rebuild the blades.
    Save(Knob, String),
}

/// Steps `row` forward, wrapping at the top of its range. Every knob is changed by
/// clicking its own row, so the panel needs no sliders.
pub(crate) fn click(settings: &Settings, row: usize) -> Effect {
    let Some(knob) = Knob::from_row(row) else {
        return Effect::None;
    };
    // Every arm is `(&'static str, String)`, so the borrow of the stepped value ends here.
    let next: (&'static str, String) = match knob {
        Knob::Enabled => (
            "enabled",
            if settings.enabled { "off" } else { "on" }.to_owned(),
        ),
        Knob::BiomeTint => (
            "biome_tint",
            if settings.biome_tint { "off" } else { "on" }.to_owned(),
        ),
        Knob::Radius => (
            "radius",
            step(f32::from(settings.radius as i16), 4.0, 8.0, 48.0, 0),
        ),
        Knob::Density => (
            "blades_per_block",
            step(settings.blades_per_block as f32, 4.0, 8.0, 64.0, 0),
        ),
        Knob::Height => (
            "blade_height",
            step(settings.blade_height, 0.05, 0.1, 0.6, 2),
        ),
        Knob::Width => (
            "blade_width",
            step(settings.blade_width, 0.1, 0.5, 1.6, 2),
        ),
    };
    Effect::Save(knob, next.1)
}

/// `value` moved up one `increment`, wrapping to `min` past `max`. `decimals` matches the
/// precision the knob is stored at, so 0.55 steps to 0.60 rather than to 0.6000000000000001.
fn step(value: f32, increment: f32, min: f32, max: f32, decimals: usize) -> String {
    let next = value + increment;
    // The epsilon keeps 1.55 + 0.1 landing past the maximum rather than on it.
    let next = if next > max + 1e-3 { min } else { next };
    format!("{next:.decimals$}")
}

/// The panel's button labels, one per row in `ROWS` order.
pub(crate) fn labels(settings: &Settings) -> Vec<String> {
    vec![
        format!("Blades: {}", on_off(settings.enabled)),
        format!("Radius: {} blocks", settings.radius),
        format!("Blades per block: {}", settings.blades_per_block),
        format!("Height: {:.2}", settings.blade_height),
        format!("Width: {:.2}", settings.blade_width),
        format!("Biome tint: {}", on_off(settings.biome_tint)),
    ]
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_survive_a_missing_store() {
        let s = load(none);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn bad_values_fall_back_and_are_clamped() {
        let store = |k: &str| match k {
            "radius" => Some("999".to_owned()),
            "blade_height" => Some("nonsense".to_owned()),
            _ => None,
        };
        let s = load(store);
        assert_eq!(s.radius, 48, "radius clamps to its maximum");
        assert_eq!(s.blade_height, Settings::default().blade_height);
    }

    #[test]
    fn density_is_a_multiple_of_four() {
        let store = |k: &str| (k == "blades_per_block").then(|| "27".to_owned());
        assert_eq!(load(store).blades_per_block, 24);
    }

    #[test]
    fn every_row_maps_to_one_knob_and_labelling_covers_them_all() {
        assert_eq!(labels(&Settings::default()).len(), ROWS.len());
        for row in 0..ROWS.len() {
            assert!(Knob::from_row(row).is_some(), "row {row}");
        }
        assert!(Knob::from_row(ROWS.len()).is_none());
    }

    #[test]
    fn numbers_wrap_past_their_maximum() {
        let mut s = Settings::default();
        s.radius = 48;
        assert_eq!(
            click(&s, 1),
            Effect::Save(Knob::Radius, "8".to_owned()),
            "48 wraps to the minimum"
        );
        s.blade_height = 0.55;
        assert_eq!(
            click(&s, 3),
            Effect::Save(Knob::Height, "0.60".to_owned())
        );
        s.blade_width = 1.55;
        assert_eq!(
            click(&s, 4),
            Effect::Save(Knob::Width, "0.50".to_owned()),
            "1.65 rounds past the maximum and wraps"
        );
    }

    #[test]
    fn flags_toggle() {
        let s = Settings::default();
        assert_eq!(
            click(&s, 0),
            Effect::Save(Knob::Enabled, "off".to_owned())
        );
        assert_eq!(
            click(&s, 5),
            Effect::Save(Knob::BiomeTint, "off".to_owned())
        );
    }

    #[test]
    fn a_click_past_the_last_row_does_nothing() {
        assert_eq!(click(&Settings::default(), 99), Effect::None);
    }
}
