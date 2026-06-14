use std::{collections::BTreeMap, mem, time::Duration};

use crate::{
    game_state::GameState,
    input::PlayerInput,
    multiplayer::{
        ClientId, InputSequence, LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, PlayerCommand, PlayerId,
        SIMULATION_HZ, SequencedPlayerCommand, SimulationTick,
    },
    player::Player,
    save::SettingsFile,
    terrain::TilePosition,
};

/// Compact player data for render/network/save-adjacent synchronization experiments.
///
/// This is not a save format. It is an explicit snapshot boundary that can later be split into
/// network snapshots, render snapshots, and persistent save models as the legacy world is migrated.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSnapshot {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub fuel: f32,
    pub hull: f32,
    pub credits: u32,
}

impl PlayerSnapshot {
    #[must_use]
    pub const fn from_player(player_id: PlayerId, player: &Player) -> Self {
        Self {
            player_id,
            x: player.x,
            y: player.y,
            velocity_x: player.velocity_x,
            velocity_y: player.velocity_y,
            fuel: player.fuel,
            hull: player.hull,
            credits: player.credits,
        }
    }
}

/// Compatibility world snapshot keyed by authoritative simulation tick.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSnapshot {
    pub tick: SimulationTick,
    pub players: Vec<PlayerSnapshot>,
}

impl WorldSnapshot {
    #[must_use]
    pub fn from_world(tick: SimulationTick, world: &WorldState) -> Self {
        Self {
            tick,
            players: world.player_snapshots(),
        }
    }
}

/// Compatibility world delta emitted after a session update.
///
/// This is intentionally event-based for now. Later phases can replace or augment it with compact
/// terrain chunk revisions, entity component changes, and acknowledgement metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDelta {
    pub tick: SimulationTick,
    pub events: Vec<WorldEvent>,
}

impl WorldDelta {
    #[must_use]
    pub const fn new(tick: SimulationTick, events: Vec<WorldEvent>) -> Self {
        Self { tick, events }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Lightweight simulation events emitted by the session compatibility layer.
///
/// These are intentionally separate from save data and renderer snapshots. As systems migrate out
/// of legacy `GameState`, this event stream becomes the bridge for audio, UI, renderer dirty
/// state, and eventually network deltas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldEvent {
    TickAdvanced {
        tick: SimulationTick,
    },
    CommandsProcessed {
        tick: SimulationTick,
        command_count: usize,
    },
    TerrainRefreshRequested,
    TerrainTilesChanged {
        positions: Vec<TilePosition>,
    },
    MessageChanged {
        message: String,
    },
    PlayerChanged {
        player_id: PlayerId,
    },
    ClientExitRequested {
        client_id: ClientId,
    },
    ClientSettingsChanged {
        client_id: ClientId,
    },
}

/// Compatibility world wrapper used to introduce explicit player identity before the legacy
/// monolithic `GameState` is fully split into authoritative world state and local client state.
#[derive(Clone, Debug)]
pub struct WorldState {
    players: BTreeMap<PlayerId, Player>,
}

impl WorldState {
    #[must_use]
    pub fn from_legacy_game(game: &GameState) -> Self {
        Self {
            players: BTreeMap::from([(LOCAL_PLAYER_ID, game.player.clone())]),
        }
    }

    #[must_use]
    pub fn player(&self, player_id: PlayerId) -> Option<&Player> {
        self.players.get(&player_id)
    }

    #[must_use]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    #[must_use]
    pub fn player_snapshots(&self) -> Vec<PlayerSnapshot> {
        self.players
            .iter()
            .map(|(player_id, player)| PlayerSnapshot::from_player(*player_id, player))
            .collect()
    }

