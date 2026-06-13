#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

mod app;
mod economy;
mod game_state;
mod input;
mod player;
mod rendering;
mod save;
mod terrain;

use app::run;

fn main() {
    run();
}
