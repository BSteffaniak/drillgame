use crate::{
    audio::AudioBus,
    input::read_input,
    input_mapping::{
        ai_commands, gamepad_commands, local_keyboard_commands, map_local_input, online_commands,
        replay_commands, split_screen_commands,
    },
    multiplayer::{FIXED_DELTA_SECONDS, PlayerCommand, SimulationTick},
    rendering::GameRenderer,
    save::{load_settings, save_settings},
    session::{
        ClientPredictionState, GameSession, PlayerSnapshot, PredictionFailure, TerrainChunkPosition,
    },
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
        observe_multiplayer_scaffolding(&mut session, mapped_input.player_commands, delta_seconds);

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

#[allow(
    clippy::too_many_lines,
    reason = "temporary compatibility observer intentionally references multiplayer scaffolding until systems are fully integrated"
)]
fn observe_multiplayer_scaffolding(
    session: &mut GameSession,
    player_commands: Vec<PlayerCommand>,
    delta_seconds: f32,
) {
    let _local_client_id = session.local_client().client_id;
    let current_tick = session.current_tick();
    let _local_player = session
        .world()
        .player(session.local_client().controlled_player_id);
    let _legacy_visual_changes = session.game().visual_changes();
    let _player_count = session.world().player_count();
    let _authoritative_summary = session.world().authoritative_summary();
    let mut world_probe = session.world().clone();
    let _scoped_command_outcome = world_probe.apply_player_command(
        crate::multiplayer::LOCAL_PLAYER_ID,
        &PlayerCommand::Movement {
            horizontal: 0.0,
            thrust: false,
            drill_down: false,
        },
    );
    world_probe.set_scanner_cooldown_seconds(crate::multiplayer::LOCAL_PLAYER_ID, 0.0);
    world_probe.set_active_drill(crate::multiplayer::LOCAL_PLAYER_ID, None);
    let _scanner_cooldown =
        world_probe.scanner_cooldown_seconds(crate::multiplayer::LOCAL_PLAYER_ID);
    let _active_drill = world_probe.active_drill(crate::multiplayer::LOCAL_PLAYER_ID);
    let _inventory_summary =
        world_probe.player_inventory_summary(crate::multiplayer::LOCAL_PLAYER_ID);
    let _mutable_local_player = world_probe.player_mut(crate::multiplayer::LOCAL_PLAYER_ID);
    let _world_snapshot = session.world_snapshot();
    let _compact_delta =
        crate::session::WorldDelta::new(session.current_tick(), Vec::new()).compact_network_delta();
    let _sequenced_commands = session.sequence_local_commands(player_commands);
    let _pending_command_count = session.pending_command_count(current_tick);
    let _processed_command_count =
        session.process_authoritative_commands_for_tick(SimulationTick::new(u64::MAX));
    let _simulation_accumulator = session.simulation_accumulator();
    let terrain_revisions = session.terrain_revisions();
    let _origin_chunk_revision = terrain_revisions.revision(TerrainChunkPosition { x: 0, y: 0 });
    let _recovery_delta = terrain_revisions.recovery_delta(
        session.current_tick(),
        TerrainChunkPosition { x: 0, y: 0 },
        0,
    );
    let _keyframe_interval_ticks = GameSession::keyframe_interval_ticks();
    let _compatibility_mode = GameSession::compatibility_mode();
    let _target_compatibility_mode = GameSession::target_compatibility_mode();
    let _planned_state_boundaries = GameSession::planned_state_boundaries();
    let _planned_transient_effect_boundaries = GameSession::planned_transient_effect_boundaries();
    let planned_player_scoped_systems = GameSession::planned_player_scoped_systems();
    let future_command_producers = [
        local_keyboard_commands(crate::input::PlayerInput::default()),
        gamepad_commands(Vec::new()),
        split_screen_commands(Vec::new()),
        online_commands(Vec::new()),
        replay_commands(Vec::new()),
        ai_commands(Vec::new()),
    ];
    let all_future_producers_authoritative = future_command_producers
        .iter()
        .all(crate::input_mapping::CommandProducer::uses_authoritative_path);
    let _ = (
        planned_player_scoped_systems,
        all_future_producers_authoritative,
    );
    let _fixed_tick_audit_items = GameSession::fixed_tick_audit_items();
    let _snapshot_purposes = GameSession::snapshot_purposes();
    let _client_presentation_fields = GameSession::client_presentation_fields();
    let _split_screen_viewports = GameSession::split_screen_viewports(session.client_count());
    let _world_event_catalog = GameSession::world_event_catalog();
    let _authoritative_counts = (
        session.world().hazard_count(),
        session.world().bomb_count(),
        session.world().infrastructure_count(),
        session.world().hazards().len(),
        session.world().bombs().len(),
        session.world().infrastructure().len(),
        session.world().service_transactions().len(),
    );
    let _world_snapshot_keyframe = session.world_snapshot().keyframe_message();
    session.apply_command_acknowledgement(&crate::multiplayer::CommandAcknowledgement {
        client_id: crate::multiplayer::LOCAL_CLIENT_ID,
        acknowledged_sequence: crate::multiplayer::InputSequence::new(0),
        authoritative_tick: session.current_tick(),
    });
    session.apply_command_rejection(&crate::multiplayer::CommandRejection {
        client_id: crate::multiplayer::LOCAL_CLIENT_ID,
        player_id: crate::multiplayer::LOCAL_PLAYER_ID,
        sequence: crate::multiplayer::InputSequence::new(0),
        reason: crate::multiplayer::CommandAcceptance::Duplicate,
        authoritative_tick: session.current_tick(),
    });
    let _network_payload = session
        .drain_world_delta()
        .compact_network_delta()
        .network_payload();
    let _network_delta_tick = session.drain_world_delta().compact_network_delta().tick();
    let _network_protocol_message = session
        .drain_world_delta()
        .compact_network_delta()
        .protocol_message();
    let _local_view = session.local_view();
    let _client_view_count = session.client_count();
    let _client_views = session.render_views();
    let render_frame_plan = session.render_frame_plan();
    let _render_view_count = render_frame_plan.view_count();
    let _render_player_for_view = render_frame_plan
        .views
        .first()
        .and_then(|view| render_frame_plan.player_for_view(view));
    let _predicted_session_movement = session.predicted_local_movement(delta_seconds);
    let prediction_presentation_plan =
        session.prediction_presentation_plan(None, delta_seconds, 0.5, 0.0);
    let _render_predicted_player_for_view = render_frame_plan.views.first().and_then(|view| {
        render_frame_plan.predicted_player_for_view(view, &prediction_presentation_plan)
    });
    let _feedback_output_count = prediction_presentation_plan.feedback_outputs.len();
    let _save_from_world = crate::save::PersistentWorldSave::from_world_and_legacy_game(
        session.world(),
        session.game(),
    );
    let _prediction_recovery_actions =
        session.prediction_recovery_actions(crate::session::TerrainChunkPosition { x: 0, y: 0 }, 0);
    let prediction = session.local_client().prediction();
    let _prediction_replay_len = prediction.replay_commands().len();
    let _prediction_buffer_len = prediction.unacknowledged_commands().len();
    let _prediction_failure_count = prediction.prediction_failures().len();
    let _prediction_correction_plan = ClientPredictionState::correction_plan(0.0, 0.0);
    let _interpolation_delay = ClientPredictionState::interpolation_delay_seconds(delta_seconds);
    let _can_extrapolate = ClientPredictionState::should_extrapolate(0.0);
    let _predicted_input_lag = prediction.predicted_input_lag_seconds();
    let snapshot =
        PlayerSnapshot::from_player(crate::multiplayer::LOCAL_PLAYER_ID, &session.game().player);
    let predicted_movement =
        ClientPredictionState::predict_local_movement(&snapshot, delta_seconds);
    let _predicted_from_snapshot = crate::session::PredictedMovement::from_snapshot(&snapshot);
    let _reconciled_movement =
        ClientPredictionState::reconcile_movement(predicted_movement, &snapshot);
    let _remote_player_presentation =
        ClientPredictionState::remote_player_presentation(&snapshot, None, 0.0, 0.0);
    let mut prediction_probe = prediction.clone();
    prediction_probe.note_prediction_failure(PredictionFailure::TerrainAlreadyChanged);
    prediction_probe.note_prediction_failure(PredictionFailure::HazardOrRescueChangedState);
    prediction_probe.note_prediction_failure(PredictionFailure::EconomyChangedState);
    prediction_probe.note_prediction_failure(PredictionFailure::ProgressionChangedState);
    let _prediction_failure_resolutions = prediction_probe.prediction_failure_resolutions();
    prediction_probe.clear_prediction_failures();
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::MovementIntent);
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::DrillContact);
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::DrillProgressVisual);
    let _pending_feedback_count = prediction_probe.pending_feedback().len();
    let _tentative_feedback_presentations = prediction_probe.tentative_feedback_presentations();
    prediction_probe.clear_feedback();
    prediction_probe.set_correction_offset(crate::session::CorrectionOffset::new(0.0, 0.0));
    let _correction_offset = prediction_probe.correction_offset();
    prediction_probe.clear_correction_offset();
    prediction_probe.push_remote_snapshot(snapshot);
    let _remote_snapshot_count =
        prediction_probe.remote_snapshot_count(crate::multiplayer::LOCAL_PLAYER_ID);
}
