use crate::player::Player;

const ORE_SELL_PRICE: u32 = 15;

pub const fn service_surface_base(player: &mut Player) {
    player.fuel = player.fuel_capacity;
    player.hull = 100.0;

    if player.cargo > 0 {
        player.credits += player.cargo * ORE_SELL_PRICE;
        player.cargo = 0;
    }
}
