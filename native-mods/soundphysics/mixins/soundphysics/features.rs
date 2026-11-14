// Which built-in Horizon feature this bundle turns on.
//
// Sound physics is not code in this bundle: it already ships in the client as
// `crates/sound-physics-core`, with its own rows under Options > Mods. The gate
// in `game/android_bundled.rs` asks only whether a loaded mod names the feature:
//
//     if features.contains("sound_physics") { ... }
//
// So the string below is the whole mod. Naming it here is the explicit way to
// say it; `jar_transpiler::infer_features` also guesses it from the id.

/// Reverb and occlusion for sounds: caves and enclosed spaces echo, walls muffle,
/// and distant sources lose their high end. Matches `features.contains("sound_physics")`.
pub const SOUND_PHYSICS: &str = "sound_physics";

/// What this bundle turns on, for anyone reading the sources.
pub const FEATURES: [&str; 1] = [SOUND_PHYSICS];
