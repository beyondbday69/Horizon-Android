// native_hooks::Game::native_tick (a hook point): advances Hold My Items 20 TPS per-hand swing progress.
fn holdmyitems_tick(game: &mut crate::game::Game<'_>) {
    game.holdmyitems_tick();
}
