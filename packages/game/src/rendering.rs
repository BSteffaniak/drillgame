#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::prelude::*;

use crate::{
    game_state::{GameState, TILE_SIZE},
    terrain::{TileKind, TilePosition},
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;
const PLAYER_DRAW_RADIUS: f32 = 12.0;

pub fn render(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.clear_background(Color::new(105, 190, 235, 255));

    let camera = camera_offset(game);

    draw_world(draw, game, camera);
    draw_player(draw, game, camera);
    draw_hud(draw, game);
}

fn draw_world(draw: &mut RaylibDrawHandle<'_>, game: &GameState, camera: Vector2) {
    draw_surface_base(draw, camera);

    let visible = visible_tile_bounds(camera, game);
    for y in visible.min_y..=visible.max_y {
        for x in visible.min_x..=visible.max_x {
            let position = TilePosition { x, y };
            let Some(tile) = game.terrain.tile(position) else {
                continue;
            };

            if tile.kind == TileKind::Air {
                continue;
            }

            draw.draw_rectangle(
                (x as f32 * TILE_SIZE - camera.x) as i32,
                (y as f32 * TILE_SIZE - camera.y) as i32,
                TILE_SIZE as i32,
                TILE_SIZE as i32,
                tile_color(tile.kind),
            );
        }
    }
}

fn draw_surface_base(draw: &mut RaylibDrawHandle<'_>, camera: Vector2) {
    let x = -camera.x;
    let y = 3.0 * TILE_SIZE - camera.y;

    draw.draw_rectangle(
        x as i32,
        y as i32,
        (24.0 * TILE_SIZE) as i32,
        64,
        Color::DARKBLUE,
    );
    draw.draw_text("BASE", x as i32 + 24, y as i32 + 18, 24, Color::WHITE);
}

fn draw_player(draw: &mut RaylibDrawHandle<'_>, game: &GameState, camera: Vector2) {
    let screen_x = game.player.x - camera.x;
    let screen_y = game.player.y - camera.y;

    draw.draw_circle_v(
        Vector2::new(screen_x, screen_y),
        PLAYER_DRAW_RADIUS,
        Color::new(235, 190, 45, 255),
    );
    draw.draw_triangle(
        Vector2::new(screen_x - 8.0, screen_y + 10.0),
        Vector2::new(screen_x + 8.0, screen_y + 10.0),
        Vector2::new(screen_x, screen_y + 22.0),
        Color::ORANGE,
    );
}

fn draw_hud(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(12, 12, 380, 138, Color::new(0, 0, 0, 170));
    draw.draw_text(
        &format!(
            "Fuel: {:03.0}/{:03.0}",
            game.player.fuel, game.player.fuel_capacity
        ),
        24,
        24,
        20,
        Color::WHITE,
    );
    draw.draw_text(
        &format!("Hull: {:03.0}", game.player.hull),
        24,
        48,
        20,
        Color::WHITE,
    );
    draw.draw_text(
        &format!(
            "Cargo: {}/{}",
            game.player.cargo, game.player.cargo_capacity
        ),
        24,
        72,
        20,
        Color::WHITE,
    );
    draw.draw_text(
        &format!("Credits: {}", game.player.credits),
        24,
        96,
        20,
        Color::WHITE,
    );
    draw.draw_text(&game.message, 24, 120, 18, Color::RAYWHITE);
}

fn camera_offset(game: &GameState) -> Vector2 {
    let max_x = game.terrain.width() as f32 * TILE_SIZE - SCREEN_WIDTH as f32;
    let max_y = game.terrain.height() as f32 * TILE_SIZE - SCREEN_HEIGHT as f32;

    Vector2::new(
        (game.player.x - SCREEN_WIDTH as f32 / 2.0).clamp(0.0, max_x),
        (game.player.y - SCREEN_HEIGHT as f32 / 2.0).clamp(0.0, max_y),
    )
}

const fn tile_color(kind: TileKind) -> Color {
    match kind {
        TileKind::Air => Color::BLANK,
        TileKind::Dirt => Color::new(116, 72, 37, 255),
        TileKind::Stone => Color::new(92, 92, 96, 255),
        TileKind::Ore => Color::new(42, 184, 124, 255),
    }
}

struct VisibleTileBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

fn visible_tile_bounds(camera: Vector2, game: &GameState) -> VisibleTileBounds {
    VisibleTileBounds {
        min_x: (camera.x / TILE_SIZE).floor().max(0.0) as i32,
        max_x: ((camera.x + SCREEN_WIDTH as f32) / TILE_SIZE)
            .ceil()
            .min(game.terrain.width() as f32 - 1.0) as i32,
        min_y: (camera.y / TILE_SIZE).floor().max(0.0) as i32,
        max_y: ((camera.y + SCREEN_HEIGHT as f32) / TILE_SIZE)
            .ceil()
            .min(game.terrain.height() as f32 - 1.0) as i32,
    }
}
