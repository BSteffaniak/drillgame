use raylib::prelude::*;

use super::ui::{UiContext, modal_rect};
use crate::{
    economy::{upgrade_offers, upgrade_tier_name},
    game_state::{
        GameState, ModalScreen, OnlineSaveAuthority, PauseOption, TILE_SIZE, TitleOption,
    },
    save::{latest_save_summary, save_slot_count, save_slot_metadata},
    session::{ClientView, PerPlayerHudSnapshot},
    terrain::{MineralKind, TileKind, TilePosition},
};

struct MinimapProjection {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    terrain_width: i32,
    origin_y: i32,
    visible_height: i32,
}

pub(super) fn draw_heat_warning_for_view(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
) {
    let depth_heat = view.camera.y > 18.0 * TILE_SIZE;
    let tile_x = (view.camera.x / TILE_SIZE) as i32;
    let tile_y = (view.camera.y / TILE_SIZE) as i32;
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
            view.viewport.x,
            view.viewport.y,
            view.viewport.width,
            view.viewport.height,
            Color::new(255, 70, 20, alpha),
        );
    }
}

pub(super) fn draw_hud_for_view(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
) {
    draw_view_hud_panel(draw, game, view, None);
}

pub(super) fn draw_hud_snapshot_for_view(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
    hud: PerPlayerHudSnapshot,
) {
    draw_view_hud_panel(draw, game, view, Some(hud));
}

#[allow(
    clippy::too_many_lines,
    reason = "top HUD composes multiple bounded panels"
)]
fn draw_view_hud_panel(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
    hud: Option<PerPlayerHudSnapshot>,
) {
    let fps = draw.get_fps();
    let mut ui = UiContext::new(draw);
    let margin = 12.0;
    let gap = 8.0;
    let origin_x = view.viewport.x as f32 + margin;
    let origin_y = view.viewport.y as f32 + margin;
    let available_width = (view.viewport.width as f32 - margin * 2.0).max(280.0);
    let metric_width = ((available_width - gap * 3.0) / 4.0).max(120.0);

    let (hull, max_hull, fuel, fuel_capacity, credits, cargo_used, cargo_capacity) = hud
        .map_or_else(
            || {
                (
                    game.player.hull,
                    game.player.max_hull(),
                    game.player.fuel,
                    game.player.fuel_capacity,
                    game.player.credits,
                    game.player.cargo_used(),
                    game.player.cargo_capacity,
                )
            },
            |hud| {
                (
                    hud.hull,
                    hud.max_hull,
                    hud.fuel,
                    hud.fuel_capacity,
                    hud.credits,
                    hud.cargo_used,
                    game.player.cargo_capacity,
                )
            },
        );

    let mut hull_panel = ui.compact_panel(Rectangle {
        x: origin_x,
        y: origin_y,
        width: metric_width,
        height: 56.0,
    });
    hull_panel.begin_clip();
    hull_panel.heading(&format!("P{} Hull", view.controlled_player_id.get()));
    hull_panel.progress_bar("", hull, max_hull, Color::SKYBLUE, Color::RED);
    drop(hull_panel);

    let mut fuel_panel = ui.compact_panel(Rectangle {
        x: origin_x + (metric_width + gap),
        y: origin_y,
        width: metric_width,
        height: 56.0,
    });
    fuel_panel.begin_clip();
    fuel_panel.heading("Fuel");
    fuel_panel.progress_bar("", fuel, fuel_capacity, Color::LIME, Color::ORANGE);
    drop(fuel_panel);

    let mut cargo_panel = ui.compact_panel(Rectangle {
        x: origin_x + (metric_width + gap) * 2.0,
        y: origin_y,
        width: metric_width,
        height: 56.0,
    });
    cargo_panel.begin_clip();
    cargo_panel.heading("Cargo");
    cargo_panel.stat_line(
        "Hold",
        &format!("{cargo_used}/{cargo_capacity}"),
        Color::LIGHTGRAY,
    );
    cargo_panel.progress_bar(
        "",
        cargo_used as f32,
        cargo_capacity as f32,
        Color::GOLD,
        Color::ORANGE,
    );
    drop(cargo_panel);

    let mut run_panel = ui.compact_panel(Rectangle {
        x: origin_x + (metric_width + gap) * 3.0,
        y: origin_y,
        width: metric_width,
        height: 56.0,
    });
    run_panel.begin_clip();
    run_panel.heading("Run");
    run_panel.stat_line("Credits", &format!("{credits} cr"), Color::GOLD);
    run_panel.stat_line(
        "Depth",
        &format!("{}m", (game.player.y / TILE_SIZE).max(0.0) as i32),
        Color::LIGHTGRAY,
    );
    drop(run_panel);

    if game.show_details {
        let detail_width = available_width.min(620.0);
        let detail_x = origin_x;
        let detail_y = origin_y + 64.0;
        let mut details = ui.compact_panel(Rectangle {
            x: detail_x,
            y: detail_y,
            width: detail_width,
            height: 132.0,
        });
        details.begin_clip();
        details.heading("Diagnostics");
        details.stat_line("FPS", &format!("{fps}"), Color::LIME);
        details.stat_line("Tick", &format!("{}", game.update_ticks), Color::LIGHTGRAY);
        details.stat_line(
            "Camera",
            &format!(
                "{:.0},{:.0}",
                view.camera.x / TILE_SIZE,
                view.camera.y / TILE_SIZE
            ),
            Color::LIGHTGRAY,
        );
        details.muted(&format!("Message: {}", game.message));
    }
}

