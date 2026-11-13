// native_hooks::Game::native_mods (a hook point): lists Hold My Items 5.1.1 in Options > Mods.
fn holdmyitems_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    mods.push(crate::game::Game::holdmyitems_mod_entry());
}
