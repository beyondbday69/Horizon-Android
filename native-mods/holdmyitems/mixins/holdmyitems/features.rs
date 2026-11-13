// Which built-in Horizon features this bundle turns on alongside Hold My Items.
//
// The client infers a bundle's features from its id and from the words in its Rust
// sources (jar_transpiler::infer_features reads `id`, title, description and the whole
// source text). Naming them here is the explicit way to say it: the string constants
// below are the feature ids the gates in `game/android_bundled.rs` check, so this file
// is what makes both features load for anyone who installs this bundle.
//
// Neither feature is code in this bundle. Both already exist in the client as
// `crates/sound-physics-core` and `crates/camera-overhaul-core`, with their own settings
// pages under Options > Mods; this bundle only asks for them.
//
// Kept as a separate file rather than folded into `native.rs` so the request is visible
// on its own, instead of buried in the HMI adapter.

/// Reverb and occlusion for sounds: caves and enclosed spaces echo, walls muffle,
/// and distant sources lose their high end. Matches `features.contains("sound_physics")`.
pub const SOUND_PHYSICS: &str = "sound_physics";

/// Camera-driven sway, bob and shake from movement, turning and impacts.
/// Matches `features.contains("camera_overhaul")`.
pub const CAMERA_OVERHAUL: &str = "camera_overhaul";

/// What this bundle turns on, for anyone reading the sources.
pub const FEATURES: [&str; 2] = [SOUND_PHYSICS, CAMERA_OVERHAUL];