pub(super) fn draw_minimap_for_view(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
    remote_players: &[crate::session::RenderWorldPlayerPresentation],
) {
    let width = (view.viewport.width / 6).clamp(86, 130);
    let height = (view.viewport.height / 8).clamp(64, 96);
    let x = view.viewport.x + view.viewport.width - width - 18;
    let y = view.viewport.y + view.viewport.height - height - 38;
    draw.draw_rectangle(
        x - 8,
        y - 8,
        width + 16,
        height + 16,
        Color::new(0, 0, 0, 150),
    );

    let terrain_width = game.terrain.width().max(1);
    let terrain_height = game.terrain.height().max(1);
    let player_tile_y = (view.camera.y / TILE_SIZE) as i32;
    let visible_height = terrain_height.min(80);
    let origin_y = if terrain_height > visible_height {
        (player_tile_y - visible_height / 2).clamp(0, terrain_height - visible_height)
    } else {
        0
    };
    let projection = MinimapProjection {
        x,
        y,
        width,
        height,
        terrain_width,
        origin_y,
        visible_height,
    };
    for ty in origin_y..origin_y + visible_height {
        for tx in 0..terrain_width {
            let Some(tile) = game.terrain.tile(TilePosition { x: tx, y: ty }) else {
                continue;
            };
            let color = marker_color(tile.kind);
            if color.a > 0 {
                draw_map_marker(draw, &projection, tx, ty, color);
            }
        }
    }
    let player_x = x + ((view.camera.x / TILE_SIZE) as i32) * width / terrain_width;
    let player_y = y + (((view.camera.y / TILE_SIZE) as i32) - origin_y) * height / visible_height;
    draw.draw_circle(player_x, player_y, 3.0, Color::SKYBLUE);
    for remote in remote_players {
        let remote_tile_x = (remote.x / TILE_SIZE) as i32;
        let remote_tile_y = (remote.y / TILE_SIZE) as i32;
        if remote_tile_y < origin_y || remote_tile_y >= origin_y + visible_height {
            continue;
        }
        let remote_x = x + remote_tile_x * width / terrain_width;
        let remote_y = y + (remote_tile_y - origin_y) * height / visible_height;
        draw.draw_circle(remote_x, remote_y, 3.0, Color::ORANGE);
        if remote.velocity_x.abs() > f32::EPSILON || remote.velocity_y.abs() > f32::EPSILON {
            let speed = (remote
                .velocity_x
                .mul_add(remote.velocity_x, remote.velocity_y * remote.velocity_y))
            .sqrt();
            let direction_x = remote.velocity_x / speed;
            let direction_y = remote.velocity_y / speed;
            draw.draw_line(
                remote_x,
                remote_y,
                remote_x + (direction_x * 8.0) as i32,
                remote_y + (direction_y * 8.0) as i32,
                Color::GOLD,
            );
        }
    }
}

