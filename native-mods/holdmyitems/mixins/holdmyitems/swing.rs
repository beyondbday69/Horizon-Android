// hand::Game::swing_hand HEAD (pinned): notifies Hold My Items of a main-hand or off-hand swing start.
fn holdmyitems_on_swing(
    game: &mut crate::game::Game<'_>,
    hand: horizon_client::first_person::Hand,
) {
    game.holdmyitems_swing_hand(hand);
}
