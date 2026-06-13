#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::prelude::*;

use crate::{
    economy::{upgrade_effect, upgrade_offers, upgrade_tier_name},
    game_state::{DrillDirection, GameState, ModalScreen, PauseOption, RunMode, TILE_SIZE},
    terrain::{ArtifactKind, MineralKind, TileKind, TilePosition},
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;
const PLAYER_DRAW_RADIUS: f32 = 12.0;

struct MinimapProjection {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    terrain_width: i32,
    terrain_height: i32,
}

pub fn render(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.clear_background(Color::new(105, 190, 235, 255));

    let camera = Vector2::new(game.camera_x, game.camera_y);

    draw_world(draw, game, camera);
    draw_particles(draw, game, camera);
    for cloud in &game.hazard_clouds {
        draw.draw_circle(
            (cloud.x - camera.x) as i32,
            (cloud.y - camera.y) as i32,
            cloud.radius,
            Color::new(90, 190, 80, 70),
        );
    }

    draw_player(draw, game, camera);
    draw_hud(draw, game);
    draw_depth_ruler(draw, game);
    draw_minimap(draw, game);

    if game.run_mode == RunMode::Title {
        draw_title(draw);
    } else if game.run_mode == RunMode::Paused {
        draw_pause(draw, game);
    }

    if let Some(modal) = game.modal {
        draw_modal(draw, game, modal);
    }

    if game.game_over {
        draw_game_over(draw, game);
    }

    if game.won_game {
        draw_ending(draw, game);
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

            let explored = game.is_explored(position);
            if explored && tile.kind == TileKind::Air {
                continue;
            }

            draw.draw_rectangle(
                (x as f32 * TILE_SIZE - camera.x) as i32,
                (y as f32 * TILE_SIZE - camera.y) as i32,
                TILE_SIZE as i32,
                TILE_SIZE as i32,
                if explored {
                    tile_color(tile.kind)
                } else {
                    Color::new(18, 14, 18, 255)
                },
            );

            if explored && tile.durability > 0 {
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
    if let Some(drill) = game.active_drill {
        let x = (drill.target.x as f32 * TILE_SIZE - camera.x) as i32;
        let y = (drill.target.y as f32 * TILE_SIZE - camera.y) as i32;
        let current_durability = game
            .terrain
            .tile(drill.target)
            .map_or(drill.initial_durability, |tile| tile.durability);
        let chipped = drill.initial_durability.saturating_sub(current_durability);
        let progress = ((f32::from(chipped) + drill.progress.clamp(0.0, 1.0))
            / f32::from(drill.initial_durability.max(1)))
        .clamp(0.0, 1.0);
        let bar_width = (TILE_SIZE * progress) as i32;
        draw.draw_rectangle(
            x + 4,
            y + TILE_SIZE as i32 - 7,
            TILE_SIZE as i32 - 8,
            3,
            Color::new(0, 0, 0, 120),
        );
        draw.draw_rectangle(
            x + 4,
            y + TILE_SIZE as i32 - 7,
            bar_width.min(TILE_SIZE as i32 - 8),
            3,
            Color::new(255, 215, 90, 220),
        );
        draw.draw_circle(x + 10, y + 11, 2.0, Color::new(255, 235, 150, 170));
        if progress > 0.45 {
            draw.draw_circle(x + 21, y + 18, 2.0, Color::new(255, 235, 150, 150));
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

fn draw_particles(draw: &mut RaylibDrawHandle<'_>, game: &GameState, camera: Vector2) {
    for particle in &game.dust_particles {
        let alpha = (particle.life / 0.35).clamp(0.0, 1.0);
        draw.draw_circle_v(
            Vector2::new(particle.x - camera.x, particle.y - camera.y),
            4.0,
            Color::new(190, 150, 105, (180.0 * alpha) as u8),
        );
    }
}

fn drill_visual_offset(game: &GameState) -> (f32, f32) {
    let Some(drill) = game.active_drill else {
        return (0.0, 0.0);
    };
    let pulse = (drill.progress * std::f32::consts::TAU * 3.0).sin() * 1.8;
    match drill.direction {
        DrillDirection::Down => (0.0, pulse.abs()),
        DrillDirection::Left => (-pulse.abs(), 0.0),
        DrillDirection::Right => (pulse.abs(), 0.0),
    }
}

fn draw_player(draw: &mut RaylibDrawHandle<'_>, game: &GameState, camera: Vector2) {
    let (offset_x, offset_y) = drill_visual_offset(game);
    let screen_x = game.player.x - camera.x + offset_x;
    let screen_y = game.player.y - camera.y + offset_y;

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
    draw_compact_status(draw, game);
    draw_message_toast(draw, game);

    if game.show_details || game.modal == Some(ModalScreen::Depot) {
        draw_detail_panel(draw, game);
    }
}

fn draw_compact_status(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(10, 10, SCREEN_WIDTH - 20, 54, Color::new(0, 0, 0, 120));
    draw_mini_bar(
        draw,
        22,
        20,
        "Fuel",
        game.player.fuel,
        game.player.fuel_capacity,
        Color::GOLD,
    );
    draw_mini_bar(
        draw,
        22,
        42,
        "Hull",
        game.player.hull,
        game.player.max_hull(),
        Color::LIME,
    );

    draw.draw_text(
        &format!("Credits {}", game.player.credits),
        340,
        20,
        18,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!("Depth {:.0}m", (game.player.y / TILE_SIZE - 5.0).max(0.0)),
        340,
        42,
        18,
        Color::RAYWHITE,
    );

    draw.draw_text(
        &format!(
            "Cargo {}/{}",
            game.player.cargo_used(),
            game.player.cargo_capacity
        ),
        520,
        20,
        18,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!(
            "Contract {}: {}/{} {}",
            game.contracts.active.title,
            game.contracts.active.progress(&game.player),
            game.contracts.active.required,
            game.contracts.active.target.name()
        ),
        520,
        42,
        18,
        Color::RAYWHITE,
    );

    draw.draw_text(
        &format!(
            "D{} E{} H{} R{} | Tab details | F5/F9",
            game.player.drill_strength,
            game.player.engine_level,
            game.player.hull_level,
            game.player.radiator_level
        ),
        810,
        31,
        18,
        Color::LIGHTGRAY,
    );
}

fn draw_mini_bar(
    draw: &mut RaylibDrawHandle<'_>,
    x: i32,
    y: i32,
    label: &str,
    value: f32,
    max: f32,
    color: Color,
) {
    let ratio = (value / max).clamp(0.0, 1.0);
    draw.draw_text(label, x, y - 5, 16, Color::WHITE);
    draw.draw_rectangle(x + 46, y - 2, 210, 14, Color::new(35, 35, 35, 220));
    draw.draw_rectangle(x + 46, y - 2, (210.0 * ratio) as i32, 14, color);
    draw.draw_rectangle_lines(x + 46, y - 2, 210, 14, Color::new(230, 230, 230, 180));
    draw.draw_text(
        &format!("{value:.0}/{max:.0}"),
        x + 266,
        y - 5,
        16,
        Color::WHITE,
    );
}

fn draw_message_toast(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    if game.message.is_empty() || game.run_mode != RunMode::Playing || game.modal.is_some() {
        return;
    }

    let text_width = i32::try_from(game.message.len()).unwrap_or(i32::MAX) * 10;
    let width = text_width.clamp(260, 820);
    let x = (SCREEN_WIDTH - width) / 2;
    let y = SCREEN_HEIGHT - 76;
    draw.draw_rectangle(x, y, width, 42, Color::new(0, 0, 0, 145));
    draw.draw_rectangle_lines(x, y, width, 42, Color::new(255, 255, 255, 120));
    draw.draw_text(&game.message, x + 16, y + 12, 18, Color::RAYWHITE);
}

fn draw_detail_panel(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let x = SCREEN_WIDTH - 330;
    let y = 82;
    let cargo_rows =
        i32::try_from(game.player.cargo.len() + game.player.artifacts.len()).unwrap_or(i32::MAX);
    let height = 156 + cargo_rows * 22;
    draw.draw_rectangle(x, y, 312, height, Color::new(0, 0, 0, 170));
    draw.draw_rectangle_lines(x, y, 312, height, Color::new(255, 255, 255, 120));

    draw.draw_text("Details", x + 14, y + 12, 22, Color::WHITE);
    draw.draw_text(
        &format!("Seed {:X}", game.terrain.seed()),
        x + 14,
        y + 40,
        16,
        Color::LIGHTGRAY,
    );
    draw.draw_text(
        &format!(
            "Contract: {}/{} {} = {} cr",
            game.contracts.active.progress(&game.player),
            game.contracts.active.required,
            game.contracts.active.target.name(),
            game.contracts.active.reward
        ),
        x + 14,
        y + 62,
        16,
        Color::RAYWHITE,
    );

    let mut row_y = y + 96;
    draw.draw_text("Cargo", x + 14, row_y, 18, Color::WHITE);
    row_y += 24;

    if game.player.cargo.is_empty() && game.player.artifacts.is_empty() {
        draw.draw_text("empty", x + 14, row_y, 16, Color::LIGHTGRAY);
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
            x + 14,
            row_y,
            16,
            mineral_color(*mineral),
        );
        row_y += 22;
    }

    for (artifact, count) in &game.player.artifacts {
        draw.draw_text(
            &format!(
                "{} x{} = {}",
                artifact.name(),
                count,
                artifact.value() * count
            ),
            x + 14,
            row_y,
            16,
            artifact_color(*artifact),
        );
        row_y += 22;
    }
}

fn draw_minimap(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let x = SCREEN_WIDTH - 168;
    let y = SCREEN_HEIGHT - 166;
    let width = 130;
    let height = 96;
    draw.draw_rectangle(
        x - 8,
        y - 24,
        width + 16,
        height + 32,
        Color::new(0, 0, 0, 150),
    );
    draw.draw_text("Mine Map", x, y - 20, 16, Color::WHITE);

    let terrain_width = game.terrain.width().max(1);
    let terrain_height = game.terrain.height().max(1);
    let projection = MinimapProjection {
        x,
        y,
        width,
        height,
        terrain_width,
        terrain_height,
    };
    for ty in 0..terrain_height {
        for tx in 0..terrain_width {
            let position = TilePosition { x: tx, y: ty };
            if !game.is_explored(position) {
                continue;
            }
            let Some(tile) = game.terrain.tile(position) else {
                continue;
            };
            let px = x + tx * width / terrain_width;
            let py = y + ty * height / terrain_height;
            let color = match tile.kind {
                TileKind::Air => Color::new(35, 35, 45, 180),
                TileKind::Lava => Color::RED,
                TileKind::Gas => Color::GREEN,
                TileKind::Ore(_) | TileKind::Artifact(_) => Color::GOLD,
                _ => Color::new(105, 80, 55, 220),
            };
            draw.draw_pixel(px, py, color);
        }
    }

    draw_map_marker(draw, &projection, 4, 7, Color::BLUE);
    draw_map_marker(draw, &projection, 12, 7, Color::MAROON);
    draw_map_marker(draw, &projection, 20, 7, Color::GREEN);
    draw_map_marker(draw, &projection, 28, 11, Color::PURPLE);

    let player_x = x + ((game.player.x / TILE_SIZE) as i32) * width / terrain_width;
    let player_y = y + ((game.player.y / TILE_SIZE) as i32) * height / terrain_height;
    draw.draw_circle(player_x, player_y, 3.0, Color::SKYBLUE);
}

fn draw_map_marker(
    draw: &mut RaylibDrawHandle<'_>,
    projection: &MinimapProjection,
    tile_x: i32,
    tile_y: i32,
    color: Color,
) {
    let x = projection.x + tile_x * projection.width / projection.terrain_width;
    let y = projection.y + tile_y * projection.height / projection.terrain_height;
    draw.draw_rectangle(x - 1, y - 1, 3, 3, color);
}

fn draw_depth_ruler(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let x = SCREEN_WIDTH - 30;
    let top = 80;
    let height = SCREEN_HEIGHT - 150;
    draw.draw_rectangle(x, top, 10, height, Color::new(0, 0, 0, 120));
    draw.draw_rectangle_lines(x, top, 10, height, Color::new(255, 255, 255, 120));

    for marker in (0..=80).step_by(20) {
        let y = top + (marker * height / 80);
        draw.draw_line(x - 8, y, x + 18, y, Color::LIGHTGRAY);
        draw.draw_text(&format!("{marker}m"), x - 58, y - 8, 14, Color::LIGHTGRAY);
    }

    let depth = (game.player.y / TILE_SIZE - 5.0).max(0.0);
    let marker_y = top + ((depth / 80.0).clamp(0.0, 1.0) * height as f32) as i32;
    draw.draw_circle(x + 5, marker_y, 6.0, Color::GOLD);
}

fn draw_modal(draw: &mut RaylibDrawHandle<'_>, game: &GameState, modal: ModalScreen) {
    draw.draw_rectangle(300, 120, 680, 440, Color::new(0, 0, 0, 220));
    draw.draw_rectangle_lines(300, 120, 680, 440, Color::RAYWHITE);

    match modal {
        ModalScreen::Fuel => {
            draw.draw_text("Fuel Station", 330, 150, 30, Color::GOLD);
            draw.draw_text(
                "Enter/E: buy as much fuel as credits allow",
                330,
                210,
                22,
                Color::WHITE,
            );
            draw.draw_text("Backspace/Esc: close", 330, 244, 20, Color::LIGHTGRAY);
            let missing = (game.player.fuel_capacity - game.player.fuel)
                .ceil()
                .max(0.0) as u32;
            let affordable = missing.min(game.player.credits);
            draw.draw_text(
                &format!(
                    "Tank: {:.0}/{:.0} | Fill cost: {missing} cr | Buying now: {affordable} units",
                    game.player.fuel, game.player.fuel_capacity
                ),
                330,
                290,
                18,
                Color::RAYWHITE,
            );
        }
        ModalScreen::Repair => {
            draw.draw_text("Repair Garage", 330, 150, 30, Color::LIME);
            draw.draw_text(
                "Enter/E: repair as much hull as credits allow",
                330,
                210,
                22,
                Color::WHITE,
            );
            draw.draw_text("Backspace/Esc: close", 330, 244, 20, Color::LIGHTGRAY);
            let missing = (game.player.max_hull() - game.player.hull).ceil().max(0.0) as u32;
            let affordable_units = missing.min(game.player.credits / 2);
            draw.draw_text(
                &format!(
                    "Hull: {:.0}/{:.0} | Full repair: {} cr | Repairing now: {} hull",
                    game.player.hull,
                    game.player.max_hull(),
                    missing * 2,
                    affordable_units
                ),
                330,
                290,
                18,
                Color::RAYWHITE,
            );
        }
        ModalScreen::Depot => draw_modal_depot(draw, game),
        ModalScreen::Shop => draw_modal_shop(draw, game),
        ModalScreen::Map => draw_large_map(draw, game),
        ModalScreen::Help => draw_help(draw),
        ModalScreen::ExitConfirm => {
            draw.draw_text("Exit to Desktop?", 330, 150, 30, Color::RED);
            draw.draw_text(
                "Enter/E confirms. Backspace/Esc cancels.",
                330,
                210,
                22,
                Color::WHITE,
            );
        }
    }
}

fn draw_modal_depot(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Ore Depot", 330, 150, 30, Color::GREEN);
    draw.draw_text(
        "Up/Down select | Enter/E confirm | Backspace/Esc close",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );

    let options = ["Complete active contract", "Sell loose cargo"];
    for (index, option) in options.iter().enumerate() {
        let y = 230 + i32::try_from(index).unwrap_or(i32::MAX) * 36;
        let color = if index == game.selected_menu_item {
            Color::YELLOW
        } else {
            Color::WHITE
        };
        draw.draw_text(option, 350, y, 22, color);
    }

    draw.draw_text(
        &format!(
            "{}: {}/{} {} for {} credits",
            game.contracts.active.title,
            game.contracts.active.progress(&game.player),
            game.contracts.active.required,
            game.contracts.active.target.name(),
            game.contracts.active.reward
        ),
        330,
        320,
        20,
        Color::RAYWHITE,
    );

    draw.draw_text(
        &format!(
            "Cargo manifest: {}/{} slots",
            game.player.cargo_used(),
            game.player.cargo_capacity
        ),
        330,
        350,
        18,
        Color::LIGHTGRAY,
    );
    let mut manifest_y = 378;
    for (mineral, count) in &game.player.cargo {
        draw.draw_text(
            &format!(
                "{} x{} = {} cr",
                mineral.name(),
                count,
                mineral.value() * count
            ),
            350,
            manifest_y,
            16,
            Color::RAYWHITE,
        );
        manifest_y += 20;
    }
    for (artifact, count) in &game.player.artifacts {
        draw.draw_text(
            &format!(
                "{} x{} = {} cr",
                artifact.name(),
                count,
                artifact.value() * count
            ),
            350,
            manifest_y,
            16,
            Color::MAGENTA,
        );
        manifest_y += 20;
    }
    if game.player.cargo_used() == 0 {
        draw.draw_text("Cargo hold empty", 350, manifest_y, 16, Color::GRAY);
    }

    if !game.last_depot_receipt.is_empty() {
        draw.draw_text("Last receipt:", 710, 350, 18, Color::LIGHTGRAY);
        for (index, line) in game.last_depot_receipt.lines().take(6).enumerate() {
            draw.draw_text(
                line,
                730,
                376 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
                16,
                Color::RAYWHITE,
            );
        }
    }
}

fn draw_large_map(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(160, 70, 960, 580, Color::new(0, 0, 0, 235));
    draw.draw_rectangle_lines(160, 70, 960, 580, Color::RAYWHITE);
    draw.draw_text("Mine Map", 190, 96, 32, Color::GOLD);
    draw.draw_text(
        "M/Esc/Backspace closes | discovered terrain only",
        190,
        132,
        18,
        Color::LIGHTGRAY,
    );

    let x = 210;
    let y = 170;
    let width = 860;
    let height = 420;
    let terrain_width = game.terrain.width().max(1);
    let terrain_height = game.terrain.height().max(1);
    draw.draw_rectangle(x, y, width, height, Color::new(12, 10, 14, 255));

    for ty in 0..terrain_height {
        for tx in 0..terrain_width {
            let position = TilePosition { x: tx, y: ty };
            if !game.is_explored(position) {
                continue;
            }
            let Some(tile) = game.terrain.tile(position) else {
                continue;
            };
            let px = x + tx * width / terrain_width;
            let py = y + ty * height / terrain_height;
            let color = match tile.kind {
                TileKind::Air => Color::new(40, 42, 55, 255),
                TileKind::Lava => Color::RED,
                TileKind::Gas => Color::GREEN,
                TileKind::Ore(mineral) if mineral.value() >= 78 => Color::ORANGE,
                TileKind::Ore(_) => Color::GOLD,
                TileKind::Artifact(_) => Color::MAGENTA,
                _ => Color::new(115, 82, 58, 255),
            };
            draw.draw_rectangle(px, py, 3, 3, color);
        }
    }

    let player_x = x + ((game.player.x / TILE_SIZE) as i32) * width / terrain_width;
    let player_y = y + ((game.player.y / TILE_SIZE) as i32) * height / terrain_height;
    draw.draw_circle(player_x, player_y, 6.0, Color::SKYBLUE);
    draw.draw_text("YOU", player_x + 8, player_y - 7, 12, Color::SKYBLUE);

    let buildings = [
        (4, 4, "FUEL", Color::BLUE),
        (12, 4, "FIX", Color::MAROON),
        (20, 4, "DEPOT", Color::GREEN),
        (30, 4, "SHOP", Color::PURPLE),
    ];
    for (tx, ty, label, color) in buildings {
        let px = x + tx * width / terrain_width;
        let py = y + ty * height / terrain_height;
        draw.draw_circle(px, py, 4.0, color);
        draw.draw_text(label, px + 6, py - 6, 10, Color::RAYWHITE);
    }

    for depth in (20..terrain_height).step_by(20) {
        let py = y + depth * height / terrain_height;
        draw.draw_line(x, py, x + width, py, Color::new(255, 255, 255, 35));
        draw.draw_text(&format!("{depth}m"), x - 44, py - 6, 12, Color::LIGHTGRAY);
    }
}

fn draw_help(draw: &mut RaylibDrawHandle<'_>) {
    draw.draw_rectangle(260, 110, 760, 500, Color::new(0, 0, 0, 235));
    draw.draw_rectangle_lines(260, 110, 760, 500, Color::RAYWHITE);
    draw.draw_text("Controls", 300, 145, 32, Color::GOLD);
    let lines = [
        "A/D or arrows: drive/steer",
        "W/Space: thrust",
        "S/Down: drill downward",
        "E/Enter: interact/confirm",
        "P/Esc: pause or close menus",
        "M: large mine map",
        "H: this help screen",
        "F5/F9: save/load",
        "Tab: details overlay",
    ];
    for (index, line) in lines.iter().enumerate() {
        draw.draw_text(
            line,
            320,
            210 + i32::try_from(index).unwrap_or(i32::MAX) * 34,
            22,
            Color::RAYWHITE,
        );
    }
}

fn draw_modal_shop(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Upgrade Shop", 330, 150, 30, Color::WHITE);
    draw.draw_text(
        "Up/Down select | Enter/E buy | Backspace/Esc close",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    let offers = upgrade_offers(&game.player);
    for (index, offer) in offers.iter().enumerate() {
        let y = 230 + i32::try_from(index).unwrap_or(i32::MAX) * 42;
        let selected = index == game.selected_menu_item;
        let affordable = game.player.credits >= offer.cost;
        let color = if selected {
            Color::YELLOW
        } else if affordable || offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            Color::RAYWHITE
        } else {
            Color::GRAY
        };
        let price = if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            "MAX".to_owned()
        } else {
            format!("{} cr", offer.cost)
        };
        draw.draw_text(
            &format!(
                "{} L{} -> {} | {}",
                upgrade_tier_name(offer.kind, offer.level),
                offer.level,
                price,
                upgrade_effect(offer.kind)
            ),
            350,
            y,
            20,
            color,
        );
    }

    if let Some(offer) = offers.get(game.selected_menu_item) {
        draw.draw_rectangle(690, 230, 330, 230, Color::new(20, 24, 36, 220));
        draw.draw_rectangle_lines(690, 230, 330, 230, Color::LIGHTGRAY);
        draw.draw_text("Upgrade Detail", 715, 255, 22, Color::GOLD);
        draw.draw_text(offer.name, 715, 292, 20, Color::RAYWHITE);
        draw.draw_text(offer.description, 715, 322, 16, Color::LIGHTGRAY);
        draw.draw_text(
            &format!("Current level: {}", offer.level),
            715,
            354,
            16,
            Color::RAYWHITE,
        );
        if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            draw.draw_text("Already at max tier", 715, 386, 16, Color::GREEN);
        } else {
            draw.draw_text(
                &format!("Next tier: {}", upgrade_tier_name(offer.kind, offer.level)),
                715,
                386,
                16,
                Color::RAYWHITE,
            );
            draw.draw_text(
                &format!("Cost: {} credits", offer.cost),
                715,
                416,
                16,
                Color::YELLOW,
            );
        }
    }
}

fn draw_title(draw: &mut RaylibDrawHandle<'_>) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 190));
    draw.draw_text("DRILLGAME", 475, 250, 54, Color::GOLD);
    draw.draw_text("Press Enter or E to start", 505, 325, 24, Color::RAYWHITE);
    draw.draw_text(
        "F5/F9 save-load once in game | P pause",
        455,
        365,
        20,
        Color::LIGHTGRAY,
    );
}

fn draw_pause(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 150));
    draw.draw_rectangle(430, 190, 420, 310, Color::new(0, 0, 0, 220));
    draw.draw_rectangle_lines(430, 190, 420, 310, Color::RAYWHITE);
    draw.draw_text("PAUSED", 548, 220, 44, Color::RAYWHITE);

    for (index, option) in PauseOption::ALL.iter().enumerate() {
        let y = 300 + i32::try_from(index).unwrap_or(i32::MAX) * 42;
        let color = if index == game.selected_pause_item {
            Color::YELLOW
        } else {
            Color::WHITE
        };
        draw.draw_text(option.label(), 520, y, 24, color);
    }

    draw.draw_text(
        "Up/Down select | Enter/E confirm | Esc/P resume",
        455,
        455,
        18,
        Color::LIGHTGRAY,
    );
}

fn draw_ending(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(5, 8, 18, 205));
    draw.draw_rectangle(300, 220, 680, 240, Color::new(0, 0, 0, 230));
    draw.draw_rectangle_lines(300, 220, 680, 240, Color::GOLD);
    draw.draw_text("STAR CORE RECOVERED", 392, 255, 40, Color::GOLD);
    draw.draw_text(&game.message, 340, 325, 20, Color::RAYWHITE);
    draw.draw_text(
        "You can keep mining, save, or exit from the pause menu.",
        382,
        380,
        20,
        Color::LIGHTGRAY,
    );
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
        TileKind::Gas => Color::new(100, 210, 120, 180),
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