pub(super) fn draw_depth_ruler_for_view(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    view: &ClientView,
) {
    let x = view.viewport.x + view.viewport.width - 20;
    let top = view.viewport.y + 72;
    let height = (view.viewport.height - 130).max(80);
    draw.draw_rectangle(x, top, 10, height, Color::new(0, 0, 0, 120));
    draw.draw_rectangle_lines(x, top, 10, height, Color::new(255, 255, 255, 120));

    let max_depth = game.terrain.height().max(80);
    let step = if max_depth > 180 { 50 } else { 20 };
    for marker in (0..=max_depth).step_by(step as usize) {
        let y = top + (marker * height / max_depth);
        draw.draw_line(x - 8, y, x + 18, y, Color::LIGHTGRAY);
    }

    let depth = (view.camera.y / TILE_SIZE - 5.0).max(0.0);
    let marker_y = top + ((depth / max_depth as f32).clamp(0.0, 1.0) * height as f32) as i32;
    draw.draw_circle(x + 5, marker_y, 6.0, Color::GOLD);
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
    if tile_y < projection.origin_y || tile_y >= projection.origin_y + projection.visible_height {
        return;
    }
    let x = projection.x + tile_x * projection.width / projection.terrain_width;
    let y = projection.y
        + (tile_y - projection.origin_y) * projection.height / projection.visible_height;
    draw.draw_rectangle(x - 1, y - 1, 3, 3, color);
}

pub(super) fn draw_modal(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    modal: ModalScreen,
    hud: Option<PerPlayerHudSnapshot>,
) {
    draw_modal_ui(draw, game, modal, hud);
}

#[allow(
    clippy::too_many_lines,
    reason = "exhaustive modal dispatch is intentionally centralized during UI migration"
)]
fn draw_modal_ui(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    modal: ModalScreen,
    hud: Option<PerPlayerHudSnapshot>,
) {
    match modal {
        ModalScreen::Depot => draw_modal_depot_ui(draw, game),
        ModalScreen::DepotReceiptHistory => draw_depot_receipt_history_ui(draw, game),
        ModalScreen::ShopConfirm => draw_confirm_modal_ui(
            draw,
            "Confirm Upgrade Purchase",
            "Enter/E buys the selected upgrade. Esc returns to the shop.",
        ),
        ModalScreen::Fuel => draw_service_modal_ui(draw, game, hud, "Fuel Station", "fuel"),
        ModalScreen::Repair => draw_service_modal_ui(draw, game, hud, "Repair Garage", "hull"),
        ModalScreen::FuelConfirm => draw_confirm_modal_ui(
            draw,
            "Confirm Fuel Purchase",
            "Enter/E confirms the selected refuel amount. Esc cancels.",
        ),
        ModalScreen::RepairConfirm => draw_confirm_modal_ui(
            draw,
            "Confirm Repair",
            "Enter/E confirms the selected hull repair. Esc cancels.",
        ),
        ModalScreen::ExitConfirm => draw_confirm_modal_ui(
            draw,
            "Exit to Desktop?",
            "Enter/E exits. Esc returns to the game.",
        ),
        ModalScreen::UnsavedExitConfirm => draw_unsaved_exit_confirm_ui(draw, game),
        ModalScreen::Shop => draw_shop_ui(draw, game),
        ModalScreen::Options => draw_options_ui(draw, game),
        ModalScreen::SaveSlots => draw_slots_ui(draw, game, true),
        ModalScreen::LoadSlots => draw_slots_ui(draw, game, false),
        ModalScreen::Bank => draw_generic_options_ui(
            draw,
            game,
            "Bank",
            "Manage deposits and debt. Esc closes.",
            &["Deposit credits", "Withdraw credits", "Pay debt"],
        ),
        ModalScreen::Explosives => draw_generic_options_ui(
            draw,
            game,
            "Explosives Shack",
            "Purchase mining explosives and blast tools. Esc closes.",
            &[
                "Small bomb pack",
                "Standard bomb pack",
                "Heavy blast charge",
                "Safety permit",
            ],
        ),
        ModalScreen::Salvage => draw_generic_options_ui(
            draw,
            game,
            "Salvage Yard",
            "Buy and deploy field infrastructure. Esc closes.",
            &[
                "Signal relay kit",
                "Survey drone kit",
                "Cargo lift kit",
                "Rock support kit",
                "Pump kit",
                "Processor kit",
            ],
        ),
        ModalScreen::Headquarters => draw_generic_options_ui(
            draw,
            game,
            "Headquarters",
            "Contracts, story briefings, finance, and expedition planning.",
            &[
                "Complete depot work",
                "Read briefing",
                "Request finance",
                "Open expedition board",
                "Research log",
                "Town development",
                "Deep claim status",
            ],
        ),
        ModalScreen::Crafting => draw_generic_options_ui(
            draw,
            game,
            "Crafting Bench",
            "Turn recovered materials into support equipment.",
            &[
                "Relay parts",
                "Drone parts",
                "Lift frame",
                "Support brace",
                "Pump assembly",
                "Processor assembly",
            ],
        ),
        ModalScreen::TownDevelopment => draw_generic_options_ui(
            draw,
            game,
            "Town Development",
            "Invest in settlement upgrades and local services.",
            &[
                "Depot expansion",
                "Fuel cooperative",
                "Repair garage",
                "Research hall",
                "Residential blocks",
                "Trade office",
            ],
        ),
        ModalScreen::ExpeditionBoard => draw_generic_options_ui(
            draw,
            game,
            "Expedition Board",
            "Review active expeditions and available claims.",
            &[
                "Accept first offer",
                "Accept second offer",
                "Review active expedition",
                "Collect completed expedition",
            ],
        ),
        ModalScreen::ResearchLog => draw_research_log_ui(draw, game),
        ModalScreen::Inventory => draw_inventory_ui(draw, game),
        ModalScreen::OnlineMultiplayer => draw_online_multiplayer_ui(draw, game),
        ModalScreen::Map => draw_map_ui(draw, game),
        ModalScreen::Help => draw_help_ui(draw),
    }
}