    fn sync_from_legacy_game(&mut self, game: &GameState) {
        self.players.insert(LOCAL_PLAYER_ID, game.player.clone());
    }
}

/// Local client state that is intentionally separate from authoritative gameplay state.
#[derive(Clone, Debug)]
pub struct ClientState {
    pub client_id: ClientId,
    pub controlled_player_id: PlayerId,
    pub master_volume: f32,
    pub fullscreen: bool,
    pub settings_dirty: bool,
    pub exit_requested: bool,
    next_input_sequence: InputSequence,
}

impl ClientState {
    #[must_use]
    pub const fn new(client_id: ClientId, controlled_player_id: PlayerId) -> Self {
        Self {
            client_id,
            controlled_player_id,
            master_volume: 0.8,
            fullscreen: false,
            settings_dirty: false,
            exit_requested: false,
            next_input_sequence: InputSequence::new(0),
        }
    }

    const fn next_sequence(&mut self) -> InputSequence {
        let sequence = self.next_input_sequence;
        self.next_input_sequence = self.next_input_sequence.next();
        sequence
    }
}

impl Default for ClientState {
    fn default() -> Self {
        Self::new(LOCAL_CLIENT_ID, LOCAL_PLAYER_ID)
    }
}

/// Compatibility session wrapper used while the monolithic `GameState` is split apart.
///
/// Long-term this should own `WorldState` plus one or more `ClientState` values. For now it keeps
/// the legacy `GameState` intact so single-player behavior can remain stable while new command,
/// tick, and client ownership paths are introduced.
#[derive(Clone, Debug)]
pub struct GameSession {
    game: GameState,
    world: WorldState,
    local_client: ClientState,
    current_tick: SimulationTick,
    simulation_accumulator: Duration,
    pending_commands: BTreeMap<SimulationTick, Vec<SequencedPlayerCommand>>,
    pending_events: Vec<WorldEvent>,
}

impl GameSession {
    #[must_use]
    pub fn new() -> Self {
        let game = GameState::new();
        let world = WorldState::from_legacy_game(&game);
        Self {
            game,
            world,
            local_client: ClientState::default(),
            current_tick: SimulationTick::default(),
            simulation_accumulator: Duration::ZERO,
            pending_commands: BTreeMap::new(),
            pending_events: Vec::new(),
        }
    }

    #[must_use]
    pub const fn game(&self) -> &GameState {
        &self.game
    }

    #[must_use]
    pub const fn world(&self) -> &WorldState {
        &self.world
    }

    #[must_use]
    pub fn world_snapshot(&self) -> WorldSnapshot {
        WorldSnapshot::from_world(self.current_tick, &self.world)
    }

    pub const fn game_mut(&mut self) -> &mut GameState {
        &mut self.game
    }

    #[must_use]
    pub const fn local_client(&self) -> &ClientState {
        &self.local_client
    }

    #[must_use]
    pub const fn current_tick(&self) -> SimulationTick {
        self.current_tick
    }

    #[must_use]
    pub const fn simulation_accumulator(&self) -> Duration {
        self.simulation_accumulator
    }

    pub fn accumulate_frame_delta(&mut self, delta_seconds: f32) -> u32 {
        self.simulation_accumulator += Duration::from_secs_f32(delta_seconds.max(0.0));
        let fixed_delta = Duration::from_nanos(1_000_000_000 / u64::from(SIMULATION_HZ));
        let steps = self.simulation_accumulator.as_nanos() / fixed_delta.as_nanos();
        let capped_steps = u32::try_from(steps).unwrap_or(u32::MAX);
        self.simulation_accumulator -= fixed_delta.saturating_mul(capped_steps);
        capped_steps
    }

    pub const fn advance_tick(&mut self) {
        self.current_tick = self.current_tick.next();
    }

    pub fn drain_events(&mut self) -> Vec<WorldEvent> {
        mem::take(&mut self.pending_events)
    }

    pub fn drain_world_delta(&mut self) -> WorldDelta {
        WorldDelta::new(self.current_tick, self.drain_events())
    }

    fn push_event(&mut self, event: WorldEvent) {
        self.pending_events.push(event);
    }

    pub const fn apply_settings(&mut self, settings: SettingsFile) {
        self.local_client.master_volume = settings.master_volume;
        self.local_client.fullscreen = settings.fullscreen;
        self.sync_client_settings_to_legacy_game();
    }

    #[must_use]
    pub const fn current_settings(&self) -> SettingsFile {
        SettingsFile {
            master_volume: self.local_client.master_volume,
            fullscreen: self.local_client.fullscreen,
        }
    }

