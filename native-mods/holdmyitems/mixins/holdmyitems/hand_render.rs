// totem::Game::item_activation_geometry HEAD (pinned): replaces `out` with Hold
// My Items' first-person arms, 3D blocks, items, per-quad animations, maps, and particles.
fn holdmyitems_on_hand_render(
    game: &crate::game::Game<'_>,
    partial: f32,
    out: &mut Vec<horizon_client::mesher::Vertex>,
) {
    game.holdmyitems_render_hands(partial, out);
}
