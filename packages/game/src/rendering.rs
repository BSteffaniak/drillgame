#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::prelude::*;

use crate::{
    economy::{SurfaceZone, upgrade_offers},
    game_state::{GameState, TILE_SIZE},
    terrain::{ArtifactKind, MineralKind, TileKind, TilePosition},
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;
const PLAYER_DRAW_RADIUS: f32 = 12.0;

pub fn render(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.clear_background(Color::new(105, 190, 235, 255));

    let camera = Vector2::new(game.camera_x, game.camera_y);

    draw_world(draw, game, camera);
    draw_player(draw, game, camera);
    draw_hud(draw, game);

    if game.current_zone == Some(SurfaceZone::Shop) {
        draw_shop(draw, game);
    }

    if game.game_over {
        draw_game_over(draw, game);
    }
}

fn draw_world(draw: &mut RaylibDrawHandle<'_>, game: &GameState, camera: Vector2) {
    draw_surface_buildings(draw, camera);

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

            if tile.durability > 0 {
                draw.draw_rectangle_lines(
                    (x as f32 * TILE_SIZE - camera.x) as i32,
                    (y as f32 * TILE_SIZE - camera.y) as i32,
                    TILE_SIZE as i32,
                    TILE_SIZE as i32,
                    Color::new(0, 0, 0, 30),
                );
            }
        }
    }
}

fn draw_surface_buildings(draw: &mut RaylibDrawHandle<'_>, camera: Vector2) {
    draw_building(draw, camera, 0.0, 8.0, Color::DARKBLUE, "FUEL");
    draw_building(draw, camera, 8.0, 8.0, Color::MAROON, "REPAIR");
    draw_building(draw, camera, 16.0, 8.0, Color::DARKGREEN, "DEPOT");
    draw_building(draw, camera, 24.0, 12.0, Color::PURPLE, "SHOP");
}

fn draw_building(
    draw: &mut RaylibDrawHandle<'_>,
    camera: Vector2,
    tile_x: f32,
    tile_width: f32,
    color: Color,
    label: &str,
) {
    let x = tile_x * TILE_SIZE - camera.x;
    let y = 3.0 * TILE_SIZE - camera.y;
    let width = tile_width * TILE_SIZE;

    draw.draw_rectangle(x as i32, y as i32, width as i32, 64, color);
    draw.draw_text(label, x as i32 + 16, y as i32 + 20, 20, Color::WHITE);
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
    if game.drill_flash_seconds > 0.0 {
        draw.draw_circle_v(
            Vector2::new(screen_x, screen_y + 20.0),
            7.0,
            Color::new(255, 230, 80, 210),
        );
    }
}

fn draw_hud(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(12, 12, 560, 242, Color::new(0, 0, 0, 175));
    draw_bar(
        draw,
        24,
        26,
        "Fuel",
        game.player.fuel,
        game.player.fuel_capacity,
        Color::GOLD,
    );
    draw_bar(
        draw,
        24,
        56,
        "Hull",
        game.player.hull,
        game.player.max_hull(),
        Color::LIME,
    );
    draw.draw_text(
        &format!(
            "Cargo: {}/{}",
            game.player.cargo_used(),
            game.player.cargo_capacity
        ),
        24,
        86,
        20,
        Color::WHITE,
    );
    draw.draw_text(
        &format!("Credits: {}", game.player.credits),
        24,
        110,
        20,
        Color::WHITE,
    );
    draw.draw_text(
        &format!(
            "Drill {} | Engine {} | Hull {} | Radiator {}",
            game.player.drill_strength,
            game.player.engine_level,
            game.player.hull_level,
            game.player.radiator_level
        ),
        24,
        134,
        18,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!(
            "Depth: {:.0}m | F5 save | F9 load",
            (game.player.y / TILE_SIZE - 5.0).max(0.0)
        ),
        24,
        158,
        18,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!(
            "Contract: {}/{} {} = {} cr",
            game.contracts.active.progress(&game.player),
            game.contracts.active.required,
            game.contracts.active.target.name(),
            game.contracts.active.reward
        ),
        24,
        188,
        18,
        Color::RAYWHITE,
    );
    draw.draw_text(&game.message, 24, 216, 18, Color::RAYWHITE);

    draw_cargo_manifest(draw, game);
}

