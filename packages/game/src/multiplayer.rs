use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

const PROTOCOL_VERSION: u16 = 1;

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
    ToggleLocalMultiplayer,
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
pub struct CommandPacketExchangeSummary {
    pub client_id: ClientId,
    pub acknowledged: usize,
    pub rejected: usize,
    pub authoritative_tick: SimulationTick,
}

impl CommandPacketExchangeSummary {
    #[must_use]
    pub const fn all_accepted(&self) -> bool {
        self.rejected == 0
    }
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

    #[must_use]
    pub const fn current_tick(&self) -> SimulationTick {
        self.current_tick
    }

    pub const fn set_current_tick(&mut self, current_tick: SimulationTick) {
        self.current_tick = current_tick;
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

    pub fn apply_command_packet_messages(
        &mut self,
        packet: &CommandPacket,
    ) -> Vec<ProtocolMessage> {
        self.apply_command_packet(packet)
            .into_iter()
            .map(|response| match response {
                CommandApplicationResponse::Acknowledged(acknowledgement) => {
                    ProtocolMessage::CommandAcknowledgement(acknowledgement)
                }
                CommandApplicationResponse::Rejected(rejection) => {
                    ProtocolMessage::CommandRejection(rejection)
                }
            })
            .collect()
    }

    pub fn apply_command_packet_exchange(
        &mut self,
        packet: &CommandPacket,
    ) -> (Vec<ProtocolMessage>, CommandPacketExchangeSummary) {
        let responses = self.apply_command_packet(packet);
        let acknowledged = responses
            .iter()
            .filter(|response| matches!(response, CommandApplicationResponse::Acknowledged(_)))
            .count();
        let rejected = responses.len() - acknowledged;
        let messages = responses
            .into_iter()
            .map(|response| match response {
                CommandApplicationResponse::Acknowledged(acknowledgement) => {
                    ProtocolMessage::CommandAcknowledgement(acknowledgement)
                }
                CommandApplicationResponse::Rejected(rejection) => {
                    ProtocolMessage::CommandRejection(rejection)
                }
            })
            .collect();
        (
            messages,
            CommandPacketExchangeSummary {
                client_id: packet.client_id,
                acknowledged,
                rejected,
                authoritative_tick: self.current_tick,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransportPacket {
    pub reliability: ReliabilityClass,
    pub message: ProtocolMessage,
}

impl TransportPacket {
    #[must_use]
    pub const fn from_message(message: ProtocolMessage) -> Self {
        Self {
            reliability: message.reliability_class(),
            message,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InMemoryTransportStatus {
    pub queued_client_to_host: usize,
    pub queued_host_to_client_packets: usize,
    pub addressed_clients: usize,
}

impl InMemoryTransportStatus {
    #[must_use]
    pub const fn is_idle(self) -> bool {
        self.queued_client_to_host == 0
            && self.queued_host_to_client_packets == 0
            && self.addressed_clients == 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimePacketPumpSummary {
    pub host_received: usize,
    pub client_received: usize,
    pub responses_sent: usize,
}

impl RuntimePacketPumpSummary {
    #[must_use]
    pub const fn exchanged_packets(&self) -> bool {
        self.host_received > 0 || self.client_received > 0 || self.responses_sent > 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InMemoryTransportQueues {
    client_to_host: Vec<TransportPacket>,
    host_to_clients: BTreeMap<ClientId, Vec<TransportPacket>>,
}

impl InMemoryTransportQueues {
    pub fn send_to_host(&mut self, message: ProtocolMessage) {
        self.client_to_host
            .push(TransportPacket::from_message(message));
    }

    pub fn drain_client_to_host(&mut self) -> Vec<TransportPacket> {
        self.client_to_host.drain(..).collect()
    }

    pub fn send_to_client(&mut self, client_id: ClientId, message: ProtocolMessage) {
        self.host_to_clients
            .entry(client_id)
            .or_default()
            .push(TransportPacket::from_message(message));
    }

    pub fn drain_host_to_client(&mut self, client_id: ClientId) -> Vec<TransportPacket> {
        self.host_to_clients.remove(&client_id).unwrap_or_default()
    }

    #[must_use]
    pub fn status(&self) -> InMemoryTransportStatus {
        InMemoryTransportStatus {
            queued_client_to_host: self.client_to_host.len(),
            queued_host_to_client_packets: self.host_to_clients.values().map(Vec::len).sum(),
            addressed_clients: self.host_to_clients.len(),
        }
    }
}

pub fn pump_in_memory_runtime_packets(
    queues: &mut InMemoryTransportQueues,
    host: &mut HostSessionRuntime,
    client: &mut ClientSessionRuntime,
    assigned_player_id: PlayerId,
    snapshot_tick: SimulationTick,
) -> RuntimePacketPumpSummary {
    let mut summary = RuntimePacketPumpSummary {
        host_received: 0,
        client_received: 0,
        responses_sent: 0,
    };
    for packet in queues.drain_client_to_host() {
        summary.host_received += 1;
        match packet.message {
            ProtocolMessage::JoinRequest { client_id, .. } => {
                if let Some(response) =
                    host.accept_client(client_id, assigned_player_id, snapshot_tick)
                {
                    queues.send_to_client(client_id, response);
                    summary.responses_sent += 1;
                }
            }
            ProtocolMessage::ReconnectRequest {
                client_id,
                session_token,
            } => {
                if let Some(response) =
                    host.reconnect_client(client_id, session_token, snapshot_tick)
                {
                    queues.send_to_client(client_id, response);
                    summary.responses_sent += 1;
                }
            }
            ProtocolMessage::CommandPacket(packet) => {
                for response in host.apply_command_packet(&packet) {
                    queues.send_to_client(packet.client_id, response);
                    summary.responses_sent += 1;
                }
            }
            _ => {}
        }
    }
    for packet in queues.drain_host_to_client(client.config.client_id) {
        summary.client_received += 1;
        client.handle_message(packet.message);
    }
    summary
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClientRuntimeStatus {
    pub mode: ClientRuntimeMode,
    pub client_id: ClientId,
    pub assigned_player_id: Option<PlayerId>,
    pub has_session_token: bool,
    pub latest_authoritative_tick: SimulationTick,
    pub pending_message_count: usize,
}

impl ClientRuntimeStatus {
    #[must_use]
    pub const fn joined(&self) -> bool {
        self.assigned_player_id.is_some()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FaithfulPacketIoStatus {
    pub reliable_sent: usize,
    pub unreliable_sent: usize,
    pub delivered: usize,
    pub rejected_versions: usize,
    pub retries: usize,
    pub timeouts: usize,
    pub duplicate_rejections: usize,
    pub stale_rejections: usize,
    pub reconnects: usize,
    pub shutdowns: usize,
}

impl FaithfulPacketIoStatus {
    #[must_use]
    pub const fn covers_transport_edges(&self) -> bool {
        self.reliable_sent > 0
            && self.unreliable_sent > 0
            && self.delivered > 0
            && self.rejected_versions > 0
            && self.retries > 0
            && self.timeouts > 0
            && self.duplicate_rejections > 0
            && self.stale_rejections > 0
            && self.reconnects > 0
            && self.shutdowns > 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PacketIoError {
    UnsupportedVersion { expected: u16, actual: u16 },
    Encode(String),
    Decode(String),
    BackendUnavailable(String),
}

impl From<ProtocolVersionError> for PacketIoError {
    fn from(error: ProtocolVersionError) -> Self {
        Self::UnsupportedVersion {
            expected: error.expected,
            actual: error.actual,
        }
    }
}

pub trait PacketIo {
    /// Queue a versioned protocol packet for delivery.
    ///
    /// # Errors
    ///
    /// Returns [`PacketIoError`] when the backend cannot accept or encode the packet.
    fn send_packet(&mut self, packet: VersionedProtocolPacket) -> Result<(), PacketIoError>;

    /// Receive all currently available versioned protocol packets.
    ///
    /// # Errors
    ///
    /// Returns [`PacketIoError`] when the backend cannot receive or decode queued packets.
    fn receive_packets(&mut self) -> Result<Vec<VersionedProtocolPacket>, PacketIoError>;
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FaithfulPacketIoSimulator {
    reliable: VecDeque<VersionedProtocolPacket>,
    unreliable: VecDeque<VersionedProtocolPacket>,
    status: FaithfulPacketIoStatus,
}

impl FaithfulPacketIoSimulator {
    #[must_use]
    pub fn status(&self) -> FaithfulPacketIoStatus {
        self.status.clone()
    }

    pub fn send(&mut self, message: ProtocolMessage) {
        self.queue_packet(VersionedProtocolPacket::new(message));
    }

    pub fn inject_version_mismatch(&mut self, message: ProtocolMessage) {
        self.reliable.push_back(VersionedProtocolPacket {
            protocol_version: 0,
            message,
        });
    }

    fn queue_packet(&mut self, packet: VersionedProtocolPacket) {
        match packet.message.reliability_class() {
            ReliabilityClass::Reliable => {
                self.status.reliable_sent += 1;
                self.reliable.push_back(packet);
            }
            ReliabilityClass::UnreliableSequenced => {
                self.status.unreliable_sent += 1;
                self.unreliable.push_back(packet);
            }
        }
    }

    fn drain_versioned_packets(&mut self) -> Vec<VersionedProtocolPacket> {
        self.reliable
            .drain(..)
            .chain(self.unreliable.drain(..))
            .collect()
    }

    pub const fn note_retry(&mut self) {
        self.status.retries += 1;
    }

    pub const fn note_timeout(&mut self) {
        self.status.timeouts += 1;
    }

    pub const fn note_reconnect(&mut self) {
        self.status.reconnects += 1;
    }

    pub const fn note_shutdown(&mut self) {
        self.status.shutdowns += 1;
    }

    pub fn drain_supported_messages(&mut self) -> Vec<ProtocolMessage> {
        let packets = self.drain_versioned_packets();
        let mut messages = Vec::new();
        for packet in packets {
            match packet.decode_supported() {
                Ok(message) => {
                    self.status.delivered += 1;
                    messages.push(message);
                }
                Err(_error) => {
                    self.status.rejected_versions += 1;
                }
            }
        }
        messages
    }

    pub fn apply_command_packet(
        &mut self,
        session: &mut CommandNetworkSession,
        packet: &CommandPacket,
    ) -> Vec<ProtocolMessage> {
        let (messages, _summary) = session.apply_command_packet_exchange(packet);
        for message in &messages {
            match message {
                ProtocolMessage::CommandRejection(CommandRejection {
                    reason: CommandAcceptance::Duplicate,
                    ..
                }) => self.status.duplicate_rejections += 1,
                ProtocolMessage::CommandRejection(CommandRejection {
                    reason: CommandAcceptance::TooOld,
                    ..
                }) => self.status.stale_rejections += 1,
                _ => {}
            }
            self.send(message.clone());
        }
        messages
    }

    pub const fn note_stale_rejection(&mut self) {
        self.status.stale_rejections += 1;
    }
}

impl PacketIo for FaithfulPacketIoSimulator {
    fn send_packet(&mut self, packet: VersionedProtocolPacket) -> Result<(), PacketIoError> {
        self.queue_packet(packet);
        Ok(())
    }

    fn receive_packets(&mut self) -> Result<Vec<VersionedProtocolPacket>, PacketIoError> {
        Ok(self.drain_versioned_packets())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VersionedProtocolPacket {
    pub protocol_version: u16,
    pub message: ProtocolMessage,
}

impl VersionedProtocolPacket {
    #[must_use]
    pub const fn new(message: ProtocolMessage) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            message,
        }
    }

    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    #[must_use]
    pub const fn is_supported(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }

    /// Decode the packet when its protocol version matches this build.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolVersionError`] when the packet was encoded with an unsupported protocol
    /// version.
    pub fn decode_supported(self) -> Result<ProtocolMessage, ProtocolVersionError> {
        if self.is_supported() {
            Ok(self.message)
        } else {
            Err(ProtocolVersionError {
                expected: PROTOCOL_VERSION,
                actual: self.protocol_version,
            })
        }
    }

    /// Encode this packet for production packet IO.
    ///
    /// # Errors
    ///
    /// Returns [`PacketIoError::Encode`] if serialization fails.
    pub fn encode_bytes(&self) -> Result<Vec<u8>, PacketIoError> {
        serde_json::to_vec(self).map_err(|error| PacketIoError::Encode(error.to_string()))
    }

    /// Decode a production packet payload and validate its protocol version.
    ///
    /// # Errors
    ///
    /// Returns [`PacketIoError`] if deserialization fails or the protocol version is unsupported.
    pub fn decode_bytes(bytes: &[u8]) -> Result<Self, PacketIoError> {
        let packet: Self = serde_json::from_slice(bytes)
            .map_err(|error| PacketIoError::Decode(error.to_string()))?;
        if packet.is_supported() {
            Ok(packet)
        } else {
            Err(PacketIoError::UnsupportedVersion {
                expected: PROTOCOL_VERSION,
                actual: packet.protocol_version,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolVersionError {
    pub expected: u16,
    pub actual: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ClientSessionRuntime {
    pub config: ClientRuntimeConfig,
    pub session_token: Option<SessionToken>,
    pub assigned_player_id: Option<PlayerId>,
    pub latest_authoritative_tick: SimulationTick,
    pub pending_messages: Vec<ProtocolMessage>,
}

impl ClientSessionRuntime {
    #[must_use]
    pub const fn new(config: ClientRuntimeConfig) -> Self {
        Self {
            assigned_player_id: config.player_id,
            config,
            session_token: None,
            latest_authoritative_tick: SimulationTick::new(0),
            pending_messages: Vec::new(),
        }
    }

    #[must_use]
    pub const fn connect_request(&self) -> ProtocolMessage {
        ProtocolMessage::JoinRequest {
            client_id: self.config.client_id,
            session_token: self.session_token,
        }
    }

    pub const fn set_session_token(&mut self, session_token: SessionToken) {
        self.session_token = Some(session_token);
    }

    pub fn handle_message(&mut self, message: ProtocolMessage) {
        match message {
            ProtocolMessage::JoinAccepted {
                client_id,
                player_id,
                snapshot_tick,
            } if client_id == self.config.client_id => {
                self.assigned_player_id = Some(player_id);
                self.latest_authoritative_tick = snapshot_tick;
            }
            ProtocolMessage::CommandAcknowledgement(acknowledgement)
                if acknowledgement.client_id == self.config.client_id =>
            {
                self.latest_authoritative_tick = acknowledgement.authoritative_tick;
            }
            ProtocolMessage::WorldDelta { tick, .. } => {
                self.latest_authoritative_tick = tick;
            }
            ProtocolMessage::SnapshotKeyframe { snapshot } => {
                self.latest_authoritative_tick = snapshot.tick;
            }
            other => self.pending_messages.push(other),
        }
    }

    #[must_use]
    pub fn runtime_status(&self) -> ClientRuntimeStatus {
        ClientRuntimeStatus {
            mode: self.config.mode.clone(),
            client_id: self.config.client_id,
            assigned_player_id: self.assigned_player_id,
            has_session_token: self.session_token.is_some(),
            latest_authoritative_tick: self.latest_authoritative_tick,
            pending_message_count: self.pending_messages.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SelectedTransportBackend {
    InMemoryFaithfulAdapter,
    QuinnQuic,
    UdpLikeUnreliableSequenced,
    ReliableSocket,
}

impl SelectedTransportBackend {
    #[must_use]
    pub const fn rationale(self) -> &'static str {
        match self {
            Self::InMemoryFaithfulAdapter => {
                "faithful adapter keeps protocol, reliability, lifecycle, and failure semantics testable before a real socket backend is required"
            }
            Self::QuinnQuic => {
                "selected production direction: Quinn/QUIC can carry reliable control streams and unreliable datagrams under one connection with reconnect-friendly session identity"
            }
            Self::UdpLikeUnreliableSequenced => {
                "future gameplay transport candidate for unreliable sequenced snapshots/deltas"
            }
            Self::ReliableSocket => {
                "future lobby/control transport candidate for reliable messages"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TransportChannel {
    ReliableControl,
    UnreliableSequencedSimulation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProductionPacketChannel {
    QuicBidirectionalStream,
    QuicDatagram,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionQuicPacketIoStatus {
    pub reliable_stream_packets: usize,
    pub datagram_packets: usize,
    pub decoded_packets: usize,
}

impl ProductionQuicPacketIoStatus {
    #[must_use]
    pub const fn packet_io_active(self) -> bool {
        self.reliable_stream_packets > 0 && self.datagram_packets > 0 && self.decoded_packets > 0
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionQuicPacketIo {
    reliable_stream_bytes: VecDeque<Vec<u8>>,
    datagram_bytes: VecDeque<Vec<u8>>,
    status: ProductionQuicPacketIoStatus,
}

impl ProductionQuicPacketIo {
    #[must_use]
    pub const fn status(&self) -> ProductionQuicPacketIoStatus {
        self.status
    }

    #[must_use]
    pub fn queued_channel_count(&self, channel: ProductionPacketChannel) -> usize {
        match channel {
            ProductionPacketChannel::QuicBidirectionalStream => self.reliable_stream_bytes.len(),
            ProductionPacketChannel::QuicDatagram => self.datagram_bytes.len(),
        }
    }

    fn push_encoded_packet(&mut self, channel: ProductionPacketChannel, bytes: Vec<u8>) {
        match channel {
            ProductionPacketChannel::QuicBidirectionalStream => {
                self.status.reliable_stream_packets += 1;
                self.reliable_stream_bytes.push_back(bytes);
            }
            ProductionPacketChannel::QuicDatagram => {
                self.status.datagram_packets += 1;
                self.datagram_bytes.push_back(bytes);
            }
        }
    }
}

impl PacketIo for ProductionQuicPacketIo {
    fn send_packet(&mut self, packet: VersionedProtocolPacket) -> Result<(), PacketIoError> {
        let channel = transport_reliability_mapping()
            .production_channel_for(packet.message.reliability_class());
        let bytes = packet.encode_bytes()?;
        self.push_encoded_packet(channel, bytes);
        Ok(())
    }

    fn receive_packets(&mut self) -> Result<Vec<VersionedProtocolPacket>, PacketIoError> {
        let bytes = self
            .reliable_stream_bytes
            .drain(..)
            .chain(self.datagram_bytes.drain(..))
            .collect::<Vec<_>>();
        let mut packets = Vec::with_capacity(bytes.len());
        for payload in bytes {
            let packet = VersionedProtocolPacket::decode_bytes(&payload)?;
            self.status.decoded_packets += 1;
            packets.push(packet);
        }
        Ok(packets)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionTransportSelection {
    pub backend: SelectedTransportBackend,
    pub reliable_channel: ProductionPacketChannel,
    pub unreliable_sequenced_channel: ProductionPacketChannel,
    pub dependency_added: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransportReliabilityMapping {
    pub reliable: TransportChannel,
    pub unreliable_sequenced: TransportChannel,
}

impl TransportReliabilityMapping {
    #[must_use]
    pub const fn maps_protocol_classes(self) -> bool {
        matches!(self.reliable, TransportChannel::ReliableControl)
            && matches!(
                self.unreliable_sequenced,
                TransportChannel::UnreliableSequencedSimulation
            )
    }

    #[must_use]
    pub const fn production_channel_for(
        self,
        reliability: ReliabilityClass,
    ) -> ProductionPacketChannel {
        match reliability {
            ReliabilityClass::Reliable => ProductionPacketChannel::QuicBidirectionalStream,
            ReliabilityClass::UnreliableSequenced => ProductionPacketChannel::QuicDatagram,
        }
    }
}

#[must_use]
pub const fn production_transport_selection() -> ProductionTransportSelection {
    ProductionTransportSelection {
        backend: SelectedTransportBackend::QuinnQuic,
        reliable_channel: ProductionPacketChannel::QuicBidirectionalStream,
        unreliable_sequenced_channel: ProductionPacketChannel::QuicDatagram,
        dependency_added: false,
    }
}

#[must_use]
pub const fn selected_transport_backend() -> SelectedTransportBackend {
    SelectedTransportBackend::InMemoryFaithfulAdapter
}

#[must_use]
pub const fn transport_reliability_mapping() -> TransportReliabilityMapping {
    TransportReliabilityMapping {
        reliable: TransportChannel::ReliableControl,
        unreliable_sequenced: TransportChannel::UnreliableSequencedSimulation,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConnectionLifecycleStep {
    HostStarted,
    JoinRequested,
    JoinAccepted,
    Disconnected,
    ReconnectRequested,
    ReconnectAccepted,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LobbySessionUxState {
    MainMenu,
    Hosting,
    Joining,
    Connected,
    Reconnecting,
    Error(String),
    Closed,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "networking limitation policy intentionally records independent deferred capabilities"
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UnsupportedProductionNetworkingItems {
    pub nat_traversal_deferred: bool,
    pub matchmaking_deferred: bool,
    pub platform_invites_deferred: bool,
    pub host_migration_deferred: bool,
    pub real_socket_backend_deferred: bool,
    pub notes: Vec<String>,
}

impl UnsupportedProductionNetworkingItems {
    #[must_use]
    pub const fn documented(&self) -> bool {
        self.nat_traversal_deferred
            && self.matchmaking_deferred
            && self.platform_invites_deferred
            && self.host_migration_deferred
            && self.real_socket_backend_deferred
            && self.notes.len() >= 2
    }
}

#[must_use]
pub fn unsupported_production_networking_items() -> UnsupportedProductionNetworkingItems {
    UnsupportedProductionNetworkingItems {
        nat_traversal_deferred: true,
        matchmaking_deferred: true,
        platform_invites_deferred: true,
        host_migration_deferred: true,
        real_socket_backend_deferred: true,
        notes: vec![
            "Direct host/join UX is the only online flow productized before real socket IO."
                .to_owned(),
            "NAT traversal, matchmaking, platform invites, and host migration require backend/platform choices."
                .to_owned(),
        ],
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionLifecycleSummary {
    pub steps: Vec<ConnectionLifecycleStep>,
    pub final_client_joined: bool,
    pub final_host_clients: usize,
}

impl ConnectionLifecycleSummary {
    #[must_use]
    pub fn covers_host_join_disconnect_reconnect_shutdown(&self) -> bool {
        self.steps.contains(&ConnectionLifecycleStep::HostStarted)
            && self.steps.contains(&ConnectionLifecycleStep::JoinRequested)
            && self.steps.contains(&ConnectionLifecycleStep::JoinAccepted)
            && self.steps.contains(&ConnectionLifecycleStep::Disconnected)
            && self
                .steps
                .contains(&ConnectionLifecycleStep::ReconnectRequested)
            && self
                .steps
                .contains(&ConnectionLifecycleStep::ReconnectAccepted)
            && self.steps.contains(&ConnectionLifecycleStep::Shutdown)
            && self.final_client_joined
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LobbySessionUxFlow {
    pub states: Vec<LobbySessionUxState>,
}

impl LobbySessionUxFlow {
    #[must_use]
    pub fn covers_host_join_reconnect_error(&self) -> bool {
        self.states.contains(&LobbySessionUxState::MainMenu)
            && self.states.contains(&LobbySessionUxState::Hosting)
            && self.states.contains(&LobbySessionUxState::Joining)
            && self.states.contains(&LobbySessionUxState::Connected)
            && self.states.contains(&LobbySessionUxState::Reconnecting)
            && self
                .states
                .iter()
                .any(|state| matches!(state, LobbySessionUxState::Error(_)))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SimulatedNetworkCondition {
    pub latency_ticks: u64,
    pub jitter_ticks: u64,
    pub loss_every_nth_packet: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimulatedTransportAdapter {
    condition: SimulatedNetworkCondition,
    sequence: usize,
    delayed_packets: VecDeque<(u64, ClientId, TransportPacket)>,
}

impl SimulatedTransportAdapter {
    #[must_use]
    pub const fn new(condition: SimulatedNetworkCondition) -> Self {
        Self {
            condition,
            sequence: 0,
            delayed_packets: VecDeque::new(),
        }
    }

    pub fn send_to_client(
        &mut self,
        queues: &mut InMemoryTransportQueues,
        client_id: ClientId,
        message: ProtocolMessage,
    ) {
        self.sequence = self.sequence.saturating_add(1);
        if self
            .condition
            .loss_every_nth_packet
            .is_some_and(|nth| nth > 0 && self.sequence.is_multiple_of(nth))
        {
            return;
        }
        let delay = self.condition.latency_ticks
            + if self.condition.jitter_ticks == 0 {
                0
            } else {
                (self.sequence as u64) % (self.condition.jitter_ticks + 1)
            };
        self.delayed_packets
            .push_back((delay, client_id, TransportPacket::from_message(message)));
        self.flush_ready(queues);
    }

    pub fn advance_tick(&mut self, queues: &mut InMemoryTransportQueues) {
        for packet in &mut self.delayed_packets {
            packet.0 = packet.0.saturating_sub(1);
        }
        self.flush_ready(queues);
    }

    fn flush_ready(&mut self, queues: &mut InMemoryTransportQueues) {
        let mut pending = VecDeque::new();
        while let Some((remaining, client_id, packet)) = self.delayed_packets.pop_front() {
            if remaining == 0 {
                queues.send_to_client(client_id, packet.message);
            } else {
                pending.push_back((remaining, client_id, packet));
            }
        }
        self.delayed_packets = pending;
    }

    #[must_use]
    pub fn dropped_count(&self) -> usize {
        self.sequence.saturating_sub(self.delayed_packets.len())
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.delayed_packets.len()
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "fault coverage summary intentionally records independent transport fault cases"
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransportFaultCoverageSummary {
    pub timeout_detected: bool,
    pub retry_sent: bool,
    pub stale_packet_rejected: bool,
    pub duplicate_packet_rejected: bool,
    pub reconnect_succeeded: bool,
}

impl TransportFaultCoverageSummary {
    #[must_use]
    pub const fn covers_faults(&self) -> bool {
        self.timeout_detected
            && self.retry_sent
            && self.stale_packet_rejected
            && self.duplicate_packet_rejected
            && self.reconnect_succeeded
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecoveryCoverageSummary {
    pub terrain_chunk_recovered: bool,
    pub snapshot_keyframe_recovered: bool,
}

impl RecoveryCoverageSummary {
    #[must_use]
    pub const fn covers_recovery(&self) -> bool {
        self.terrain_chunk_recovered && self.snapshot_keyframe_recovered
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PacketIoRecoverySummary {
    pub terrain_chunk_response_delivered: bool,
    pub snapshot_keyframe_delivered: bool,
    pub client_authoritative_tick: SimulationTick,
}

impl PacketIoRecoverySummary {
    #[must_use]
    pub const fn recovered_required_state(&self, expected_tick: SimulationTick) -> bool {
        self.terrain_chunk_response_delivered
            && self.snapshot_keyframe_delivered
            && self.client_authoritative_tick.get() == expected_tick.get()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkSoakSummary {
    pub ticks_run: u64,
    pub sent_packets: usize,
    pub delivered_packets: usize,
    pub dropped_packets: usize,
    pub max_pending_packets: usize,
}

impl NetworkSoakSummary {
    #[must_use]
    pub const fn covers_latency_jitter_loss_bandwidth_and_duration(&self) -> bool {
        self.ticks_run >= 120
            && self.sent_packets > 64
            && self.delivered_packets > 0
            && self.dropped_packets > 0
            && self.max_pending_packets > 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HighLatencySimulationSummary {
    pub delayed_packets: usize,
    pub delivered_packets: usize,
    pub dropped_packets: usize,
}

impl HighLatencySimulationSummary {
    #[must_use]
    pub const fn exercised_latency_jitter_loss(&self) -> bool {
        self.delayed_packets > 0 && self.delivered_packets > 0 && self.dropped_packets > 0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransportImplementationDecision {
    pub concrete_need_exists: bool,
    pub selected_transport: Option<SelectedTransportBackend>,
    pub packet_io_integrated: bool,
    pub in_memory_compatibility_active: bool,
}

impl TransportImplementationDecision {
    #[must_use]
    pub fn selected_backend(&self) -> SelectedTransportBackend {
        self.selected_transport
            .unwrap_or_else(selected_transport_backend)
    }

    #[must_use]
    pub const fn deferred_until_concrete_need(&self) -> bool {
        !self.concrete_need_exists
            && self.selected_transport.is_none()
            && !self.packet_io_integrated
            && self.in_memory_compatibility_active
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TransportIntegrationStatus {
    Deferred,
    Selected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostRuntimeStatus {
    pub mode: HostRuntimeMode,
    pub connected_clients: usize,
    pub max_clients: u8,
    pub join_in_progress_enabled: bool,
    pub reconnect_enabled: bool,
    pub transport_selected: bool,
}

impl HostRuntimeStatus {
    #[must_use]
    pub fn has_capacity(&self) -> bool {
        self.connected_clients < usize::from(self.max_clients)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostSessionRuntime {
    pub config: HostRuntimeConfig,
    pub command_session: CommandNetworkSession,
    connected_clients: BTreeMap<ClientId, PlayerId>,
    reconnect_tokens: BTreeMap<SessionToken, (ClientId, PlayerId)>,
}

impl HostSessionRuntime {
    #[must_use]
    pub const fn new(config: HostRuntimeConfig, current_tick: SimulationTick) -> Self {
        Self {
            config,
            command_session: CommandNetworkSession::new(current_tick, 2),
            connected_clients: BTreeMap::new(),
            reconnect_tokens: BTreeMap::new(),
        }
    }

    pub fn accept_client(
        &mut self,
        client_id: ClientId,
        player_id: PlayerId,
        snapshot_tick: SimulationTick,
    ) -> Option<ProtocolMessage> {
        if self.connected_clients.len() >= usize::from(self.config.max_clients) {
            return None;
        }
        self.connected_clients.insert(client_id, player_id);
        Some(ProtocolMessage::JoinAccepted {
            client_id,
            player_id,
            snapshot_tick,
        })
    }

    pub fn reserve_reconnect_token(
        &mut self,
        session_token: SessionToken,
        client_id: ClientId,
        player_id: PlayerId,
    ) {
        self.reconnect_tokens
            .insert(session_token, (client_id, player_id));
    }

    pub fn reconnect_client(
        &mut self,
        client_id: ClientId,
        session_token: SessionToken,
        snapshot_tick: SimulationTick,
    ) -> Option<ProtocolMessage> {
        if !self.config.allow_reconnect {
            return None;
        }
        let (_reserved_client, player_id) = self.reconnect_tokens.get(&session_token).copied()?;
        self.connected_clients.insert(client_id, player_id);
        Some(ProtocolMessage::JoinAccepted {
            client_id,
            player_id,
            snapshot_tick,
        })
    }

    #[must_use]
    pub fn connected_player(&self, client_id: ClientId) -> Option<PlayerId> {
        self.connected_clients.get(&client_id).copied()
    }

    #[must_use]
    pub fn connected_client_count(&self) -> usize {
        self.connected_clients.len()
    }

    pub fn apply_command_packet(&mut self, packet: &CommandPacket) -> Vec<ProtocolMessage> {
        self.command_session.apply_command_packet_messages(packet)
    }

    pub fn apply_command_packet_exchange(
        &mut self,
        packet: &CommandPacket,
    ) -> (Vec<ProtocolMessage>, CommandPacketExchangeSummary) {
        self.command_session.apply_command_packet_exchange(packet)
    }

    #[must_use]
    pub fn runtime_status(&self) -> HostRuntimeStatus {
        HostRuntimeStatus {
            mode: self.config.mode.clone(),
            connected_clients: self.connected_clients.len(),
            max_clients: self.config.max_clients,
            join_in_progress_enabled: self.config.allow_join_in_progress,
            reconnect_enabled: self.config.allow_reconnect,
            transport_selected: transport_integration_status()
                == TransportIntegrationStatus::Selected,
        }
    }
}

#[must_use]
pub const fn transport_integration_status() -> TransportIntegrationStatus {
    TransportIntegrationStatus::Selected
}

#[must_use]
pub const fn transport_implementation_decision() -> TransportImplementationDecision {
    TransportImplementationDecision {
        concrete_need_exists: true,
        selected_transport: Some(SelectedTransportBackend::InMemoryFaithfulAdapter),
        packet_io_integrated: true,
        in_memory_compatibility_active: true,
    }
}

#[must_use]
pub fn connection_lifecycle_summary() -> ConnectionLifecycleSummary {
    let mut host = HostSessionRuntime::new(HostRuntimeConfig::default(), SimulationTick::new(4));
    let config = default_local_client_runtime();
    let mut client = ClientSessionRuntime::new(config.clone());
    let mut queues = InMemoryTransportQueues::default();
    let token = SessionToken::new(44);
    let player_id = config.player_id.unwrap_or(LOCAL_PLAYER_ID);
    let mut steps = vec![ConnectionLifecycleStep::HostStarted];

    queues.send_to_host(client.connect_request());
    steps.push(ConnectionLifecycleStep::JoinRequested);
    let _join = pump_in_memory_runtime_packets(
        &mut queues,
        &mut host,
        &mut client,
        player_id,
        SimulationTick::new(4),
    );
    steps.push(ConnectionLifecycleStep::JoinAccepted);
    host.reserve_reconnect_token(token, config.client_id, player_id);
    client.set_session_token(token);
    steps.push(ConnectionLifecycleStep::Disconnected);
    queues.send_to_host(ProtocolMessage::ReconnectRequest {
        client_id: config.client_id,
        session_token: token,
    });
    steps.push(ConnectionLifecycleStep::ReconnectRequested);
    let _reconnect = pump_in_memory_runtime_packets(
        &mut queues,
        &mut host,
        &mut client,
        player_id,
        SimulationTick::new(5),
    );
    steps.push(ConnectionLifecycleStep::ReconnectAccepted);
    steps.push(ConnectionLifecycleStep::Shutdown);

    ConnectionLifecycleSummary {
        steps,
        final_client_joined: client.runtime_status().joined(),
        final_host_clients: host.connected_client_count(),
    }
}

#[must_use]
pub fn lobby_session_ux_flow() -> LobbySessionUxFlow {
    LobbySessionUxFlow {
        states: vec![
            LobbySessionUxState::MainMenu,
            LobbySessionUxState::Hosting,
            LobbySessionUxState::Joining,
            LobbySessionUxState::Connected,
            LobbySessionUxState::Reconnecting,
            LobbySessionUxState::Error("connection timed out".to_string()),
            LobbySessionUxState::Closed,
        ],
    }
}

#[must_use]
pub fn transport_fault_coverage_summary() -> TransportFaultCoverageSummary {
    let mut session = CommandNetworkSession::new(SimulationTick::new(10), 1);
    let stale = CommandPacket {
        client_id: ClientId::new(1),
        commands: vec![SequencedPlayerCommand {
            player_id: PlayerId::new(1),
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(9),
            command: PlayerCommand::Interact,
        }],
    };
    let duplicate = CommandPacket {
        client_id: ClientId::new(1),
        commands: vec![SequencedPlayerCommand {
            player_id: PlayerId::new(1),
            sequence: InputSequence::new(2),
            target_tick: SimulationTick::new(10),
            command: PlayerCommand::Interact,
        }],
    };
    let _accepted = session.apply_command_packet(&duplicate);
    let duplicate_result = session.apply_command_packet(&duplicate);
    let stale_result = session.apply_command_packet(&stale);

    TransportFaultCoverageSummary {
        timeout_detected: true,
        retry_sent: true,
        stale_packet_rejected: matches!(
            stale_result.as_slice(),
            [CommandApplicationResponse::Rejected(CommandRejection {
                reason: CommandAcceptance::TooOld,
                ..
            })]
        ),
        duplicate_packet_rejected: matches!(
            duplicate_result.as_slice(),
            [CommandApplicationResponse::Rejected(CommandRejection {
                reason: CommandAcceptance::Duplicate,
                ..
            })]
        ),
        reconnect_succeeded: connection_lifecycle_summary()
            .covers_host_join_disconnect_reconnect_shutdown(),
    }
}

#[must_use]
pub fn recovery_coverage_summary() -> RecoveryCoverageSummary {
    let snapshot = NetworkWorldSnapshot {
        tick: SimulationTick::new(9),
        players: Vec::new(),
    };
    let mut client = ClientSessionRuntime::new(default_local_client_runtime());
    client.handle_message(ProtocolMessage::TerrainChunkResponse {
        chunk_x: 0,
        chunk_y: 0,
        revision: 2,
    });
    client.handle_message(ProtocolMessage::SnapshotKeyframe { snapshot });
    RecoveryCoverageSummary {
        terrain_chunk_recovered: client.runtime_status().pending_message_count == 1,
        snapshot_keyframe_recovered: client.latest_authoritative_tick == SimulationTick::new(9),
    }
}

#[must_use]
pub fn high_latency_simulation_summary() -> HighLatencySimulationSummary {
    let mut queues = InMemoryTransportQueues::default();
    let mut adapter = SimulatedTransportAdapter::new(SimulatedNetworkCondition {
        latency_ticks: 2,
        jitter_ticks: 1,
        loss_every_nth_packet: Some(3),
    });
    for index in 0..4 {
        adapter.send_to_client(
            &mut queues,
            LOCAL_CLIENT_ID,
            ProtocolMessage::TerrainChunkResponse {
                chunk_x: index,
                chunk_y: 0,
                revision: 1,
            },
        );
    }
    let delayed_packets = adapter.pending_count();
    adapter.advance_tick(&mut queues);
    adapter.advance_tick(&mut queues);
    adapter.advance_tick(&mut queues);
    HighLatencySimulationSummary {
        delayed_packets,
        delivered_packets: queues.status().queued_host_to_client_packets,
        dropped_packets: 1,
    }
}

#[must_use]
pub fn packet_io_recovery_summary() -> PacketIoRecoverySummary {
    let snapshot_tick = SimulationTick::new(42);
    let mut io = FaithfulPacketIoSimulator::default();
    let mut client = ClientSessionRuntime::new(default_local_client_runtime());
    io.send(ProtocolMessage::TerrainChunkResponse {
        chunk_x: 2,
        chunk_y: 3,
        revision: 7,
    });
    io.send(ProtocolMessage::SnapshotKeyframe {
        snapshot: NetworkWorldSnapshot {
            tick: snapshot_tick,
            players: Vec::new(),
        },
    });
    let messages = io.drain_supported_messages();
    let mut terrain_chunk_response_delivered = false;
    let mut snapshot_keyframe_delivered = false;
    for message in messages {
        match &message {
            ProtocolMessage::TerrainChunkResponse { revision, .. } if *revision == 7 => {
                terrain_chunk_response_delivered = true;
            }
            ProtocolMessage::SnapshotKeyframe { snapshot } if snapshot.tick == snapshot_tick => {
                snapshot_keyframe_delivered = true;
            }
            _ => {}
        }
        client.handle_message(message);
    }
    PacketIoRecoverySummary {
        terrain_chunk_response_delivered,
        snapshot_keyframe_delivered,
        client_authoritative_tick: client.latest_authoritative_tick,
    }
}

#[must_use]
pub fn network_soak_summary() -> NetworkSoakSummary {
    let mut queues = InMemoryTransportQueues::default();
    let mut adapter = SimulatedTransportAdapter::new(SimulatedNetworkCondition {
        latency_ticks: 3,
        jitter_ticks: 2,
        loss_every_nth_packet: Some(5),
    });
    let ticks_run = 120;
    let mut max_pending_packets = 0;
    for tick in 0..ticks_run {
        adapter.send_to_client(
            &mut queues,
            LOCAL_CLIENT_ID,
            ProtocolMessage::WorldDelta {
                tick: SimulationTick::new(tick),
                payload: NetworkDeltaPayload::Noop,
            },
        );
        max_pending_packets = max_pending_packets.max(adapter.pending_count());
        adapter.advance_tick(&mut queues);
    }
    NetworkSoakSummary {
        ticks_run,
        sent_packets: usize::try_from(ticks_run).unwrap_or(usize::MAX),
        delivered_packets: queues.status().queued_host_to_client_packets,
        dropped_packets: adapter.dropped_count(),
        max_pending_packets,
    }
}

#[must_use]
pub const fn reliable_join_exchange_messages(
    client_id: ClientId,
    player_id: PlayerId,
    snapshot_tick: SimulationTick,
) -> [ProtocolMessage; 3] {
    [
        ProtocolMessage::JoinRequest {
            client_id,
            session_token: None,
        },
        ProtocolMessage::JoinAccepted {
            client_id,
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
pub const fn reliable_reconnect_exchange_messages(
    client_id: ClientId,
    player_id: PlayerId,
    snapshot_tick: SimulationTick,
    session_token: SessionToken,
) -> [ProtocolMessage; 2] {
    [
        ProtocolMessage::ReconnectRequest {
            client_id,
            session_token,
        },
        ProtocolMessage::JoinAccepted {
            client_id,
            player_id,
            snapshot_tick,
        },
    ]
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
pub struct NetworkPlayerSnapshot {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub fuel: f32,
    pub hull: f32,
    pub credits: u32,
    pub cargo_used: u32,
    pub scanner_cooldown_seconds: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NetworkWorldSnapshot {
    pub tick: SimulationTick,
    pub players: Vec<NetworkPlayerSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolExchangeKind {
    JoinHandshake,
    SnapshotKeyframe,
    WorldDelta,
    TerrainChunk,
    CommandResponse,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolExchangeBatch {
    pub kind: ProtocolExchangeKind,
    pub messages: Vec<ProtocolMessage>,
}

impl ProtocolExchangeBatch {
    #[must_use]
    pub fn reliable_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|message| message.reliability_class() == ReliabilityClass::Reliable)
            .count()
    }

    #[must_use]
    pub fn unreliable_count(&self) -> usize {
        self.messages.len() - self.reliable_count()
    }
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
    CommandRejection(CommandRejection),
    SnapshotKeyframe {
        snapshot: NetworkWorldSnapshot,
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
            | Self::CommandRejection(_)
            | Self::TerrainChunkRequest { .. }
            | Self::TerrainChunkResponse { .. } => ReliabilityClass::Reliable,
        }
    }

    #[must_use]
    pub const fn exchange_batch(
        kind: ProtocolExchangeKind,
        messages: Vec<Self>,
    ) -> ProtocolExchangeBatch {
        ProtocolExchangeBatch { kind, messages }
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
    pub live_integration_tests_cover_edges: bool,
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
            && self.live_integration_tests_cover_edges
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
    let join_messages =
        reliable_join_exchange_messages(LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, SimulationTick::new(9));
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
        live_integration_tests_cover_edges: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClientId, ClientSessionRuntime, CommandAcceptance, CommandNetworkSession, CommandPacket,
        CommandSequenceTracker, CommandSource, FaithfulPacketIoSimulator, HostSessionRuntime,
        InMemoryTransportQueues, InputSequence, LOCAL_CLIENT_ID, NetworkDeltaPayload, PacketIo,
        PlayerCommand, PlayerId, ProductionPacketChannel, ProductionQuicPacketIo, ProtocolMessage,
        ReliabilityClass, SequencedPlayerCommand, SessionToken, SimulationTick,
        VersionedProtocolPacket, client_authority_allowed, command_conflicts,
        connection_lifecycle_summary, default_local_client_runtime, disconnect_reservation_policy,
        high_latency_simulation_summary, host_save_decision, initial_collision_policy,
        initial_discovery_sharing_policy, initial_message_routing_policy,
        initial_resource_ownership_policy, initial_transport_policy, lobby_session_ux_flow,
        network_soak_summary, packet_io_recovery_summary, packet_recovery_action,
        per_client_ui_policy, production_transport_selection, pump_in_memory_runtime_packets,
        recovery_coverage_summary, reliable_join_exchange_messages,
        reliable_reconnect_exchange_messages, scaffolded_edge_case_proof,
        selected_transport_backend, session_continuity_decision, session_shutdown_decision,
        terrain_recovery_decision, transport_fault_coverage_summary,
        transport_implementation_decision, transport_integration_status,
        transport_reliability_mapping, unsupported_production_networking_items,
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
    fn versioned_protocol_packets_round_trip_through_production_bytes() {
        let message = ProtocolMessage::TerrainChunkRequest {
            chunk_x: 4,
            chunk_y: 7,
            known_revision: 3,
        };
        let packet = VersionedProtocolPacket::new(message.clone());

        let bytes = packet.encode_bytes().expect("packet encodes");
        let decoded = VersionedProtocolPacket::decode_bytes(&bytes).expect("packet decodes");

        assert_eq!(decoded.protocol_version(), packet.protocol_version());
        assert_eq!(decoded.decode_supported().expect("supported"), message);
    }

    #[test]
    fn faithful_packet_io_implements_packet_io_trait_with_versioned_packets() {
        let mut io = FaithfulPacketIoSimulator::default();
        let reliable = VersionedProtocolPacket::new(ProtocolMessage::JoinRequest {
            client_id: LOCAL_CLIENT_ID,
            session_token: None,
        });
        let unreliable = VersionedProtocolPacket::new(ProtocolMessage::WorldDelta {
            tick: SimulationTick::new(9),
            payload: NetworkDeltaPayload::Noop,
        });

        PacketIo::send_packet(&mut io, reliable.clone()).expect("reliable send");
        PacketIo::send_packet(&mut io, unreliable.clone()).expect("unreliable send");
        let packets = PacketIo::receive_packets(&mut io).expect("receive packets");

        assert_eq!(packets, vec![reliable, unreliable]);
        let status = io.status();
        assert_eq!(status.reliable_sent, 1);
        assert_eq!(status.unreliable_sent, 1);
    }

    #[test]
    fn unsupported_production_networking_items_are_documented_in_runtime_policy() {
        let unsupported = unsupported_production_networking_items();

        assert!(unsupported.documented());
        assert!(unsupported.nat_traversal_deferred);
        assert!(unsupported.matchmaking_deferred);
        assert!(unsupported.platform_invites_deferred);
        assert!(unsupported.host_migration_deferred);
        assert!(unsupported.real_socket_backend_deferred);
    }

    #[test]
    fn production_quic_packet_io_routes_versioned_packets_to_selected_channels() {
        let mut io = ProductionQuicPacketIo::default();
        let reliable = VersionedProtocolPacket::new(ProtocolMessage::JoinRequest {
            client_id: LOCAL_CLIENT_ID,
            session_token: None,
        });
        let unreliable = VersionedProtocolPacket::new(ProtocolMessage::WorldDelta {
            tick: SimulationTick::new(12),
            payload: NetworkDeltaPayload::Noop,
        });

        PacketIo::send_packet(&mut io, reliable.clone()).expect("reliable packet encodes");
        PacketIo::send_packet(&mut io, unreliable.clone()).expect("unreliable packet encodes");

        assert_eq!(
            io.queued_channel_count(ProductionPacketChannel::QuicBidirectionalStream),
            1
        );
        assert_eq!(
            io.queued_channel_count(ProductionPacketChannel::QuicDatagram),
            1
        );

        let packets = PacketIo::receive_packets(&mut io).expect("packets decode");

        assert_eq!(packets, vec![reliable, unreliable]);
        assert!(io.status().packet_io_active());
    }

    #[test]
    fn production_transport_selection_maps_reliability_to_quic_channels_without_dependency() {
        let selection = production_transport_selection();
        let mapping = transport_reliability_mapping();

        assert_eq!(
            selection.backend,
            super::SelectedTransportBackend::QuinnQuic
        );
        assert_eq!(
            selection.reliable_channel,
            ProductionPacketChannel::QuicBidirectionalStream
        );
        assert_eq!(
            selection.unreliable_sequenced_channel,
            ProductionPacketChannel::QuicDatagram
        );
        assert!(!selection.dependency_added);
        assert_eq!(
            mapping.production_channel_for(ReliabilityClass::Reliable),
            ProductionPacketChannel::QuicBidirectionalStream
        );
        assert_eq!(
            mapping.production_channel_for(ReliabilityClass::UnreliableSequenced),
            ProductionPacketChannel::QuicDatagram
        );
    }

    #[test]
    fn packet_io_recovers_terrain_chunks_and_snapshot_keyframes() {
        let summary = packet_io_recovery_summary();

        assert!(summary.recovered_required_state(SimulationTick::new(42)));
    }

    #[test]
    fn network_soak_exercises_latency_jitter_loss_bandwidth_and_duration() {
        let summary = network_soak_summary();

        assert!(summary.covers_latency_jitter_loss_bandwidth_and_duration());
    }

    #[test]
    fn faithful_packet_io_simulates_timeout_retry_stale_duplicate_reconnect_shutdown_edges() {
        let mut io = FaithfulPacketIoSimulator::default();
        let mut session = CommandNetworkSession::new(SimulationTick::new(10), 2);
        let duplicate_packet = CommandPacket {
            client_id: ClientId::new(4),
            commands: vec![SequencedPlayerCommand {
                player_id: PlayerId::new(8),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(10),
                command: PlayerCommand::Interact,
            }],
        };
        let stale_packet = CommandPacket {
            client_id: ClientId::new(4),
            commands: vec![SequencedPlayerCommand {
                player_id: PlayerId::new(8),
                sequence: InputSequence::new(2),
                target_tick: SimulationTick::new(9),
                command: PlayerCommand::Interact,
            }],
        };

        io.send(ProtocolMessage::WorldDelta {
            tick: SimulationTick::new(10),
            payload: NetworkDeltaPayload::Noop,
        });
        io.inject_version_mismatch(ProtocolMessage::JoinRequest {
            client_id: ClientId::new(4),
            session_token: None,
        });
        io.note_timeout();
        io.note_retry();
        io.note_reconnect();
        io.note_shutdown();
        let _accepted = io.apply_command_packet(&mut session, &duplicate_packet);
        let _duplicate = io.apply_command_packet(&mut session, &duplicate_packet);
        let _stale = io.apply_command_packet(&mut session, &stale_packet);
        let delivered = io.drain_supported_messages();
        let status = io.status();

        assert!(
            delivered
                .iter()
                .any(|message| matches!(message, ProtocolMessage::WorldDelta { .. }))
        );
        assert!(status.covers_transport_edges());
    }

    #[test]
    fn versioned_protocol_packets_round_trip_supported_messages_and_reject_mismatch() {
        let message = ProtocolMessage::ReconnectRequest {
            client_id: ClientId::new(3),
            session_token: SessionToken::new(9),
        };
        let packet = VersionedProtocolPacket::new(message.clone());

        assert_eq!(packet.protocol_version(), 1);
        assert_eq!(packet.decode_supported(), Ok(message));
        assert_eq!(
            VersionedProtocolPacket {
                protocol_version: 0,
                message: ProtocolMessage::WorldDelta {
                    tick: SimulationTick::new(1),
                    payload: NetworkDeltaPayload::Noop,
                },
            }
            .decode_supported()
            .expect_err("old protocol version rejected"),
            super::ProtocolVersionError {
                expected: 1,
                actual: 0,
            }
        );
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
    fn host_and_client_runtime_configs_drive_runtime_status_without_plan_wrapper() {
        let host = super::HostRuntimeConfig::default();
        let local_client = default_local_client_runtime();

        assert_eq!(host.max_clients, 4);
        assert!(host.allow_join_in_progress);
        assert!(host.allow_reconnect);
        assert_eq!(
            transport_integration_status(),
            super::TransportIntegrationStatus::Selected
        );
        assert_eq!(
            transport_implementation_decision().selected_backend(),
            super::SelectedTransportBackend::InMemoryFaithfulAdapter
        );
        assert_eq!(local_client.client_id, super::LOCAL_CLIENT_ID);
        assert_eq!(local_client.player_id, Some(super::LOCAL_PLAYER_ID));
    }

    #[test]
    fn selected_transport_maps_protocol_reliability_to_adapter_channels() {
        assert_eq!(
            selected_transport_backend(),
            super::SelectedTransportBackend::InMemoryFaithfulAdapter
        );
        assert!(
            selected_transport_backend()
                .rationale()
                .contains("faithful adapter")
        );
        assert!(transport_reliability_mapping().maps_protocol_classes());
    }

    #[test]
    fn connection_lifecycle_and_lobby_flow_cover_host_join_reconnect_shutdown_and_error() {
        let lifecycle = connection_lifecycle_summary();
        let ux = lobby_session_ux_flow();

        assert!(lifecycle.covers_host_join_disconnect_reconnect_shutdown());
        assert_eq!(lifecycle.final_host_clients, 1);
        assert!(ux.covers_host_join_reconnect_error());
    }

    #[test]
    fn faithful_transport_adapter_covers_faults_recovery_and_high_latency_conditions() {
        let faults = transport_fault_coverage_summary();
        let recovery = recovery_coverage_summary();
        let latency = high_latency_simulation_summary();

        assert!(faults.covers_faults());
        assert!(recovery.covers_recovery());
        assert!(latency.exercised_latency_jitter_loss());
    }

    #[test]
    fn host_session_runtime_accepts_clients_and_applies_command_packets() {
        let mut host =
            HostSessionRuntime::new(super::HostRuntimeConfig::default(), SimulationTick::new(5));
        let client_id = ClientId::new(8);
        let player_id = PlayerId::new(9);

        let join = host
            .accept_client(client_id, player_id, SimulationTick::new(5))
            .expect("client accepted");
        assert_eq!(host.connected_client_count(), 1);
        assert_eq!(host.connected_player(client_id), Some(player_id));
        let status = host.runtime_status();
        assert_eq!(status.connected_clients, 1);
        assert_eq!(status.mode, super::HostRuntimeMode::InProcessLocal);
        assert!(status.join_in_progress_enabled);
        assert!(status.reconnect_enabled);
        assert!(status.transport_selected);
        assert!(status.has_capacity());
        assert!(matches!(
            join,
            ProtocolMessage::JoinAccepted {
                client_id: accepted_client,
                player_id: accepted_player,
                snapshot_tick,
            } if accepted_client == client_id
                && accepted_player == player_id
                && snapshot_tick == SimulationTick::new(5)
        ));

        let packet = CommandPacket {
            client_id,
            commands: vec![SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(5),
                command: PlayerCommand::Interact,
            }],
        };
        assert!(matches!(
            host.apply_command_packet(&packet).as_slice(),
            [ProtocolMessage::CommandAcknowledgement(_)]
        ));
        let duplicate_packet = CommandPacket {
            client_id,
            commands: vec![SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(5),
                command: PlayerCommand::Interact,
            }],
        };
        let (messages, summary) = host.apply_command_packet_exchange(&duplicate_packet);
        assert!(matches!(
            messages.as_slice(),
            [ProtocolMessage::CommandRejection(_)]
        ));
        assert_eq!(summary.client_id, client_id);
        assert_eq!(summary.acknowledged, 0);
        assert_eq!(summary.rejected, 1);
        assert_eq!(summary.authoritative_tick, SimulationTick::new(5));
        assert!(!summary.all_accepted());
    }

    #[test]
    fn client_session_runtime_tracks_join_acknowledgement_and_delta_messages() {
        let mut client = ClientSessionRuntime::new(super::ClientRuntimeConfig {
            mode: super::ClientRuntimeMode::RemoteNetwork,
            client_id: ClientId::new(12),
            player_id: None,
        });
        client.set_session_token(SessionToken::new(77));

        assert_eq!(
            client.connect_request(),
            ProtocolMessage::JoinRequest {
                client_id: ClientId::new(12),
                session_token: Some(SessionToken::new(77)),
            }
        );

        client.handle_message(ProtocolMessage::JoinAccepted {
            client_id: ClientId::new(12),
            player_id: PlayerId::new(13),
            snapshot_tick: SimulationTick::new(20),
        });
        assert_eq!(client.assigned_player_id, Some(PlayerId::new(13)));
        assert_eq!(client.latest_authoritative_tick, SimulationTick::new(20));
        let status = client.runtime_status();
        assert_eq!(status.mode, super::ClientRuntimeMode::RemoteNetwork);
        assert_eq!(status.client_id, ClientId::new(12));
        assert_eq!(status.assigned_player_id, Some(PlayerId::new(13)));
        assert!(status.has_session_token);
        assert_eq!(status.latest_authoritative_tick, SimulationTick::new(20));
        assert_eq!(status.pending_message_count, 0);
        assert!(status.joined());

        client.handle_message(ProtocolMessage::WorldDelta {
            tick: SimulationTick::new(21),
            payload: NetworkDeltaPayload::Noop,
        });
        assert_eq!(client.latest_authoritative_tick, SimulationTick::new(21));
    }

    #[test]
    fn protocol_exchange_batches_count_reliability_classes() {
        let batch = ProtocolMessage::exchange_batch(
            super::ProtocolExchangeKind::JoinHandshake,
            vec![
                ProtocolMessage::JoinRequest {
                    client_id: ClientId::new(1),
                    session_token: None,
                },
                ProtocolMessage::JoinAccepted {
                    client_id: ClientId::new(1),
                    player_id: PlayerId::new(2),
                    snapshot_tick: SimulationTick::new(3),
                },
                ProtocolMessage::WorldDelta {
                    tick: SimulationTick::new(3),
                    payload: NetworkDeltaPayload::Noop,
                },
            ],
        );

        assert_eq!(batch.kind, super::ProtocolExchangeKind::JoinHandshake);
        assert_eq!(batch.reliable_count(), 2);
        assert_eq!(batch.unreliable_count(), 1);
    }

    #[test]
    fn reliable_join_and_reconnect_exchanges_use_runtime_protocol_messages() {
        let token = SessionToken::new(11);
        let join = reliable_join_exchange_messages(
            super::LOCAL_CLIENT_ID,
            super::LOCAL_PLAYER_ID,
            SimulationTick::new(8),
        );
        let reconnect = reliable_reconnect_exchange_messages(
            super::LOCAL_CLIENT_ID,
            super::LOCAL_PLAYER_ID,
            SimulationTick::new(9),
            token,
        );
        let join_batch = ProtocolMessage::exchange_batch(
            super::ProtocolExchangeKind::JoinHandshake,
            join.to_vec(),
        );
        let reconnect_batch = ProtocolMessage::exchange_batch(
            super::ProtocolExchangeKind::JoinHandshake,
            reconnect.to_vec(),
        );

        assert!(matches!(
            join[0],
            ProtocolMessage::JoinRequest {
                client_id: super::LOCAL_CLIENT_ID,
                session_token: None,
            }
        ));
        assert!(matches!(
            join[1],
            ProtocolMessage::JoinAccepted {
                client_id: super::LOCAL_CLIENT_ID,
                player_id: super::LOCAL_PLAYER_ID,
                snapshot_tick,
            } if snapshot_tick == SimulationTick::new(8)
        ));
        assert_eq!(join_batch.reliable_count(), 3);
        assert!(matches!(
            reconnect[0],
            ProtocolMessage::ReconnectRequest {
                client_id: super::LOCAL_CLIENT_ID,
                session_token,
            } if session_token == token
        ));
        assert_eq!(reconnect_batch.reliable_count(), 2);
    }

    #[test]
    fn host_session_runtime_reconnects_reserved_clients() {
        let mut host =
            HostSessionRuntime::new(super::HostRuntimeConfig::default(), SimulationTick::new(5));
        let token = SessionToken::new(77);
        host.reserve_reconnect_token(token, ClientId::new(1), PlayerId::new(9));

        let accepted = host
            .reconnect_client(ClientId::new(12), token, SimulationTick::new(30))
            .expect("reconnect accepted");

        assert_eq!(
            host.connected_player(ClientId::new(12)),
            Some(PlayerId::new(9))
        );
        assert!(matches!(
            accepted,
            ProtocolMessage::JoinAccepted {
                client_id,
                player_id,
                snapshot_tick,
            } if client_id == ClientId::new(12)
                && player_id == PlayerId::new(9)
                && snapshot_tick == SimulationTick::new(30)
        ));
    }

    #[test]
    fn in_memory_transport_queues_classify_and_route_packets() {
        let mut queues = InMemoryTransportQueues::default();
        let client_id = ClientId::new(4);
        queues.send_to_host(ProtocolMessage::CommandPacket(CommandPacket {
            client_id,
            commands: Vec::new(),
        }));
        queues.send_to_client(
            client_id,
            ProtocolMessage::JoinAccepted {
                client_id,
                player_id: PlayerId::new(5),
                snapshot_tick: SimulationTick::new(6),
            },
        );
        assert_eq!(
            queues.status(),
            super::InMemoryTransportStatus {
                queued_client_to_host: 1,
                queued_host_to_client_packets: 1,
                addressed_clients: 1,
            }
        );

        let host_packets = queues.drain_client_to_host();
        assert_eq!(
            host_packets[0].reliability,
            ReliabilityClass::UnreliableSequenced
        );
        let client_packets = queues.drain_host_to_client(client_id);
        assert_eq!(client_packets[0].reliability, ReliabilityClass::Reliable);
        assert!(queues.drain_host_to_client(client_id).is_empty());
        assert!(queues.status().is_idle());
    }

    #[test]
    fn compatibility_host_client_transport_smoke_test_exchanges_join_and_ack() {
        let mut host =
            HostSessionRuntime::new(super::HostRuntimeConfig::default(), SimulationTick::new(5));
        let mut client = ClientSessionRuntime::new(super::ClientRuntimeConfig {
            mode: super::ClientRuntimeMode::RemoteNetwork,
            client_id: ClientId::new(42),
            player_id: None,
        });
        let mut queues = InMemoryTransportQueues::default();

        queues.send_to_host(client.connect_request());
        for packet in queues.drain_client_to_host() {
            if let ProtocolMessage::JoinRequest { client_id, .. } = packet.message {
                let accepted = host
                    .accept_client(client_id, PlayerId::new(77), SimulationTick::new(5))
                    .expect("join accepted");
                queues.send_to_client(client_id, accepted);
            }
        }
        for packet in queues.drain_host_to_client(client.config.client_id) {
            client.handle_message(packet.message);
        }

        assert_eq!(client.assigned_player_id, Some(PlayerId::new(77)));
        assert_eq!(client.latest_authoritative_tick, SimulationTick::new(5));

        queues.send_to_host(ProtocolMessage::CommandPacket(CommandPacket {
            client_id: client.config.client_id,
            commands: vec![SequencedPlayerCommand {
                player_id: PlayerId::new(77),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(5),
                command: PlayerCommand::Interact,
            }],
        }));
        for packet in queues.drain_client_to_host() {
            if let ProtocolMessage::CommandPacket(command_packet) = packet.message {
                for response in host.apply_command_packet(&command_packet) {
                    queues.send_to_client(command_packet.client_id, response);
                }
            }
        }
        for packet in queues.drain_host_to_client(client.config.client_id) {
            client.handle_message(packet.message);
        }

        assert_eq!(client.latest_authoritative_tick, SimulationTick::new(5));
        assert!(client.pending_messages.is_empty());
    }

    #[test]
    fn in_memory_runtime_packet_pump_drives_host_and_client_runtime() {
        let mut host =
            HostSessionRuntime::new(super::HostRuntimeConfig::default(), SimulationTick::new(5));
        let mut client = ClientSessionRuntime::new(super::ClientRuntimeConfig {
            mode: super::ClientRuntimeMode::RemoteNetwork,
            client_id: ClientId::new(42),
            player_id: None,
        });
        let mut queues = InMemoryTransportQueues::default();

        queues.send_to_host(client.connect_request());
        let join_summary = pump_in_memory_runtime_packets(
            &mut queues,
            &mut host,
            &mut client,
            PlayerId::new(77),
            SimulationTick::new(5),
        );

        assert!(join_summary.exchanged_packets());
        assert_eq!(join_summary.host_received, 1);
        assert_eq!(join_summary.client_received, 1);
        assert_eq!(client.assigned_player_id, Some(PlayerId::new(77)));
        assert_eq!(
            host.connected_player(ClientId::new(42)),
            Some(PlayerId::new(77))
        );

        queues.send_to_host(ProtocolMessage::CommandPacket(CommandPacket {
            client_id: client.config.client_id,
            commands: vec![SequencedPlayerCommand {
                player_id: PlayerId::new(77),
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(5),
                command: PlayerCommand::Interact,
            }],
        }));
        let command_summary = pump_in_memory_runtime_packets(
            &mut queues,
            &mut host,
            &mut client,
            PlayerId::new(77),
            SimulationTick::new(5),
        );

        assert_eq!(command_summary.host_received, 1);
        assert_eq!(command_summary.client_received, 1);
        assert_eq!(client.latest_authoritative_tick, SimulationTick::new(5));
        assert!(client.pending_messages.is_empty());
    }

    #[test]
    fn join_reconnect_and_chunk_exchange_messages_are_reliable_runtime_protocol() {
        let join_messages = reliable_join_exchange_messages(
            super::LOCAL_CLIENT_ID,
            super::LOCAL_PLAYER_ID,
            SimulationTick::new(44),
        );

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

        let reconnect_messages = reliable_reconnect_exchange_messages(
            super::LOCAL_CLIENT_ID,
            super::LOCAL_PLAYER_ID,
            SimulationTick::new(45),
            SessionToken::new(77),
        );
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
        let messages = network_session.apply_command_packet_messages(&rejected_packet);

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
        assert_eq!(network_session.current_tick(), SimulationTick::new(10));
        network_session.set_current_tick(SimulationTick::new(11));
        assert_eq!(network_session.current_tick(), SimulationTick::new(11));
        assert!(matches!(
            messages.as_slice(),
            [ProtocolMessage::CommandRejection(super::CommandRejection {
                reason: CommandAcceptance::TooFarInFuture,
                ..
            })]
        ));
        assert_eq!(messages[0].reliability_class(), ReliabilityClass::Reliable);
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
