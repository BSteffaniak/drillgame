use raylib::prelude::*;

use super::SCREEN_WIDTH;
use crate::{
    economy::SurfaceZone,
    game_state::{GameState, ServiceAnimation},
};

pub(super) fn draw_interior(draw: &mut RaylibDrawHandle<'_>, game: &GameState) {
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
