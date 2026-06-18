use std::sync::mpsc;

use crate::{
    audio::AudioBus,
    game_state::{
        GameState, OnlineNetworkTaskRequest, OnlineNetworkTaskResult, RealOnlineSessionController,
    },
    input::{
        combine_player_input, read_gamepad_input, read_input, read_input_with_arrow_aliases,
        read_primary_keyboard_input, read_primary_keyboard_input_with_arrow_aliases,
        read_secondary_keyboard_input,
    },
    input_mapping::{
        ai_commands, gamepad_commands, local_keyboard_commands, map_local_input, online_commands,
        replay_commands, split_screen_commands,
    },
    multiplayer::{
        ClientRuntimeConfig, ClientRuntimeMode, ClientSessionRuntime, FIXED_DELTA_SECONDS,
        HostRuntimeConfig, HostSessionRuntime, PlayerCommand, SimulationTick,
    },
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

enum OnlineTaskCompletion {
    Hosted(Result<RealOnlineSessionController, String>),
    JoinedDescriptor(Result<(RealOnlineSessionController, std::path::PathBuf), String>),
    Connected(Result<RealOnlineSessionController, String>),
    Reconnected(Result<RealOnlineSessionController, String>),
}

enum OnlineDescriptorAcceptCompletion {
    Accepted(Result<RealOnlineSessionController, String>),
}

struct OnlineTaskDispatcher {
    runtime: Option<tokio::runtime::Runtime>,
    controller: Option<RealOnlineSessionController>,
    pending_completion: Option<mpsc::Receiver<OnlineTaskCompletion>>,
    pending_descriptor_accept: Option<mpsc::Receiver<OnlineDescriptorAcceptCompletion>>,
    tick_accumulator_seconds: f32,
    live_tick_sequence: u32,
}

impl OnlineTaskDispatcher {
    fn new() -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(1)
                .build()
                .ok(),
            controller: None,
            pending_completion: None,
            pending_descriptor_accept: None,
            tick_accumulator_seconds: 0.0,
            live_tick_sequence: 0,
        }
    }

    fn poll_descriptor_accept(&mut self, game: &mut GameState) {
        let Some(receiver) = &self.pending_descriptor_accept else {
            return;
        };
        let Ok(completion) = receiver.try_recv() else {
            return;
        };
        self.pending_descriptor_accept = None;
        match completion {
            OnlineDescriptorAcceptCompletion::Accepted(Ok(controller)) => {
                self.controller = Some(controller);
                game.apply_online_diagnostics(
                    "descriptor-host-accepted",
                    "client accepted; awaiting ticks",
                );
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Hosted(
                    crate::game_state::RealOnlineSessionUxSnapshot::from_descriptor_host_accepted(
                        Some(1),
                    ),
                ));
            }
            OnlineDescriptorAcceptCompletion::Accepted(Err(error)) => {
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Failed(error));
            }
        }
    }

    fn poll_completions(&mut self, game: &mut GameState) {
        let Some(receiver) = &self.pending_completion else {
            return;
        };
        let Ok(completion) = receiver.try_recv() else {
            return;
        };
        self.pending_completion = None;
        match completion {
            OnlineTaskCompletion::Hosted(Ok(controller)) => {
                let snapshot =
                    crate::game_state::RealOnlineSessionUxSnapshot::from_host_descriptor_ready(
                        Some(1),
                        &game.online_descriptor_path,
                    );
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Hosted(snapshot));
                game.apply_online_diagnostics(
                    "descriptor-host-pending",
                    "waiting for descriptor client",
                );
                self.spawn_descriptor_accept(controller);
            }
            OnlineTaskCompletion::JoinedDescriptor(Ok((controller, path))) => {
                self.controller = Some(controller);
                game.apply_online_diagnostics(
                    "descriptor-client-connected",
                    "join accepted; awaiting ticks",
                );
                game.apply_online_network_task_result(OnlineNetworkTaskResult::JoinedDescriptor(
                    crate::game_state::RealOnlineSessionUxSnapshot::from_descriptor_client_connected(
                        Some(2),
                        &path,
                    ),
                ));
            }
            OnlineTaskCompletion::Connected(Ok(controller)) => {
                self.controller = Some(controller);
                game.apply_online_diagnostics("combined-localhost", "connected; awaiting ticks");
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Connected(
                    crate::game_state::RealOnlineSessionUxSnapshot::from_joined_session(Some(1)),
                ));
            }
            OnlineTaskCompletion::Reconnected(Ok(controller)) => {
                self.controller = Some(controller);
                game.apply_online_diagnostics("combined-localhost", "reconnected; awaiting ticks");
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Reconnected(
                    crate::game_state::RealOnlineSessionUxSnapshot::from_reconnect(Some(1)),
                ));
            }
            OnlineTaskCompletion::Hosted(Err(error))
            | OnlineTaskCompletion::JoinedDescriptor(Err(error))
            | OnlineTaskCompletion::Connected(Err(error))
            | OnlineTaskCompletion::Reconnected(Err(error)) => {
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Failed(error));
            }
        }
    }

    fn spawn_descriptor_accept(&mut self, mut controller: RealOnlineSessionController) {
        let Some(runtime) = &self.runtime else {
            self.controller = Some(controller);
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending_descriptor_accept = Some(receiver);
        runtime.spawn(async move {
            let mut game = GameState::new();
            let result = controller
                .accept_descriptor_client(&mut game)
                .await
                .map(|()| controller)
                .map_err(|error| format!("{error:?}"));
            let _ignored = sender.send(OnlineDescriptorAcceptCompletion::Accepted(result));
        });
    }

    fn spawn_host_descriptor(
        &mut self,
        path: std::path::PathBuf,
        bind_addr: std::net::SocketAddr,
        advertise_addr: std::net::SocketAddr,
    ) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending_completion = Some(receiver);
        runtime.spawn(async move {
            let mut game = GameState::new();
            game.online_host_bind_addr = bind_addr;
            game.online_host_advertise_addr = advertise_addr;
            let result =
                RealOnlineSessionController::host_descriptor_file_pending(&mut game, &path)
                    .map_err(|error| format!("{error:?}"));
            let _ignored = sender.send(OnlineTaskCompletion::Hosted(result));
        });
    }

    fn spawn_join_descriptor(&mut self, path: std::path::PathBuf) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending_completion = Some(receiver);
        runtime.spawn(async move {
            let mut game = GameState::new();
            let result = RealOnlineSessionController::connect_descriptor_client(&mut game, &path)
                .await
                .map(|controller| (controller, path))
                .map_err(|error| format!("{error:?}"));
            let _ignored = sender.send(OnlineTaskCompletion::JoinedDescriptor(result));
        });
    }

    fn spawn_connect(&mut self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending_completion = Some(receiver);
        runtime.spawn(async move {
            let mut game = GameState::new();
            let result = RealOnlineSessionController::connect_localhost(&mut game)
                .await
                .map_err(|error| format!("{error:?}"));
            let _ignored = sender.send(OnlineTaskCompletion::Connected(result));
        });
    }

    fn spawn_reconnect(&mut self, mut controller: RealOnlineSessionController) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.pending_completion = Some(receiver);
        runtime.spawn(async move {
            let mut game = GameState::new();
            let result = controller
                .reconnect(&mut game, crate::multiplayer::SessionToken::new(1))
                .await
                .map(|()| controller)
                .map_err(|error| format!("{error:?}"));
            let _ignored = sender.send(OnlineTaskCompletion::Reconnected(result));
        });
    }

    fn drain_and_execute(&mut self, game: &mut GameState) {
        self.poll_descriptor_accept(game);
        self.poll_completions(game);
        let Some(request) = game.take_online_network_task_request() else {
            return;
        };

        if self.pending_completion.is_some() && request != OnlineNetworkTaskRequest::Shutdown {
            game.apply_online_network_task_result(OnlineNetworkTaskResult::Failed(
                "Online network task already in progress".to_owned(),
            ));
            return;
        }

        match request {
            OnlineNetworkTaskRequest::HostDirectConnect
            | OnlineNetworkTaskRequest::JoinDirectConnect => {
                self.spawn_connect();
            }
            OnlineNetworkTaskRequest::HostDescriptorFile { path } => {
                game.online_session_status_message = format!(
                    "Preparing host descriptor at {}; waiting for remote miner after descriptor is written.",
                    path.display()
                );
                self.spawn_host_descriptor(
                    path,
                    game.online_host_bind_addr,
                    game.online_host_advertise_addr,
                );
            }
            OnlineNetworkTaskRequest::JoinDescriptorFile { path } => {
                self.spawn_join_descriptor(path);
            }
            OnlineNetworkTaskRequest::ReconnectDirectConnect => {
                if let Some(controller) = self.controller.take() {
                    self.spawn_reconnect(controller);
                } else {
                    game.apply_online_network_task_result(OnlineNetworkTaskResult::Failed(
                        "No active online session to reconnect".to_owned(),
                    ));
                }
            }
            OnlineNetworkTaskRequest::Shutdown => {
                if let (Some(runtime), Some(controller)) = (&self.runtime, &mut self.controller) {
                    match controller.mode_label() {
                        "descriptor-host-accepted" => {
                            let _ignored = runtime.block_on(
                                controller
                                    .descriptor_host_send_session_ended("host ended the session"),
                            );
                        }
                        "descriptor-client-connected" => {
                            let _ignored =
                                runtime.block_on(controller.descriptor_client_send_session_ended(
                                    "joined client left the session",
                                ));
                        }
                        _ => {}
                    }
                }
                self.controller = None;
                self.pending_completion = None;
                self.pending_descriptor_accept = None;
                game.apply_online_network_task_result(OnlineNetworkTaskResult::Shutdown);
            }
        }
    }

    fn live_session_tick_input(
        &mut self,
        session: &GameSession,
    ) -> crate::multiplayer::QuinnSessionTickInput {
        let local_client = session.local_client();
        let player_id = local_client.controlled_player_id;
        let tick = session.current_tick().get().saturating_add(1);
        let sequence = self.live_tick_sequence;
        self.live_tick_sequence = self.live_tick_sequence.wrapping_add(1);
        let chunk_coord = i32::try_from(tick).unwrap_or(i32::MAX);
        let snapshot = session.world_snapshot().network_snapshot();
        let correction_probe = snapshot.players.first().map(|player| {
            (
                player.x,
                player.y,
                player.clone(),
                crate::multiplayer::SimulationTick::new(tick.saturating_add(2)),
            )
        });
        crate::multiplayer::QuinnSessionTickInput {
            command_packet: Some(crate::multiplayer::CommandPacket {
                client_id: local_client.client_id,
                commands: vec![crate::multiplayer::SequencedPlayerCommand {
                    player_id,
                    sequence: crate::multiplayer::InputSequence::new(sequence),
                    target_tick: crate::multiplayer::SimulationTick::new(tick),
                    command: crate::multiplayer::PlayerCommand::Movement {
                        horizontal: 0.0,
                        thrust: false,
                        drill_down: false,
                    },
                }],
            }),
            snapshot: Some(snapshot),
            delta: Some((
                crate::multiplayer::SimulationTick::new(tick.saturating_add(1)),
                crate::multiplayer::NetworkDeltaPayload::Noop,
            )),
            terrain_chunk_request: Some((chunk_coord, chunk_coord, 1, 2)),
            correction_probe,
        }
    }

    fn tick_diagnostic(summary: &crate::multiplayer::QuinnSessionTickSummary) -> String {
        format!(
            "command={}, snapshot={}, delta={}, chunk={}, correction={}",
            summary.command_summary.is_some(),
            summary.snapshot_replicated,
            summary.delta_replicated,
            summary.terrain_chunk_response.is_some(),
            summary.correction_summary.is_some()
        )
    }

    fn drive_scheduled_tick(&mut self, session: &mut GameSession, delta_seconds: f32) {
        if self.controller.is_none() {
            self.tick_accumulator_seconds = 0.0;
            return;
        }
        self.tick_accumulator_seconds += delta_seconds;
        if self.tick_accumulator_seconds < FIXED_DELTA_SECONDS {
            return;
        }
        self.tick_accumulator_seconds = 0.0;
        let input = self.live_session_tick_input(session);
        let Some(controller) = &mut self.controller else {
            return;
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                session.game_mut().apply_online_network_task_result(
                    OnlineNetworkTaskResult::Failed(error.to_string()),
                );
                return;
            }
        };
        let mode_label = controller.mode_label();
        let result = match mode_label {
            "descriptor-host-accepted" => runtime.block_on(
                controller.drive_descriptor_host_outbound_tick(session.game_mut(), input),
            ),
            "descriptor-client-connected" => runtime.block_on(
                controller.drive_descriptor_client_outbound_tick(session.game_mut(), input),
            ),
            "descriptor-host-pending" => Ok(crate::multiplayer::QuinnSessionTickSummary {
                command_summary: None,
                snapshot_replicated: false,
                delta_replicated: false,
                terrain_chunk_response: None,
                correction_summary: None,
            }),
            _ => runtime
                .block_on(controller.drive_tick_input(session.game_mut(), input))
                .map(|telemetry| telemetry.summary),
        };
        match result {
            Ok(summary) => session
                .game_mut()
                .apply_online_diagnostics(mode_label, Self::tick_diagnostic(&summary)),
            Err(error) => {
                session.game_mut().apply_online_network_task_result(
                    OnlineNetworkTaskResult::Failed(format!("{error:?}")),
                );
            }
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "main loop coordinates platform input, session stepping, audio, saving, and rendering"
)]
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
    let mut online_tasks = OnlineTaskDispatcher::new();

    while !session.should_exit() {
        let delta_seconds = raylib.get_frame_time();
        debug_assert!(
            delta_seconds <= FRAME_DELTA_SPIKE_WARN_SECONDS,
            "large frame delta detected before fixed-tick simulation migration"
        );
        let exit_requested = raylib.window_should_close();
        let split_screen_active = session.client_count() > 1;
        let input = if split_screen_active {
            read_input(&mut raylib, exit_requested)
        } else {
            read_input_with_arrow_aliases(&mut raylib, exit_requested)
        };
        let mapped_input = map_local_input(input);
        if mapped_input
            .client_actions
            .contains(&crate::multiplayer::ClientAction::ToggleLocalMultiplayer)
        {
            let enabled_split_screen = session.enable_default_local_split_screen();
            if enabled_split_screen {
                let player_slots = u8::try_from(session.client_count()).unwrap_or(u8::MAX);
                session
                    .game_mut()
                    .mark_local_multiplayer_active(player_slots);
            }
        }
        if session.game_mut().take_local_multiplayer_request() {
            let _ = session.enable_default_local_split_screen();
            let player_slots = u8::try_from(session.client_count()).unwrap_or(u8::MAX);
            session
                .game_mut()
                .mark_local_multiplayer_active(player_slots);
        }
        online_tasks.drain_and_execute(session.game_mut());
        online_tasks.drive_scheduled_tick(&mut session, delta_seconds);
        session.apply_client_actions(
            crate::multiplayer::LOCAL_CLIENT_ID,
            &mapped_input.client_actions,
        );
        let secondary_input = (session.client_count() > 1).then(|| {
            let keyboard = read_secondary_keyboard_input(&raylib);
            read_gamepad_input(&raylib, 0)
                .map_or(keyboard, |gamepad| combine_player_input(keyboard, gamepad))
        });
        let primary_input = if session.client_count() > 1 {
            read_primary_keyboard_input(&mut raylib)
        } else {
            read_primary_keyboard_input_with_arrow_aliases(&mut raylib)
        };
        for local_input in crate::input_mapping::local_split_screen_inputs(
            crate::multiplayer::LOCAL_CLIENT_ID,
            primary_input,
            session.secondary_local_client_id(),
            secondary_input,
        ) {
            let _batch =
                session.route_command_producer(local_input.client_id, local_input.producer);
        }
        observe_multiplayer_scaffolding(&mut session, delta_seconds);

        let authoritative_input = session.update_legacy_input_from_authoritative_commands(input);
        session.update_legacy(authoritative_input, delta_seconds);
        let world_delta = session.drain_world_delta();
        let _world_delta_is_empty = world_delta.is_empty();
        if input.fullscreen {
            raylib.toggle_fullscreen();
        }
        let settings_dirty = session.take_settings_dirty();
        if settings_dirty {
            session.note_save_session_transition_for_prediction();
        }
        if (input.fullscreen || input.volume_up || input.volume_down || settings_dirty)
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
        session.update_remote_timing_from_network_sample(0.0, 0.0);
        let _remote_timing = session.remote_timing();
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);
        let live_render_output = session.live_render_frame_output(&prediction_plan);
        renderer.render_live_frame_output(&mut draw, session.game(), &live_render_output);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "temporary compatibility observer intentionally references multiplayer scaffolding until systems are fully integrated"
)]
fn observe_multiplayer_scaffolding(session: &mut GameSession, delta_seconds: f32) {
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
    let _authoritative_dependency_split = world_probe
        .authoritative_dependency_summary()
        .authoritative_path_split();
    let _implementation_complete = world_probe
        .implementation_completion_summary()
        .primary_migration_complete_or_deferred();
    let _transient_effects_split = world_probe.transient_effect_routing_summary().split();
    let _survival_summary =
        world_probe.player_survival_snapshot(crate::multiplayer::LOCAL_PLAYER_ID);
    let player_scope_proof = world_probe.player_scoped_gameplay_proof(
        crate::multiplayer::LOCAL_PLAYER_ID,
        crate::multiplayer::LOCAL_PLAYER_ID,
    );
    let _player_scope_proof_complete =
        player_scope_proof.map(crate::session::PlayerScopedGameplayProof::complete);
    let _mutable_local_player = world_probe.player_mut(crate::multiplayer::LOCAL_PLAYER_ID);
    let _world_authoritative_complete = world_probe.authoritative_gameplay_ownership_complete();
    let _world_authoritative_domain_count = world_probe.authoritative_runtime_domain_count();
    let _world_legacy_adapter_restricted = world_probe.legacy_gameplay_adapter_restricted();
    let _world_snapshot = session.world_snapshot();
    let _live_keyframe_message = session.live_snapshot_keyframe_message();
    let _command_network_tick = session.command_network_session().current_tick();
    let _world_terrain_width = session.world().terrain().width();
    let mut terrain_probe = session.world().clone();
    let _terrain_probe_chip_result =
        terrain_probe.chip_active_drill_target(crate::multiplayer::LOCAL_PLAYER_ID);
    let _compact_delta =
        crate::session::WorldDelta::new(session.current_tick(), Vec::new()).compact_network_delta();
    let _sequenced_commands = session.sequence_local_commands(Vec::new());
    let _producer_batches = session.route_command_producers(
        crate::multiplayer::LOCAL_CLIENT_ID,
        [replay_commands(Vec::new()), ai_commands(Vec::new())],
    );
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
    let legacy_mutation_inventory = GameSession::legacy_gameplay_mutation_inventory();
    let variable_delta_audit = GameSession::variable_delta_audit_summary();
    let _legacy_mutation_inventory_complete = legacy_mutation_inventory.inventory_complete();
    let _variable_delta_complete = variable_delta_audit.gameplay_delta_audit_complete();
    let _frame_rate_invariance =
        session.world().player_count() > 0 && session.frame_rate_invariance_proof().complete();
    let _replay_determinism =
        GameSession::replay_determinism_proof(vec![PlayerCommand::Refuel]).complete();
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
    let production_transport = crate::multiplayer::production_transport_selection();
    let _ = production_transport.dependency_added;
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
        session
            .world()
            .discovered_tile_count(crate::multiplayer::LOCAL_PLAYER_ID),
        session
            .world()
            .failure_state(crate::multiplayer::LOCAL_PLAYER_ID)
            .is_some(),
    );
    let mut bomb_probe = session.world().clone();
    let _bomb_probe_results = bomb_probe.age_and_detonate_bombs(0.0);
    let mut economy_probe = session.world().clone();
    let _economy_probe_contract =
        economy_probe.apply_contract_reward(crate::multiplayer::LOCAL_PLAYER_ID, 0);
    let _economy_probe_expedition =
        economy_probe.start_expedition(crate::multiplayer::LOCAL_PLAYER_ID, 0);
    let _economy_probe_debt = economy_probe.repay_debt(crate::multiplayer::LOCAL_PLAYER_ID, 0);
    let _economy_probe_victory =
        economy_probe.award_victory(crate::multiplayer::LOCAL_PLAYER_ID, 0);
    let _economy_probe_won = economy_probe.won_game();
    let _world_snapshot_keyframe = session.world_snapshot().keyframe_message();
    let _live_snapshot_batch = session.live_snapshot_exchange_batch();
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
    let mut runtime_queues = crate::multiplayer::InMemoryTransportQueues::default();
    let mut host_runtime = crate::multiplayer::HostSessionRuntime::new(
        crate::multiplayer::HostRuntimeConfig::default(),
        session.current_tick(),
    );
    let mut client_runtime = crate::multiplayer::ClientSessionRuntime::new(
        crate::multiplayer::default_local_client_runtime(),
    );
    runtime_queues.send_to_host(client_runtime.connect_request());
    let _runtime_pump_summary = crate::multiplayer::pump_in_memory_runtime_packets(
        &mut runtime_queues,
        &mut host_runtime,
        &mut client_runtime,
        crate::multiplayer::LOCAL_PLAYER_ID,
        session.current_tick(),
    );
    let _network_payload = session
        .drain_world_delta()
        .compact_network_delta()
        .network_payload();
    let _network_delta_tick = session.drain_world_delta().compact_network_delta().tick();
    let _network_delta_summary = session
        .drain_world_delta()
        .compact_network_delta()
        .summary();
    let _network_protocol_message = session
        .drain_world_delta()
        .compact_network_delta()
        .protocol_message();
    let probe_delta = crate::session::WorldDelta::new(session.current_tick(), Vec::new());
    let _live_delta_batch = GameSession::live_world_delta_exchange_batch(&probe_delta);
    let _live_chunk_batch = session
        .live_terrain_chunk_exchange_batch(crate::session::TerrainChunkPosition { x: 0, y: 0 }, 0);
    let _local_view = session.local_view();
    let _client_view_count = session.client_count();
    let _client_views = session.render_views();
    let mut local_multiplayer_probe = session.clone();
    let added_local_client = local_multiplayer_probe.add_local_client_player(
        crate::multiplayer::ClientId::new(2),
        crate::multiplayer::PlayerId::new(2),
    );
    if added_local_client {
        let _split_screen_batch = local_multiplayer_probe
            .route_split_screen_player_commands(crate::multiplayer::ClientId::new(2), Vec::new());
        let _live_delta_message = local_multiplayer_probe.drain_live_world_delta_message();
        let _live_command_responses =
            local_multiplayer_probe.apply_live_command_packet(&crate::multiplayer::CommandPacket {
                client_id: crate::multiplayer::ClientId::new(2),
                commands: Vec::new(),
            });
        local_multiplayer_probe.observe_live_remote_player_snapshots();
        let _live_network_integration = local_multiplayer_probe.exercise_live_network_integration(
            crate::multiplayer::ClientId::new(3),
            crate::multiplayer::PlayerId::new(3),
            crate::multiplayer::SessionToken::new(7),
            crate::session::TerrainChunkPosition { x: 0, y: 0 },
            0,
        );
    }
    let render_frame_plan = session.render_frame_plan();
    let world_ownership = session.world().ownership_summary();
    let client_ownership = session.local_client().ownership_summary();
    let _world_fully_split = world_ownership.fully_split();
    let _client_fully_split = client_ownership.fully_split();
    let _keyboard_command_policy =
        GameSession::command_source_policy(crate::multiplayer::CommandSource::Keyboard);
    let fixed_tick_migration_summary = GameSession::fixed_tick_migration_summary();
    let _fixed_tick_audit_complete = fixed_tick_migration_summary.audit_complete();
    let gameplay_event_routing_summary = GameSession::gameplay_event_routing_summary();
    let _presentation_events_separated =
        gameplay_event_routing_summary.separates_local_presentation();
    let _render_view_count = render_frame_plan.view_count();
    let _render_player_for_view = render_frame_plan
        .views
        .first()
        .and_then(|view| render_frame_plan.player_for_view(view));
    let _predicted_session_movement = session.predicted_local_movement(delta_seconds);
    let _local_movement_prediction_plan = session.local_movement_prediction_plan(delta_seconds);
    let prediction_presentation_plan =
        session.live_prediction_presentation_plan(delta_seconds, 0.5, 0.0);
    let _legacy_prediction_presentation_plan =
        session.prediction_presentation_plan(None, delta_seconds, 0.5, 0.0);
    let _render_predicted_player_for_view = render_frame_plan.views.first().and_then(|view| {
        render_frame_plan.predicted_player_for_view(view, &prediction_presentation_plan)
    });
    let _render_remote_players_for_view = render_frame_plan.views.first().map(|view| {
        render_frame_plan.remote_player_presentations(view, &prediction_presentation_plan)
    });
    let _render_viewport_plans = render_frame_plan.viewport_plans(&prediction_presentation_plan);
    let live_render_output = session.live_render_frame_output(&prediction_presentation_plan);
    let _split_screen_ready = live_render_output.ready_for_live_render_path();
    let _live_render_counts = (
        live_render_output.clipped_viewport_count(),
        live_render_output.hud_count(),
    );
    let _world_player_presentations = render_frame_plan.views.first().map(|view| {
        render_frame_plan.world_player_presentations_for_view(view, &prediction_presentation_plan)
    });
    let _render_hud_snapshots = render_frame_plan.hud_snapshots();
    let _client_presentation_snapshots = render_frame_plan.client_presentation_snapshots();
    let _survival_snapshots = render_frame_plan.survival_snapshots();
    let _feedback_output_count = prediction_presentation_plan.feedback_outputs.len();
    let save_from_world = crate::save::PersistentWorldSave::from_world_and_legacy_game(
        session.world(),
        session.game(),
    );
    let save_restore_summary = save_from_world.restore_summary();
    let mut save_restore_world_probe = session.world().clone();
    save_from_world.restore_into_world(&mut save_restore_world_probe);
    let _save_roster_matches_players = save_restore_summary.roster_matches_persistent_players();
    let _prediction_recovery_actions =
        session.prediction_recovery_actions(crate::session::TerrainChunkPosition { x: 0, y: 0 }, 0);
    let _prediction_failure_recovery_plan = session
        .prediction_failure_recovery_plan(crate::session::TerrainChunkPosition { x: 0, y: 0 }, 0);
    let _prediction_failure_application_summary = session.prediction_failure_application_summary(
        crate::session::TerrainChunkPosition { x: 0, y: 0 },
        0,
    );
    let snapshot_chunk_recovery_plan = session
        .snapshot_chunk_recovery_plan(crate::session::TerrainChunkPosition { x: 0, y: 0 }, 0);
    let _snapshot_chunk_recovered_revision = snapshot_chunk_recovery_plan.recovered_revision();
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
    let reconciled_movement =
        ClientPredictionState::reconcile_movement(predicted_movement, &snapshot);
    let _replayed_reconciliation =
        crate::session::ReplayedReconciliation::from_authoritative_snapshot(
            &snapshot,
            prediction.unacknowledged_commands(),
        );
    let _correction_presentation_frame =
        crate::session::CorrectionPresentationFrame::from_reconciliation(&reconciled_movement, 0.5);
    let _remote_player_presentation =
        ClientPredictionState::remote_player_presentation(&snapshot, None, 0.0, 0.0);
    let mut prediction_probe = prediction.clone();
    prediction_probe.note_prediction_failure(PredictionFailure::TerrainAlreadyChanged);
    prediction_probe.note_prediction_failure(PredictionFailure::HazardOrRescueChangedState);
    prediction_probe.note_prediction_failure(PredictionFailure::EconomyChangedState);
    prediction_probe.note_prediction_failure(PredictionFailure::ProgressionChangedState);
    prediction_probe.note_prediction_failure(PredictionFailure::CommandRejected);
    let _prediction_failure_resolutions = prediction_probe.prediction_failure_resolutions();
    let _unacknowledged_replay_complete =
        prediction_probe.unacknowledged_replay_is_complete(crate::multiplayer::LOCAL_PLAYER_ID);
    let _prediction_recovery_actions = prediction_probe.prediction_recovery_actions(
        crate::multiplayer::LOCAL_PLAYER_ID,
        session.terrain_revisions(),
        session.current_tick(),
        crate::session::TerrainChunkPosition { x: 0, y: 0 },
        0,
    );
    let _prediction_tuning = crate::session::PredictionCorrectionTuning::default_gameplay_feel();
    let _prediction_tuning_classifies_offsets =
        crate::session::PredictionCorrectionTuning::classifies_expected_offsets();
    let prediction_debug_snapshot = prediction_probe.prediction_debug_snapshot(0.0, 0, 1, 1);
    let _prediction_debug_visible = prediction_debug_snapshot.visible_to_debug_overlay();
    prediction_probe.clear_prediction_failures();
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::MovementIntent);
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::DrillContact);
    prediction_probe.push_feedback(crate::session::LocalTentativeFeedback::DrillProgressVisual);
    let _pending_feedback_count = prediction_probe.pending_feedback().len();
    let _tentative_feedback_presentations = prediction_probe.tentative_feedback_presentations();
    let tentative_feedback_frame = prediction_probe.tentative_feedback_frame();
    let _has_tentative_feedback = tentative_feedback_frame.has_drill_feedback();
    prediction_probe.clear_feedback();
    prediction_probe.set_correction_offset(crate::session::CorrectionOffset::new(0.0, 0.0));
    let _correction_offset = prediction_probe.correction_offset();
    prediction_probe.clear_correction_offset();
    prediction_probe.push_remote_snapshot(snapshot);
    let _remote_snapshot_count =
        prediction_probe.remote_snapshot_count(crate::multiplayer::LOCAL_PLAYER_ID);
    let host_runtime_probe =
        HostSessionRuntime::new(HostRuntimeConfig::default(), SimulationTick::default());
    let host_runtime_status = host_runtime_probe.runtime_status();
    let _host_has_capacity = host_runtime_status.has_capacity();
    let client_runtime_probe = ClientSessionRuntime::new(ClientRuntimeConfig {
        mode: ClientRuntimeMode::RemoteNetwork,
        client_id: crate::multiplayer::LOCAL_CLIENT_ID,
        player_id: None,
    });
    let client_runtime_status = client_runtime_probe.runtime_status();
    let _client_joined = client_runtime_status.joined();
    let transport_queues_probe = crate::multiplayer::InMemoryTransportQueues::default();
    let _transport_idle = transport_queues_probe.status().is_idle();
    let protocol_exchange_batch = crate::multiplayer::ProtocolMessage::exchange_batch(
        crate::multiplayer::ProtocolExchangeKind::WorldDelta,
        vec![crate::multiplayer::ProtocolMessage::WorldDelta {
            tick: SimulationTick::default(),
            payload: crate::multiplayer::NetworkDeltaPayload::Noop,
        }],
    );
    let _protocol_exchange_unreliable_count = protocol_exchange_batch.unreliable_count();
    let join_messages = crate::multiplayer::reliable_join_exchange_messages(
        crate::multiplayer::LOCAL_CLIENT_ID,
        crate::multiplayer::LOCAL_PLAYER_ID,
        SimulationTick::default(),
    );
    let _join_flow_message_count = join_messages.len();
    let mut command_network_probe =
        crate::multiplayer::CommandNetworkSession::new(SimulationTick::default(), 2);
    let (_command_exchange_messages, command_exchange_summary) = command_network_probe
        .apply_command_packet_exchange(&crate::multiplayer::CommandPacket {
            client_id: crate::multiplayer::LOCAL_CLIENT_ID,
            commands: Vec::new(),
        });
    let _command_exchange_all_accepted = command_exchange_summary.all_accepted();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::{OnlineSessionUxState, RunMode};

    fn wait_for_online_completion(dispatcher: &mut OnlineTaskDispatcher, game: &mut GameState) {
        for _ in 0..50 {
            dispatcher.drain_and_execute(game);
            if dispatcher.pending_completion.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("online task did not complete");
    }

    #[test]
    fn online_task_dispatcher_executes_queued_connect_scheduled_tick_and_shutdown() {
        let mut game = GameState::new();
        let mut session = GameSession::new();
        let mut dispatcher = OnlineTaskDispatcher::new();

        game.online_network_task_request = Some(OnlineNetworkTaskRequest::HostDirectConnect);
        dispatcher.drain_and_execute(&mut game);
        wait_for_online_completion(&mut dispatcher, &mut game);
        dispatcher.drive_scheduled_tick(&mut session, FIXED_DELTA_SECONDS);

        assert_eq!(
            session.game().online_session_state,
            OnlineSessionUxState::Connected,
            "{}",
            session.game().message
        );
        assert!(session.game().message.contains("Real Quinn tick"));
        assert_eq!(game.run_mode, crate::game_state::RunMode::Playing);
        assert_eq!(game.modal, None);
        assert!(game.online_network_task_request.is_none());

        game.online_network_task_request = Some(OnlineNetworkTaskRequest::Shutdown);
        dispatcher.drain_and_execute(&mut game);

        assert_eq!(game.online_session_state, OnlineSessionUxState::Shutdown);
    }

    #[test]
    fn online_task_dispatcher_hosts_descriptor_file_without_entering_gameplay() {
        let mut game = GameState::new();
        let unique_path = std::env::temp_dir().join(format!(
            "drillgame-ui-host-descriptor-{}.json",
            std::process::id()
        ));
        let _ignored = std::fs::remove_file(&unique_path);
        game.online_descriptor_path = unique_path.clone();
        game.online_host_bind_addr = "127.0.0.1:0".parse().expect("bind addr parses");
        game.online_host_advertise_addr = "127.0.0.1:4242".parse().expect("advertise addr parses");
        let mut dispatcher = OnlineTaskDispatcher::new();

        game.online_network_task_request = Some(OnlineNetworkTaskRequest::HostDescriptorFile {
            path: unique_path.clone(),
        });
        dispatcher.drain_and_execute(&mut game);
        wait_for_online_completion(&mut dispatcher, &mut game);

        assert_eq!(game.online_session_state, OnlineSessionUxState::Hosting);
        assert_eq!(game.run_mode, crate::game_state::RunMode::Title);
        assert!(game.message.contains("Host descriptor ready"));
        assert!(
            std::fs::read_to_string(&unique_path)
                .expect("descriptor file written")
                .contains("127.0.0.1:4242")
        );
        assert!(dispatcher.controller.is_none());
        assert!(dispatcher.pending_descriptor_accept.is_some());
        let _ignored = std::fs::remove_file(unique_path);
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn online_task_dispatcher_accepts_descriptor_join_after_host_publish() {
        let unique_path = std::env::temp_dir().join(format!(
            "drillgame-ui-host-join-descriptor-{}.json",
            std::process::id()
        ));
        let _ignored = std::fs::remove_file(&unique_path);
        let mut host_game = GameState::new();
        host_game.online_descriptor_path = unique_path.clone();
        host_game.online_host_bind_addr = "127.0.0.1:0".parse().expect("bind addr parses");
        host_game.online_host_advertise_addr =
            "127.0.0.1:0".parse().expect("advertise addr parses");
        let mut host_dispatcher = OnlineTaskDispatcher::new();
        host_game.online_network_task_request =
            Some(OnlineNetworkTaskRequest::HostDescriptorFile {
                path: unique_path.clone(),
            });
        host_dispatcher.drain_and_execute(&mut host_game);
        wait_for_online_completion(&mut host_dispatcher, &mut host_game);
        assert_eq!(
            host_game.online_session_state,
            OnlineSessionUxState::Hosting
        );
        assert!(host_dispatcher.pending_descriptor_accept.is_some());

        let mut join_game = GameState::new();
        let mut join_dispatcher = OnlineTaskDispatcher::new();
        join_game.online_network_task_request =
            Some(OnlineNetworkTaskRequest::JoinDescriptorFile {
                path: unique_path.clone(),
            });
        join_dispatcher.drain_and_execute(&mut join_game);
        for _ in 0..100 {
            host_dispatcher.drain_and_execute(&mut host_game);
            join_dispatcher.drain_and_execute(&mut join_game);
            if host_dispatcher.pending_descriptor_accept.is_none()
                && join_dispatcher.pending_completion.is_none()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(
            host_game.online_session_state,
            OnlineSessionUxState::Connected
        );
        assert_ne!(host_game.run_mode, RunMode::Playing);
        assert_eq!(
            join_game.online_session_state,
            OnlineSessionUxState::Connected
        );
        assert_eq!(
            host_dispatcher
                .controller
                .as_ref()
                .expect("accepted host controller")
                .mode_label(),
            "descriptor-host-accepted"
        );
        assert_eq!(
            join_dispatcher
                .controller
                .as_ref()
                .expect("joined client controller")
                .mode_label(),
            "descriptor-client-connected"
        );

        let mut join_session = GameSession::new();
        join_session.game_mut().online_player_name = "Client QA".to_owned();
        join_session.game_mut().online_local_ready = true;
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS);
        assert!(
            join_session
                .game()
                .online_session_status_message
                .contains("Descriptor client sent live command tick")
        );
        let mut host_session = GameSession::new();
        host_session.game_mut().online_player_name = "Host QA".to_owned();
        {
            let host_player = host_session
                .world_mut()
                .player_mut(crate::multiplayer::LOCAL_PLAYER_ID)
                .expect("host local player exists");
            host_player.x = 123.0;
            host_player.y = 456.0;
            host_player.velocity_x = 7.0;
            host_player.velocity_y = -8.0;
            host_player.fuel = 321.0;
            host_player.hull = 654.0;
            host_player.credits = 999;
            host_player
                .cargo
                .insert(crate::terrain::MineralKind::Copper, 3);
            host_player
                .cargo
                .insert(crate::terrain::MineralKind::Gold, 2);
        }
        host_session
            .world_mut()
            .set_scanner_cooldown_seconds(crate::multiplayer::LOCAL_PLAYER_ID, 3.5);
        host_session.game_mut().terrain.set_kind(
            crate::terrain::TilePosition { x: 18, y: 19 },
            crate::terrain::TileKind::Lava,
        );
        join_session.game_mut().terrain.set_kind(
            crate::terrain::TilePosition { x: 18, y: 19 },
            crate::terrain::TileKind::Air,
        );
        host_session.game_mut().online_local_ready = true;
        host_session.game_mut().run_mode = RunMode::Playing;
        host_dispatcher.drive_scheduled_tick(&mut host_session, FIXED_DELTA_SECONDS);
        assert!(
            host_session
                .game()
                .online_session_status_message
                .contains("command=true")
        );
        assert!(host_session.game().online_remote_player_ready);
        assert_eq!(
            host_session.game().online_remote_player_name.as_deref(),
            Some("Client QA")
        );
        assert!(
            host_session
                .game()
                .online_last_replication_status
                .contains("host sent")
        );
        assert!(
            host_session
                .game()
                .online_last_terrain_status
                .contains("answered chunk")
        );
        assert_eq!(
            host_session.game().online_diagnostic_controller_mode,
            "descriptor-host-accepted"
        );
        assert!(
            host_session
                .game()
                .online_diagnostic_last_tick
                .contains("command=true")
        );
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS);
        assert!(
            join_session
                .game()
                .online_session_status_message
                .contains("received")
        );
        assert!(join_session.game().online_remote_player_ready);
        assert_eq!(
            join_session.game().online_remote_player_name.as_deref(),
            Some("Host QA")
        );
        assert_eq!(join_session.game().run_mode, RunMode::Playing);
        assert!((join_session.game().player.x - 123.0).abs() < f32::EPSILON);
        assert!((join_session.game().player.y - 456.0).abs() < f32::EPSILON);
        assert!((join_session.game().player.velocity_x - 7.0).abs() < f32::EPSILON);
        assert!((join_session.game().player.velocity_y + 8.0).abs() < f32::EPSILON);
        assert!((join_session.game().player.fuel - 321.0).abs() < f32::EPSILON);
        assert!((join_session.game().player.hull - 654.0).abs() < f32::EPSILON);
        assert_eq!(join_session.game().player.credits, 999);
        assert_eq!(
            join_session
                .game()
                .player
                .cargo
                .get(&crate::terrain::MineralKind::Copper),
            Some(&3)
        );
        assert_eq!(
            join_session
                .game()
                .player
                .cargo
                .get(&crate::terrain::MineralKind::Gold),
            Some(&2)
        );
        assert_eq!(join_session.game().player.cargo_used(), 5);
        assert!((join_session.game().scanner_cooldown_seconds - 3.5).abs() < f32::EPSILON);
        assert_eq!(join_session.game().online_remote_player_snapshots.len(), 1);
        let remote_player = &join_session.game().online_remote_player_snapshots[0];
        assert_eq!(remote_player.player_id, crate::multiplayer::LOCAL_PLAYER_ID);
        assert!((remote_player.x - 123.0).abs() < f32::EPSILON);
        assert!((remote_player.y - 456.0).abs() < f32::EPSILON);
        assert_eq!(remote_player.credits, 999);
        assert_eq!(remote_player.cargo_used, 5);
        assert_eq!(
            remote_player
                .cargo
                .get(&crate::terrain::MineralKind::Copper),
            Some(&3)
        );
        assert!(
            join_session
                .game()
                .online_multiplayer_status_lines()
                .iter()
                .any(|line| {
                    line.contains("Remote snapshot players")
                        && line.contains("p1")
                        && line.contains("Copperx3")
                })
        );
        assert!(
            join_session
                .game()
                .online_last_replication_status
                .contains("received")
        );
        assert!(
            join_session
                .game()
                .online_last_replicated_player_status
                .contains("tick")
        );
        assert!(
            join_session
                .game()
                .online_last_terrain_status
                .contains("applied chunk")
        );
        assert_eq!(
            join_session
                .game()
                .terrain
                .tile(crate::terrain::TilePosition { x: 18, y: 19 })
                .expect("replicated terrain tile exists")
                .kind,
            crate::terrain::TileKind::Lava
        );
        assert!(
            join_session
                .game()
                .visual_changes
                .changed_tiles
                .contains(&crate::terrain::TilePosition { x: 18, y: 19 })
        );
        assert_eq!(
            join_session.game().online_diagnostic_controller_mode,
            "descriptor-client-connected"
        );

        host_session.game_mut().online_network_task_request =
            Some(OnlineNetworkTaskRequest::Shutdown);
        host_dispatcher.drain_and_execute(host_session.game_mut());
        assert_eq!(
            host_session.game().online_session_state,
            OnlineSessionUxState::Shutdown
        );
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS);
        assert_eq!(
            join_session.game().online_session_state,
            OnlineSessionUxState::Shutdown
        );
        assert!(
            join_session
                .game()
                .online_session_status_message
                .contains("ended by host")
        );
        let _ignored = std::fs::remove_file(unique_path);
    }

    #[test]
    fn online_task_dispatcher_reports_missing_join_descriptor_file() {
        let mut game = GameState::new();
        let mut dispatcher = OnlineTaskDispatcher::new();
        game.online_network_task_request = Some(OnlineNetworkTaskRequest::JoinDescriptorFile {
            path: std::path::PathBuf::from("/tmp/drillgame-missing-join-descriptor.json"),
        });

        dispatcher.drain_and_execute(&mut game);
        wait_for_online_completion(&mut dispatcher, &mut game);

        assert_eq!(game.online_session_state, OnlineSessionUxState::Error);
        assert!(game.message.contains("host descriptor could not be read"));
        assert!(game.message.contains("descriptor file/path"));
        assert!(game.online_network_task_request.is_none());
    }

    #[test]
    fn online_task_dispatcher_builds_tick_payload_from_live_session_state() {
        let mut dispatcher = OnlineTaskDispatcher::new();
        let session = GameSession::new();
        let input = dispatcher.live_session_tick_input(&session);
        let packet = input.command_packet.expect("command packet");
        let command = packet.commands.first().expect("sequenced command");

        assert_eq!(packet.client_id, session.local_client().client_id);
        assert_eq!(
            command.player_id,
            session.local_client().controlled_player_id
        );
        assert_eq!(command.target_tick.get(), session.current_tick().get() + 1);
        assert!(!input.snapshot.expect("snapshot").players.is_empty());
    }

    #[test]
    fn online_task_dispatcher_reports_reconnect_without_active_session() {
        let mut game = GameState::new();
        let mut dispatcher = OnlineTaskDispatcher::new();

        game.online_network_task_request = Some(OnlineNetworkTaskRequest::ReconnectDirectConnect);
        dispatcher.drain_and_execute(&mut game);

        assert_eq!(game.online_session_state, OnlineSessionUxState::Error);
        assert!(game.message.contains("Rejoin"));
    }
}
