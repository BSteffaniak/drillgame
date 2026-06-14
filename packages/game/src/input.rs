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
    pub menu_left: bool,
    pub menu_right: bool,
    pub details: bool,
    pub save: bool,
    pub load: bool,
    pub selected_upgrade: Option<usize>,
    pub map: bool,
    pub help: bool,
    pub volume_up: bool,
    pub volume_down: bool,
    pub fullscreen: bool,
    pub bomb: bool,
    pub scan: bool,
    pub place_relay: bool,
    pub place_drone: bool,
    pub place_lift: bool,
    pub place_support: bool,
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
        menu_left: raylib.is_key_pressed(KeyboardKey::KEY_LEFT)
            || raylib.is_key_pressed(KeyboardKey::KEY_A),
        menu_right: raylib.is_key_pressed(KeyboardKey::KEY_RIGHT)
            || raylib.is_key_pressed(KeyboardKey::KEY_D),
        details: raylib.is_key_down(KeyboardKey::KEY_TAB),
        save: raylib.is_key_pressed(KeyboardKey::KEY_F5),
        load: raylib.is_key_pressed(KeyboardKey::KEY_F9),
        selected_upgrade: selected_upgrade(raylib),
        map: raylib.is_key_pressed(KeyboardKey::KEY_M),
        help: raylib.is_key_pressed(KeyboardKey::KEY_H),
        volume_up: raylib.is_key_pressed(KeyboardKey::KEY_EQUAL)
            || raylib.is_key_pressed(KeyboardKey::KEY_KP_ADD),
        volume_down: raylib.is_key_pressed(KeyboardKey::KEY_MINUS)
            || raylib.is_key_pressed(KeyboardKey::KEY_KP_SUBTRACT),
        fullscreen: raylib.is_key_pressed(KeyboardKey::KEY_F11),
        bomb: raylib.is_key_pressed(KeyboardKey::KEY_B),
        scan: raylib.is_key_pressed(KeyboardKey::KEY_C),
        place_relay: raylib.is_key_pressed(KeyboardKey::KEY_R),
        place_drone: raylib.is_key_pressed(KeyboardKey::KEY_T),
        place_lift: raylib.is_key_pressed(KeyboardKey::KEY_L),
        place_support: raylib.is_key_pressed(KeyboardKey::KEY_U),
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
    } else if raylib.is_key_pressed(KeyboardKey::KEY_SEVEN) {
        Some(6)
    } else if raylib.is_key_pressed(KeyboardKey::KEY_EIGHT) {
        Some(7)
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