fn draw_service_modal_ui(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    hud: Option<PerPlayerHudSnapshot>,
    title: &str,
    service: &str,
) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(680, 420));
    panel.title(title);
    panel.muted("Up/Down: small/half/full | Enter/E buy selected | Esc close");
    panel.separator();
    if service == "fuel" {
        let fuel = hud.map_or(game.player.fuel, |hud| hud.fuel);
        let capacity = hud.map_or(game.player.fuel_capacity, |hud| hud.fuel_capacity);
        let credits = hud.map_or(game.player.credits, |hud| hud.credits);
        let missing = (capacity - fuel).ceil().max(0.0) as u32;
        panel.label(&format!(
            "Tank: {fuel:.0}/{capacity:.0}. Fill cost: {missing} cr. Credits: {credits}."
        ));
    } else {
        let hull = hud.map_or(game.player.hull, |hud| hud.hull);
        let capacity = hud.map_or_else(|| game.player.max_hull(), |hud| hud.max_hull);
        let credits = hud.map_or(game.player.credits, |hud| hud.credits);
        panel.label(&format!(
            "Hull: {hull:.0}/{capacity:.0}. Credits available: {credits}."
        ));
    }
    for (index, option) in ["Small 25%", "Half 50%", "Full 100%"].iter().enumerate() {
        panel.option(index == game.selected_menu_item, option, None);
    }
}

fn draw_confirm_modal_ui(draw: &mut RaylibDrawHandle<'_>, title: &str, body: &str) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(600, 260));
    panel.title(title);
    panel.separator();
    panel.label(body);
}

fn draw_unsaved_exit_confirm_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(620, 340));
    panel.title("Unsaved Progress");
    panel.muted("Choose what to do before leaving the game.");
    panel.separator();
    for (index, option) in ["Save and exit", "Exit without saving", "Cancel"]
        .iter()
        .enumerate()
    {
        panel.option(index == game.selected_menu_item, option, None);
    }
}

fn draw_shop_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(820, 540));
    panel.begin_clip();
    panel.title("Upgrade Shop");
    panel.muted("Up/Down select | Enter/E buy | Esc close");
    panel.separator();
    for (index, offer) in upgrade_offers(&game.player).iter().enumerate() {
        panel.option(
            index == game.selected_menu_item,
            &format!("{} — {} cr", offer.name, offer.cost),
            Some(&format!(
                "Tier {} {}",
                offer.level + 1,
                upgrade_tier_name(offer.kind, offer.level)
            )),
        );
    }
}

