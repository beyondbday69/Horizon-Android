// native_hooks::Game::native_settings_click (a hook point): handles clicks in Options > Mods > Hold My Items.
fn holdmyitems_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id == crate::game::holdmyitems::ID {
        game.holdmyitems_settings_click(button);
    }
}
