use crate::{game_state::GameState, input::read_input, rendering::render};

const WINDOW_WIDTH: i32 = 1280;
const WINDOW_HEIGHT: i32 = 720;
const TARGET_FPS: u32 = 60;

pub fn run() {
    let (mut raylib, thread) = raylib::init()
        .size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .title("Drillgame")
        .build();

    raylib.set_target_fps(TARGET_FPS);
    raylib.set_exit_key(None);

    let mut game = GameState::new();

    while !raylib.window_should_close() && !game.request_exit {
        let delta_seconds = raylib.get_frame_time();
        let input = read_input(&raylib);

        game.update(input, delta_seconds);

        let mut draw = raylib.begin_drawing(&thread);
        render(&mut draw, &game);
    }
}
