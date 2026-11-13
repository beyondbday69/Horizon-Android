// native_hooks::Game::native_frame (a hook point): steps Hold My Items spring-damper physics,
// plays hand-switch/bell/flower sounds, and pre-caches local map vertices.
fn holdmyitems_frame(game: &mut crate::game::Game<'_>, elapsed: std::time::Duration) {
    game.holdmyitems_frame(elapsed);
}
