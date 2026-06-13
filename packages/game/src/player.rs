#![allow(
    clippy::cast_possible_truncation,
    reason = "player world positions intentionally convert to terrain tile coordinates"
)]

use crate::terrain::TilePosition;

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub fuel: f32,
    pub fuel_capacity: f32,
    pub hull: f32,
    pub cargo: u32,
    pub cargo_capacity: u32,
    pub credits: u32,
    pub drill_strength: u8,
}

impl Player {
    #[must_use]
    pub const fn new(spawn_x: f32, spawn_y: f32) -> Self {
        Self {
            x: spawn_x,
            y: spawn_y,
            velocity_x: 0.0,
            velocity_y: 0.0,
            fuel: 100.0,
            fuel_capacity: 100.0,
            hull: 100.0,
            cargo: 0,
            cargo_capacity: 12,
            credits: 0,
            drill_strength: 1,
        }
    }

    #[must_use]
    pub fn tile_position(self, tile_size: f32) -> TilePosition {
        TilePosition {
            x: (self.x / tile_size).floor() as i32,
            y: (self.y / tile_size).floor() as i32,
        }
    }

    #[must_use]
    pub const fn has_cargo_space(self) -> bool {
        self.cargo < self.cargo_capacity
    }
}
