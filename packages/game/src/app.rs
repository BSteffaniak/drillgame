use crate::{
    audio::AudioBus,
    game_state::GameState,
    input::read_input,
    rendering::render,
    save::{SettingsFile, load_settings, save_settings},
};

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

    let settings = load_settings();
    let mut game = GameState::new();
    game.master_volume = settings.master_volume;
    game.fullscreen = settings.fullscreen;
    if game.fullscreen {
        raylib.toggle_fullscreen();
    }
    let audio = match AudioBus::new() {
        Ok(audio) => Some(audio),
        Err(error) => {
            eprintln!("Audio disabled: {error}");
            None
        }
    };

    while !raylib.window_should_close() && !game.request_exit {
        let delta_seconds = raylib.get_frame_time();
        let input = read_input(&raylib);

        game.update(input, delta_seconds);
        if input.fullscreen {
            raylib.toggle_fullscreen();
        }
        if (input.fullscreen || input.volume_up || input.volume_down)
            && let Err(error) = save_settings(SettingsFile {
                master_volume: game.master_volume,
                fullscreen: game.fullscreen,
            })
        {
            eprintln!("Settings save failed: {error}");
        }
        if let Some(audio) = &audio {
            audio.set_volume(game.master_volume);
            audio.play(&game.sound_cues);
        }

        let mut draw = raylib.begin_drawing(&thread);
        render(&mut draw, &game);
    }
}
