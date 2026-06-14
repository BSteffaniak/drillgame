use crate::{
    game_state::GameState,
    input::PlayerInput,
    multiplayer::{
        ClientId, InputSequence, LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, PlayerCommand, PlayerId,
        SequencedPlayerCommand, SimulationTick,
    },
    save::SettingsFile,
};

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
    local_client: ClientState,
    current_tick: SimulationTick,
}

impl GameSession {
    #[must_use]
    pub fn new() -> Self {
        Self {
            game: GameState::new(),
            local_client: ClientState::default(),
            current_tick: SimulationTick::default(),
        }
    }

    #[must_use]
    pub const fn game(&self) -> &GameState {
        &self.game
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

    const fn sync_client_settings_from_legacy_game(&mut self) {
        self.local_client.master_volume = self.game.master_volume;
        self.local_client.fullscreen = self.game.fullscreen;
        self.local_client.settings_dirty |= self.game.settings_dirty;
        self.local_client.exit_requested |= self.game.request_exit;
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

    pub fn update_legacy(&mut self, input: PlayerInput, delta_seconds: f32) {
        self.sync_client_settings_to_legacy_game();
        self.game.update(input, delta_seconds);
        self.sync_client_settings_from_legacy_game();
    }
}

impl Default for GameSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::multiplayer::{LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, PlayerCommand};

    use super::GameSession;

    #[test]
    fn session_starts_with_single_player_compatibility_client() {
        let session = GameSession::new();

        assert_eq!(session.local_client().client_id, LOCAL_CLIENT_ID);
        assert_eq!(session.local_client().controlled_player_id, LOCAL_PLAYER_ID);
    }

    #[test]
    fn local_commands_are_sequenced_for_future_acknowledgement() {
        let mut session = GameSession::new();

        let commands = session.sequence_local_commands(vec![PlayerCommand::Interact]);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(commands[0].sequence.get(), 0);
        assert_eq!(commands[0].target_tick, session.current_tick());
    }
}
