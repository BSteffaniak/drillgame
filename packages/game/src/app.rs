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
    input_mapping::map_local_input,
    multiplayer::{FIXED_DELTA_SECONDS, PlayerCommand},
    rendering::GameRenderer,
    save::{load_settings, save_settings},
    session::GameSession,
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

    fn gameplay_commands_for_network_tick(
        game: &GameState,
        commands: Vec<PlayerCommand>,
    ) -> Vec<PlayerCommand> {
        if game.run_mode == crate::game_state::RunMode::Playing {
            commands
        } else {
            Vec::new()
        }
    }

    fn live_session_tick_input(
        &mut self,
        session: &mut GameSession,
        local_player_commands: Vec<PlayerCommand>,
    ) -> crate::multiplayer::QuinnSessionTickInput {
        let local_client_id = session.local_client().client_id;
        let local_player_id = session.local_client().controlled_player_id;
        let online_player_slot = session.game().online_player_slot;
        let player_id = online_player_slot.map_or(local_player_id, |slot| {
            crate::multiplayer::PlayerId::new(u64::from(slot))
        });
        let client_id = if online_player_slot == Some(2) {
            crate::multiplayer::ClientId::new(1)
        } else {
            local_client_id
        };
        let descriptor_client_connected = self
            .controller
            .as_ref()
            .is_some_and(|controller| controller.mode_label() == "descriptor-client-connected");
        if descriptor_client_connected || online_player_slot.is_some() {
            let update_existing_from_legacy = online_player_slot.is_none();
            let _seeded = session.ensure_local_online_player_presentation_from_legacy_view(
                player_id,
                update_existing_from_legacy,
            );
        }
        let sequence = self.live_tick_sequence;
        self.live_tick_sequence = self.live_tick_sequence.wrapping_add(1);
        session.live_session_tick_input_from_world(
            client_id,
            player_id,
            sequence,
            local_player_commands,
        )
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

    fn apply_accepted_remote_commands(
        session: &mut GameSession,
        summary: Option<&crate::multiplayer::CommandPacketExchangeSummary>,
    ) -> usize {
        summary.map_or(0, |summary| {
            session.apply_accepted_online_remote_commands(summary)
        })
    }

    fn sync_remote_presentations_to_session(session: &mut GameSession) -> usize {
        let remote_players = session.game().online_remote_player_snapshots.clone();
        if remote_players.is_empty() {
            return 0;
        }
        let remote_players = remote_players
            .into_iter()
            .map(|remote| crate::multiplayer::NetworkPlayerSnapshot {
                player_id: remote.player_id,
                x: remote.x,
                y: remote.y,
                velocity_x: remote.velocity_x,
                velocity_y: remote.velocity_y,
                fuel: remote.fuel,
                hull: remote.hull,
                credits: remote.credits,
                cargo_used: remote.cargo_used,
                cargo: remote.cargo,
                artifacts: remote.artifacts,
                materials: remote.materials,
                loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                scanner_cooldown_seconds: 0.0,
            })
            .collect::<Vec<_>>();
        let summary = session.apply_replicated_player_delta_to_world_presentation(
            session.current_tick(),
            &remote_players,
        );
        summary.remote_players_updated
    }

    fn drive_scheduled_tick(
        &mut self,
        session: &mut GameSession,
        delta_seconds: f32,
        local_player_commands: Vec<PlayerCommand>,
    ) {
        if self.controller.is_none() {
            self.tick_accumulator_seconds = 0.0;
            return;
        }
        self.tick_accumulator_seconds += delta_seconds;
        if self.tick_accumulator_seconds < FIXED_DELTA_SECONDS {
            return;
        }
        self.tick_accumulator_seconds = 0.0;
        let local_player_commands =
            Self::gameplay_commands_for_network_tick(session.game(), local_player_commands);
        let input = self.live_session_tick_input(session, local_player_commands);
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
            Ok(summary) => {
                let applied_remote_commands = if mode_label == "descriptor-host-accepted" {
                    Self::apply_accepted_remote_commands(session, summary.command_summary.as_ref())
                } else {
                    0
                };
                let synced_replicated_players = if mode_label == "descriptor-client-connected" {
                    let online_player_slot = session.game().online_player_slot;
                    if let Some(slot) = online_player_slot {
                        let player_id = crate::multiplayer::PlayerId::new(u64::from(slot));
                        let _synced_local = session
                            .ensure_local_online_player_presentation_from_legacy_view(
                                player_id, true,
                            );
                    }
                    Self::sync_remote_presentations_to_session(session)
                } else {
                    0
                };
                let diagnostic = match (applied_remote_commands, synced_replicated_players) {
                    (0, 0) => Self::tick_diagnostic(&summary),
                    (commands, 0) => format!(
                        "{}; applied_remote_commands={commands}",
                        Self::tick_diagnostic(&summary)
                    ),
                    (0, players) => format!(
                        "{}; synced_replicated_players={players}",
                        Self::tick_diagnostic(&summary)
                    ),
                    (commands, players) => format!(
                        "{}; applied_remote_commands={commands}; synced_replicated_players={players}",
                        Self::tick_diagnostic(&summary)
                    ),
                };
                session
                    .game_mut()
                    .apply_online_diagnostics(mode_label, diagnostic);
            }
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
        online_tasks.drive_scheduled_tick(
            &mut session,
            delta_seconds,
            mapped_input.player_commands.clone(),
        );
        session.apply_client_actions(
            crate::multiplayer::LOCAL_CLIENT_ID,
            &mapped_input.client_actions,
        );
        if mapped_input
            .client_actions
            .contains(&crate::multiplayer::ClientAction::ExitRequested)
            && session
                .game_mut()
                .request_online_shutdown_from_gameplay_exit()
        {
            online_tasks.drain_and_execute(session.game_mut());
        }
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

        let authority_update = session.update_frame_from_session_authority(input, delta_seconds);
        if session.should_exit() {
            online_tasks.drain_and_execute(session.game_mut());
        }
        let _legacy_bridge_active = authority_update.legacy_bridge_active();
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

        renderer.sync_delta(&mut raylib, &thread, session.game_mut(), &world_delta);

        let mut draw = raylib.begin_drawing(&thread);
        session.update_remote_timing_from_network_sample(0.0, 0.0);
        let _remote_timing = session.remote_timing();
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);
        let live_render_output = session.live_render_frame_output(&prediction_plan);
        renderer.render_live_frame_output(&mut draw, session.game(), &live_render_output);
    }
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
    fn live_session_tick_input_uses_real_local_player_commands() {
        let mut session = GameSession::new();
        let mut dispatcher = OnlineTaskDispatcher::new();
        let input = dispatcher.live_session_tick_input(
            &mut session,
            vec![
                PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: true,
                    drill_down: true,
                },
                PlayerCommand::UseScanner,
            ],
        );

        let packet = input.command_packet.expect("command packet is generated");
        assert_eq!(packet.commands.len(), 2);
        assert_eq!(
            packet.commands[0].command,
            PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: true,
                drill_down: true,
            }
        );
        assert_eq!(packet.commands[1].command, PlayerCommand::UseScanner);
        assert_eq!(packet.commands[0].sequence, packet.commands[1].sequence);
        assert!(input.snapshot.is_some());
        assert!(matches!(
            input.delta,
            Some((_, crate::multiplayer::NetworkDeltaPayload::Players { .. }))
        ));
        assert!(input.terrain_chunk_request.is_some());
    }

    #[test]
    fn live_session_tick_input_uses_joined_client_network_identity() {
        let mut session = GameSession::new();
        session.game_mut().online_player_slot = Some(2);
        let mut dispatcher = OnlineTaskDispatcher::new();
        let input = dispatcher.live_session_tick_input(
            &mut session,
            vec![PlayerCommand::Movement {
                horizontal: -1.0,
                thrust: false,
                drill_down: false,
            }],
        );

        let packet = input.command_packet.expect("command packet is generated");
        let command = packet.commands.first().expect("sequenced command");
        assert_eq!(packet.client_id, crate::multiplayer::ClientId::new(1));
        assert_eq!(command.player_id, crate::multiplayer::PlayerId::new(2));
    }

    #[test]
    fn accepted_remote_commands_create_online_client_and_move_remote_player() {
        let mut session = GameSession::new();
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: crate::multiplayer::ClientId::new(1),
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: session.current_tick(),
            accepted_commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: crate::multiplayer::PlayerId::new(2),
                sequence: crate::multiplayer::InputSequence::new(7),
                target_tick: session.current_tick(),
                command: PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: false,
                    drill_down: false,
                },
            }],
        };

        let applied =
            OnlineTaskDispatcher::apply_accepted_remote_commands(&mut session, Some(&summary));

        assert_eq!(applied, 1);
        assert!(session.has_client(crate::multiplayer::ClientId::new(2)));
        let remote_player = session
            .world_snapshot()
            .network_snapshot()
            .players
            .into_iter()
            .find(|player| player.player_id == crate::multiplayer::PlayerId::new(2))
            .expect("remote player exists");
        assert!(remote_player.velocity_x > 0.0 || remote_player.x > session.game().player.x);
    }

    #[test]
    fn accepted_remote_drill_commands_mine_authoritative_terrain_through_app_session_path() {
        let mut session = GameSession::new();
        let remote_client = crate::multiplayer::ClientId::new(2);
        let remote_player = crate::multiplayer::PlayerId::new(2);
        assert!(session.add_local_client_player(remote_client, remote_player));
        let mut player = session.game().player.clone();
        player.x = crate::game_state::TILE_SIZE * 10.0;
        player.y = crate::game_state::TILE_SIZE * 10.0;
        player.drill_strength = 4;
        *session
            .world_mut()
            .player_mut(remote_player)
            .expect("remote player exists") = player;
        let target = crate::terrain::TilePosition { x: 10, y: 11 };
        assert!(
            session
                .world_mut()
                .terrain_mut()
                .set_kind(target, crate::terrain::TileKind::Dirt)
        );
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: remote_client,
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: session.current_tick(),
            accepted_commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: remote_player,
                sequence: crate::multiplayer::InputSequence::new(8),
                target_tick: session.current_tick(),
                command: PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: true,
                },
            }],
        };

        let applied =
            OnlineTaskDispatcher::apply_accepted_remote_commands(&mut session, Some(&summary));
        let advance = session.advance_authoritative_world_ticks(30);
        let delta = session.drain_world_delta().compact_network_delta();

        assert_eq!(applied, 1);
        assert!(advance.terrain_events > 0);
        assert_eq!(
            session
                .world()
                .terrain()
                .tile(target)
                .expect("target tile exists")
                .kind,
            crate::terrain::TileKind::Air
        );
        assert!(matches!(
            delta,
            crate::session::CompactWorldDelta::TerrainChunks { .. }
        ));
    }

    #[test]
    fn accepted_remote_service_commands_are_host_authoritative_through_app_session_path() {
        let mut session = GameSession::new();
        let remote_client = crate::multiplayer::ClientId::new(2);
        let remote_player = crate::multiplayer::PlayerId::new(2);
        assert!(session.add_local_client_player(remote_client, remote_player));
        {
            let player = session
                .world_mut()
                .player_mut(remote_player)
                .expect("remote player exists");
            player.credits = 10_000;
            player.fuel = 1.0;
            player.hull = 1.0;
        }
        let target_tick = session.current_tick();
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: remote_client,
            acknowledged: 3,
            rejected: 0,
            authoritative_tick: target_tick,
            accepted_commands: vec![
                crate::multiplayer::SequencedPlayerCommand {
                    player_id: remote_player,
                    sequence: crate::multiplayer::InputSequence::new(9),
                    target_tick,
                    command: PlayerCommand::Refuel,
                },
                crate::multiplayer::SequencedPlayerCommand {
                    player_id: remote_player,
                    sequence: crate::multiplayer::InputSequence::new(10),
                    target_tick,
                    command: PlayerCommand::Repair,
                },
                crate::multiplayer::SequencedPlayerCommand {
                    player_id: remote_player,
                    sequence: crate::multiplayer::InputSequence::new(11),
                    target_tick,
                    command: PlayerCommand::BuyUpgrade { index: 0 },
                },
            ],
        };

        let applied =
            OnlineTaskDispatcher::apply_accepted_remote_commands(&mut session, Some(&summary));
        let player = session
            .world()
            .player(remote_player)
            .expect("remote player exists");
        let transaction_kinds = session
            .world()
            .service_transactions()
            .iter()
            .map(|transaction| transaction.kind)
            .collect::<Vec<_>>();

        assert_eq!(applied, 3);
        assert!(player.fuel > 1.0);
        assert!(player.hull > 1.0);
        assert!(transaction_kinds.contains(&crate::session::PlayerTransactionKind::Refuel));
        assert!(transaction_kinds.contains(&crate::session::PlayerTransactionKind::Repair));
        assert!(transaction_kinds.contains(&crate::session::PlayerTransactionKind::BuyUpgrade));
        assert!(session.game().player.credits < 10_000);
    }

    #[test]
    fn accepted_remote_rescue_updates_authoritative_survival_snapshot_for_replication() {
        let mut session = GameSession::new();
        let remote_client = crate::multiplayer::ClientId::new(2);
        let remote_player = crate::multiplayer::PlayerId::new(2);
        assert!(session.add_local_client_player(remote_client, remote_player));
        {
            let player = session
                .world_mut()
                .player_mut(remote_player)
                .expect("remote player exists");
            player.fuel = 0.0;
            player.hull = 0.0;
            player.x = 500.0;
            player.y = 900.0;
        }
        let before = session
            .world()
            .player_survival_snapshot(remote_player)
            .expect("survival snapshot before rescue");
        let target_tick = session.current_tick();
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: remote_client,
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: target_tick,
            accepted_commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: remote_player,
                sequence: crate::multiplayer::InputSequence::new(12),
                target_tick,
                command: PlayerCommand::Rescue,
            }],
        };

        let applied =
            OnlineTaskDispatcher::apply_accepted_remote_commands(&mut session, Some(&summary));
        let after = session
            .world()
            .player_survival_snapshot(remote_player)
            .expect("survival snapshot after rescue");
        let replicated = session
            .world_snapshot()
            .network_snapshot()
            .players
            .into_iter()
            .find(|player| player.player_id == remote_player)
            .expect("remote player replicated");

        assert_eq!(applied, 1);
        assert!(before.disabled);
        assert!(!after.disabled);
        assert!(after.fuel > before.fuel);
        assert!(after.hull > before.hull);
        assert!((replicated.fuel - after.fuel).abs() < f32::EPSILON);
        assert!((replicated.hull - after.hull).abs() < f32::EPSILON);
    }

    #[test]
    fn live_session_tick_input_requests_chunk_containing_visible_player() {
        let mut session = GameSession::new();
        session
            .world_mut()
            .player_mut(crate::multiplayer::LOCAL_PLAYER_ID)
            .expect("local world player exists")
            .x = crate::game_state::TILE_SIZE * 33.0;
        session
            .world_mut()
            .player_mut(crate::multiplayer::LOCAL_PLAYER_ID)
            .expect("local world player exists")
            .y = crate::game_state::TILE_SIZE * 18.0;
        let mut dispatcher = OnlineTaskDispatcher::new();

        let input = dispatcher.live_session_tick_input(&mut session, Vec::new());

        assert_eq!(input.terrain_chunk_request, Some((2, 1, 0, 0)));
    }

    #[test]
    fn replicated_remote_presentations_sync_into_session_world() {
        let mut session = GameSession::new();
        session.game_mut().online_remote_player_snapshots.push(
            crate::game_state::OnlineRemotePlayerPresentation {
                player_id: crate::multiplayer::PlayerId::new(2),
                x: 42.0,
                y: 84.0,
                velocity_x: 3.0,
                velocity_y: -1.0,
                fuel: 77.0,
                hull: 66.0,
                credits: 55,
                cargo_used: 4,
                cargo: std::collections::BTreeMap::new(),
                artifacts: std::collections::BTreeMap::new(),
                materials: std::collections::BTreeMap::new(),
            },
        );

        let synced = OnlineTaskDispatcher::sync_remote_presentations_to_session(&mut session);

        assert_eq!(synced, 1);
        assert!(session.has_client(crate::multiplayer::ClientId::new(2)));
        let snapshot = session.world_snapshot().network_snapshot();
        let remote = snapshot
            .players
            .iter()
            .find(|player| player.player_id == crate::multiplayer::PlayerId::new(2))
            .expect("remote player syncs into world");
        assert!((remote.x - 42.0).abs() < f32::EPSILON);
        assert!((remote.y - 84.0).abs() < f32::EPSILON);
        assert_eq!(remote.credits, 55);
    }

    #[test]
    fn live_session_tick_input_keeps_idle_movement_when_no_commands_are_available() {
        let mut session = GameSession::new();
        let mut dispatcher = OnlineTaskDispatcher::new();
        let input = dispatcher.live_session_tick_input(&mut session, Vec::new());

        let packet = input.command_packet.expect("idle packet is generated");
        assert_eq!(packet.commands.len(), 1);
        assert_eq!(
            packet.commands[0].command,
            PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: false,
            }
        );
    }

    #[test]
    fn online_task_dispatcher_executes_queued_connect_scheduled_tick_and_shutdown() {
        let mut game = GameState::new();
        let mut session = GameSession::new();
        let mut dispatcher = OnlineTaskDispatcher::new();

        game.online_network_task_request = Some(OnlineNetworkTaskRequest::HostDirectConnect);
        dispatcher.drain_and_execute(&mut game);
        wait_for_online_completion(&mut dispatcher, &mut game);
        dispatcher.drive_scheduled_tick(&mut session, FIXED_DELTA_SECONDS, Vec::new());

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
        join_session.game_mut().player.x = crate::game_state::TILE_SIZE * 18.0;
        join_session.game_mut().player.y = crate::game_state::TILE_SIZE * 19.0;
        join_session.game_mut().online_local_ready = true;
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS, Vec::new());
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
        let terrain_position = crate::terrain::TilePosition { x: 18, y: 19 };
        host_session
            .world_mut()
            .terrain_mut()
            .set_kind(terrain_position, crate::terrain::TileKind::Lava);
        let _revisions = host_session.mark_world_terrain_tiles_changed([terrain_position]);
        host_session.game_mut().terrain.set_kind(
            crate::terrain::TilePosition { x: 18, y: 19 },
            crate::terrain::TileKind::Air,
        );
        join_session.game_mut().terrain.set_kind(
            crate::terrain::TilePosition { x: 18, y: 19 },
            crate::terrain::TileKind::Air,
        );
        host_session.game_mut().online_local_ready = true;
        host_session.game_mut().run_mode = RunMode::Playing;
        host_dispatcher.drive_scheduled_tick(&mut host_session, FIXED_DELTA_SECONDS, Vec::new());
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
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS, Vec::new());
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
        let joined_world_player = join_session
            .world()
            .player(crate::multiplayer::PlayerId::new(2))
            .expect("joined client local player synced into session world");
        assert!((joined_world_player.x - 123.0).abs() < f32::EPSILON);
        assert_eq!(joined_world_player.credits, 999);
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
        join_dispatcher.drive_scheduled_tick(&mut join_session, FIXED_DELTA_SECONDS, Vec::new());
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
        let mut session = GameSession::new();
        let input = dispatcher.live_session_tick_input(&mut session, Vec::new());
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
