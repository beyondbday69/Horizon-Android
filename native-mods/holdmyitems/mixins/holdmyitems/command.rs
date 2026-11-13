// native_hooks::Game::native_command (a hook point): handles `/hmi` and `/holdmyitems` commands.
fn holdmyitems_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    game.holdmyitems_command(command, ran);
}
