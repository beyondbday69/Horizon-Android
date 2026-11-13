// native_hooks::Game::native_key (a hook point): tracks Hold My Items' Inspect keybind.
fn holdmyitems_on_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    game.holdmyitems_key(key, pressed);
}
