use raylib::prelude::*;

#[allow(
    clippy::struct_excessive_bools,
    reason = "input snapshots are simple edge/level booleans"
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    pub horizontal: f32,
    pub thrust: bool,
    pub drill_down: bool,
    pub interact: bool,
    pub confirm: bool,
    pub cancel: bool,
    pub pause: bool,
    pub menu_up: bool,
    pub menu_down: bool,
    pub details: bool,
    pub save: bool,
    pub load: bool,
    pub selected_upgrade: Option<usize>,
    pub map: bool,
    pub help: bool,
}

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
        interact: raylib.is_key_pressed(KeyboardKey::KEY_E),
        confirm: raylib.is_key_pressed(KeyboardKey::KEY_ENTER)
            || raylib.is_key_pressed(KeyboardKey::KEY_E),
        cancel: raylib.is_key_pressed(KeyboardKey::KEY_BACKSPACE)
            || raylib.is_key_pressed(KeyboardKey::KEY_ESCAPE),
        pause: raylib.is_key_pressed(KeyboardKey::KEY_P),
        menu_up: raylib.is_key_pressed(KeyboardKey::KEY_UP)
            || raylib.is_key_pressed(KeyboardKey::KEY_W),
        menu_down: raylib.is_key_pressed(KeyboardKey::KEY_DOWN)
            || raylib.is_key_pressed(KeyboardKey::KEY_S),
        details: raylib.is_key_down(KeyboardKey::KEY_TAB),
        save: raylib.is_key_pressed(KeyboardKey::KEY_F5),
        load: raylib.is_key_pressed(KeyboardKey::KEY_F9),
        selected_upgrade: selected_upgrade(raylib),
        map: raylib.is_key_pressed(KeyboardKey::KEY_M),
        help: raylib.is_key_pressed(KeyboardKey::KEY_H),
    }
}

fn selected_upgrade(raylib: &RaylibHandle) -> Option<usize> {
    if raylib.is_key_pressed(KeyboardKey::KEY_ONE) {
        Some(0)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_TWO) {
        Some(1)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_THREE) {
        Some(2)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_FOUR) {
        Some(3)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_FIVE) {
        Some(4)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_SIX) {
        Some(5)
    } else {
        None
    }
}

const fn horizontal_axis(left: bool, right: bool) -> f32 {
    match (left, right) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        _ => 0.0,
    }
}
