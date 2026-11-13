// native_hooks::Game::native_settings (a hook point): populates Options > Mods > Hold My Items buttons.
fn holdmyitems_page(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id == crate::game::holdmyitems::ID {
        buttons.extend(game.holdmyitems_settings());
    }
}
