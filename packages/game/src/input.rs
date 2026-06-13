#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    pub horizontal: f32,
    pub thrust: bool,
    pub drill_down: bool,
}

use raylib::prelude::*;

#[must_use]
pub fn read_input(raylib: &RaylibHandle) -> PlayerInput {
    let left = raylib.is_key_down(KeyboardKey::KEY_A) || raylib.is_key_down(KeyboardKey::KEY_LEFT);
    let right =
        raylib.is_key_down(KeyboardKey::KEY_D) || raylib.is_key_down(KeyboardKey::KEY_RIGHT);
    let up = raylib.is_key_down(KeyboardKey::KEY_W)
        || raylib.is_key_down(KeyboardKey::KEY_UP)
        || raylib.is_key_down(KeyboardKey::KEY_SPACE);
    let down = raylib.is_key_down(KeyboardKey::KEY_S) || raylib.is_key_down(KeyboardKey::KEY_DOWN);

    PlayerInput {
        horizontal: horizontal_axis(left, right),
        thrust: up,
        drill_down: down,
    }
}

const fn horizontal_axis(left: bool, right: bool) -> f32 {
    match (left, right) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        _ => 0.0,
    }
}
