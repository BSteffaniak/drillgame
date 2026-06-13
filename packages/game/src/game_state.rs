#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "world coordinates intentionally cross integer tile and floating render spaces"
)]

use std::fmt::Write as _;

use crate::{
    economy::{
        PurchaseError, SurfaceZone, buy_upgrade, refuel, repair, sell_cargo, upgrade_offers,
    },
    input::PlayerInput,
    player::Player,
    terrain::{MineResult, Terrain, TileKind, TilePosition},
};

pub const TILE_SIZE: f32 = 32.0;
const WORLD_WIDTH: i32 = 120;
const WORLD_HEIGHT: i32 = 90;
const GRAVITY: f32 = 780.0;
const HORIZONTAL_ACCELERATION: f32 = 900.0;
const THRUST_ACCELERATION: f32 = 1_250.0;
const MAX_HORIZONTAL_SPEED: f32 = 260.0;
const MAX_FALL_SPEED: f32 = 560.0;
const DRAG: f32 = 0.86;
const FUEL_BURN_PER_SECOND: f32 = 5.0;
const DRILL_FUEL_COST: f32 = 0.45;
const PLAYER_RADIUS: f32 = 12.0;
const SAFE_LANDING_SPEED: f32 = 330.0;
const CRASH_DAMAGE_SCALE: f32 = 0.11;

#[derive(Debug)]
pub struct GameState {
    pub terrain: Terrain,
    pub player: Player,
    pub message: String,
    pub current_zone: Option<SurfaceZone>,
    pub game_over: bool,
}

