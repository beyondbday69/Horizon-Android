// native_hooks::Game::native_commands (a hook point): registers `/hmi` and `/holdmyitems` for chat completion.
fn holdmyitems_commands(commands: &mut Vec<&'static str>) {
    if !commands.contains(&crate::game::holdmyitems::COMMAND_HMI) {
        commands.push(crate::game::holdmyitems::COMMAND_HMI);
    }
    if !commands.contains(&crate::game::holdmyitems::COMMAND_HOLDMYITEMS) {
        commands.push(crate::game::holdmyitems::COMMAND_HOLDMYITEMS);
    }
}