    #[must_use]
    pub const fn should_exit(&self) -> bool {
        self.local_client.exit_requested || self.game.request_exit
    }

    #[must_use]
    pub const fn master_volume(&self) -> f32 {
        self.local_client.master_volume
    }

    #[must_use]
    pub const fn fullscreen(&self) -> bool {
        self.local_client.fullscreen
    }

    pub const fn take_settings_dirty(&mut self) -> bool {
        let legacy_dirty = self.game.take_settings_dirty();
        let client_dirty = self.local_client.settings_dirty;
        self.local_client.settings_dirty = false;
        legacy_dirty || client_dirty
    }

    fn sync_client_settings_from_legacy_game(&mut self) {
        let settings_changed = (self.local_client.master_volume - self.game.master_volume).abs()
            > f32::EPSILON
            || self.local_client.fullscreen != self.game.fullscreen
            || self.game.settings_dirty;
        let exit_requested = self.game.request_exit && !self.local_client.exit_requested;

        self.local_client.master_volume = self.game.master_volume;
        self.local_client.fullscreen = self.game.fullscreen;
        self.local_client.settings_dirty |= self.game.settings_dirty;
        self.local_client.exit_requested |= self.game.request_exit;

        if settings_changed {
            self.push_event(WorldEvent::ClientSettingsChanged {
                client_id: self.local_client.client_id,
            });
        }
        if exit_requested {
            self.push_event(WorldEvent::ClientExitRequested {
                client_id: self.local_client.client_id,
            });
        }
    }

    const fn sync_client_settings_to_legacy_game(&mut self) {
        self.game.master_volume = self.local_client.master_volume;
        self.game.fullscreen = self.local_client.fullscreen;
        self.game.settings_dirty = self.local_client.settings_dirty;
    }

    pub fn sequence_local_commands(
        &mut self,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        let sequenced = self.sequence_commands_for_local_player(commands);
        self.buffer_commands(sequenced.clone());
        sequenced
    }

    fn sequence_commands_for_local_player(
        &mut self,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        let player_id = self.local_client.controlled_player_id;
        let target_tick = self.current_tick;

        commands
            .into_iter()
            .map(|command| SequencedPlayerCommand {
                player_id,
                sequence: self.local_client.next_sequence(),
                target_tick,
                command,
            })
            .collect()
    }

    fn buffer_commands(&mut self, commands: Vec<SequencedPlayerCommand>) {
        for command in commands {
            self.pending_commands
                .entry(command.target_tick)
                .or_default()
                .push(command);
        }
    }

    #[must_use]
    pub fn pending_command_count(&self, tick: SimulationTick) -> usize {
        self.pending_commands.get(&tick).map_or(0, Vec::len)
    }

    pub fn drain_commands_for_tick(&mut self, tick: SimulationTick) -> Vec<SequencedPlayerCommand> {
        self.pending_commands.remove(&tick).unwrap_or_default()
    }

    pub fn update_legacy(&mut self, input: PlayerInput, delta_seconds: f32) {
        let fixed_steps = self.accumulate_frame_delta(delta_seconds);
        for _ in 0..fixed_steps {
            let tick = self.current_tick;
            let tick_commands = self.drain_commands_for_tick(tick);
            self.push_event(WorldEvent::CommandsProcessed {
                tick,
                command_count: tick_commands.len(),
            });
            self.advance_tick();
            self.push_event(WorldEvent::TickAdvanced {
                tick: self.current_tick,
            });
        }
        self.sync_client_settings_to_legacy_game();
        let previous_message = self.game.message.clone();
        let previous_player = self.game.player.clone();
        let previous_request_exit = self.game.request_exit;
        self.game.update(input, delta_seconds);
        self.capture_legacy_events(&previous_message, &previous_player, previous_request_exit);
        self.sync_client_settings_from_legacy_game();
        self.world.sync_from_legacy_game(&self.game);
    }