fn draw_options_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(640, 360));
    panel.title("Options");
    panel.muted("Up/Down select | Left/Right adjust | Esc close");
    panel.separator();
    let options = [
        format!("Master volume: {:.0}%", game.master_volume * 100.0),
        format!("Fullscreen: {}", if game.fullscreen { "on" } else { "off" }),
        "Back".to_owned(),
    ];
    for (index, option) in options.iter().enumerate() {
        panel.option(index == game.selected_menu_item, option, None);
    }
}

fn draw_slots_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState, saving: bool) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(760, 480));
    panel.begin_clip();
    panel.title(if saving { "Save Game" } else { "Load Game" });
    panel.muted("Up/Down select | Enter/E confirm | Esc close");
    panel.separator();
    for slot in 0..save_slot_count() {
        let label = match save_slot_metadata(slot) {
            Some(metadata) => format!(
                "Slot {} — depth {}m, {} credits",
                slot + 1,
                metadata.depth,
                metadata.credits
            ),
            None => format!("Slot {} — empty", slot + 1),
        };
        panel.option(slot == game.selected_menu_item, &label, None);
    }
}

fn draw_generic_options_ui(
    draw: &mut RaylibDrawHandle<'_>,
    game: &GameState,
    title: &str,
    help: &str,
    options: &[&str],
) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(740, 500));
    panel.begin_clip();
    panel.title(title);
    panel.muted(help);
    panel.separator();
    for (index, option) in options.iter().enumerate() {
        panel.option(index == game.selected_menu_item, option, None);
    }
}
fn draw_depot_receipt_history_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(780, 520));
    panel.begin_clip();
    panel.title("Depot Receipts");
    panel.muted("Recent sales and contract payouts. Esc closes.");
    panel.separator();
    if game.depot_receipts.is_empty() {
        panel.label("No depot receipts yet.");
    } else {
        for (index, receipt) in game.depot_receipts.iter().rev().take(10).enumerate() {
            panel.heading(&format!("Receipt {}", index + 1));
            for line in receipt.lines().take(5) {
                panel.muted(line);
            }
            panel.separator();
        }
    }
}

fn draw_research_log_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(820, 540));
    panel.begin_clip();
    panel.title("Research Log");
    panel.muted("Collected discoveries and mining intelligence. Esc closes.");
    panel.separator();
    panel.label(&format!(
        "Warnings: {} | World events: {}",
        game.warning_summary(),
        game.active_world_event_summary()
    ));
    panel.label(&format!(
        "Deep instability: {:.0}% | deepest reached: {}m",
        game.deep_instability.min(100.0),
        game.deepest_tile_reached
    ));
    panel.separator();
    panel.muted("Use the collection log and expedition board for detailed objectives while the UI migration continues to consolidate data models.");
}

#[allow(
    clippy::too_many_lines,
    reason = "inventory modal summarizes cargo, kits, and rig state"
)]
fn draw_inventory_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(880, 560));
    panel.begin_clip();
    panel.title("Inventory");
    panel.muted("Tab/Esc/Backspace closes | cargo, artifacts, consumables, and field kits");
    panel.separator();
    panel.progress_bar(
        "Cargo",
        game.player.cargo_used() as f32,
        game.player.cargo_capacity as f32,
        Color::GOLD,
        Color::ORANGE,
    );
    panel.stat_line(
        "Credits",
        &format!("{} cr", game.player.credits),
        Color::GOLD,
    );
    panel.stat_line("Bombs", &format!("{}", game.player.bombs), Color::ORANGE);
    panel.separator();
    panel.heading("Minerals");
    if game.player.cargo.is_empty() {
        panel.muted("No minerals onboard.");
    } else {
        for (mineral, quantity) in &game.player.cargo {
            let value = game
                .mineral_market_value(*mineral)
                .saturating_mul(*quantity);
            panel.stat_line(
                mineral.name(),
                &format!("x{quantity} | est. {value} cr"),
                Color::RAYWHITE,
            );
        }
    }
    panel.separator();
    panel.heading("Artifacts");
    if game.player.artifacts.is_empty() {
        panel.muted("No artifacts onboard.");
    } else {
        for (artifact, quantity) in &game.player.artifacts {
            panel.stat_line(
                artifact.name(),
                &format!(
                    "x{quantity} | base {} cr",
                    artifact.value().saturating_mul(*quantity)
                ),
                Color::MAGENTA,
            );
        }
    }
    panel.separator();
    panel.heading("Field Kits");
    for (label, count) in [
        ("Signal relays", game.player.signal_relay_kits),
        ("Survey drones", game.player.survey_drone_kits),
        ("Cargo lifts", game.player.cargo_lift_kits),
        ("Tunnel supports", game.player.tunnel_support_kits),
        ("Pump stations", game.player.pump_station_kits),
        ("Ore processors", game.player.ore_processor_kits),
    ] {
        panel.stat_line(
            label,
            &format!("{count}"),
            if count > 0 { Color::LIME } else { Color::GRAY },
        );
    }
    panel.separator();
    panel.heading("Rig");
    panel.stat_line(
        "Drill",
        &format!("tier {}", game.player.drill_strength),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Engine",
        &format!("tier {}", game.player.engine_level),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Fuel Tank",
        &format!("tier {}", game.player.fuel_tank_level),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Cargo Bay",
        &format!("tier {}", game.player.cargo_bay_level),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Hull",
        &format!("tier {}", game.player.hull_level),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Radiator",
        &format!("tier {}", game.player.radiator_level),
        Color::LIGHTGRAY,
    );
    panel.stat_line(
        "Scanner",
        &format!("tier {}", game.player.scanner_level),
        Color::LIGHTGRAY,
    );
}

