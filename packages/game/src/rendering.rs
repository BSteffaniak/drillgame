#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::{ffi, prelude::*};

mod interior;
mod layout;
mod screen;
mod terrain;
mod world;

use interior::draw_interior;
use screen::{
    draw_depth_ruler_for_view, draw_ending, draw_game_over, draw_heat_warning_for_view,
    draw_hud_for_view, draw_hud_snapshot_for_view, draw_minimap_for_view, draw_modal, draw_pause,
    draw_title,
};
use terrain::TerrainRenderer;
pub use world::render_camera;
use world::{
    draw_particles, draw_placed_bombs, draw_player, draw_scanner_marks, draw_world, world_camera,
};

use crate::{
    game_state::{GameState, RunMode},
    session::{
        ClientView, LiveRenderFrameOutput, RenderViewportPlan, SplitScreenLayout, WorldDelta,
        WorldEvent, split_screen_layout,
    },
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;

pub struct GameRenderer {
    terrain: TerrainRenderer,
    ui_fonts: layout::UiFonts,
}

fn centered_camera_for_player(
    game: &GameState,
    player_x: f32,
    player_y: f32,
    viewport: crate::session::Viewport,
) -> Vector2 {
    let screen_width = viewport.width.max(1) as f32;
    let screen_height = viewport.height.max(1) as f32;
    let max_x = game.terrain.width() as f32 * crate::game_state::TILE_SIZE - screen_width;
    let max_y = game.terrain.height() as f32 * crate::game_state::TILE_SIZE - screen_height;
    Vector2::new(
        (player_x - screen_width / 2.0).clamp(0.0, max_x.max(0.0)),
        (player_y - screen_height / 2.0).clamp(-12.0 * crate::game_state::TILE_SIZE, max_y),
    )
}

fn draw_online_remote_players_from_legacy_snapshots(
    draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>,
    game: &GameState,
) {
    for remote in game.online_remote_world_presentations() {
        world::draw_remote_player(draw, &remote.as_render_player());
    }
}

fn draw_local_player_override(
    draw: &mut RaylibMode2D<'_, RaylibDrawHandle<'_>>,
    game: &GameState,
    world_players: &[crate::session::RenderWorldPlayerPresentation],
) {
    if let Some(player) = world_players.iter().find(|player| player.local_to_view) {
        world::draw_player_at(draw, game, player.x, player.y);
    } else {
        draw_player(draw, game);
    }
    for player in world_players.iter().filter(|player| !player.local_to_view) {
        world::draw_remote_player(draw, player);
    }
    if world_players.is_empty() {
        draw_online_remote_players_from_legacy_snapshots(draw, game);
    }
}

impl GameRenderer {
    #[must_use]
    #[allow(
        dead_code,
        reason = "legacy compatibility renderer path kept during live render output migration"
    )]
    pub const fn split_screen_layout(client_count: usize) -> SplitScreenLayout {
        split_screen_layout(client_count)
    }

    pub fn new(raylib: &mut RaylibHandle, thread: &RaylibThread, game: &GameState) -> Self {
        Self {
            terrain: TerrainRenderer::new(raylib, thread, game),
            ui_fonts: layout::UiFonts::raylib_default(),
        }
    }

    pub fn sync_delta(
        &mut self,
        raylib: &mut RaylibHandle,
        thread: &RaylibThread,
        game: &mut GameState,
        delta: &WorldDelta,
    ) {
        for event in &delta.events {
            match event {
                WorldEvent::TerrainRefreshRequested => self.terrain.mark_all_dirty(),
                WorldEvent::TerrainTilesChanged { positions } => {
                    for tile in positions {
                        self.terrain.mark_tile_dirty(*tile);
                    }
                }
                WorldEvent::TerrainChunksChanged { revisions } => {
                    for revision in revisions {
                        self.terrain
                            .mark_chunk_dirty(revision.position.x, revision.position.y);
                    }
                }
                WorldEvent::TickAdvanced { .. }
                | WorldEvent::CommandsProcessed { .. }
                | WorldEvent::SnapshotKeyframeReady { .. }
                | WorldEvent::MessageChanged { .. }
                | WorldEvent::PlayerChanged { .. }
                | WorldEvent::CargoChanged { .. }
                | WorldEvent::PlayerDamaged { .. }
                | WorldEvent::DrillProgressed { .. }
                | WorldEvent::PurchaseCompleted { .. }
                | WorldEvent::RescueTriggered { .. }
                | WorldEvent::PlayerSurvivalChanged { .. }
                | WorldEvent::BombPlaced { .. }
                | WorldEvent::HazardChanged
                | WorldEvent::ImportantEffectTriggered
                | WorldEvent::ClientExitRequested { .. }
                | WorldEvent::ClientSettingsChanged { .. } => {}
            }
        }
        let visual_changes = game.drain_visual_changes();
        if visual_changes.full_terrain_refresh {
            self.terrain.mark_all_dirty();
        }
        for tile in visual_changes.changed_tiles {
            self.terrain.mark_tile_dirty(tile);
        }
        self.terrain.sync(raylib, thread, game);
    }

    #[allow(
        dead_code,
        reason = "legacy compatibility renderer path kept during live render output migration"
    )]
    pub fn render_client_views(
        &self,
        draw: &mut RaylibDrawHandle<'_>,
        game: &GameState,
        views: &[&ClientView],
    ) {
        let _layout = Self::split_screen_layout(views.len());
        for view in views {
            let viewport = view.viewport;
            unsafe {
                ffi::BeginScissorMode(viewport.x, viewport.y, viewport.width, viewport.height);
            }
            self.render_client_view(draw, game, view, &[], None);
            unsafe {
                ffi::EndScissorMode();
            }
        }
    }

    pub fn render_live_frame_output(
        &self,
        draw: &mut RaylibDrawHandle<'_>,
        game: &GameState,
        output: &LiveRenderFrameOutput,
    ) {
        for plan in &output.viewport_plans {
            let world_players = output
                .world_players_by_view
                .iter()
                .find_map(|(client_id, players)| {
                    (*client_id == plan.client_id).then_some(players.as_slice())
                })
                .unwrap_or(&[]);
            let hud = output
                .hud_snapshots
                .iter()
                .find(|hud| {
                    plan.local_player
                        .is_some_and(|player| player.player_id == hud.player_id)
                })
                .copied();
            self.render_viewport_plan(draw, game, plan, world_players, hud);
        }
    }

    pub fn render_viewport_plan(
        &self,
        draw: &mut RaylibDrawHandle<'_>,
        game: &GameState,
        plan: &RenderViewportPlan,
        world_players: &[crate::session::RenderWorldPlayerPresentation],
        hud: Option<crate::session::PerPlayerHudSnapshot>,
    ) {
        if plan.clip_enabled {
            let viewport = plan.viewport;
            unsafe {
                ffi::BeginScissorMode(viewport.x, viewport.y, viewport.width, viewport.height);
            }
        }
        let view = ClientView {
            client_id: plan.client_id,
            controlled_player_id: plan
                .local_player
                .map_or(crate::multiplayer::LOCAL_PLAYER_ID, |player| {
                    player.player_id
                }),
            camera: plan.local_player.map_or_else(
                || world::render_camera(game),
                |player| centered_camera_for_player(game, player.x, player.y, plan.viewport),
            ),
            viewport: plan.viewport,
            run_mode: game.run_mode,
        };
        self.render_client_view(draw, game, &view, world_players, hud);
        if plan.clip_enabled {
            unsafe {
                ffi::EndScissorMode();
            }
        }
    }

    pub fn render_client_view(
        &self,
        draw: &mut RaylibDrawHandle<'_>,
        game: &GameState,
        view: &ClientView,
        world_players: &[crate::session::RenderWorldPlayerPresentation],
        hud: Option<crate::session::PerPlayerHudSnapshot>,
    ) {
        layout::set_current_fonts(self.ui_fonts);
        draw.clear_background(Color::new(105, 190, 235, 255));

        let camera = view.camera;

        if view.run_mode == RunMode::Interior {
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

            draw_local_player_override(&mut world_draw, game, world_players);
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
        if view.run_mode != RunMode::Interior {
            draw_heat_warning_for_view(draw, game, view);
        }
        if let Some(hud) = hud {
            draw_hud_snapshot_for_view(draw, game, view, hud);
        } else {
            draw_hud_for_view(draw, game, view);
        }
        if view.run_mode != RunMode::Interior {
            draw_depth_ruler_for_view(draw, game, view);
            let legacy_remote_players;
            let minimap_remote_players = if world_players.is_empty() {
                legacy_remote_players = game
                    .online_remote_world_presentations()
                    .into_iter()
                    .map(|remote| remote.as_render_player())
                    .collect::<Vec<_>>();
                legacy_remote_players.as_slice()
            } else {
                world_players
            };
            draw_minimap_for_view(draw, game, view, minimap_remote_players);
        }

        if view.run_mode == RunMode::Title {
            draw_title(draw, game);
        } else if view.run_mode == RunMode::Paused {
            draw_pause(draw, game);
        }

        if let Some(modal) = game.modal {
            draw_modal(draw, game, modal, hud);
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
