// native_hooks::Game::native_visuals (a hook point): emits dynamic camera-local colored light
// when holding a torch, lantern, lava bucket, or brewing stand.
fn holdmyitems_visuals(
    game: &crate::game::Game<'_>,
    lights: &mut Vec<crate::game::native_hooks::NativeColoredLight>,
) {
    game.holdmyitems_visuals(lights);
}
