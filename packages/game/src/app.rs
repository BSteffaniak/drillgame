use crate::{
    audio::AudioBus,
    input::read_input,
    input_mapping::map_local_input,
    multiplayer::FIXED_DELTA_SECONDS,
    rendering::GameRenderer,
    save::{load_settings, save_settings},
    session::GameSession,
};

const WINDOW_WIDTH: i32 = 1280;
const WINDOW_HEIGHT: i32 = 720;
const TARGET_FPS: u32 = 60;
const FRAME_DELTA_SPIKE_WARN_SECONDS: f32 = FIXED_DELTA_SECONDS * 15.0;

pub fn run() {
    let (mut raylib, thread) = raylib::init()
        .size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .title("Drillgame")
        .build();

    raylib.set_target_fps(TARGET_FPS);
    raylib.set_exit_key(None);

    let settings = load_settings();
    let mut session = GameSession::new();
    session.apply_settings(settings);
    if session.fullscreen() {
        raylib.toggle_fullscreen();
    }
    let audio = match AudioBus::new() {
        Ok(audio) => Some(audio),
        Err(error) => {
            eprintln!("Audio disabled: {error}");
            None
        }
    };

    let mut renderer = GameRenderer::new(&mut raylib, &thread, session.game());

    while !session.should_exit() {
        let delta_seconds = raylib.get_frame_time();
        debug_assert!(
            delta_seconds <= FRAME_DELTA_SPIKE_WARN_SECONDS,
            "large frame delta detected before fixed-tick simulation migration"
        );
        let exit_requested = raylib.window_should_close();
        let input = read_input(&raylib, exit_requested);
        let mapped_input = map_local_input(input);
        let _local_client_id = session.local_client().client_id;
        let current_tick = session.current_tick();
        let _local_player = session
            .world()
            .player(session.local_client().controlled_player_id);
        let _legacy_visual_changes = session.game().visual_changes();
        let _player_count = session.world().player_count();
        let _world_snapshot = session.world_snapshot();
        let _sequenced_commands = session.sequence_local_commands(mapped_input.player_commands);
        let _pending_command_count = session.pending_command_count(current_tick);
        let _simulation_accumulator = session.simulation_accumulator();
        let terrain_revisions = session.terrain_revisions();
        let _origin_chunk_revision =
            terrain_revisions.revision(crate::session::TerrainChunkPosition { x: 0, y: 0 });
        let _keyframe_interval_ticks = GameSession::keyframe_interval_ticks();
        let _local_view = session.local_view();
        let _client_view_count = session.client_count();
        let _client_views = session.render_views();

        let _prediction_replay_len = session.local_client().prediction().replay_commands().len();
        let _prediction_buffer_len = session
            .local_client()
            .prediction()
            .unacknowledged_commands()
            .len();

        let _prediction_correction_plan =
            crate::session::ClientPredictionState::correction_plan(0.0, 0.0);
        let _interpolation_delay =
            crate::session::ClientPredictionState::interpolation_delay_seconds(delta_seconds);
        let _can_extrapolate = crate::session::ClientPredictionState::should_extrapolate(0.0);
        let _predicted_input_lag = session
            .local_client()
            .prediction()
            .predicted_input_lag_seconds();
        let mut prediction_probe = session.local_client().prediction().clone();
        prediction_probe.push_remote_snapshot(crate::session::PlayerSnapshot::from_player(
            crate::multiplayer::LOCAL_PLAYER_ID,
            &session.game().player,
        ));
        let _remote_snapshot_count =
            prediction_probe.remote_snapshot_count(crate::multiplayer::LOCAL_PLAYER_ID);

        session.update_legacy(input, delta_seconds);
        let world_delta = session.drain_world_delta();
        let _world_delta_is_empty = world_delta.is_empty();
        if input.fullscreen {
            raylib.toggle_fullscreen();
        }
        if (input.fullscreen
            || input.volume_up
            || input.volume_down
            || session.take_settings_dirty())
            && let Err(error) = save_settings(session.current_settings())
        {
            eprintln!("Settings save failed: {error}");
        }
        if let Some(audio) = &audio {
            audio.set_volume(session.master_volume());
            audio.play(&session.game().sound_cues);
        }

        renderer.sync_delta(&mut raylib, &thread, session.game(), &world_delta);

        let mut draw = raylib.begin_drawing(&thread);
        let client_views = session.client_views();
        renderer.render_client_views(&mut draw, session.game(), &client_views);
    }
}
