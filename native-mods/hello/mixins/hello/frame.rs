// Injected at the `frame` hook point (once a frame, while the game plays): the mod says hello
// once, in chat, so a build that carries it is unmistakable. This file is the whole mod.
fn hello_frame(game: &mut crate::game::Game<'_>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SAID: AtomicBool = AtomicBool::new(false);
    if !game.native_playing() || SAID.swap(true, Ordering::Relaxed) {
        return;
    }
    game.system_message(horizon_client::text::Text::literal(
        "hello: this is a native mod, built into the game — Horizon's own source was not edited.",
    ));
}
