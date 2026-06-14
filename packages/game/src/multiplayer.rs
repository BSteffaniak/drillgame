use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Stable identity for a player participating in an authoritative simulation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PlayerId(u64);

impl PlayerId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable identity for a local or remote client view/input source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ClientId(u64);

impl ClientId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Monotonic authoritative simulation tick.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SimulationTick(u64);

impl SimulationTick {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Monotonic per-client input sequence used for acknowledgement and reconciliation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InputSequence(u32);

impl InputSequence {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Session token placeholder used to reserve reconnect identity without tying the simulation to a
/// transport implementation yet.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SessionToken(u128);

impl SessionToken {
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

/// Single-player compatibility player id used while the current game is migrated.
pub const LOCAL_PLAYER_ID: PlayerId = PlayerId::new(1);

/// Single-player compatibility client id used while the current game is migrated.
pub const LOCAL_CLIENT_ID: ClientId = ClientId::new(1);

/// Fixed-tick simulation rate targeted by the multiplayer-ready architecture.
pub const SIMULATION_HZ: u32 = 60;

/// Fixed-tick simulation delta in seconds.
pub const FIXED_DELTA_SECONDS: f32 = 1.0 / 60.0;

/// Local/client-only actions that should not be treated as authoritative world commands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ClientAction {
    Confirm,
    Cancel,
    Pause,
    MenuUp,
    MenuDown,
    MenuLeft,
    MenuRight,
    ToggleDetails,
    Save,
    Load,
    ToggleMap,
    ToggleHelp,
    VolumeUp,
    VolumeDown,
    ToggleFullscreen,
    ExitRequested,
}

/// Authoritative gameplay commands submitted by a player.
///
/// This intentionally represents gameplay intent rather than keyboard/gamepad state so the same
/// path can be used by local input, split-screen clients, online clients, replay, or AI.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum PlayerCommand {
    Movement {
        horizontal: f32,
        thrust: bool,
        drill_down: bool,
    },
    Interact,
    Cancel,
    Confirm,
    UseScanner,
    PlaceBomb,
    PlaceInfrastructure {
        slot: u8,
    },
    SelectUpgrade {
        index: usize,
    },
    BuyUpgrade {
        index: usize,
    },
    Refuel,
    Repair,
    SellCargo,
    Rescue,
}

/// Command packet metadata needed for future network acknowledgement and reconciliation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SequencedPlayerCommand {
    pub player_id: PlayerId,
    pub sequence: InputSequence,
    pub target_tick: SimulationTick,
    pub command: PlayerCommand,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandPacket {
    pub client_id: ClientId,
    pub commands: Vec<SequencedPlayerCommand>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandAcknowledgement {
    pub client_id: ClientId,
    pub acknowledged_sequence: InputSequence,
    pub authoritative_tick: SimulationTick,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReliabilityClass {
    Reliable,
    UnreliableSequenced,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ProtocolMessage {
    JoinRequest {
        client_id: ClientId,
        session_token: Option<SessionToken>,
    },
    JoinAccepted {
        client_id: ClientId,
        player_id: PlayerId,
        snapshot_tick: SimulationTick,
    },
    ReconnectRequest {
        client_id: ClientId,
        session_token: SessionToken,
    },
    CommandPacket(CommandPacket),
    CommandAcknowledgement(CommandAcknowledgement),
    SnapshotKeyframe {
        tick: SimulationTick,
    },
    WorldDelta {
        tick: SimulationTick,
    },
    TerrainChunkRequest {
        chunk_x: i32,
        chunk_y: i32,
        known_revision: u64,
    },
    TerrainChunkResponse {
        chunk_x: i32,
        chunk_y: i32,
        revision: u64,
    },
}

impl ProtocolMessage {
    #[must_use]
    pub const fn reliability_class(&self) -> ReliabilityClass {
        match self {
            Self::CommandPacket(_) | Self::SnapshotKeyframe { .. } | Self::WorldDelta { .. } => {
                ReliabilityClass::UnreliableSequenced
            }
            Self::JoinRequest { .. }
            | Self::JoinAccepted { .. }
            | Self::ReconnectRequest { .. }
            | Self::CommandAcknowledgement(_)
            | Self::TerrainChunkRequest { .. }
            | Self::TerrainChunkResponse { .. } => ReliabilityClass::Reliable,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandAcceptance {
    Accepted,
    Duplicate,
    TooOld,
    TooFarInFuture,
}

#[derive(Clone, Debug, Default)]
pub struct CommandSequenceTracker {
    latest_sequences: BTreeMap<(ClientId, PlayerId), InputSequence>,
}

impl CommandSequenceTracker {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            latest_sequences: BTreeMap::new(),
        }
    }

    pub fn accept_command(
        &mut self,
        client_id: ClientId,
        command: &SequencedPlayerCommand,
        current_tick: SimulationTick,
        max_future_ticks: u64,
    ) -> CommandAcceptance {
        if command.target_tick < current_tick {
            return CommandAcceptance::TooOld;
        }
        if command.target_tick.get().saturating_sub(current_tick.get()) > max_future_ticks {
            return CommandAcceptance::TooFarInFuture;
        }

        let key = (client_id, command.player_id);
        if self
            .latest_sequences
            .get(&key)
            .is_some_and(|sequence| *sequence >= command.sequence)
        {
            return CommandAcceptance::Duplicate;
        }

        self.latest_sequences.insert(key, command.sequence);
        CommandAcceptance::Accepted
    }

    #[must_use]
    pub fn latest_sequence(
        &self,
        client_id: ClientId,
        player_id: PlayerId,
    ) -> Option<InputSequence> {
        self.latest_sequences.get(&(client_id, player_id)).copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRoutingPolicy {
    SharedWorldLogAndPerClientHud,
    PerClientOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceOwnershipPolicy {
    PerPlayer,
    SharedTeam,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoverySharingPolicy {
    SharedAcrossSession,
    PerPlayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollisionPolicy {
    PlayerCollisionDisabled,
    PlayerCollisionEnabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportPolicy {
    TransportAgnosticProtocolFirst,
}

#[must_use]
pub const fn initial_message_routing_policy() -> MessageRoutingPolicy {
    MessageRoutingPolicy::SharedWorldLogAndPerClientHud
}

#[must_use]
pub const fn initial_resource_ownership_policy() -> ResourceOwnershipPolicy {
    ResourceOwnershipPolicy::PerPlayer
}

#[must_use]
pub const fn initial_discovery_sharing_policy() -> DiscoverySharingPolicy {
    DiscoverySharingPolicy::SharedAcrossSession
}

#[must_use]
pub const fn initial_collision_policy() -> CollisionPolicy {
    CollisionPolicy::PlayerCollisionDisabled
}

#[must_use]
pub const fn initial_transport_policy() -> TransportPolicy {
    TransportPolicy::TransportAgnosticProtocolFirst
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerClientUiPolicy {
    SharedLegacyUi,
    IndependentClientUi,
}

#[must_use]
pub const fn per_client_ui_policy(client_count: usize) -> PerClientUiPolicy {
    if client_count <= 1 {
        PerClientUiPolicy::SharedLegacyUi
    } else {
        PerClientUiPolicy::IndependentClientUi
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostSaveDecision {
    SaveImmediately,
    CoordinateConnectedClients,
}

#[must_use]
pub const fn host_save_decision(connected_client_count: usize) -> HostSaveDecision {
    if connected_client_count <= 1 {
        HostSaveDecision::SaveImmediately
    } else {
        HostSaveDecision::CoordinateConnectedClients
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionShutdownDecision {
    Continue,
    EndSession,
    RemoveDisconnectedClient,
}

#[must_use]
pub const fn session_shutdown_decision(
    host_left: bool,
    client_left: bool,
    shutdown_requested: bool,
) -> SessionShutdownDecision {
    if shutdown_requested || host_left {
        SessionShutdownDecision::EndSession
    } else if client_left {
        SessionShutdownDecision::RemoveDisconnectedClient
    } else {
        SessionShutdownDecision::Continue
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CommandConflict {
    SameTickMining,
    SimultaneousEconomyAction,
}

#[must_use]
pub fn command_conflicts(commands: &[SequencedPlayerCommand]) -> BTreeSet<CommandConflict> {
    let mut mining_by_tick = BTreeMap::<SimulationTick, usize>::new();
    let mut economy_by_tick = BTreeMap::<SimulationTick, usize>::new();

    for command in commands {
        match command.command {
            PlayerCommand::Movement {
                drill_down: true, ..
            } => *mining_by_tick.entry(command.target_tick).or_default() += 1,
            PlayerCommand::BuyUpgrade { .. }
            | PlayerCommand::Refuel
            | PlayerCommand::Repair
            | PlayerCommand::SellCargo => {
                *economy_by_tick.entry(command.target_tick).or_default() += 1;
            }
            PlayerCommand::Movement { .. }
            | PlayerCommand::Interact
            | PlayerCommand::Cancel
            | PlayerCommand::Confirm
            | PlayerCommand::UseScanner
            | PlayerCommand::PlaceBomb
            | PlayerCommand::PlaceInfrastructure { .. }
            | PlayerCommand::SelectUpgrade { .. }
            | PlayerCommand::Rescue => {}
        }
    }

    let mut conflicts = BTreeSet::new();
    if mining_by_tick.values().any(|count| *count > 1) {
        conflicts.insert(CommandConflict::SameTickMining);
    }
    if economy_by_tick.values().any(|count| *count > 1) {
        conflicts.insert(CommandConflict::SimultaneousEconomyAction);
    }
    conflicts
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainRecoveryDecision {
    UpToDate,
    RequestChunk,
}

#[must_use]
pub const fn terrain_recovery_decision(
    known_revision: u64,
    authoritative_revision: u64,
) -> TerrainRecoveryDecision {
    if known_revision == authoritative_revision {
        TerrainRecoveryDecision::UpToDate
    } else {
        TerrainRecoveryDecision::RequestChunk
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionContinuityDecision {
    ReservePlayerForReconnect,
    AssignNewPlayer,
}

#[must_use]
pub const fn session_continuity_decision(
    known_token: Option<SessionToken>,
    reconnect_token: Option<SessionToken>,
) -> SessionContinuityDecision {
    match (known_token, reconnect_token) {
        (Some(known), Some(reconnect)) if known.get() == reconnect.get() => {
            SessionContinuityDecision::ReservePlayerForReconnect
        }
        _ => SessionContinuityDecision::AssignNewPlayer,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClientId, CommandAcceptance, CommandSequenceTracker, InputSequence, PlayerCommand,
        PlayerId, ProtocolMessage, ReliabilityClass, SequencedPlayerCommand, SessionToken,
        SimulationTick, command_conflicts, host_save_decision, initial_collision_policy,
        initial_discovery_sharing_policy, initial_message_routing_policy,
        initial_resource_ownership_policy, initial_transport_policy, per_client_ui_policy,
        session_continuity_decision, session_shutdown_decision, terrain_recovery_decision,
    };

    #[test]
    fn simulation_tick_advances_monotonically() {
        let tick = SimulationTick::new(41);

        assert_eq!(tick.next().get(), 42);
    }

    #[test]
    fn input_sequence_wraps_on_overflow() {
        let sequence = InputSequence::new(u32::MAX);

        assert_eq!(sequence.next().get(), 0);
    }

    #[test]
    fn protocol_messages_classify_reliability() {
        let command_message = ProtocolMessage::WorldDelta {
            tick: SimulationTick::new(7),
        };
        let reconnect_message = ProtocolMessage::ReconnectRequest {
            client_id: ClientId::new(3),
            session_token: SessionToken::new(99),
        };

        assert_eq!(
            command_message.reliability_class(),
            ReliabilityClass::UnreliableSequenced
        );
        assert_eq!(
            reconnect_message.reliability_class(),
            ReliabilityClass::Reliable
        );
    }

    #[test]
    fn command_sequence_tracker_rejects_duplicates_and_bad_ticks() {
        let mut tracker = CommandSequenceTracker::new();
        let client_id = ClientId::new(4);
        let player_id = PlayerId::new(8);
        let command = SequencedPlayerCommand {
            player_id,
            sequence: InputSequence::new(2),
            target_tick: SimulationTick::new(10),
            command: PlayerCommand::Interact,
        };

        assert_eq!(
            tracker.accept_command(client_id, &command, SimulationTick::new(9), 4),
            CommandAcceptance::Accepted
        );
        assert_eq!(
            tracker.accept_command(client_id, &command, SimulationTick::new(9), 4),
            CommandAcceptance::Duplicate
        );
        assert_eq!(
            tracker.accept_command(client_id, &command, SimulationTick::new(11), 4),
            CommandAcceptance::TooOld
        );

        let far_future_command = SequencedPlayerCommand {
            target_tick: SimulationTick::new(100),
            sequence: InputSequence::new(3),
            ..command
        };
        assert_eq!(
            tracker.accept_command(client_id, &far_future_command, SimulationTick::new(10), 4),
            CommandAcceptance::TooFarInFuture
        );
        assert_eq!(
            tracker.latest_sequence(client_id, player_id),
            Some(InputSequence::new(2))
        );
    }

    #[test]
    fn command_conflict_detector_flags_edge_cases() {
        let commands = vec![
            SequencedPlayerCommand {
                player_id: PlayerId::new(1),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(2),
                command: PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: true,
                },
            },
            SequencedPlayerCommand {
                player_id: PlayerId::new(2),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(2),
                command: PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: true,
                },
            },
            SequencedPlayerCommand {
                player_id: PlayerId::new(1),
                sequence: InputSequence::new(2),
                target_tick: SimulationTick::new(3),
                command: PlayerCommand::Refuel,
            },
            SequencedPlayerCommand {
                player_id: PlayerId::new(2),
                sequence: InputSequence::new(2),
                target_tick: SimulationTick::new(3),
                command: PlayerCommand::Repair,
            },
        ];

        let conflicts = command_conflicts(&commands);

        assert!(conflicts.contains(&super::CommandConflict::SameTickMining));
        assert!(conflicts.contains(&super::CommandConflict::SimultaneousEconomyAction));
    }

    #[test]
    fn terrain_recovery_detects_revision_mismatch() {
        assert_eq!(
            terrain_recovery_decision(4, 4),
            super::TerrainRecoveryDecision::UpToDate
        );
        assert_eq!(
            terrain_recovery_decision(3, 4),
            super::TerrainRecoveryDecision::RequestChunk
        );
    }

    #[test]
    fn session_continuity_uses_matching_reconnect_token() {
        let token = SessionToken::new(123);

        assert_eq!(
            session_continuity_decision(Some(token), Some(token)),
            super::SessionContinuityDecision::ReservePlayerForReconnect
        );
        assert_eq!(
            session_continuity_decision(Some(token), Some(SessionToken::new(999))),
            super::SessionContinuityDecision::AssignNewPlayer
        );
    }

    #[test]
    fn split_screen_ui_policy_changes_for_multiple_clients() {
        assert_eq!(
            per_client_ui_policy(1),
            super::PerClientUiPolicy::SharedLegacyUi
        );
        assert_eq!(
            per_client_ui_policy(2),
            super::PerClientUiPolicy::IndependentClientUi
        );
    }

    #[test]
    fn host_save_coordinates_when_clients_are_connected() {
        assert_eq!(
            host_save_decision(1),
            super::HostSaveDecision::SaveImmediately
        );
        assert_eq!(
            host_save_decision(2),
            super::HostSaveDecision::CoordinateConnectedClients
        );
    }

    #[test]
    fn session_shutdown_policy_handles_host_and_client_leaves() {
        assert_eq!(
            session_shutdown_decision(false, false, false),
            super::SessionShutdownDecision::Continue
        );
        assert_eq!(
            session_shutdown_decision(false, true, false),
            super::SessionShutdownDecision::RemoveDisconnectedClient
        );
        assert_eq!(
            session_shutdown_decision(true, false, false),
            super::SessionShutdownDecision::EndSession
        );
        assert_eq!(
            session_shutdown_decision(false, false, true),
            super::SessionShutdownDecision::EndSession
        );
    }

    #[test]
    fn initial_multiplayer_policy_decisions_are_explicit() {
        assert_eq!(
            initial_message_routing_policy(),
            super::MessageRoutingPolicy::SharedWorldLogAndPerClientHud
        );
        assert_eq!(
            initial_resource_ownership_policy(),
            super::ResourceOwnershipPolicy::PerPlayer
        );
        assert_eq!(
            initial_discovery_sharing_policy(),
            super::DiscoverySharingPolicy::SharedAcrossSession
        );
        assert_eq!(
            initial_collision_policy(),
            super::CollisionPolicy::PlayerCollisionDisabled
        );
        assert_eq!(
            initial_transport_policy(),
            super::TransportPolicy::TransportAgnosticProtocolFirst
        );
    }
}
