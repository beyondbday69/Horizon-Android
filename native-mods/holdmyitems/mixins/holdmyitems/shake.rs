// native_hooks::Game::native_camera_shake (a hook point): adds Hold My Items eating, drinking,
// walking, and landing camera feedback.
fn holdmyitems_shake(
    game: &crate::game::Game<'_>,
    partial: f32,
    shake: &mut crate::game::native_hooks::NativeShake,
) {
    game.holdmyitems_shake(partial, shake);
}
