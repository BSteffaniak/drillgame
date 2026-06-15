#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "rendering APIs use integer pixels while camera math uses floats"
)]

use raylib::{ffi, prelude::*};

mod interior;
mod screen;
mod terrain;
mod world;

use interior::draw_interior;
use screen::{
    draw_depth_ruler_for_view, draw_ending, draw_game_over, draw_heat_warning, draw_hud_for_view,
    draw_minimap_for_view, draw_modal, draw_pause, draw_title,
};
use terrain::TerrainRenderer;
pub use world::render_camera;
use world::{
    draw_particles, draw_placed_bombs, draw_player, draw_scanner_marks, draw_world, world_camera,
};

use crate::{
    game_state::{GameState, RunMode},
    session::{ClientView, SplitScreenLayout, WorldDelta, WorldEvent, split_screen_layout},
};

const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 720;

pub struct GameRenderer {
    terrain: TerrainRenderer,
}

impl GameRenderer {
    #[must_use]
    pub const fn split_screen_layout(client_count: usize) -> SplitScreenLayout {
        split_screen_layout(client_count)
    }

    pub fn new(raylib: &mut RaylibHandle, thread: &RaylibThread, game: &GameState) -> Self {
        Self {
            terrain: TerrainRenderer::new(raylib, thread, game),
        }
    }

    pub fn sync_delta(
        &mut self,
        raylib: &mut RaylibHandle,
        thread: &RaylibThread,
        game: &GameState,
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
                WorldEvent::TickAdvanced { .. }
                | WorldEvent::CommandsProcessed { .. }
                | WorldEvent::TerrainChunksChanged { .. }
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
        self.terrain.sync(raylib, thread, game);
    }

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
            self.render_client_view(draw, game, view);
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
    ) {
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
        if view.run_mode != RunMode::Interior {
            draw_heat_warning(draw, game);
        }
        draw_hud_for_view(draw, game, view);
        if view.run_mode != RunMode::Interior {
            draw_depth_ruler_for_view(draw, game, view);
            draw_minimap_for_view(draw, game, view);
        }

        if view.run_mode == RunMode::Title {
            draw_title(draw, game);
        } else if view.run_mode == RunMode::Paused {
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
