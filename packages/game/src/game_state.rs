#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "world coordinates intentionally cross integer tile and floating render spaces"
)]

use crate::{
    economy::service_surface_base,
    input::PlayerInput,
    player::Player,
    terrain::{Terrain, TileKind, TilePosition},
};

pub const TILE_SIZE: f32 = 32.0;
const WORLD_WIDTH: i32 = 120;
const WORLD_HEIGHT: i32 = 90;
const GRAVITY: f32 = 780.0;
const HORIZONTAL_ACCELERATION: f32 = 900.0;
const THRUST_ACCELERATION: f32 = 1_250.0;
const MAX_HORIZONTAL_SPEED: f32 = 260.0;
const MAX_FALL_SPEED: f32 = 520.0;
const DRAG: f32 = 0.86;
const FUEL_BURN_PER_SECOND: f32 = 5.0;
const DRILL_FUEL_COST: f32 = 0.35;
const PLAYER_RADIUS: f32 = 12.0;

#[derive(Debug)]
pub struct GameState {
    pub terrain: Terrain,
    pub player: Player,
    pub message: String,
}

impl GameState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            terrain: Terrain::new(WORLD_WIDTH, WORLD_HEIGHT),
            player: Player::new(12.0 * TILE_SIZE, 4.0 * TILE_SIZE),
            message: "Mine ore, manage fuel, and return to base.".to_owned(),
        }
    }

    pub fn update(&mut self, input: PlayerInput, delta_seconds: f32) {
        self.apply_movement(input, delta_seconds);
        self.try_mine(input);
        self.service_base_if_needed();
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;

        self.player.velocity_x += input.horizontal * HORIZONTAL_ACCELERATION * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * delta_seconds;
            self.player.fuel = (self.player.fuel - FUEL_BURN_PER_SECOND * delta_seconds).max(0.0);
        }

        self.player.velocity_y += GRAVITY * delta_seconds;
        self.player.velocity_x *= DRAG.powf(delta_seconds * 60.0);
        self.player.velocity_x = self
            .player
            .velocity_x
            .clamp(-MAX_HORIZONTAL_SPEED, MAX_HORIZONTAL_SPEED);
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
            if delta_y != 0.0 {
                self.player.velocity_y = 0.0;
            }
            return;
        }

        self.player.x = next_x.clamp(0.0, (self.terrain.width() as f32 - 1.0) * TILE_SIZE);
        self.player.y = next_y.clamp(0.0, (self.terrain.height() as f32 - 1.0) * TILE_SIZE);
    }

    fn collides(&self, x: f32, y: f32) -> bool {
        collision_points(x, y)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    fn try_mine(&mut self, input: PlayerInput) {
        let current_tile = self.player.tile_position(TILE_SIZE);
        let target = if input.drill_down {
            TilePosition {
                x: current_tile.x,
                y: current_tile.y + 1,
            }
        } else {
            TilePosition {
                x: current_tile.x + facing_direction(self.player.velocity_x),
                y: current_tile.y,
            }
        };

        if self.player.fuel < DRILL_FUEL_COST {
            "Out of fuel. Limp back to the surface base.".clone_into(&mut self.message);
            return;
        }

        let Some(mined) = self.terrain.mine(target, self.player.drill_strength) else {
            return;
        };

        self.player.fuel -= DRILL_FUEL_COST;

        if mined == TileKind::Ore && self.player.has_cargo_space() {
            self.player.cargo += 1;
            "Ore loaded into cargo hold.".clone_into(&mut self.message);
        } else if mined == TileKind::Ore {
            "Cargo full. Return to base to sell.".clone_into(&mut self.message);
        } else {
            "Tunnel opened.".clone_into(&mut self.message);
        }
    }

    fn service_base_if_needed(&mut self) {
        if self.player.y <= 5.0 * TILE_SIZE && self.player.x <= 24.0 * TILE_SIZE {
            let had_cargo = self.player.cargo > 0;
            service_surface_base(&mut self.player);
            if had_cargo {
                "Cargo sold, hull repaired, fuel topped off.".clone_into(&mut self.message);
            } else {
                "Surface base: fuel and repairs topped off.".clone_into(&mut self.message);
            }
        }
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
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

fn facing_direction(velocity_x: f32) -> i32 {
    if velocity_x < -5.0 { -1 } else { 1 }
}
