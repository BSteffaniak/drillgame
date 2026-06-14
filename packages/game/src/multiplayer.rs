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

#[cfg(test)]
mod tests {
    use super::{
        ClientId, InputSequence, ProtocolMessage, ReliabilityClass, SessionToken, SimulationTick,
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
}