    fn capture_legacy_events(
        &mut self,
        previous_message: &str,
        previous_player: &Player,
        previous_request_exit: bool,
    ) {
        if previous_message != self.game.message {
            self.push_event(WorldEvent::MessageChanged {
                message: self.game.message.clone(),
            });
        }
        if previous_player != &self.game.player {
            self.push_event(WorldEvent::PlayerChanged {
                player_id: LOCAL_PLAYER_ID,
            });
        }
        if !previous_request_exit && self.game.request_exit {
            self.push_event(WorldEvent::ClientExitRequested {
                client_id: self.local_client.client_id,
            });
        }
        if self.game.visual_changes.full_terrain_refresh {
            self.push_event(WorldEvent::TerrainRefreshRequested);
        }
        if !self.game.visual_changes.changed_tiles.is_empty() {
            self.push_event(WorldEvent::TerrainTilesChanged {
                positions: self.game.visual_changes.changed_tiles.clone(),
            });
        }
    }
}

impl Default for GameSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::multiplayer::{LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, PlayerCommand};

    use super::GameSession;

    #[test]
    fn session_starts_with_single_player_compatibility_client() {
        let session = GameSession::new();

        assert_eq!(session.local_client().client_id, LOCAL_CLIENT_ID);
        assert_eq!(session.local_client().controlled_player_id, LOCAL_PLAYER_ID);
    }

    #[test]
    fn session_world_tracks_legacy_local_player() {
        let session = GameSession::new();

        assert_eq!(session.world().player_count(), 1);
        assert!(session.world().player(LOCAL_PLAYER_ID).is_some());
    }

    #[test]
    fn world_snapshot_contains_tick_and_players() {
        let session = GameSession::new();

        let snapshot = session.world_snapshot();

        assert_eq!(snapshot.tick, session.current_tick());
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].player_id, LOCAL_PLAYER_ID);
    }

    #[test]
    fn local_commands_are_sequenced_for_future_acknowledgement() {
        let mut session = GameSession::new();

        let commands = session.sequence_local_commands(vec![PlayerCommand::Interact]);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(commands[0].sequence.get(), 0);
        assert_eq!(commands[0].target_tick, session.current_tick());
        assert_eq!(session.pending_command_count(session.current_tick()), 1);
    }

    #[test]
    fn buffered_commands_are_drained_by_tick() {
        let mut session = GameSession::new();
        let tick = session.current_tick();
        session.sequence_local_commands(vec![PlayerCommand::Interact]);

        let commands = session.drain_commands_for_tick(tick);

        assert_eq!(commands.len(), 1);
        assert_eq!(session.pending_command_count(tick), 0);
    }

    #[test]
    fn frame_delta_accumulator_reports_fixed_steps() {
        let mut session = GameSession::new();

        let steps = session.accumulate_frame_delta(crate::multiplayer::FIXED_DELTA_SECONDS * 2.5);

        assert_eq!(steps, 2);
        assert!(session.simulation_accumulator() > Duration::ZERO);
    }

    #[test]
    fn advancing_tick_uses_simulation_tick_wrapper() {
        let mut session = GameSession::new();

        session.advance_tick();

        assert_eq!(session.current_tick().get(), 1);
    }

    #[test]
    fn legacy_update_emits_tick_events() {
        let mut session = GameSession::new();
        session.sequence_local_commands(vec![PlayerCommand::Interact]);

        session.update_legacy(
            crate::input::PlayerInput::default(),
            crate::multiplayer::FIXED_DELTA_SECONDS,
        );
        let events = session.drain_events();

        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::CommandsProcessed {
                command_count: 1,
                ..
            }
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::TickAdvanced { .. }))
        );
    }

    #[test]
    fn world_delta_drains_pending_events() {
        let mut session = GameSession::new();
        session.sequence_local_commands(vec![PlayerCommand::Interact]);
        session.update_legacy(
            crate::input::PlayerInput::default(),
            crate::multiplayer::FIXED_DELTA_SECONDS,
        );

        let delta = session.drain_world_delta();

        assert_eq!(delta.tick, session.current_tick());
        assert!(!delta.is_empty());
        assert!(session.drain_world_delta().is_empty());
    }
}