fn draw_online_multiplayer_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(1040, 620));
    panel.begin_clip();
    panel.title("Online Multiplayer");
    panel.muted("Direct-connect setup for two running game windows. Host writes a descriptor; client joins with that file.");
    panel.separator();
    let options = [
        "Host descriptor session",
        "Join descriptor session",
        "Reconnect",
        "Descriptor path",
        "Inspect descriptor file",
        "Host bind addr",
        "Host advertise addr",
        "Client bind addr",
        "Cycle gameplay tick count",
        "Simulate timeout",
        "Show error",
        "Shutdown session",
        "Toggle ready",
        "Start online gameplay",
        "Back",
    ];
    for (index, option) in options.iter().enumerate() {
        panel.option(
            index == game.selected_menu_item,
            option,
            Some(online_selected_action_help(game, index)),
        );
    }
    panel.separator();
    panel.heading("Connection");
    panel.muted(&format!(
        "Descriptor: {}",
        game.online_descriptor_path.display()
    ));
    panel.muted(&format!("Host bind: {}", game.online_host_bind_addr));
    panel.muted(&format!(
        "Host advertise: {}",
        game.online_host_advertise_addr
    ));
    panel.muted(&format!("Client bind: {}", game.online_client_bind_addr));
    let lobby = game.online_lobby_presentation();
    panel.separator();
    panel.heading("Lobby");
    panel.muted(&online_peer_summary("Local", &lobby.local));
    panel.muted(&online_peer_summary("Remote", &lobby.remote));
    panel.muted(&lobby.guidance);
}

fn draw_map_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(980, 610));
    panel.title("Mine Map");
    panel.muted("M/Esc/Backspace closes | discovered terrain only");
    panel.separator();
    panel.label(&format!(
        "Position: {:.0}m lateral, depth {}m | Deepest reached: {}m",
        game.player.x / TILE_SIZE,
        (game.player.y / TILE_SIZE).max(0.0) as i32,
        game.deepest_tile_reached
    ));
    drop(panel);

    let map_x = 210;
    let map_y = 245;
    let map_w = 860;
    let map_h = 300;
    draw.draw_rectangle(map_x, map_y, map_w, map_h, Color::new(12, 10, 14, 255));
    draw.draw_rectangle_lines(map_x, map_y, map_w, map_h, Color::new(190, 205, 220, 230));
    let terrain_width = game.terrain.width().max(1);
    let terrain_height = game.terrain.height().max(1);
    for ty in 0..terrain_height {
        for tx in 0..terrain_width {
            let position = TilePosition { x: tx, y: ty };
            if !game.is_explored(position) {
                continue;
            }
            let Some(tile) = game.terrain.tile(position) else {
                continue;
            };
            let color = match tile.kind {
                TileKind::Air => Color::new(40, 42, 55, 255),
                TileKind::Foundation => Color::new(135, 125, 105, 255),
                TileKind::Lava | TileKind::MagmaVent => Color::RED,
                TileKind::Gas => Color::GREEN,
                TileKind::ExplosivePocket => Color::ORANGE,
                TileKind::PressurePocket => Color::SKYBLUE,
                TileKind::Ore(_) => Color::GOLD,
                TileKind::Artifact(_) => Color::MAGENTA,
                _ => Color::new(115, 82, 58, 255),
            };
            let px = map_x + tx * map_w / terrain_width;
            let py = map_y + ty * map_h / terrain_height;
            let pw = ((tx + 1) * map_w / terrain_width - tx * map_w / terrain_width).max(1);
            let ph = ((ty + 1) * map_h / terrain_height - ty * map_h / terrain_height).max(1);
            draw.draw_rectangle(px, py, pw, ph, color);
        }
    }
    let player_map_x = map_x + ((game.player.x / TILE_SIZE) as i32) * map_w / terrain_width;
    let player_map_y = map_y + ((game.player.y / TILE_SIZE) as i32) * map_h / terrain_height;
    draw.draw_circle(player_map_x, player_map_y, 5.0, Color::SKYBLUE);
    draw.draw_circle_lines(player_map_x, player_map_y, 8.0, Color::RAYWHITE);
}