fn draw_bar(
    draw: &mut RaylibDrawHandle<'_>,
    x: i32,
    y: i32,
    label: &str,
    value: f32,
    max: f32,
    color: Color,
) {
    let ratio = (value / max).clamp(0.0, 1.0);
    draw.draw_text(label, x, y - 2, 18, Color::WHITE);
    draw.draw_rectangle(x + 70, y, 210, 18, Color::new(35, 35, 35, 255));
    draw.draw_rectangle(x + 70, y, (210.0 * ratio) as i32, 18, color);
    draw.draw_rectangle_lines(x + 70, y, 210, 18, Color::WHITE);
    draw.draw_text(
        &format!("{value:.0}/{max:.0}"),
        x + 292,
        y - 2,
        18,
        Color::WHITE,
    );
}

fn draw_cargo_manifest(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut y = 214;
    let cargo_rows =
        i32::try_from(game.player.cargo.len() + game.player.artifacts.len()).unwrap_or(i32::MAX);
    draw.draw_rectangle(
        12,
        y - 10,
        260,
        32 + cargo_rows * 22,
        Color::new(0, 0, 0, 150),
    );
    draw.draw_text("Cargo Manifest", 24, y, 18, Color::WHITE);
    y += 24;

    if game.player.cargo.is_empty() && game.player.artifacts.is_empty() {
        draw.draw_text("empty", 24, y, 16, Color::LIGHTGRAY);
        return;
    }

    for (mineral, count) in &game.player.cargo {
        draw.draw_text(
            &format!(
                "{} x{} = {}",
                mineral.name(),
                count,
                mineral.value() * count
            ),
            24,
            y,
            16,
            mineral_color(*mineral),
        );
        y += 22;
    }

    for (artifact, count) in &game.player.artifacts {
        draw.draw_text(
            &format!(
                "{} x{} = {}",
                artifact.name(),
                count,
                artifact.value() * count
            ),
            24,
            y,
            16,
            artifact_color(*artifact),
        );
        y += 22;
    }
}

fn draw_shop(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(780, 18, 480, 190, Color::new(0, 0, 0, 185));
    draw.draw_text("Upgrade Shop", 800, 34, 24, Color::WHITE);

    for (index, offer) in upgrade_offers(&game.player).iter().enumerate() {
        let y = 70 + i32::try_from(index).unwrap_or(i32::MAX) * 26;
        let price = if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            "MAX".to_owned()
        } else {
            format!("{} cr", offer.cost)
        };
        draw.draw_text(
            &format!(
                "{}: {} L{} -> {} | {}",
                index + 1,
                offer.name,
                offer.level,
                price,
                offer.description
            ),
            800,
            y,
            18,
            Color::RAYWHITE,
        );
    }
}

fn draw_game_over(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 150));
    draw.draw_rectangle(360, 270, 560, 130, Color::new(35, 20, 20, 240));
    draw.draw_text("EMERGENCY", 535, 294, 34, Color::RED);
    draw.draw_text(&game.message, 395, 340, 20, Color::WHITE);
    draw.draw_text(
        "Press E to pay rescue fee and return to base",
        430,
        368,
        18,
        Color::RAYWHITE,
    );
}

const fn tile_color(kind: TileKind) -> Color {
    match kind {
        TileKind::Air => Color::BLANK,
        TileKind::Dirt => Color::new(116, 72, 37, 255),
        TileKind::Clay => Color::new(141, 86, 55, 255),
        TileKind::Stone => Color::new(92, 92, 96, 255),
        TileKind::HardRock => Color::new(54, 54, 60, 255),
        TileKind::Lava => Color::new(255, 84, 28, 255),
        TileKind::Ore(mineral) => mineral_color(mineral),
        TileKind::Artifact(artifact) => artifact_color(artifact),
    }
}

const fn mineral_color(mineral: MineralKind) -> Color {
    match mineral {
        MineralKind::Copper => Color::new(184, 102, 42, 255),
        MineralKind::Iron => Color::new(168, 150, 132, 255),
        MineralKind::Silver => Color::new(205, 220, 225, 255),
        MineralKind::Gold => Color::GOLD,
        MineralKind::Emerald => Color::GREEN,
        MineralKind::Ruby => Color::RED,
        MineralKind::Diamond => Color::SKYBLUE,
    }
}

const fn artifact_color(artifact: ArtifactKind) -> Color {
    match artifact {
        ArtifactKind::Fossil => Color::BEIGE,
        ArtifactKind::OldCircuit => Color::VIOLET,
        ArtifactKind::BuriedIdol => Color::PINK,
        ArtifactKind::StarCore => Color::new(120, 220, 255, 255),
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
