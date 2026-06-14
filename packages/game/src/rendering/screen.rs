use raylib::prelude::*;

use super::terrain::{artifact_color, mineral_color};
use super::{SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::{
    economy::{
        DeepClaimStatus, TownBuilding, UpgradeKind, upgrade_effect, upgrade_offers,
        upgrade_tier_name,
    },
    game_state::{
        GameState, ModalScreen, PauseOption, RecipeKind, RunMode, SideContractKind, TILE_SIZE,
    },
    save::{save_slot_count, save_slot_exists, save_slot_metadata},
    surface::SURFACE_BUILDINGS,
    terrain::{ArtifactKind, MineralKind, StrategicResourceKind, TileKind, TilePosition},
};

struct MinimapProjection {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    terrain_width: i32,
    terrain_height: i32,
}

pub(super) fn draw_heat_warning(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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

pub(super) fn draw_hud(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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

    draw.draw_text(&game.town_event, 22, 96, 16, Color::LIGHTGRAY);
    draw.draw_text(
        &game.active_world_event_summary(),
        22,
        116,
        16,
        Color::ORANGE,
    );
    draw.draw_text(
        &format!("Deep instability: {:.0}%", game.deep_instability.min(100.0)),
        22,
        136,
        16,
        if game.deep_instability >= 70.0 {
            Color::RED
        } else {
            Color::LIGHTGRAY
        },
    );
    draw_expedition_tracker(draw, game);
    if game.player.scanner_level > 0 {
        let scanner = if game.scanner_cooldown_seconds > 0.0 {
            format!("Scanner cooldown {:.1}s", game.scanner_cooldown_seconds)
        } else {
            "Scanner ready (C)".to_owned()
        };
        draw.draw_text(&scanner, 22, 156, 16, Color::SKYBLUE);
    }
    draw_infrastructure_kit_prompts(draw, game);

    if game.escape_sequence_seconds > 0.0 {
        draw.draw_rectangle(470, 70, 340, 34, Color::new(90, 0, 0, 185));
        draw.draw_rectangle_lines(470, 70, 340, 34, Color::RED);
        draw.draw_text(
            &format!("CORE ESCAPE {:.0}s", game.escape_sequence_seconds.ceil()),
            500,
            78,
            22,
            Color::RAYWHITE,
        );
    }

    if game.show_details || game.modal == Some(ModalScreen::Depot) {
        draw_detail_panel(draw, game);
    }
    if game.show_details {
        draw_debug_stats(draw, game);
    }
}

fn draw_infrastructure_kit_prompts(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let prompts = [
        (
            game.player.signal_relay_kits,
            format!(
                "Signal relays: {} kit(s), {} online (R to place)",
                game.player.signal_relay_kits,
                game.signal_relay_count()
            ),
        ),
        (
            game.player.survey_drone_kits,
            format!(
                "Survey drones: {} kit(s) (T to place)",
                game.player.survey_drone_kits
            ),
        ),
        (
            game.player.cargo_lift_kits,
            format!(
                "Cargo lifts: {} kit(s) (L to place)",
                game.player.cargo_lift_kits
            ),
        ),
        (
            game.player.tunnel_support_kits,
            format!(
                "Tunnel supports: {} kit(s) (U to place)",
                game.player.tunnel_support_kits
            ),
        ),
        (
            game.player.pump_station_kits,
            format!(
                "Pump stations: {} kit(s) (O to place)",
                game.player.pump_station_kits
            ),
        ),
        (
            game.player.ore_processor_kits,
            format!(
                "Ore processors: {} kit(s) (P to place)",
                game.player.ore_processor_kits
            ),
        ),
    ];
    let mut row = 0;
    for (count, prompt) in prompts {
        if count == 0 {
            continue;
        }
        draw.draw_text(&prompt, 720, 74 + row * 22, 16, Color::GREEN);
        row += 1;
    }
}

fn draw_expedition_tracker(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    if game.active_expeditions.is_empty() {
        return;
    }
    draw.draw_text("Expeditions:", 22, 138, 16, Color::GREEN);
    for (index, expedition) in game.active_expeditions.iter().take(3).enumerate() {
        draw.draw_text(
            &game.expedition_status_line(*expedition),
            36,
            160 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
            15,
            Color::RAYWHITE,
        );
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
            "D{} E{} H{} R{} S{} B{} Debt{} | C scan",
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

pub(super) fn draw_minimap(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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
                TileKind::Foundation => Color::new(135, 125, 105, 220),
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

    for marker in &game.scan_markers {
        draw_map_marker(
            draw,
            &projection,
            marker.position.x,
            marker.position.y,
            marker_color(marker.kind),
        );
    }

    for item in &game.infrastructure {
        let color = match item.kind {
            crate::game_state::InfrastructureKind::SignalRelay => Color::SKYBLUE,
            crate::game_state::InfrastructureKind::SurveyDrone => Color::GREEN,
            crate::game_state::InfrastructureKind::CargoLift => Color::GOLD,
            crate::game_state::InfrastructureKind::TunnelSupport => Color::ORANGE,
            crate::game_state::InfrastructureKind::PumpStation => Color::BLUE,
            crate::game_state::InfrastructureKind::OreProcessor => Color::PURPLE,
        };
        draw_map_marker(draw, &projection, item.position.x, item.position.y, color);
    }

    for warning in &game.collapse_warnings {
        draw_map_marker(draw, &projection, warning.x, warning.y, Color::RED);
    }

    for building in SURFACE_BUILDINGS {
        draw_map_marker(
            draw,
            &projection,
            building.tile_x + building.tile_width / 2,
            7,
            Color::RAYWHITE,
        );
    }
    let player_x = x + ((game.player.x / TILE_SIZE) as i32) * width / terrain_width;
    let player_y = y + ((game.player.y / TILE_SIZE) as i32) * height / terrain_height;
    if game.scanner_pulse_seconds > 0.0 {
        draw.draw_circle_lines(
            player_x,
            player_y,
            10.0 + game.scanner_pulse_seconds * 12.0,
            Color::SKYBLUE,
        );
    }
    draw.draw_circle(player_x, player_y, 3.0, Color::SKYBLUE);
}

const fn marker_color(kind: TileKind) -> Color {
    match kind {
        TileKind::Ore(_) => Color::GOLD,
        TileKind::Artifact(_) => Color::MAGENTA,
        TileKind::Gas
        | TileKind::Lava
        | TileKind::MagmaVent
        | TileKind::ExplosivePocket
        | TileKind::PressurePocket => Color::RED,
        _ => Color::WHITE,
    }
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

pub(super) fn draw_depth_ruler(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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

pub(super) fn draw_modal(draw: &mut RaylibDrawHandle<'_>, game: &GameState, modal: ModalScreen) {
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
        ModalScreen::Bank => draw_bank(draw, game),
        ModalScreen::Explosives => draw_explosives(draw, game),
        ModalScreen::Salvage => draw_salvage(draw, game),
        ModalScreen::Options => draw_options(draw, game),
        ModalScreen::SaveSlots => draw_save_slots(draw, game, true),
        ModalScreen::LoadSlots => draw_save_slots(draw, game, false),
        ModalScreen::Map => draw_large_map(draw, game),
        ModalScreen::Help => draw_help(draw),
        ModalScreen::TownDevelopment => draw_town_development(draw, game),
        ModalScreen::ExpeditionBoard => draw_expedition_board(draw, game),
        ModalScreen::ResearchLog => draw_research_log(draw, game),
        ModalScreen::Crafting => draw_crafting(draw, game),
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

fn draw_options_list(
    draw: &mut RaylibDrawHandle<'_>,
    selected: usize,
    x: i32,
    y: i32,
    options: &[String],
) {
    draw.draw_text(
        "Up/Down choose | Enter/E confirm | Backspace/Esc close",
        x,
        y - 35,
        18,
        Color::LIGHTGRAY,
    );
    for (index, option) in options.iter().enumerate() {
        let color = if index == selected {
            Color::YELLOW
        } else {
            Color::RAYWHITE
        };
        draw.draw_text(
            option,
            x,
            y + i32::try_from(index).unwrap_or(i32::MAX) * 42,
            22,
            color,
        );
    }
}

fn draw_bank(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Iron Ledger Bank", 330, 150, 30, Color::GOLD);
    let finance = if game.player.loan_debt == 0 {
        "Take loan: +250 now, owe 300".to_owned()
    } else {
        format!("Pay debt: {} owed", game.player.loan_debt)
    };
    let insurance = if game.player.insured {
        "Insurance active".to_owned()
    } else {
        "Buy rescue insurance: 90 cr".to_owned()
    };
    let side = if game.active_side_contracts.len() >= 3 {
        "Side board full (3 active)".to_owned()
    } else {
        format!(
            "Post side contract ({}/3 active)",
            game.active_side_contracts.len()
        )
    };
    draw_options_list(
        draw,
        game.selected_menu_item,
        360,
        230,
        &[finance, insurance, side],
    );
}

fn draw_explosives(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Nix's Explosive Shack", 330, 150, 30, Color::RED);
    draw.draw_text(
        &format!("Bombs carried: {}", game.player.bombs),
        360,
        200,
        20,
        Color::RAYWHITE,
    );
    draw_options_list(
        draw,
        game.selected_menu_item,
        360,
        245,
        &[
            "3 timed charges: 55 cr".to_owned(),
            "7 timed charges: 120 cr".to_owned(),
            "Ask Nix for one risky freebie".to_owned(),
        ],
    );
}

fn draw_salvage(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Mara's Salvage Yard", 330, 150, 30, Color::LIME);
    draw.draw_text(
        &format!("Lost cargo markers: {}", game.lost_cargo_count),
        360,
        200,
        20,
        Color::RAYWHITE,
    );
    draw_options_list(
        draw,
        game.selected_menu_item,
        360,
        245,
        &[
            "Recover lost cargo markers".to_owned(),
            "Patch hull free".to_owned(),
            "Sell scrap telemetry: +35 cr".to_owned(),
        ],
    );
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
    let mut options = vec![
        "Complete active contract".to_owned(),
        "Ask for briefing/radio intel".to_owned(),
        finance,
    ];
    if game.deep_claim_status == DeepClaimStatus::Unlocked {
        options.push("Deep Claim town development".to_owned());
        options.push("Expedition board".to_owned());
        options.push("Research log".to_owned());
        options.push("Crafting bench".to_owned());
    }
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

fn draw_crafting(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Crafting Bench", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        "Enter/E crafts selected recipe using strategic materials.",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    let materials = [
        StrategicResourceKind::AncientAlloy,
        StrategicResourceKind::CoreShard,
        StrategicResourceKind::CrystalLens,
    ];
    for (index, material) in materials.iter().enumerate() {
        draw.draw_text(
            &format!(
                "{}: {}",
                material.name(),
                game.player.materials.get(material).copied().unwrap_or(0)
            ),
            330 + i32::try_from(index).unwrap_or(i32::MAX) * 190,
            220,
            18,
            Color::SKYBLUE,
        );
    }
    for (index, recipe) in RecipeKind::ALL.iter().enumerate() {
        let color = if index == game.selected_menu_item {
            Color::YELLOW
        } else {
            Color::RAYWHITE
        };
        let costs = recipe
            .cost()
            .iter()
            .map(|(material, count)| format!("{} {count}", material.name()))
            .collect::<Vec<_>>()
            .join(", ");
        draw.draw_text(
            &format!("{} - {} [{}]", recipe.name(), recipe.description(), costs),
            350,
            270 + i32::try_from(index).unwrap_or(i32::MAX) * 34,
            20,
            color,
        );
    }
    draw.draw_text(
        &format!(
            "Crafted: bulkheads {} | sorters {} | relay kits {} | drone kits {} | lift kits {} | support kits {} | pump kits {} | processor kits {}",
            game.player.crafted_bulkheads,
            game.player.crafted_sorters,
            game.player.signal_relay_kits,
            game.player.survey_drone_kits,
            game.player.cargo_lift_kits,
            game.player.tunnel_support_kits,
            game.player.pump_station_kits,
            game.player.ore_processor_kits
        ),
        350,
        450,
        18,
        Color::GREEN,
    );
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn draw_collection_rewards(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text(
        &format!(
            "Rewards claimed: minerals {} | artifacts {} | hazards {}",
            yes_no(
                game.collection_log
                    .rewards_claimed
                    .contains(&crate::game_state::CollectionRewardKind::Minerals)
            ),
            yes_no(
                game.collection_log
                    .rewards_claimed
                    .contains(&crate::game_state::CollectionRewardKind::Artifacts)
            ),
            yes_no(
                game.collection_log
                    .rewards_claimed
                    .contains(&crate::game_state::CollectionRewardKind::Hazards)
            )
        ),
        330,
        246,
        18,
        Color::GREEN,
    );
}

fn draw_research_log(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Research Log", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        "Discover minerals by mining/scanning. Discover hazards by scanner contact.",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    let mineral_total = 10;
    let artifact_total = 4;
    draw.draw_text(
        &format!(
            "Minerals: {}/{} | Artifacts: {}/{} | Hazards: {} | Strata: {} | Stories: {}",
            game.collection_log.minerals.len(),
            mineral_total,
            game.collection_log.artifacts.len(),
            artifact_total,
            game.collection_log.hazards.len(),
            game.collection_log.strata.len(),
            game.collection_log.story_records.len()
        ),
        330,
        220,
        20,
        Color::SKYBLUE,
    );
    draw_collection_rewards(draw, game);
    let minerals = [
        MineralKind::Copper,
        MineralKind::Iron,
        MineralKind::Silver,
        MineralKind::Gold,
        MineralKind::Emerald,
        MineralKind::Ruby,
        MineralKind::Diamond,
        MineralKind::Platinum,
        MineralKind::Uranium,
        MineralKind::Mythril,
    ];
    for (index, mineral) in minerals.iter().enumerate() {
        let known = game.collection_log.minerals.contains(mineral);
        draw.draw_text(
            if known { mineral.name() } else { "???" },
            350 + i32::try_from(index % 2).unwrap_or_default() * 160,
            265 + i32::try_from(index / 2).unwrap_or_default() * 28,
            20,
            if known { Color::RAYWHITE } else { Color::GRAY },
        );
    }
    let artifacts = [
        ArtifactKind::Fossil,
        ArtifactKind::OldCircuit,
        ArtifactKind::BuriedIdol,
        ArtifactKind::StarCore,
    ];
    for (index, artifact) in artifacts.iter().enumerate() {
        let known = game.collection_log.artifacts.contains(artifact);
        draw.draw_text(
            if known { artifact.name() } else { "???" },
            350 + i32::try_from(index % 2).unwrap_or_default() * 190,
            430 + i32::try_from(index / 2).unwrap_or_default() * 28,
            20,
            if known { Color::RAYWHITE } else { Color::GRAY },
        );
    }
    draw_research_log_sidebar(draw, game);
}

fn draw_research_log_sidebar(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("NPC Stories", 620, 250, 20, Color::GOLD);
    let stories = [
        crate::game_state::NpcStoryRecord::ValeIntro,
        crate::game_state::NpcStoryRecord::IonaSilverWarning,
        crate::game_state::NpcStoryRecord::KadeRelicSignal,
        crate::game_state::NpcStoryRecord::ValeThermalWarning,
        crate::game_state::NpcStoryRecord::KadeStarCoreSignal,
        crate::game_state::NpcStoryRecord::ValeStarCoreSecured,
    ];
    for (index, story) in stories.iter().enumerate() {
        let known = game.collection_log.story_records.contains(story);
        draw.draw_text(
            if known { story.title() } else { "???" },
            620,
            280 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
            16,
            if known { Color::RAYWHITE } else { Color::GRAY },
        );
    }
    draw.draw_text("Hazards", 620, 410, 20, Color::ORANGE);
    for (index, hazard) in game.collection_log.hazards.iter().enumerate() {
        draw.draw_text(
            hazard.name(),
            620,
            438 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
            16,
            Color::RAYWHITE,
        );
    }
    draw.draw_text("Materials", 620, 545, 20, Color::SKYBLUE);
    let materials = [
        StrategicResourceKind::AncientAlloy,
        StrategicResourceKind::CoreShard,
        StrategicResourceKind::CrystalLens,
    ];
    for (index, material) in materials.iter().enumerate() {
        let count = game.player.materials.get(material).copied().unwrap_or(0);
        draw.draw_text(
            &format!("{}: {count}", material.name()),
            620,
            570 + i32::try_from(index).unwrap_or(i32::MAX) * 20,
            16,
            Color::RAYWHITE,
        );
    }
}

fn draw_expedition_board(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Expedition Board", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        "Enter/E accepts offer or abandons selected active expedition | Depot completes",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    draw.draw_text("Offers", 330, 225, 22, Color::SKYBLUE);
    if game.expedition_offers.is_empty() {
        draw.draw_text(
            "No offers posted. Reopen board to refresh.",
            350,
            265,
            20,
            Color::GRAY,
        );
    }
    for (index, expedition) in game.expedition_offers.iter().enumerate() {
        let color = if index == game.selected_menu_item {
            Color::YELLOW
        } else {
            Color::RAYWHITE
        };
        draw.draw_text(
            &game.expedition_status_line(*expedition),
            350,
            265 + i32::try_from(index).unwrap_or(i32::MAX) * 34,
            20,
            color,
        );
    }
    draw.draw_text(
        "Active (select + Enter/E to abandon)",
        330,
        390,
        22,
        Color::GREEN,
    );
    if game.active_expeditions.is_empty() {
        draw.draw_text("No active expeditions.", 350, 430, 20, Color::GRAY);
    }
    for (index, expedition) in game.active_expeditions.iter().enumerate() {
        let active_menu_index = game.expedition_offers.len() + index;
        draw.draw_text(
            &game.expedition_status_line(*expedition),
            350,
            430 + i32::try_from(index).unwrap_or(i32::MAX) * 28,
            19,
            if active_menu_index == game.selected_menu_item {
                Color::YELLOW
            } else {
                Color::RAYWHITE
            },
        );
    }
}

fn draw_town_development(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_text("Deep Claim Town Development", 330, 150, 30, Color::GOLD);
    draw.draw_text(
        "Post-core charter investments | Enter/E upgrades | Esc closes",
        330,
        184,
        18,
        Color::LIGHTGRAY,
    );
    draw.draw_text(
        &format!(
            "Reputation: {} ({}) | {}",
            game.town_development.reputation,
            game.reputation_rank(),
            game.advanced_permit_status()
        ),
        330,
        215,
        20,
        Color::SKYBLUE,
    );
    let alloy = game
        .player
        .materials
        .get(&StrategicResourceKind::AncientAlloy)
        .copied()
        .unwrap_or(0);
    draw.draw_text(
        &format!("Ancient Alloy: {alloy} (needed after level 1 upgrades)"),
        330,
        238,
        18,
        Color::LIGHTGRAY,
    );
    for (index, building) in TownBuilding::ALL.iter().enumerate() {
        let level = game.town_development.level(*building);
        let cost = game.town_development.upgrade_cost(*building);
        let color = if index == game.selected_menu_item {
            Color::YELLOW
        } else if game.player.credits >= cost {
            Color::RAYWHITE
        } else {
            Color::GRAY
        };
        draw.draw_text(
            &format!(
                "{} L{} -> L{} | {} cr",
                building.name(),
                level,
                level + 1,
                cost
            ),
            350,
            260 + i32::try_from(index).unwrap_or(i32::MAX) * 38,
            21,
            color,
        );
    }
    draw.draw_text(
        "Current effects: Depot prices, scanner radius/cooldown, salvage fees, bomb bundles.",
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
    draw.draw_text("Active side contracts:", 700, 230, 18, Color::LIGHTGRAY);
    if game.active_side_contracts.is_empty() {
        draw.draw_text("None posted", 720, 258, 16, Color::GRAY);
    }
    for (index, contract) in game.active_side_contracts.iter().enumerate() {
        let label = match contract.kind {
            SideContractKind::Cargo => {
                format!("Cargo: {} x{}", contract.target.name(), contract.required)
            }
            SideContractKind::DepthSurvey => format!("Survey: reach {}m", contract.required),
            SideContractKind::HazardScan => format!("Scan: {} hazards", contract.required),
            SideContractKind::Rush => format!(
                "Rush: {} x{} by day {}",
                contract.target.name(),
                contract.required,
                contract.expires_day.unwrap_or(0)
            ),
        };
        draw.draw_text(
            &label,
            720,
            258 + i32::try_from(index).unwrap_or(i32::MAX) * 22,
            16,
            Color::RAYWHITE,
        );
    }

    let trend = match game.market_history.as_slice() {
        [.., previous, current] if current > previous => "trend ↑",
        [.., previous, current] if current < previous => "trend ↓",
        _ => "trend →",
    };
    draw.draw_text(
        &format!("Market board ({trend}):"),
        700,
        350,
        18,
        Color::GOLD,
    );
    let minerals = [
        MineralKind::Copper,
        MineralKind::Iron,
        MineralKind::Silver,
        MineralKind::Gold,
        MineralKind::Emerald,
        MineralKind::Ruby,
        MineralKind::Diamond,
        MineralKind::Platinum,
        MineralKind::Uranium,
        MineralKind::Mythril,
    ];
    for (index, mineral) in minerals.iter().enumerate() {
        let current = game.mineral_market_factor(*mineral);
        let previous = game
            .previous_mineral_market_factor(*mineral)
            .unwrap_or(current);
        let trend = match current.cmp(&previous) {
            std::cmp::Ordering::Greater => "↑",
            std::cmp::Ordering::Less => "↓",
            std::cmp::Ordering::Equal => "→",
        };
        let label = if current >= 120 {
            "high"
        } else if current <= 90 {
            "low"
        } else {
            "avg"
        };
        draw.draw_text(
            &format!(
                "{}: {} cr {trend} {label}",
                mineral.name(),
                game.mineral_market_value(*mineral)
            ),
            720,
            378 + i32::try_from(index).unwrap_or(i32::MAX) * 18,
            14,
            mineral_color(*mineral),
        );
    }

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
                TileKind::Foundation => Color::new(135, 125, 105, 255),
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

    for marker in &game.scan_markers {
        let px = x + marker.position.x * width / terrain_width;
        let py = y + marker.position.y * height / terrain_height;
        draw.draw_rectangle(px - 2, py - 2, 5, 5, marker_color(marker.kind));
    }

    draw_large_map_infrastructure(
        draw,
        game,
        (x, y, width, height),
        (terrain_width, terrain_height),
    );

    let player_x = x + ((game.player.x / TILE_SIZE) as i32) * width / terrain_width;
    let player_y = y + ((game.player.y / TILE_SIZE) as i32) * height / terrain_height;
    draw.draw_circle(player_x, player_y, 6.0, Color::SKYBLUE);
    draw.draw_text("YOU", player_x + 8, player_y - 7, 12, Color::SKYBLUE);

    for warning in &game.collapse_warnings {
        let px = x + warning.x * width / terrain_width;
        let py = y + warning.y * height / terrain_height;
        draw.draw_circle_lines(px, py, 6.0, Color::RED);
    }

    if let (Some(cargo_x), Some(cargo_y)) = (game.lost_cargo_x, game.lost_cargo_y) {
        let marker_x = x + ((cargo_x / TILE_SIZE) as i32) * width / terrain_width;
        let marker_y = y + ((cargo_y / TILE_SIZE) as i32) * height / terrain_height;
        draw.draw_rectangle(marker_x - 4, marker_y - 4, 8, 8, Color::GOLD);
        draw.draw_text("LOST", marker_x + 6, marker_y - 6, 12, Color::GOLD);
    }

    for building in SURFACE_BUILDINGS {
        let px = x + (building.tile_x + building.tile_width / 2) * width / terrain_width;
        let py = y + 4 * height / terrain_height;
        draw.draw_circle(px, py, 4.0, Color::RAYWHITE);
        draw.draw_text(building.label, px + 6, py - 6, 10, Color::RAYWHITE);
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
        "Legend: gold ore | orange rare/blast | magenta artifact | red lava/vent | green gas | cyan pressure | sky relay | blue you",
        190,
        612,
        16,
        Color::LIGHTGRAY,
    );
}

fn draw_large_map_infrastructure(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    map_rect: (i32, i32, i32, i32),
    terrain_size: (i32, i32),
) {
    let (x, y, width, height) = map_rect;
    let (terrain_width, terrain_height) = terrain_size;
    for item in &game.infrastructure {
        let px = x + item.position.x * width / terrain_width;
        let py = y + item.position.y * height / terrain_height;
        let (label, color) = match item.kind {
            crate::game_state::InfrastructureKind::SignalRelay => ("R", Color::SKYBLUE),
            crate::game_state::InfrastructureKind::SurveyDrone => ("D", Color::GREEN),
            crate::game_state::InfrastructureKind::CargoLift => ("L", Color::GOLD),
            crate::game_state::InfrastructureKind::TunnelSupport => ("S", Color::ORANGE),
            crate::game_state::InfrastructureKind::PumpStation => ("P", Color::BLUE),
            crate::game_state::InfrastructureKind::OreProcessor => ("O", Color::PURPLE),
        };
        draw.draw_circle_lines(px, py, 6.0, color);
        draw.draw_text(label, px + 7, py - 6, 10, color);
    }
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

pub(super) fn draw_title(draw: &mut RaylibDrawHandle<'_>) {
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

pub(super) fn draw_pause(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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

pub(super) fn draw_ending(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(5, 8, 18, 205));
    draw.draw_rectangle(300, 180, 680, 340, Color::new(0, 0, 0, 230));
    draw.draw_rectangle_lines(300, 180, 680, 340, Color::GOLD);
    draw.draw_text("STAR CORE RECOVERED", 392, 215, 40, Color::GOLD);
    draw.draw_text(&game.message, 340, 285, 20, Color::RAYWHITE);
    let stats = [
        format!("Total earnings: {} cr", game.total_earnings),
        format!("Contracts completed: {}", game.contracts.completed),
        format!("Deepest depth: {}m", game.deepest_tile_reached),
        format!("Best return depth: {}m", game.best_return_depth),
        format!("Most valuable run: {} cr", game.most_valuable_cargo_run),
        format!("Resources refined: {}", game.total_resources_refined),
        format!("Badges earned: {}", game.challenge_badges.len()),
        format!(
            "Cosmetic skins: {}",
            game.cosmetic_skins
                .iter()
                .map(|skin| skin.title())
                .collect::<Vec<_>>()
                .join(", ")
        ),
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

pub(super) fn draw_game_over(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    if game.won_game {
        draw.draw_rectangle(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT, Color::new(0, 0, 0, 170));
        draw.draw_rectangle(330, 190, 620, 260, Color::new(18, 24, 42, 245));
        draw.draw_rectangle_lines(330, 190, 620, 260, Color::GOLD);
        draw.draw_text("STAR CORE SECURED", 455, 220, 34, Color::GOLD);
        draw.draw_text(
            &format!("Deepest depth: {}m", game.deepest_tile_reached),
            390,
            280,
            22,
            Color::RAYWHITE,
        );
        draw.draw_text(
            &format!("Total earnings: {} cr", game.total_earnings),
            390,
            312,
            22,
            Color::RAYWHITE,
        );
        draw.draw_text(
            &format!("Rescues called: {}", game.rescue_count),
            390,
            344,
            22,
            Color::RAYWHITE,
        );
        draw.draw_text(
            &format!("Contracts completed: {}", game.contracts.completed),
            390,
            376,
            22,
            Color::RAYWHITE,
        );
        draw.draw_text(
            "You can keep mining this save after closing the summary.",
            390,
            414,
            18,
            Color::LIGHTGRAY,
        );
        return;
    }
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