fn draw_help_ui(draw: &mut RaylibDrawHandle<'_>) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(760, 500));
    panel.begin_clip();
    panel.title("Controls");
    panel.muted("Esc/Backspace closes. Controls are context-sensitive.");
    panel.separator();
    for line in [
        "Move: A/D or Left/Right",
        "Thrust: W or Up",
        "Drill down: S or Down",
        "Interact/confirm: E or Enter",
        "Bomb: B | Scanner: C | Map: M",
        "Infrastructure: R/T/L/U/O/P depending on available kits",
        "Pause/back: Esc",
        "Fullscreen: F11",
    ] {
        panel.label(line);
    }
}

fn online_peer_summary(
    label: &str,
    peer: &crate::game_state::OnlinePeerLobbyPresentation,
) -> String {
    let slot = peer
        .slot
        .map_or_else(|| "unassigned".to_owned(), |slot| slot.to_string());
    format!(
        "{label}: {} | role={} | slot={} | ready={} | connected={} | save={}",
        peer.name,
        peer.role_label,
        slot,
        if peer.ready { "yes" } else { "no" },
        if peer.connected { "yes" } else { "no" },
        online_save_authority_label(peer.save_authority)
    )
}

const fn online_save_authority_label(authority: OnlineSaveAuthority) -> &'static str {
    match authority {
        OnlineSaveAuthority::LocalPlayer => "local",
        OnlineSaveAuthority::RemoteHost => "remote host",
    }
}