impl GameState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            terrain: Terrain::new(WORLD_WIDTH, WORLD_HEIGHT),
            player: Player::new(12.0 * TILE_SIZE, 4.0 * TILE_SIZE),
            message: "Mine ore, sell cargo, and buy upgrades. Press E at surface buildings."
                .to_owned(),
            current_zone: None,
            game_over: false,
        }
    }

    pub fn update(&mut self, input: PlayerInput, delta_seconds: f32) {
        if self.game_over {
            self.handle_rescue(input);
            return;
        }

        self.current_zone = surface_zone_at(self.player.x, self.player.y);
        self.handle_interaction(input);
        self.apply_movement(input, delta_seconds);
        self.try_mine(input);
        self.update_status_messages();
        self.check_failure();
    }

    fn handle_interaction(&mut self, input: PlayerInput) {
        if let Some(index) = input.selected_upgrade {
            self.try_buy_upgrade(index);
            return;
        }

        if !input.interact {
            return;
        }

        match self.current_zone {
            Some(SurfaceZone::Fuel) => {
                refuel(&mut self.player);
                "Fuel tank filled.".clone_into(&mut self.message);
            }
            Some(SurfaceZone::Repair) => {
                repair(&mut self.player);
                "Hull repaired.".clone_into(&mut self.message);
            }
            Some(SurfaceZone::Depot) => {
                let payout = sell_cargo(&mut self.player);
                if payout == 0 {
                    "No cargo to sell.".clone_into(&mut self.message);
                } else {
                    self.message = format!("Sold cargo for {payout} credits.");
                }
            }
            Some(SurfaceZone::Shop) => {
                "Upgrade shop: press 1-5 to buy listed upgrades.".clone_into(&mut self.message);
            }
            None => "No surface service here.".clone_into(&mut self.message),
        }
    }

    fn try_buy_upgrade(&mut self, index: usize) {
        if self.current_zone != Some(SurfaceZone::Shop) {
            return;
        }

        match buy_upgrade(&mut self.player, index) {
            Ok(offer) => self.message = format!("Bought {} upgrade.", offer.name),
            Err(PurchaseError::InvalidSelection) => {
                "Unknown upgrade selection.".clone_into(&mut self.message);
            }
            Err(PurchaseError::MaxLevel) => {
                "That upgrade is already maxed.".clone_into(&mut self.message);
            }
            Err(PurchaseError::NotEnoughCredits) => {
                "Not enough credits for that upgrade.".clone_into(&mut self.message);
            }
        }
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;
        let engine_multiplier = 1.0 + f32::from(self.player.engine_level.saturating_sub(1)) * 0.18;

        self.player.velocity_x +=
            input.horizontal * HORIZONTAL_ACCELERATION * engine_multiplier * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * engine_multiplier * delta_seconds;
            self.player.fuel = (self.player.fuel - FUEL_BURN_PER_SECOND * delta_seconds).max(0.0);
        }

        self.player.velocity_y += GRAVITY * delta_seconds;
        self.player.velocity_x *= DRAG.powf(delta_seconds * 60.0);
        self.player.velocity_x = self.player.velocity_x.clamp(
            -MAX_HORIZONTAL_SPEED * engine_multiplier,
            MAX_HORIZONTAL_SPEED * engine_multiplier,
        );
        self.player.velocity_y = self
            .player
            .velocity_y
            .clamp(-MAX_FALL_SPEED, MAX_FALL_SPEED);

        self.move_axis(self.player.velocity_x * delta_seconds, 0.0);
        self.move_axis(0.0, self.player.velocity_y * delta_seconds);
    }

    fn move_axis(&mut self, delta_x: f32, delta_y: f32) {
        let next_x = self.player.x + delta_x;
        let next_y = self.player.y + delta_y;

        if self.collides(next_x, next_y) {
            if delta_x != 0.0 {
                self.player.velocity_x = 0.0;
            }
            if delta_y > 0.0 {
                self.apply_landing_damage();
            }
            if delta_y != 0.0 {
                self.player.velocity_y = 0.0;
            }
            return;
        }

        self.player.x = next_x.clamp(0.0, (self.terrain.width() as f32 - 1.0) * TILE_SIZE);
        self.player.y = next_y.clamp(0.0, (self.terrain.height() as f32 - 1.0) * TILE_SIZE);
    }

    fn apply_landing_damage(&mut self) {
        if self.player.velocity_y <= SAFE_LANDING_SPEED {
            return;
        }

        let damage = (self.player.velocity_y - SAFE_LANDING_SPEED) * CRASH_DAMAGE_SCALE;
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.message = format!("Hard landing! Hull took {damage:.0} damage.");
    }

    fn collides(&self, x: f32, y: f32) -> bool {
        collision_points(x, y)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    fn try_mine(&mut self, input: PlayerInput) {
        let Some(target) = mine_target(&self.player, input) else {
            return;
        };

        if self.player.fuel < DRILL_FUEL_COST {
            "Out of fuel. Reach a fuel station or await rescue.".clone_into(&mut self.message);
            return;
        }

        match self.terrain.mine(target, self.player.drill_strength) {
            MineResult::Blocked => {}
            MineResult::TooHard => {
                "That layer is too hard. Upgrade your drill.".clone_into(&mut self.message);
            }
            MineResult::Chipped => {
                self.player.fuel -= DRILL_FUEL_COST;
                "Drilling...".clone_into(&mut self.message);
            }
            MineResult::Mined(mined) => self.collect_mined_tile(mined),
        }
    }

    fn collect_mined_tile(&mut self, mined: TileKind) {
        self.player.fuel -= DRILL_FUEL_COST;

        if let TileKind::Ore(mineral) = mined {
            if self.player.add_cargo(mineral) {
                self.message = format!("Loaded {} ore worth {}.", mineral.name(), mineral.value());
            } else {
                "Cargo full. Return to depot to sell.".clone_into(&mut self.message);
            }
        } else {
            "Tunnel opened.".clone_into(&mut self.message);
        }
    }

    fn update_status_messages(&mut self) {
        if let Some(zone) = self.current_zone {
            self.message = match zone {
                SurfaceZone::Fuel => "Fuel Station: press E to refuel.".to_owned(),
                SurfaceZone::Repair => "Repair Garage: press E to repair hull.".to_owned(),
                SurfaceZone::Depot => "Ore Depot: press E to sell cargo.".to_owned(),
                SurfaceZone::Shop => shop_prompt(&self.player),
            };
        }
    }

    fn check_failure(&mut self) {
        if self.player.hull <= 0.0 {
            self.game_over = true;
            "Hull destroyed! Press E for emergency rescue.".clone_into(&mut self.message);
        } else if self.player.fuel <= 0.0 && self.player.y > 6.0 * TILE_SIZE {
            self.game_over = true;
            "Out of fuel underground! Press E for emergency rescue.".clone_into(&mut self.message);
        }
    }

    fn handle_rescue(&mut self, input: PlayerInput) {
        if !input.interact {
            return;
        }

        let fee = self.player.credits.min(50);
        self.player.credits -= fee;
        self.player.x = 12.0 * TILE_SIZE;
        self.player.y = 4.0 * TILE_SIZE;
        self.player.velocity_x = 0.0;
        self.player.velocity_y = 0.0;
        self.player.fuel = self.player.fuel_capacity * 0.5;
        self.player.hull = self.player.max_hull() * 0.5;
        self.game_over = false;
        self.message = format!("Emergency rescue completed. Fee: {fee} credits.");
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

fn mine_target(player: &Player, input: PlayerInput) -> Option<TilePosition> {
    if !input.drill_down && input.horizontal == 0.0 {
        return None;
    }

    let current_tile = player.tile_position(TILE_SIZE);
    Some(if input.drill_down {
        TilePosition {
            x: current_tile.x,
            y: current_tile.y + 1,
        }
    } else {
        TilePosition {
            x: current_tile.x + facing_direction(input.horizontal),
            y: current_tile.y,
        }
    })
}

fn surface_zone_at(x: f32, y: f32) -> Option<SurfaceZone> {
    if y > 5.5 * TILE_SIZE {
        return None;
    }

    match (x / TILE_SIZE).floor() as i32 {
        0..=7 => Some(SurfaceZone::Fuel),
        8..=15 => Some(SurfaceZone::Repair),
        16..=23 => Some(SurfaceZone::Depot),
        24..=35 => Some(SurfaceZone::Shop),
        _ => None,
    }
}

fn shop_prompt(player: &Player) -> String {
    let offers = upgrade_offers(player);
    let mut prompt = String::from("Upgrade Shop: ");
    for (index, offer) in offers.iter().enumerate() {
        let label = if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            "MAX".to_owned()
        } else {
            offer.cost.to_string()
        };
        let _ = write!(prompt, "{}:{}({label}) ", index + 1, offer.name);
    }
    prompt
}

fn collision_points(x: f32, y: f32) -> [TilePosition; 4] {
    [
        point_to_tile(x - PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x - PLAYER_RADIUS, y + PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y + PLAYER_RADIUS),
    ]
}

fn point_to_tile(x: f32, y: f32) -> TilePosition {
    TilePosition {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

fn facing_direction(horizontal: f32) -> i32 {
    if horizontal < 0.0 { -1 } else { 1 }
}
