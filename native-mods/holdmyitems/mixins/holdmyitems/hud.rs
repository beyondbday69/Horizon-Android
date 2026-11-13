// native_hooks::Game::native_hud (a hook point): renders any queued Hold My Items debug lines.
fn holdmyitems_hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
) {
    game.holdmyitems_hud(batch);
}