const fn online_selected_action_help(game: &GameState, selected: usize) -> &'static str {
    match selected {
        0 => "Host: write descriptor, keep this window open, then wait for the other player.",
        1 => "Join: point at the host's descriptor file, then connect as the joined client.",
        2 => "Reconnect: retry with the previous session token after a disconnect.",
        3 => "Descriptor path: choose the JSON file App A shares with App B.",
        4 => "Inspect descriptor: verify host address and session metadata before joining.",
        5 => "Host bind: local socket address the host listens on.",
        6 => "Host advertise: address written into the descriptor for the client.",
        7 => "Client bind: local socket address the joined client uses.",
        8 => "Gameplay ticks: length for command-line smoke gameplay tasks.",
        12 => "Ready: toggle this player ready once the remote player is connected.",
        13 => {
            if game.online_host_owns_save {
                "Start: host enters gameplay and sends the authoritative start signal."
            } else {
                "Start: clients wait here; only the host can begin the authoritative session."
            }
        }
        14 => "Back: return to the previous menu without changing the current session.",
        _ => "This action is for diagnostics or session lifecycle control.",
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "depot panel combines sales, contracts, and receipt previews"
)]
fn draw_modal_depot_ui(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(920, 560));
    panel.begin_clip();
    panel.title("Ore Depot");
    panel.muted("Up/Down select | Enter/E confirm | Esc/Backspace close");
    panel.separator();

    let options = [
        (
            "Complete active contract",
            format!(
                "{}: {}/{} {} for {} credits",
                game.contracts.active.title,
                game.contracts.active.progress(&game.player),
                game.contracts.active.required,
                game.contracts.active.target.name(),
                game.contracts.active.reward
            ),
        ),
        (
            "Sell loose cargo",
            format!(
                "Cargo manifest: {}/{} slots",
                game.player.cargo_used(),
                game.player.cargo_capacity
            ),
        ),
        (
            "Auto-sort low-grade cargo",
            "Keep valuable ores while clearing cheap cargo from the hold.".to_owned(),
        ),
        (
            "Sell scan data",
            format!("Market: {}", game.active_world_event_summary()),
        ),
        (
            "Receipt history",
            format!("{} saved receipts", game.depot_receipts.len()),
        ),
    ];
    for (index, (label, detail)) in options.iter().enumerate() {
        panel.option(index == game.selected_menu_item, label, Some(detail));
    }

    panel.separator();
    panel.heading("Current cargo");
    if game.player.cargo_used() == 0 {
        panel.muted("Cargo hold empty");
    } else {
        for (mineral, count) in &game.player.cargo {
            panel.label(&format!(
                "{} x{} = {} cr",
                mineral.name(),
                count,
                mineral.value() * count
            ));
        }
        for (artifact, count) in &game.player.artifacts {
            panel.label(&format!(
                "{} x{} = {} cr",
                artifact.name(),
                count,
                artifact.value() * count
            ));
        }
    }

    panel.separator();
    panel.heading("Market snapshot");
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
    for mineral in minerals {
        let current = game.mineral_market_factor(mineral);
        let previous = game
            .previous_mineral_market_factor(mineral)
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
        panel.muted(&format!(
            "{}: {} cr {trend} {label}",
            mineral.name(),
            game.mineral_market_value(mineral)
        ));
    }

    if !game.last_depot_receipt.is_empty() {
        panel.separator();
        panel.heading("Last receipt");
        for line in game.last_depot_receipt.lines().take(6) {
            panel.muted(line);
        }
    }
}

pub(super) fn draw_title(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(620, 520));
    panel.title("DRILLGAME");
    panel.muted("A frontier mining run awaits below.");
    panel.separator();
    let options = GameState::title_options();
    for (index, option) in options.iter().enumerate() {
        panel.option(index == game.selected_title_item, option.label(), None);
    }
    if options.contains(&TitleOption::Resume)
        && let Some(meta) = latest_save_summary()
    {
        panel.separator();
        panel.muted(&format!(
            "Last save: depth {}m | {} cr | {:.0} min",
            meta.depth,
            meta.credits,
            (meta.play_seconds / 60.0).floor()
        ));
    }
    panel.separator();
    panel.muted("Up/Down select | Enter/E confirm | Esc exits | F11 fullscreen");
}

pub(super) fn draw_pause(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(560, 430));
    panel.title("Paused");
    panel.muted("Up/Down select | Enter/E confirm | Esc resume");
    panel.separator();
    for (index, option) in PauseOption::ALL.iter().enumerate() {
        panel.option(index == game.selected_pause_item, option.label(), None);
    }
    panel.separator();
    panel.muted(&format!(
        "Depth {}m | {} credits | hull {:.0}/{:.0} | fuel {:.0}/{:.0}",
        (game.player.y / TILE_SIZE).max(0.0) as i32,
        game.player.credits,
        game.player.hull,
        game.player.max_hull(),
        game.player.fuel,
        game.player.fuel_capacity
    ));
}

pub(super) fn draw_ending(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(680, 430));
    panel.title("Star Core Secured");
    panel.muted("Run summary");
    panel.separator();
    panel.label(&format!("Deepest depth: {}m", game.deepest_tile_reached));
    panel.label(&format!("Total earnings: {} cr", game.total_earnings));
    panel.label(&format!("Rescues called: {}", game.rescue_count));
    panel.label(&format!(
        "Contracts completed: {}",
        game.contracts.completed
    ));
    panel.separator();
    panel.muted("You can keep mining this save after closing the summary.");
}

pub(super) fn draw_game_over(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
    let mut ui = UiContext::new(draw);
    ui.draw_dimmed_backdrop();
    let mut panel = ui.panel(modal_rect(620, 320));
    panel.title("Emergency");
    panel.label(&game.message);
    panel.separator();
    panel.muted("Press E to pay the rescue fee and return to base.");
}
