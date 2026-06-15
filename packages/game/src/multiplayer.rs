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
pub struct CommandRejection {
    pub client_id: ClientId,
    pub player_id: PlayerId,
    pub sequence: InputSequence,
    pub reason: CommandAcceptance,
    pub authoritative_tick: SimulationTick,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CommandApplicationResponse {
    Acknowledged(CommandAcknowledgement),
    Rejected(CommandRejection),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandNetworkSession {
    tracker: CommandSequenceTracker,
    current_tick: SimulationTick,
    max_future_ticks: u64,
}

impl CommandNetworkSession {
    #[must_use]
    pub const fn new(current_tick: SimulationTick, max_future_ticks: u64) -> Self {
        Self {
            tracker: CommandSequenceTracker::new(),
            current_tick,
            max_future_ticks,
        }
    }

    pub fn apply_command_packet(
        &mut self,
        packet: &CommandPacket,
    ) -> Vec<CommandApplicationResponse> {
        packet
            .commands
            .iter()
            .map(|command| {
                let acceptance = self.tracker.accept_command(
                    packet.client_id,
                    command,
                    self.current_tick,
                    self.max_future_ticks,
                );
                if acceptance == CommandAcceptance::Accepted {
                    CommandApplicationResponse::Acknowledged(CommandAcknowledgement {
                        client_id: packet.client_id,
                        acknowledged_sequence: command.sequence,
                        authoritative_tick: self.current_tick,
                    })
                } else {
                    CommandApplicationResponse::Rejected(CommandRejection {
                        client_id: packet.client_id,
                        player_id: command.player_id,
                        sequence: command.sequence,
                        reason: acceptance,
                        authoritative_tick: self.current_tick,
                    })
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReliabilityClass {
    Reliable,
    UnreliableSequenced,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HostRuntimeMode {
    InProcessLocal,
    DedicatedServer,
    CloudSession,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostRuntimeConfig {
    pub mode: HostRuntimeMode,
    pub max_clients: u8,
    pub allow_join_in_progress: bool,
    pub allow_reconnect: bool,
}

impl Default for HostRuntimeConfig {
    fn default() -> Self {
        Self {
            mode: HostRuntimeMode::InProcessLocal,
            max_clients: 4,
            allow_join_in_progress: true,
            allow_reconnect: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ClientRuntimeMode {
    LocalInput,
    RemoteNetwork,
    Replay,
    Ai,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClientRuntimeConfig {
    pub mode: ClientRuntimeMode,
    pub client_id: ClientId,
    pub player_id: Option<PlayerId>,
}

#[must_use]
pub const fn default_local_client_runtime() -> ClientRuntimeConfig {
    ClientRuntimeConfig {
        mode: ClientRuntimeMode::LocalInput,
        client_id: LOCAL_CLIENT_ID,
        player_id: Some(LOCAL_PLAYER_ID),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TransportIntegrationStatus {
    Deferred,
    Selected,
}

#[must_use]
pub const fn transport_integration_status() -> TransportIntegrationStatus {
    TransportIntegrationStatus::Deferred
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkRuntimePlan {
    pub host: HostRuntimeConfig,
    pub local_client: ClientRuntimeConfig,
    pub transport_selected: bool,
}

impl Default for NetworkRuntimePlan {
    fn default() -> Self {
        Self {
            host: HostRuntimeConfig::default(),
            local_client: default_local_client_runtime(),
            transport_selected: false,
        }
    }
}

impl NetworkRuntimePlan {
    #[must_use]
    pub fn reliable_exchange_messages(
        &self,
        snapshot_tick: SimulationTick,
    ) -> [ProtocolMessage; 3] {
        let player_id = self.local_client.player_id.unwrap_or(LOCAL_PLAYER_ID);
        [
            ProtocolMessage::JoinRequest {
                client_id: self.local_client.client_id,
                session_token: None,
            },
            ProtocolMessage::JoinAccepted {
                client_id: self.local_client.client_id,
                player_id,
                snapshot_tick,
            },
            ProtocolMessage::TerrainChunkRequest {
                chunk_x: 0,
                chunk_y: 0,
                known_revision: 0,
            },
        ]
    }

    #[must_use]
    pub fn reconnect_messages(&self, session_token: SessionToken) -> [ProtocolMessage; 2] {
        let player_id = self.local_client.player_id.unwrap_or(LOCAL_PLAYER_ID);
        [
            ProtocolMessage::ReconnectRequest {
                client_id: self.local_client.client_id,
                session_token,
            },
            ProtocolMessage::JoinAccepted {
                client_id: self.local_client.client_id,
                player_id,
                snapshot_tick: SimulationTick::default(),
            },
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkTerrainChunkRevision {
    pub chunk_x: i32,
    pub chunk_y: i32,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NetworkDeltaPayload {
    Noop,
    TerrainChunks {
        revisions: Vec<NetworkTerrainChunkRevision>,
    },
    Players {
        players: Vec<PlayerId>,
    },
    KeyframeRequired,
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
        payload: NetworkDeltaPayload,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CommandAcceptance {
    Accepted,
    Duplicate,
    TooOld,
    TooFarInFuture,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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
pub enum ClientAuthoritativeDomain {
    Terrain,
    Cargo,
    Credits,
    Upgrades,
    Damage,
    Contracts,
    Progression,
}

#[must_use]
pub const fn client_authority_allowed(_domain: ClientAuthoritativeDomain) -> bool {
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketRecoveryAction {
    None,
    RequestChunk,
    AwaitKeyframe,
}

#[must_use]
pub const fn packet_recovery_action(
    known_revision: u64,
    authoritative_revision: u64,
    keyframe_due: bool,
) -> PacketRecoveryAction {
    if known_revision == authoritative_revision {
        PacketRecoveryAction::None
    } else if keyframe_due {
        PacketRecoveryAction::AwaitKeyframe
    } else {
        PacketRecoveryAction::RequestChunk
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandSource {
    Keyboard,
    Gamepad,
    SplitScreenClient,
    OnlineClient,
    Replay,
    Ai,
}

impl CommandSource {
    #[must_use]
    pub const fn uses_authoritative_command_path(self) -> bool {
        match self {
            Self::Keyboard
            | Self::Gamepad
            | Self::SplitScreenClient
            | Self::OnlineClient
            | Self::Replay
            | Self::Ai => true,
        }
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
pub enum DisconnectReservationPolicy {
    ReserveForReconnect,
    ReleaseImmediately,
}

#[must_use]
pub const fn disconnect_reservation_policy(underground: bool) -> DisconnectReservationPolicy {
    if underground {
        DisconnectReservationPolicy::ReserveForReconnect
    } else {
        DisconnectReservationPolicy::ReleaseImmediately
    }
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
    NearbyTickMining,
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
    let mining_ticks = mining_by_tick.keys().copied().collect::<Vec<_>>();
    if mining_ticks
        .windows(2)
        .any(|window| window[1].get().saturating_sub(window[0].get()) <= 1)
    {
        conflicts.insert(CommandConflict::NearbyTickMining);
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

#[allow(
    clippy::struct_excessive_bools,
    reason = "edge-case proof summary intentionally records checklist-style scaffold coverage"
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeCaseProofSummary {
    pub mining_conflicts_detected: bool,
    pub economy_conflict_detected: bool,
    pub underground_disconnect_reserves_player: bool,
    pub reconnect_reserves_identity: bool,
    pub join_carries_snapshot_and_player: bool,
    pub terrain_mismatch_requests_recovery: bool,
    pub command_rejections_detected: bool,
    pub prediction_and_policy_helpers_available: bool,
}

impl EdgeCaseProofSummary {
    #[must_use]
    pub const fn all_scaffolded_edges_covered(&self) -> bool {
        self.mining_conflicts_detected
            && self.economy_conflict_detected
            && self.underground_disconnect_reserves_player
            && self.reconnect_reserves_identity
            && self.join_carries_snapshot_and_player
            && self.terrain_mismatch_requests_recovery
            && self.command_rejections_detected
            && self.prediction_and_policy_helpers_available
    }
}

#[must_use]
pub fn scaffolded_edge_case_proof() -> EdgeCaseProofSummary {
    let mining_commands = [
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
            player_id: PlayerId::new(3),
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(3),
            command: PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: true,
            },
        },
        SequencedPlayerCommand {
            player_id: PlayerId::new(4),
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(4),
            command: PlayerCommand::BuyUpgrade { index: 0 },
        },
        SequencedPlayerCommand {
            player_id: PlayerId::new(5),
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(4),
            command: PlayerCommand::Repair,
        },
    ];
    let conflicts = command_conflicts(&mining_commands);
    let runtime_plan = NetworkRuntimePlan::default();
    let join_messages = runtime_plan.reliable_exchange_messages(SimulationTick::new(9));
    let mut command_session = CommandNetworkSession::new(SimulationTick::new(10), 1);
    let rejection_packet = CommandPacket {
        client_id: ClientId::new(7),
        commands: vec![SequencedPlayerCommand {
            player_id: PlayerId::new(8),
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(20),
            command: PlayerCommand::Interact,
        }],
    };

    EdgeCaseProofSummary {
        mining_conflicts_detected: conflicts.contains(&CommandConflict::SameTickMining)
            && conflicts.contains(&CommandConflict::NearbyTickMining),
        economy_conflict_detected: conflicts.contains(&CommandConflict::SimultaneousEconomyAction),
        underground_disconnect_reserves_player: disconnect_reservation_policy(true)
            == DisconnectReservationPolicy::ReserveForReconnect,
        reconnect_reserves_identity: session_continuity_decision(
            Some(SessionToken::new(4)),
            Some(SessionToken::new(4)),
        ) == SessionContinuityDecision::ReservePlayerForReconnect,
        join_carries_snapshot_and_player: matches!(
            join_messages[1],
            ProtocolMessage::JoinAccepted {
                player_id: LOCAL_PLAYER_ID,
                snapshot_tick,
                ..
            } if snapshot_tick.get() == 9
        ),
        terrain_mismatch_requests_recovery: terrain_recovery_decision(1, 2)
            == TerrainRecoveryDecision::RequestChunk,
        command_rejections_detected: matches!(
            command_session
                .apply_command_packet(&rejection_packet)
                .as_slice(),
            [CommandApplicationResponse::Rejected(_)]
        ),
        prediction_and_policy_helpers_available: initial_message_routing_policy()
            == MessageRoutingPolicy::SharedWorldLogAndPerClientHud
            && per_client_ui_policy(2) == PerClientUiPolicy::IndependentClientUi
            && host_save_decision(2) == HostSaveDecision::CoordinateConnectedClients
            && session_shutdown_decision(true, false, false) == SessionShutdownDecision::EndSession,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClientId, CommandAcceptance, CommandNetworkSession, CommandPacket, CommandSequenceTracker,
        CommandSource, InputSequence, NetworkDeltaPayload, NetworkRuntimePlan, PlayerCommand,
        PlayerId, ProtocolMessage, ReliabilityClass, SequencedPlayerCommand, SessionToken,
        SimulationTick, client_authority_allowed, command_conflicts, default_local_client_runtime,
        disconnect_reservation_policy, host_save_decision, initial_collision_policy,
        initial_discovery_sharing_policy, initial_message_routing_policy,
        initial_resource_ownership_policy, initial_transport_policy, packet_recovery_action,
        per_client_ui_policy, scaffolded_edge_case_proof, session_continuity_decision,
        session_shutdown_decision, terrain_recovery_decision, transport_integration_status,
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
            payload: NetworkDeltaPayload::Noop,
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
    fn network_runtime_plan_scaffolds_host_and_client_roles_without_transport() {
        let plan = NetworkRuntimePlan::default();
        let local_client = default_local_client_runtime();

        assert_eq!(plan.host.max_clients, 4);
        assert!(plan.host.allow_join_in_progress);
        assert!(plan.host.allow_reconnect);
        assert!(!plan.transport_selected);
        assert_eq!(
            transport_integration_status(),
            super::TransportIntegrationStatus::Deferred
        );
        assert_eq!(plan.local_client, local_client);
        assert_eq!(local_client.client_id, super::LOCAL_CLIENT_ID);
        assert_eq!(local_client.player_id, Some(super::LOCAL_PLAYER_ID));
    }

    #[test]
    fn runtime_plan_builds_join_reconnect_and_chunk_exchange_messages() {
        let plan = NetworkRuntimePlan::default();
        let join_messages = plan.reliable_exchange_messages(SimulationTick::new(44));

        assert!(
            join_messages
                .iter()
                .all(|message| { message.reliability_class() == ReliabilityClass::Reliable })
        );
        assert!(matches!(
            join_messages[0],
            ProtocolMessage::JoinRequest { .. }
        ));
        assert!(matches!(
            join_messages[1],
            ProtocolMessage::JoinAccepted { .. }
        ));
        assert!(matches!(
            join_messages[2],
            ProtocolMessage::TerrainChunkRequest { .. }
        ));

        let reconnect_messages = plan.reconnect_messages(SessionToken::new(77));
        assert!(matches!(
            reconnect_messages[0],
            ProtocolMessage::ReconnectRequest { .. }
        ));
        assert!(matches!(
            reconnect_messages[1],
            ProtocolMessage::JoinAccepted { .. }
        ));
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
    fn command_network_session_acknowledges_and_rejects_packets() {
        let client_id = ClientId::new(7);
        let player_id = PlayerId::new(9);
        let mut network_session = CommandNetworkSession::new(SimulationTick::new(10), 2);
        let accepted_packet = CommandPacket {
            client_id,
            commands: vec![SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(10),
                command: PlayerCommand::Interact,
            }],
        };
        let rejected_packet = CommandPacket {
            client_id,
            commands: vec![SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(2),
                target_tick: SimulationTick::new(20),
                command: PlayerCommand::Interact,
            }],
        };

        let accepted = network_session.apply_command_packet(&accepted_packet);
        let rejected = network_session.apply_command_packet(&rejected_packet);

        assert!(matches!(
            accepted.as_slice(),
            [super::CommandApplicationResponse::Acknowledged(_)]
        ));
        assert!(matches!(
            rejected.as_slice(),
            [super::CommandApplicationResponse::Rejected(
                super::CommandRejection {
                    reason: CommandAcceptance::TooFarInFuture,
                    ..
                }
            )]
        ));
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
                player_id: PlayerId::new(3),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(3),
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
        assert!(conflicts.contains(&super::CommandConflict::NearbyTickMining));
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
    fn underground_disconnects_are_reserved_for_reconnect() {
        assert_eq!(
            disconnect_reservation_policy(true),
            super::DisconnectReservationPolicy::ReserveForReconnect
        );
        assert_eq!(
            disconnect_reservation_policy(false),
            super::DisconnectReservationPolicy::ReleaseImmediately
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
    fn scaffolded_edge_case_proof_covers_all_design_helpers() {
        let proof = scaffolded_edge_case_proof();

        assert!(proof.all_scaffolded_edges_covered());
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

    #[test]
    fn all_command_sources_use_authoritative_command_path() {
        let sources = [
            CommandSource::Keyboard,
            CommandSource::Gamepad,
            CommandSource::SplitScreenClient,
            CommandSource::OnlineClient,
            CommandSource::Replay,
            CommandSource::Ai,
        ];

        assert!(
            sources
                .into_iter()
                .all(CommandSource::uses_authoritative_command_path)
        );
    }

    #[test]
    fn packet_recovery_uses_chunks_or_keyframes_for_revision_mismatch() {
        assert_eq!(
            packet_recovery_action(7, 7, false),
            super::PacketRecoveryAction::None
        );
        assert_eq!(
            packet_recovery_action(6, 7, false),
            super::PacketRecoveryAction::RequestChunk
        );
        assert_eq!(
            packet_recovery_action(6, 7, true),
            super::PacketRecoveryAction::AwaitKeyframe
        );
    }

    #[test]
    fn clients_are_never_authoritative_for_world_progression_domains() {
        let domains = [
            super::ClientAuthoritativeDomain::Terrain,
            super::ClientAuthoritativeDomain::Cargo,
            super::ClientAuthoritativeDomain::Credits,
            super::ClientAuthoritativeDomain::Upgrades,
            super::ClientAuthoritativeDomain::Damage,
            super::ClientAuthoritativeDomain::Contracts,
            super::ClientAuthoritativeDomain::Progression,
        ];

        assert!(
            domains
                .into_iter()
                .all(|domain| !client_authority_allowed(domain))
        );
    }
}
