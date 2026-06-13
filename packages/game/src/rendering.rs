#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::prelude::*;

mod terrain;

use terrain::{TerrainRenderer, artifact_color, mineral_color};

use crate::{
    economy::{SurfaceZone, UpgradeKind, upgrade_effect, upgrade_offers, upgrade_tier_name},
    game_state::{
        DrillDirection, GameState, ModalScreen, PauseOption, RunMode, ServiceAnimation, TILE_SIZE,
    },
    save::{save_slot_count, save_slot_exists, save_slot_metadata},
    terrain::{TileKind, TilePosition},
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;

struct MinimapProjection {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    terrain_width: i32,
    terrain_height: i32,
}

pub struct GameRenderer {
    terrain: TerrainRenderer,
}

impl GameRenderer {
    pub fn new(raylib: &mut RaylibHandle, thread: &RaylibThread, game: &GameState) -> Self {
        Self {
            terrain: TerrainRenderer::new(raylib, thread, game),
        }
    }

    pub fn sync(&mut self, raylib: &mut RaylibHandle, thread: &RaylibThread, game: &mut GameState) {
        let visual_changes = game.take_visual_changes();
        if visual_changes.full_terrain_refresh {
            self.terrain.mark_all_dirty();
        }
        for tile in visual_changes.changed_tiles {
            self.terrain.mark_tile_dirty(tile);
        }
        self.terrain.sync(raylib, thread, game);
    }

    pub fn render(&self, draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
        draw.clear_background(Color::new(105, 190, 235, 255));

        let camera = render_camera(game);

        if game.run_mode == RunMode::Interior {
            draw_interior(draw, game);
        } else {
            let mut world_draw = draw.begin_mode2D(world_camera(camera));
            draw_world(&mut world_draw, game, camera, &self.terrain);
            draw_particles(&mut world_draw, game);
            draw_placed_bombs(&mut world_draw, game);
            draw_scanner_marks(&mut world_draw, game);
            for cloud in &game.hazard_clouds {
                world_draw.draw_circle_v(
                    Vector2::new(cloud.x, cloud.y),
                    cloud.radius,
                    Color::new(90, 190, 80, 70),
                );
            }

            draw_player(&mut world_draw, game);
        }
        if game.screen_flash_seconds > 0.0 {
            let alpha = (game.screen_flash_seconds * 500.0).clamp(0.0, 180.0) as u8;
            draw.draw_rectangle(
                0,
                0,
                SCREEN_WIDTH,
                SCREEN_HEIGHT,
                Color::new(255, 70, 30, alpha),
            );
        }
        if game.run_mode != RunMode::Interior {
            draw_heat_warning(draw, game);
        }
        draw_hud(draw, game);
        if game.run_mode != RunMode::Interior {
            draw_depth_ruler(draw, game);
            draw_minimap(draw, game);
        }

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

        draw.draw_text(
            &format!("Vol {:.0}% (+/-)", game.master_volume * 100.0),
            1030,
            20,
            18,
            Color::LIGHTGRAY,
        );

        if game.won_game {
            draw_ending(draw, game);
        }
    }
}

fn draw_interior(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let zone = game.interior_zone.unwrap_or(SurfaceZone::Depot);
    let (wall, trim, title) = interior_theme(zone);
    draw.clear_background(wall);
    draw.draw_rectangle(0, 455, SCREEN_WIDTH, 265, Color::new(38, 32, 28, 255));
    draw.draw_rectangle(35, 130, 1210, 380, Color::new(18, 18, 24, 220));
    draw.draw_rectangle_lines(35, 130, 1210, 380, trim);
    draw.draw_text(title, 65, 150, 30, Color::RAYWHITE);

    draw.draw_rectangle(58, 338, 48, 118, Color::new(55, 32, 20, 255));
    draw.draw_rectangle_lines(58, 338, 48, 118, Color::GOLD);
    draw.draw_text("EXIT", 55, 310, 18, Color::GOLD);

    draw_interior_props(draw, zone);
    draw_service_animation(draw, game, zone);
    let service_x = interior_screen_x(interior_service_x_render(zone));
    draw.draw_text("Press E", (service_x - 42.0) as i32, 292, 18, Color::YELLOW);

    let player_x = interior_screen_x(game.interior_x);
    draw.draw_rectangle((player_x - 11.0) as i32, 402, 22, 38, Color::GOLD);
    draw.draw_circle(player_x as i32, 392, 10.0, Color::SKYBLUE);
    let visor_offset = if game.interior_facing >= 0.0 { 5 } else { -13 };
    draw.draw_rectangle(player_x as i32 + visor_offset, 389, 8, 5, Color::DARKBLUE);
    draw.draw_text(
        "A/D walk | E use counter/door | Backspace/Esc exits",
        375,
        650,
        20,
        Color::LIGHTGRAY,
    );
}

fn draw_service_animation(draw: &mut RaylibDrawHandle<'_>, game: &GameState, zone: SurfaceZone) {
    let Some(animation) = game.service_animation else {
        return;
    };
    let pulse = (game.service_animation_seconds * 18.0) as i32;
    match animation {
        ServiceAnimation::Fuel if zone == SurfaceZone::Fuel => {
            draw.draw_line_ex(
                Vector2::new(820.0, 372.0),
                Vector2::new(620.0, 420.0),
                5.0,
                Color::YELLOW,
            );
            draw.draw_circle(620, 420, 10.0 + (pulse.rem_euclid(6)) as f32, Color::GOLD);
            draw.draw_text("FUELING", 610, 365, 24, Color::GOLD);
        }
        ServiceAnimation::Repair if zone == SurfaceZone::Repair => {
            draw.draw_rectangle(672, 392 - pulse.rem_euclid(12), 235, 8, Color::ORANGE);
            draw.draw_text("REPAIR CREW", 615, 365, 24, Color::ORANGE);
        }
        _ => {}
    }
}

fn draw_interior_props(draw: &mut RaylibDrawHandle<'_>, zone: SurfaceZone) {
    match zone {
        SurfaceZone::Fuel => {
            draw.draw_rectangle(760, 330, 70, 120, Color::DARKBLUE);
            draw.draw_circle(795, 350, 18.0, Color::GOLD);
            draw.draw_line(830, 370, 900, 420, Color::BLACK);
            draw.draw_text("PUMPS", 742, 300, 22, Color::GOLD);
        }
        SurfaceZone::Repair => {
            draw.draw_rectangle(690, 418, 190, 18, Color::MAROON);
            draw.draw_rectangle(725, 350, 18, 82, Color::GRAY);
            draw.draw_rectangle(825, 350, 18, 82, Color::GRAY);
            draw.draw_text("LIFT", 742, 300, 22, Color::ORANGE);
        }
        SurfaceZone::Depot => {
            draw.draw_rectangle(800, 385, 125, 55, Color::BROWN);
            draw.draw_rectangle_lines(800, 385, 125, 55, Color::GOLD);
            draw.draw_rectangle(690, 345, 95, 95, Color::DARKGREEN);
            draw.draw_text("SCALE", 692, 315, 22, Color::GOLD);
        }
        SurfaceZone::Headquarters => {
            draw.draw_rectangle(690, 310, 300, 90, Color::new(18, 24, 42, 255));
            draw.draw_rectangle_lines(690, 310, 300, 90, Color::SKYBLUE);
            draw.draw_circle(735, 355, 26.0, Color::DARKBLUE);
            draw.draw_text("RADIO / CONTRACTS", 705, 275, 22, Color::SKYBLUE);
        }
        SurfaceZone::Shop => {
            draw.draw_rectangle(675, 300, 320, 35, Color::PURPLE);
            for index in 0..6 {
                draw.draw_rectangle(695 + index * 48, 352, 28, 70, Color::DARKPURPLE);
            }
            draw.draw_text("UPGRADE WALL", 705, 265, 22, Color::MAGENTA);
        }
    }
}

const fn interior_screen_x(room_x: f32) -> f32 {
    55.0 + room_x * 1.85
}

const fn interior_service_x_render(zone: SurfaceZone) -> f32 {
    match zone {
        SurfaceZone::Fuel => 430.0,
        SurfaceZone::Repair => 405.0,
        SurfaceZone::Depot => 455.0,
        SurfaceZone::Headquarters => 390.0,
        SurfaceZone::Shop => 450.0,
    }
}

const fn interior_theme(zone: SurfaceZone) -> (Color, Color, &'static str) {
    match zone {
        SurfaceZone::Fuel => (
            Color::new(18, 30, 48, 255),
            Color::GOLD,
            "Fuel Station Interior",
        ),
        SurfaceZone::Repair => (
            Color::new(42, 22, 22, 255),
            Color::ORANGE,
            "Repair Garage Interior",
        ),
        SurfaceZone::Depot => (
            Color::new(18, 36, 25, 255),
            Color::GREEN,
            "Ore Depot Interior",
        ),
        SurfaceZone::Headquarters => (
            Color::new(22, 20, 44, 255),
            Color::SKYBLUE,
            "Borealis HQ Interior",
        ),
        SurfaceZone::Shop => (
            Color::new(34, 20, 42, 255),
            Color::MAGENTA,
            "Upgrade Shop Interior",
        ),
    }
}

fn render_camera(game: &GameState) -> Vector2 {
    let mut camera = Vector2::new(game.camera_x, game.camera_y);
    if game.camera_shake_seconds > 0.0 {
        let pulse = (game.camera_shake_seconds * 70.0).sin();
        camera.x += pulse * game.camera_shake_strength;
        camera.y += (game.camera_shake_seconds * 53.0).cos() * game.camera_shake_strength * 0.7;
    }
    camera
}

fn world_camera(camera: Vector2) -> Camera2D {
    Camera2D {
        offset: Vector2::zero(),
        target: camera,
        rotation: 0.0,
        zoom: 1.0,
    }
}

fn draw_world(
    draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>,
    game: &GameState,
    camera: Vector2,
    terrain: &TerrainRenderer,
) {
    draw_surface_buildings(draw);
    terrain.draw(draw, camera);

    if let Some(drill) = game.active_drill {
        let x = (drill.target.x as f32 * TILE_SIZE) as i32;
        let y = (drill.target.y as f32 * TILE_SIZE) as i32;
        let current_durability = game
            .terrain
            .tile(drill.target)
            .map_or(drill.initial_durability, |tile| tile.durability);
        let chipped = drill.initial_durability.saturating_sub(current_durability);
        let progress = ((f32::from(chipped) + drill.progress.clamp(0.0, 1.0))
            / f32::from(drill.initial_durability.max(1)))
        .clamp(0.0, 1.0);
        draw.draw_rectangle_lines(
            x - 2,
            y - 2,
            TILE_SIZE as i32 + 4,
            TILE_SIZE as i32 + 4,
            Color::YELLOW,
        );
        if drill.direction == DrillDirection::Down {
            let pulse = (game.update_ticks as f32 * 0.4).sin().abs();
            draw.draw_circle(
                x + TILE_SIZE as i32 / 2,
                y + TILE_SIZE as i32 / 2,
                10.0 + pulse * 8.0,
                Color::new(255, 180, 70, 85),
            );
        }
        if matches!(game.terrain.tile(drill.target), Some(tile) if tile.kind == TileKind::Gas) {
            let pulse = (game.update_ticks as f32 * 0.22).sin().abs();
            draw.draw_circle(
                x + TILE_SIZE as i32 / 2,
                y + TILE_SIZE as i32 / 2,
                10.0 + pulse * 8.0,
                Color::new(95, 230, 90, 80),
            );
            draw.draw_text("GAS", x + 4, y + 4, 12, Color::GREEN);
        }
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

fn draw_surface_buildings(draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>) {
    draw_building(draw, 0.0, 8.0, Color::DARKBLUE, "FUEL");
    draw_building(draw, 8.0, 8.0, Color::MAROON, "REPAIR");
    draw_building(draw, 16.0, 8.0, Color::DARKGREEN, "DEPOT");
    draw_building(draw, 24.0, 8.0, Color::DARKPURPLE, "HQ");
    draw_building(draw, 32.0, 12.0, Color::PURPLE, "SHOP");
}

fn draw_building(
    draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>,
    tile_x: f32,
    tile_width: f32,
    color: Color,
    label: &str,
) {
    let x = tile_x * TILE_SIZE;
    let y = 3.0 * TILE_SIZE;
    let width = tile_width * TILE_SIZE;

    draw.draw_rectangle(x as i32, y as i32, width as i32, 64, color);
    draw.draw_triangle(
        Vector2::new(x, y),
        Vector2::new(x + width * 0.5, y - 24.0),
        Vector2::new(x + width, y),
        Color::new(35, 35, 45, 255),
    );
    draw.draw_rectangle(
        (x + width - 22.0) as i32,
        (y + 18.0) as i32,
        14,
        46,
        Color::new(35, 25, 18, 255),
    );
    draw.draw_rectangle((x + 10.0) as i32, (y + 12.0) as i32, 22, 18, Color::SKYBLUE);
    match label {
        "FUEL" => draw.draw_circle(
            (x + width - 42.0) as i32,
            (y + 28.0) as i32,
            10.0,
            Color::GOLD,
        ),
        "REPAIR" => {
            draw.draw_rectangle(
                (x + width - 50.0) as i32,
                (y + 23.0) as i32,
                24,
                8,
                Color::RAYWHITE,
            );
            draw.draw_rectangle(
                (x + width - 42.0) as i32,
                (y + 15.0) as i32,
                8,
                24,
                Color::RAYWHITE,
            );
        }
        "DEPOT" => draw.draw_rectangle(
            (x + width - 54.0) as i32,
            (y + 23.0) as i32,
            28,
            20,
            Color::BROWN,
        ),
        "HQ" => draw.draw_triangle(
            Vector2::new(x + width - 52.0, y + 42.0),
            Vector2::new(x + width - 38.0, y + 14.0),
            Vector2::new(x + width - 24.0, y + 42.0),
            Color::SKYBLUE,
        ),
        _ => draw.draw_circle_lines(
            (x + width - 40.0) as i32,
            (y + 30.0) as i32,
            13.0,
            Color::MAGENTA,
        ),
    }
    draw.draw_text(label, x as i32 + 16, y as i32 + 40, 20, Color::WHITE);
}

fn draw_particles(draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>, game: &GameState) {
    if let (Some(x), Some(y)) = (game.lost_cargo_x, game.lost_cargo_y) {
        draw.draw_rectangle((x - 8.0) as i32, (y - 7.0) as i32, 16, 14, Color::GOLD);
        draw.draw_rectangle_lines((x - 8.0) as i32, (y - 7.0) as i32, 16, 14, Color::BROWN);
        draw.draw_line(
            (x - 8.0) as i32,
            y as i32,
            (x + 8.0) as i32,
            y as i32,
            Color::BROWN,
        );
        draw.draw_text("LOST", (x + 8.0) as i32, (y - 8.0) as i32, 12, Color::GOLD);
    }

    for particle in &game.dust_particles {
        let alpha = (particle.life / 0.35).clamp(0.0, 1.0);
        draw.draw_circle_v(
            Vector2::new(particle.x, particle.y),
            4.0,
            Color::new(190, 150, 105, (180.0 * alpha) as u8),
        );
    }
    for boulder in &game.falling_boulders {
        let wobble = if boulder.warning_seconds > 0.0 {
            (boulder.warning_seconds * 60.0).sin() * 3.0
        } else {
            0.0
        };
        let color = if boulder.warning_seconds > 0.0 {
            Color::new(180, 80, 55, 255)
        } else {
            Color::new(95, 80, 70, 255)
        };
        draw.draw_circle_v(Vector2::new(boulder.x + wobble, boulder.y), 8.0, color);
        draw.draw_circle_lines(boulder.x as i32, boulder.y as i32, 8.0, Color::DARKGRAY);
    }
    for spark in &game.spark_particles {
        let alpha = (spark.life / 0.45).clamp(0.0, 1.0);
        draw.draw_circle_v(
            Vector2::new(spark.x, spark.y),
            2.5,
            Color::new(255, 190, 60, (220.0 * alpha) as u8),
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

fn draw_placed_bombs(draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>, game: &GameState) {
    for bomb in &game.placed_bombs {
        let x = bomb.x as i32;
        let y = bomb.y as i32;
        draw.draw_circle(x, y, 7.0, Color::BLACK);
        draw.draw_circle_lines(x, y, 8.0, Color::RED);
        draw.draw_text(
            &format!("{:.1}", bomb.timer_seconds.max(0.0)),
            x - 11,
            y - 24,
            14,
            Color::YELLOW,
        );
    }
}

fn draw_scanner_marks(draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>, game: &GameState) {
    if game.player.scanner_level == 0 {
        return;
    }
    let center = game.player.tile_position(TILE_SIZE);
    let radius = 3 + i32::from(game.player.scanner_level) * 2;
    for y in center.y - radius..=center.y + radius {
        for x in center.x - radius..=center.x + radius {
            if (x - center.x).abs() + (y - center.y).abs() > radius {
                continue;
            }
            let Some(tile) = game.terrain.tile(TilePosition { x, y }) else {
                continue;
            };
            let color = match tile.kind {
                TileKind::Ore(_) => Color::GOLD,
                TileKind::Artifact(_) => Color::MAGENTA,
                TileKind::Gas
                | TileKind::Lava
                | TileKind::MagmaVent
                | TileKind::ExplosivePocket
                | TileKind::PressurePocket => Color::RED,
                _ => continue,
            };
            draw.draw_circle_v(
                Vector2::new(
                    x as f32 * TILE_SIZE + TILE_SIZE * 0.5,
                    y as f32 * TILE_SIZE + TILE_SIZE * 0.5,
                ),
                3.0,
                color,
            );
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "player rendering includes upgrade visual variants"
)]
fn draw_player(draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>, game: &GameState) {
    let (offset_x, offset_y) = drill_visual_offset(game);
    let screen_x = game.player.x + offset_x;
    let screen_y = game.player.y + offset_y;

    let hull_color = if game.player.hull < game.player.max_hull() * 0.3 {
        Color::new(210, 95, 60, 255)
    } else {
        Color::new(235, 190, 45, 255)
    };
    draw.draw_rectangle(
        (screen_x - 14.0) as i32,
        (screen_y - 10.0) as i32,
        28,
        22,
        hull_color,
    );
    draw.draw_rectangle_lines(
        (screen_x - 14.0) as i32,
        (screen_y - 10.0) as i32,
        28,
        22,
        Color::BROWN,
    );
    draw.draw_rectangle(
        (screen_x - 7.0) as i32,
        (screen_y - 17.0) as i32,
        14,
        8,
        Color::SKYBLUE,
    );
    draw.draw_circle(
        (screen_x - 10.0) as i32,
        (screen_y + 13.0) as i32,
        4.0,
        Color::DARKGRAY,
    );
    draw.draw_circle(
        (screen_x + 10.0) as i32,
        (screen_y + 13.0) as i32,
        4.0,
        Color::DARKGRAY,
    );

    let direction = game
        .active_drill
        .map_or(DrillDirection::Down, |drill| drill.direction);
    match direction {
        DrillDirection::Down => draw.draw_triangle(
            Vector2::new(screen_x - 6.0, screen_y + 13.0),
            Vector2::new(screen_x + 6.0, screen_y + 13.0),
            Vector2::new(screen_x, screen_y + 28.0),
            Color::ORANGE,
        ),
        DrillDirection::Left => draw.draw_triangle(
            Vector2::new(screen_x - 15.0, screen_y - 4.0),
            Vector2::new(screen_x - 15.0, screen_y + 8.0),
            Vector2::new(screen_x - 29.0, screen_y + 2.0),
            Color::ORANGE,
        ),
        DrillDirection::Right => draw.draw_triangle(
            Vector2::new(screen_x + 15.0, screen_y - 4.0),
            Vector2::new(screen_x + 15.0, screen_y + 8.0),
            Vector2::new(screen_x + 29.0, screen_y + 2.0),
            Color::ORANGE,
        ),
    }
    if game.player.velocity_y < -40.0 {
        draw.draw_triangle(
            Vector2::new(screen_x - 8.0, screen_y + 16.0),
            Vector2::new(screen_x + 8.0, screen_y + 16.0),
            Vector2::new(screen_x, screen_y + 34.0),
            Color::new(255, 95, 20, 220),
        );
    }
    if game.player.hull < game.player.max_hull() * 0.35 {
        let smoke_alpha = 90 + ((game.player.hull as i32).rem_euclid(40) as u8);
        draw.draw_circle(
            (screen_x - 16.0) as i32,
            (screen_y - 18.0) as i32,
            5.0,
            Color::new(60, 60, 60, smoke_alpha),
        );
        draw.draw_circle(
            (screen_x - 22.0) as i32,
            (screen_y - 25.0) as i32,
            7.0,
            Color::new(45, 45, 45, smoke_alpha.saturating_sub(25)),
        );
    }
    if game.player.scanner_level > 0 {
        draw.draw_circle_lines(
            screen_x as i32,
            (screen_y - 24.0) as i32,
            8.0 + f32::from(game.player.scanner_level) * 2.0,
            Color::SKYBLUE,
        );
        draw.draw_line(
            screen_x as i32,
            (screen_y - 16.0) as i32,
            screen_x as i32,
            (screen_y - 30.0) as i32,
            Color::SKYBLUE,
        );
    }
    if game.player.radiator_level > 1 {
        for fin in 0..game.player.radiator_level {
            draw.draw_rectangle(
                (screen_x + 16.0) as i32,
                (screen_y - 14.0 + f32::from(fin) * 5.0) as i32,
                10,
                2,
                Color::ORANGE,
            );
        }
    }
    if game.player.hull_level > 2 {
        draw.draw_rectangle_lines(
            (screen_x - 19.0) as i32,
            (screen_y - 18.0) as i32,
            38,
            36,
            Color::GOLD,
        );
    }
    if game.drill_flash_seconds > 0.0 {
        draw.draw_circle_v(
            Vector2::new(screen_x, screen_y + 20.0),
            7.0,
            Color::new(255, 230, 80, 210),
        );
    }
}

fn draw_heat_warning(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let depth_heat = game.player.y > 18.0 * TILE_SIZE;
    let tile_x = (game.player.x / TILE_SIZE) as i32;
    let tile_y = (game.player.y / TILE_SIZE) as i32;
    let near_lava = (-2..=2).any(|dy| {
        (-2..=2).any(|dx| {
            matches!(
                game.terrain.tile(TilePosition {
                    x: tile_x + dx,
                    y: tile_y + dy,
                }),
                Some(tile) if tile.kind == TileKind::Lava
            )
        })
    });
    if depth_heat || near_lava {
        let alpha = if near_lava { 70 } else { 35 };
        draw.draw_rectangle(
            0,
            0,
            SCREEN_WIDTH,
            SCREEN_HEIGHT,
            Color::new(255, 70, 20, alpha),
        );
        draw.draw_text("HEAT", 1110, 48, 22, Color::ORANGE);
    }
}

fn draw_hud(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw_compact_status(draw, game);
    draw_message_toast(draw, game);

    draw.draw_text(
        &format!(
            "Objective: {} {}/{} {}",
            game.contracts.active.title,
            game.contracts.active.progress(&game.player),
            game.contracts.active.required,
            game.contracts.active.target.name()
        ),
        22,
        74,
        18,
        Color::RAYWHITE,
    );

    if game.show_details || game.modal == Some(ModalScreen::Depot) {
        draw_detail_panel(draw, game);
    }
    if game.show_details {
        draw_debug_stats(draw, game);
    }
}

fn draw_debug_stats(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let fps = if game.last_delta_seconds > 0.0 {
        1.0 / game.last_delta_seconds
    } else {
        0.0
    };
    draw.draw_rectangle(980, 90, 270, 96, Color::new(0, 0, 0, 145));
    draw.draw_rectangle_lines(980, 90, 270, 96, Color::DARKGRAY);
    draw.draw_text("Debug", 995, 102, 18, Color::SKYBLUE);
    draw.draw_text(
        &format!(
            "Frame {:.1} ms / {:.0} fps",
            game.last_delta_seconds * 1000.0,
            fps
        ),
        995,
        126,
        16,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!(
            "Ticks {} | play {:.1}m",
            game.update_ticks,
            game.play_seconds / 60.0
        ),
        995,
        148,
        16,
        Color::RAYWHITE,
    );
    draw.draw_text(
        &format!(
            "Particles d{} s{} b{}",
            game.dust_particles.len(),
            game.spark_particles.len(),
            game.falling_boulders.len()
        ),
        995,
        170,
        16,
        Color::RAYWHITE,
    );
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
            "D{} E{} H{} R{} S{} B{} Debt{} | Tab",
            game.player.drill_strength,
            game.player.engine_level,
            game.player.hull_level,
            game.player.radiator_level,
            game.player.scanner_level,
            game.player.bombs,
            game.player.loan_debt
        ),
        810,
        31,
        18,
        Color::LIGHTGRAY,
    );
    if game.escape_sequence_seconds > 0.0 {
        draw.draw_text(
            &format!("CORE CASCADE {:.0}s", game.escape_sequence_seconds),
            1010,
            42,
            18,
            Color::RED,
        );
    }
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
                TileKind::Lava | TileKind::MagmaVent => Color::RED,
                TileKind::Gas => Color::GREEN,
                TileKind::ExplosivePocket => Color::ORANGE,
                TileKind::PressurePocket => Color::SKYBLUE,
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

fn draw_service_confirm(draw: &mut RaylibDrawHandle<'_>, game: &GameState, service: &str) {
    let label = match game.selected_menu_item {
        0 => "small 25%",
        1 => "half 50%",
        _ => "full 100%",
    };
    draw.draw_text("Confirm Service", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        &format!("Buy {label} {service}?"),
        330,
        220,
        24,
        Color::RAYWHITE,
    );
    draw.draw_text(
        "Enter/E confirms | Backspace/Esc cancels",
        330,
        280,
        20,
        Color::WHITE,
    );
}

fn draw_service_options(draw: &mut RaylibDrawHandle<'_>, selected: usize, x: i32, y: i32) {
    let options = ["Small 25%", "Half 50%", "Full 100%"];
    for (index, option) in options.iter().enumerate() {
        let color = if index == selected {
            Color::YELLOW
        } else {
            Color::RAYWHITE
        };
        draw.draw_text(
            option,
            x,
            y + i32::try_from(index).unwrap_or(i32::MAX) * 28,
            20,
            color,
        );
    }
}

fn draw_modal(draw: &mut RaylibDrawHandle<'_>, game: &GameState, modal: ModalScreen) {
    draw.draw_rectangle(300, 120, 680, 440, Color::new(0, 0, 0, 220));
    draw.draw_rectangle_lines(300, 120, 680, 440, Color::RAYWHITE);

    match modal {
        ModalScreen::Fuel => {
            draw.draw_text("Fuel Station", 330, 150, 30, Color::GOLD);
            draw.draw_text(
                "Up/Down: small/half/full | Enter/E buy selected",
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
            draw_service_options(draw, game.selected_menu_item, 330, 330);
        }
        ModalScreen::FuelConfirm => draw_service_confirm(draw, game, "fuel"),
        ModalScreen::Repair => {
            draw.draw_text("Repair Garage", 330, 150, 30, Color::LIME);
            draw.draw_text(
                "Up/Down: small/half/full | Enter/E repair selected",
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
            draw_service_options(draw, game.selected_menu_item, 330, 330);
        }
        ModalScreen::RepairConfirm => draw_service_confirm(draw, game, "repair"),
        ModalScreen::Depot => draw_modal_depot(draw, game),
        ModalScreen::Headquarters => draw_headquarters(draw, game),
        ModalScreen::DepotReceiptHistory => draw_depot_receipt_history(draw, game),
        ModalScreen::Shop => draw_modal_shop(draw, game),
        ModalScreen::ShopConfirm => draw_shop_confirm(draw, game),
        ModalScreen::Options => draw_options(draw, game),
        ModalScreen::SaveSlots => draw_save_slots(draw, game, true),
        ModalScreen::LoadSlots => draw_save_slots(draw, game, false),
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

fn draw_depot_receipt_history(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Depot Receipt History", 330, 150, 30, Color::GREEN);
    draw.draw_text(
        "Enter/E or Backspace/Esc returns",
        330,
        190,
        18,
        Color::LIGHTGRAY,
    );
    if game.depot_receipts.is_empty() {
        draw.draw_text("No sales recorded yet.", 350, 245, 22, Color::GRAY);
        return;
    }
    for (index, receipt) in game.depot_receipts.iter().rev().enumerate() {
        let y = 235 + i32::try_from(index).unwrap_or(i32::MAX) * 72;
        draw.draw_text(&format!("Sale {}", index + 1), 350, y, 18, Color::GOLD);
        for (line_index, line) in receipt.lines().take(2).enumerate() {
            draw.draw_text(
                line,
                370,
                y + 24 + i32::try_from(line_index).unwrap_or(i32::MAX) * 18,
                15,
                Color::RAYWHITE,
            );
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "depot modal composes manifest, receipt, and history"
)]
fn draw_headquarters(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("HQ - Borealis Mining Office", 330, 150, 30, Color::GOLD);
    draw.draw_rectangle(330, 195, 590, 120, Color::new(18, 24, 42, 230));
    draw.draw_rectangle_lines(330, 195, 590, 120, Color::SKYBLUE);
    let briefing = match game.deepest_tile_reached {
        0..=19 => {
            "Director Vale: First contract proves the claim. Keep cargo clean and receipts cleaner."
        }
        20..=39 => "Mechanic Iona: Clay gives way to silver caverns. Bring upgrades, not excuses.",
        40..=59 => {
            "Surveyor Kade: Relics, gas, and pressure pockets start arguing with the maps here."
        }
        60..=79 => {
            "Director Vale: Thermal strata will eat cheap hulls. Radiators first, heroics second."
        }
        _ => "Surveyor Kade: Core harmonics below. If the radio screams, keep drilling anyway.",
    };
    draw.draw_text(briefing, 350, 220, 18, Color::RAYWHITE);
    let finance = if game.player.loan_debt == 0 {
        "Take HQ advance loan (+250 now, owe 300)".to_owned()
    } else {
        format!("Pay HQ debt (owed {})", game.player.loan_debt)
    };
    let options = [
        "Complete active contract".to_owned(),
        "Ask for briefing/radio intel".to_owned(),
        finance,
    ];
    for (index, option) in options.iter().enumerate() {
        draw.draw_text(
            option,
            360,
            355 + i32::try_from(index).unwrap_or(i32::MAX) * 44,
            24,
            if index == game.selected_menu_item {
                Color::YELLOW
            } else {
                Color::RAYWHITE
            },
        );
    }
    draw.draw_text(
        "Enter/E confirms | Backspace/Esc exits HQ",
        330,
        505,
        18,
        Color::LIGHTGRAY,
    );
}

#[allow(
    clippy::too_many_lines,
    reason = "depot panel combines sales, contracts, and receipt previews"
)]
fn draw_modal_depot(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Ore Depot", 330, 150, 30, Color::GREEN);
    draw.draw_text(
        "Up/Down select | Enter/E confirm | Backspace/Esc close",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );

    let options = [
        "Complete active contract",
        "Sell loose cargo",
        "Receipt history",
    ];
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
    if !game.depot_receipts.is_empty() {
        draw.draw_text("Receipt history:", 710, 500, 18, Color::LIGHTGRAY);
        for (index, receipt) in game.depot_receipts.iter().rev().take(3).enumerate() {
            let total_line = receipt.lines().last().unwrap_or("receipt");
            draw.draw_text(
                total_line,
                730,
                526 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
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
                TileKind::Lava | TileKind::MagmaVent => Color::RED,
                TileKind::Gas => Color::GREEN,
                TileKind::ExplosivePocket => Color::ORANGE,
                TileKind::PressurePocket => Color::SKYBLUE,
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

    if let (Some(cargo_x), Some(cargo_y)) = (game.lost_cargo_x, game.lost_cargo_y) {
        let marker_x = x + ((cargo_x / TILE_SIZE) as i32) * width / terrain_width;
        let marker_y = y + ((cargo_y / TILE_SIZE) as i32) * height / terrain_height;
        draw.draw_rectangle(marker_x - 4, marker_y - 4, 8, 8, Color::GOLD);
        draw.draw_text("LOST", marker_x + 6, marker_y - 6, 12, Color::GOLD);
    }

    let buildings = [
        (4, 4, "FUEL", Color::BLUE),
        (12, 4, "FIX", Color::MAROON),
        (20, 4, "DEPOT", Color::GREEN),
        (28, 4, "HQ", Color::DARKPURPLE),
        (38, 4, "SHOP", Color::PURPLE),
    ];
    for (tx, ty, label, color) in buildings {
        let px = x + tx * width / terrain_width;
        let py = y + ty * height / terrain_height;
        draw.draw_circle(px, py, 4.0, color);
        draw.draw_text(label, px + 6, py - 6, 10, Color::RAYWHITE);
    }

    if let (Some(rescue_x), Some(rescue_y)) = (game.last_rescue_x, game.last_rescue_y) {
        let marker_x = x + ((rescue_x / TILE_SIZE) as i32) * width / terrain_width;
        let marker_y = y + ((rescue_y / TILE_SIZE) as i32) * height / terrain_height;
        draw.draw_circle_lines(marker_x, marker_y, 7.0, Color::RED);
        draw.draw_text("RESCUE", marker_x + 9, marker_y - 7, 10, Color::RED);
    }

    for depth in (20..terrain_height).step_by(20) {
        let py = y + depth * height / terrain_height;
        draw.draw_line(x, py, x + width, py, Color::new(255, 255, 255, 35));
        draw.draw_text(&format!("{depth}m"), x - 44, py - 6, 12, Color::LIGHTGRAY);
    }
    draw.draw_text(
        "Legend: gold ore | orange rare/blast | magenta artifact | red lava/vent | green gas | cyan pressure | blue you",
        190,
        612,
        16,
        Color::LIGHTGRAY,
    );
}

fn draw_help(draw: &mut RaylibDrawHandle<'_>) {
    draw.draw_rectangle(260, 110, 760, 500, Color::new(0, 0, 0, 235));
    draw.draw_rectangle_lines(260, 110, 760, 500, Color::RAYWHITE);
    draw.draw_text("Controls", 300, 145, 32, Color::GOLD);
    let lines = [
        "A/D or arrows: drive/steer",
        "W/Space: thrust | S/Down: drill downward",
        "E/Enter: interact/confirm | Backspace/Esc: close",
        "P: pause; pause menu has slots and options",
        "M: large mine map with ore, hazards, rescue, lost cargo",
        "H: this help screen | Tab: details overlay",
        "F5/F9: quick save/load | F11: fullscreen",
        "Surface: Fuel, Repair, Depot, HQ, Shop",
        "HQ: contracts and radio briefings",
        "Return safely for trip-streak bonuses",
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

fn draw_shop_confirm(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let offers = upgrade_offers(&game.player);
    let Some(offer) = offers.get(game.selected_menu_item) else {
        return;
    };
    draw.draw_text("Confirm Purchase", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        &format!("Buy {} for {} credits?", offer.name, offer.cost),
        330,
        215,
        24,
        Color::RAYWHITE,
    );
    draw.draw_text(offer.description, 330, 252, 18, Color::LIGHTGRAY);
    draw.draw_text(
        "Enter/E confirms | Backspace/Esc cancels",
        330,
        310,
        20,
        Color::WHITE,
    );
}

#[allow(
    clippy::too_many_lines,
    reason = "shop tab screen includes category list and stat preview"
)]
fn draw_modal_shop(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Upgrade Shop", 330, 150, 30, Color::WHITE);
    draw.draw_text(
        "Up/Down select | Enter/E buy | Backspace/Esc close",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    draw.draw_text(
        "[1] Drill [2] Tank [3] Cargo [4] Engine [5] Hull [6] Radiator [7] Scanner [8] Bombs",
        330,
        208,
        16,
        Color::LIGHTGRAY,
    );
    let offers = upgrade_offers(&game.player);
    for (index, offer) in offers.iter().enumerate() {
        let y = 242 + i32::try_from(index).unwrap_or(i32::MAX) * 42;
        let selected = index == game.selected_menu_item;
        let affordable = game.player.credits >= offer.cost;
        let color = if selected {
            Color::YELLOW
        } else if affordable
            || (offer.kind != UpgradeKind::BombPack
                && offer.level >= crate::economy::MAX_UPGRADE_LEVEL)
        {
            Color::RAYWHITE
        } else {
            Color::GRAY
        };
        let price = if offer.kind != UpgradeKind::BombPack
            && offer.level >= crate::economy::MAX_UPGRADE_LEVEL
        {
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
            let missing = offer.cost.saturating_sub(game.player.credits);
            draw.draw_text(
                &format!("Cost: {} cr | Missing: {} cr", offer.cost, missing),
                715,
                416,
                16,
                Color::YELLOW,
            );
            draw.draw_text(
                &upgrade_stat_preview(game, offer.kind),
                715,
                446,
                16,
                Color::RAYWHITE,
            );
            draw.draw_text(
                if missing == 0 {
                    "Affordable"
                } else {
                    "Need more credits"
                },
                715,
                476,
                16,
                if missing == 0 {
                    Color::GREEN
                } else {
                    Color::RED
                },
            );
        }
    }
}

fn upgrade_stat_preview(game: &GameState, kind: UpgradeKind) -> String {
    match kind {
        UpgradeKind::Drill => format!(
            "Drill speed: L{} -> L{}",
            game.player.drill_strength,
            game.player.drill_strength.saturating_add(1)
        ),
        UpgradeKind::FuelTank => format!(
            "Fuel: {:.0} -> {:.0}",
            game.player.fuel_capacity,
            game.player.fuel_capacity + 50.0
        ),
        UpgradeKind::CargoBay => format!(
            "Cargo: {} -> {} slots",
            game.player.cargo_capacity,
            game.player.cargo_capacity + 8
        ),
        UpgradeKind::Engine => format!(
            "Thrust: L{} -> L{}",
            game.player.engine_level,
            game.player.engine_level.saturating_add(1)
        ),
        UpgradeKind::Hull => format!(
            "Hull: {:.0} -> {:.0}",
            game.player.max_hull(),
            game.player.max_hull() + 35.0
        ),
        UpgradeKind::Radiator => format!(
            "Safe heat depth: L{} -> L{}",
            game.player.radiator_level,
            game.player.radiator_level.saturating_add(1)
        ),
        UpgradeKind::Scanner => format!(
            "Scanner: L{} -> L{}",
            game.player.scanner_level,
            game.player.scanner_level.saturating_add(1)
        ),
        UpgradeKind::BombPack => format!(
            "Bomb inventory: {} -> {}",
            game.player.bombs,
            game.player.bombs + 3
        ),
    }
}

fn draw_options(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Options", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        "Enter/E changes selected option. F11 toggles fullscreen immediately.",
        330,
        198,
        18,
        Color::LIGHTGRAY,
    );
    let rows = [
        format!("Volume up  | current {:.0}%", game.master_volume * 100.0),
        format!("Volume down| current {:.0}%", game.master_volume * 100.0),
        format!(
            "Fullscreen preference: {}",
            if game.fullscreen { "on" } else { "off" }
        ),
    ];
    for (index, row) in rows.iter().enumerate() {
        draw.draw_text(
            row,
            350,
            255 + i32::try_from(index).unwrap_or(i32::MAX) * 42,
            22,
            if index == game.selected_menu_item {
                Color::YELLOW
            } else {
                Color::RAYWHITE
            },
        );
    }
}

fn draw_save_slots(draw: &mut RaylibDrawHandle<'_>, game: &GameState, saving: bool) {
    draw.draw_text(
        if saving { "Save Slots" } else { "Load Slots" },
        330,
        150,
        30,
        Color::GOLD,
    );
    draw.draw_text(
        "Up/Down choose slot | Enter/E confirm | Esc closes",
        330,
        198,
        18,
        Color::LIGHTGRAY,
    );
    for slot in 0..save_slot_count() {
        let exists = save_slot_exists(slot);
        let label = if exists { "occupied" } else { "empty" };
        let detail = save_slot_metadata(slot).map_or_else(
            || label.to_owned(),
            |meta| {
                format!(
                    "depth {}m | {} cr | cargo {}/{} | contracts {} | time {}m | saved {}{}",
                    meta.depth,
                    meta.credits,
                    meta.cargo_used,
                    meta.cargo_capacity,
                    meta.contracts_completed,
                    (meta.play_seconds / 60.0).floor() as u32,
                    meta.modified_unix_seconds
                        .map_or_else(|| "unknown".to_owned(), |seconds| format!("unix {seconds}")),
                    if meta.won_game { " | core secured" } else { "" }
                )
            },
        );
        draw.draw_text(
            &format!("Slot {} - {detail}", slot + 1),
            360,
            255 + i32::try_from(slot).unwrap_or(i32::MAX) * 46,
            24,
            if slot == game.selected_menu_item {
                Color::YELLOW
            } else if exists || saving {
                Color::RAYWHITE
            } else {
                Color::GRAY
            },
        );
    }
}

fn draw_title(draw: &mut RaylibDrawHandle<'_>) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 190));
    draw.draw_text("DRILLGAME", 475, 250, 54, Color::GOLD);
    draw.draw_text("Press Enter or E to start", 505, 325, 24, Color::RAYWHITE);
    draw.draw_text(
        "P pause/options/slots | H help | M map | F11 fullscreen",
        455,
        365,
        20,
        Color::LIGHTGRAY,
    );
}

fn draw_pause(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 150));
    draw.draw_rectangle(430, 170, 420, 360, Color::new(0, 0, 0, 220));
    draw.draw_rectangle_lines(430, 170, 420, 360, Color::RAYWHITE);
    draw.draw_text("PAUSED", 548, 200, 44, Color::RAYWHITE);

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
    draw.draw_rectangle(300, 180, 680, 340, Color::new(0, 0, 0, 230));
    draw.draw_rectangle_lines(300, 180, 680, 340, Color::GOLD);
    draw.draw_text("STAR CORE RECOVERED", 392, 215, 40, Color::GOLD);
    draw.draw_text(&game.message, 340, 285, 20, Color::RAYWHITE);
    let stats = [
        format!("Total earnings: {} cr", game.total_earnings),
        format!("Contracts completed: {}", game.contracts.completed),
        format!("Deepest depth: {}m", game.deepest_tile_reached),
        format!("Rescues: {}", game.rescue_count),
        format!("Artifacts found: {}", game.artifacts_found),
        format!("Debt remaining: {} cr", game.player.loan_debt),
        format!(
            "Bombs left / scanner tier: {} / {}",
            game.player.bombs, game.player.scanner_level
        ),
    ];
    for (index, stat) in stats.iter().enumerate() {
        draw.draw_text(
            stat,
            390,
            330 + i32::try_from(index).unwrap_or(i32::MAX) * 24,
            20,
            Color::RAYWHITE,
        );
    }
    draw.draw_text(
        "You can keep mining, save, or exit from the pause menu.",
        382,
        475,
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
