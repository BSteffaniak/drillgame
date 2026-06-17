#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "world coordinates intentionally cross integer tile and floating render spaces"
)]

use std::{
    fmt::Write as _,
    mem,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    contract::ContractLog,
    economy::{
        DeepClaimStatus, PurchaseError, SurfaceZone, TownBuilding, TownDevelopment, buy_upgrade,
        refuel_amount, repair_amount, sell_cargo, upgrade_offers, upgrade_tier_name,
    },
    input::PlayerInput,
    multiplayer::{QuinnSessionTickSummary, SocketDrivenCorrectionSummary},
    player::Player,
    save::{
        load_game, load_game_slot, load_latest_game, save_exists, save_game, save_game_slot,
        save_slot_count, saves_exist,
    },
    surface::surface_building_at_tile,
    terrain::{
        ArtifactKind, DeepStratum, MineResult, MineralKind, StrategicResourceKind, Terrain,
        TileKind, TilePosition, deep_stratum_at_depth,
    },
};

pub const TILE_SIZE: f32 = 32.0;
const WORLD_WIDTH: i32 = 240;
const WORLD_HEIGHT: i32 = 220;
const GRAVITY: f32 = 780.0;
const HORIZONTAL_ACCELERATION: f32 = 900.0;
const THRUST_ACCELERATION: f32 = 1_250.0;
const MAX_HORIZONTAL_SPEED: f32 = 260.0;
const MAX_FALL_SPEED: f32 = 560.0;
const DRAG: f32 = 0.86;
const FUEL_BURN_PER_SECOND: f32 = 5.0;
const DRILL_FUEL_COST: f32 = 0.45;
const PLAYER_RADIUS: f32 = 10.5;
const SAFE_LANDING_SPEED: f32 = 330.0;
const CRASH_DAMAGE_SCALE: f32 = 0.11;
const BOULDER_DAMAGE: f32 = 8.0;
const BOULDER_WARNING_SECONDS: f32 = 0.85;
const BOULDER_SPAWN_CHANCE: u64 = 16;
const HEAT_START_DEPTH: f32 = 18.0 * TILE_SIZE;
const HEAT_DAMAGE_PER_SECOND: f32 = 3.5;
const CAMERA_SMOOTHING: f32 = 8.0;
const SKY_FLIGHT_HEIGHT_TILES: f32 = 12.0;
const MIN_PLAYER_Y: f32 = -SKY_FLIGHT_HEIGHT_TILES * TILE_SIZE;
const EXPLORATION_VISUAL_CHANGE_RADIUS_TILES: i32 = 12;
const CAMERA_INTRO_SECONDS: f32 = 1.0;
const CAMERA_INTRO_DROP_DISTANCE: f32 = 260.0;
const WORLD_SEED: u64 = 0xD1_11_6A_4E;
const PLAYER_SPAWN_X: f32 = 97.0 * TILE_SIZE;
const PLAYER_SPAWN_Y: f32 = 4.0 * TILE_SIZE;

const fn default_master_volume() -> f32 {
    0.8
}

const fn default_fullscreen() -> bool {
    false
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DrillDirection {
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DrillState {
    pub target: TilePosition,
    pub direction: DrillDirection,
    pub progress: f32,
    pub initial_durability: u8,
    pub seconds_per_chip: f32,
    pub sound_timer: f32,
    pub dust_timer: f32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum RigSlot {
    DrillHead,
    Engine,
    HullPlating,
    Radiator,
    CargoModule,
    ScannerModule,
    UtilityModule,
    EmergencySystem,
}

impl RigSlot {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::DrillHead => "Drill Head",
            Self::Engine => "Engine",
            Self::HullPlating => "Hull Plating",
            Self::Radiator => "Radiator",
            Self::CargoModule => "Cargo Module",
            Self::ScannerModule => "Scanner Module",
            Self::UtilityModule => "Utility Module",
            Self::EmergencySystem => "Emergency System",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum RigPartKind {
    TitanDrill,
    NeedleDrill,
    ResonanceDrill,
    LightweightEngine,
    HaulerEngine,
    BurstEngine,
    ThermalHull,
    ImpactHull,
    PressureHull,
    ProspectorScanner,
    HazardScanner,
    RelicScanner,
    CargoBalloon,
    ArmoredCargoBay,
    SortedCargoRack,
}

impl RigPartKind {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::TitanDrill => "Titan Drill",
            Self::NeedleDrill => "Needle Drill",
            Self::ResonanceDrill => "Resonance Drill",
            Self::LightweightEngine => "Lightweight Engine",
            Self::HaulerEngine => "Hauler Engine",
            Self::BurstEngine => "Burst Engine",
            Self::ThermalHull => "Thermal Hull",
            Self::ImpactHull => "Impact Hull",
            Self::PressureHull => "Pressure Hull",
            Self::ProspectorScanner => "Prospector Scanner",
            Self::HazardScanner => "Hazard Scanner",
            Self::RelicScanner => "Relic Scanner",
            Self::CargoBalloon => "Cargo Balloon",
            Self::ArmoredCargoBay => "Armored Cargo Bay",
            Self::SortedCargoRack => "Sorted Cargo Rack",
        }
    }

    #[must_use]
    pub const fn slot(self) -> RigSlot {
        match self {
            Self::TitanDrill | Self::NeedleDrill | Self::ResonanceDrill => RigSlot::DrillHead,
            Self::LightweightEngine | Self::HaulerEngine | Self::BurstEngine => RigSlot::Engine,
            Self::ThermalHull | Self::ImpactHull | Self::PressureHull => RigSlot::HullPlating,
            Self::ProspectorScanner | Self::HazardScanner | Self::RelicScanner => {
                RigSlot::ScannerModule
            }
            Self::CargoBalloon | Self::ArmoredCargoBay | Self::SortedCargoRack => {
                RigSlot::CargoModule
            }
        }
    }

    #[must_use]
    pub const fn rarity(self) -> &'static str {
        match self {
            Self::ResonanceDrill | Self::PressureHull | Self::RelicScanner => "legendary",
            Self::TitanDrill | Self::BurstEngine | Self::ArmoredCargoBay => "rare",
            _ => "standard",
        }
    }

    #[must_use]
    pub const fn tradeoff(self) -> &'static str {
        match self {
            Self::TitanDrill => "slow, high power, high fuel burn",
            Self::NeedleDrill => "fast through soft material, poor hard-rock performance",
            Self::ResonanceDrill => {
                "strong against crystal/ancient material, unstable near hazards"
            }
            Self::LightweightEngine => "agile, low cargo tolerance",
            Self::HaulerEngine => "slow, handles heavy cargo well",
            Self::BurstEngine => "strong thrust, inefficient fuel use",
            Self::ThermalHull => "heat resistant, weak crash protection",
            Self::ImpactHull => "crash resistant, poor heat resistance",
            Self::PressureHull => "deep-zone specialist",
            Self::ProspectorScanner => "finds ore veins, misses hazards",
            Self::HazardScanner => "finds danger, limited ore detection",
            Self::RelicScanner => "finds artifacts and anomalies",
            Self::CargoBalloon => "huge capacity, bad handling",
            Self::ArmoredCargoBay => "protects cargo, lower capacity",
            Self::SortedCargoRack => "bonuses for diverse loads",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum LegendaryBlueprint {
    StarPlating,
    VoidTank,
    RelicSorter,
}

impl LegendaryBlueprint {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::StarPlating => "Star Plating",
            Self::VoidTank => "Void Tank",
            Self::RelicSorter => "Relic Sorter",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum ChallengeBadge {
    DeepReturn,
    HighRiskReturn,
    ValuableHaul,
}

impl ChallengeBadge {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::DeepReturn => "Deep Return",
            Self::HighRiskReturn => "High-Risk Return",
            Self::ValuableHaul => "Valuable Haul",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum CosmeticRigSkin {
    BronzeTrim,
    HazardStripes,
    StarChrome,
}

impl CosmeticRigSkin {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::BronzeTrim => "Bronze Trim",
            Self::HazardStripes => "Hazard Stripes",
            Self::StarChrome => "Star Chrome",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunMode {
    Title,
    Playing,
    Interior,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OnlineSessionUxState {
    Idle,
    Hosting,
    Joining,
    Connected,
    Reconnecting,
    Timeout,
    Error,
    Disconnected,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OnlineNetworkTaskRequest {
    HostDirectConnect,
    JoinDirectConnect,
    HostDescriptorFile { path: PathBuf },
    JoinDescriptorFile { path: PathBuf },
    ReconnectDirectConnect,
    Shutdown,
}

#[allow(
    dead_code,
    reason = "online task reducer is exercised by tests until desktop event-loop ownership calls it"
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OnlineNetworkTaskResult {
    Hosted(RealOnlineSessionUxSnapshot),
    JoinedDescriptor(RealOnlineSessionUxSnapshot),
    Connected(RealOnlineSessionUxSnapshot),
    Reconnected(RealOnlineSessionUxSnapshot),
    Failed(String),
    Shutdown,
}

#[allow(
    dead_code,
    reason = "real online controller is exercised by tests until desktop async UI ownership calls it"
)]
pub enum RealOnlineSessionMode {
    CombinedLocalhost(crate::multiplayer::QuinnOnlineSession),
    DescriptorHostPending {
        listener: crate::multiplayer::QuinnHostListener,
        descriptor_path: PathBuf,
        descriptor: crate::multiplayer::QuinnHostConnectionDescriptor,
    },
    DescriptorHostAccepted {
        host_runtime: crate::multiplayer::HostSessionRuntime,
        host_io: crate::multiplayer::QuinnPacketIo,
        descriptor_path: PathBuf,
        descriptor: crate::multiplayer::QuinnHostConnectionDescriptor,
    },
    DescriptorClientConnected {
        client_runtime: crate::multiplayer::ClientSessionRuntime,
        connector: crate::multiplayer::QuinnClientConnector,
        packet_io: crate::multiplayer::QuinnPacketIo,
        descriptor_path: PathBuf,
        descriptor: crate::multiplayer::QuinnHostConnectionDescriptor,
    },
}

impl RealOnlineSessionMode {
    const fn session_mut(&mut self) -> Option<&mut crate::multiplayer::QuinnOnlineSession> {
        match self {
            Self::CombinedLocalhost(session) => Some(session),
            Self::DescriptorHostPending { .. }
            | Self::DescriptorHostAccepted { .. }
            | Self::DescriptorClientConnected { .. } => None,
        }
    }

    const fn label(&self) -> &'static str {
        match self {
            Self::CombinedLocalhost(_) => "combined-localhost",
            Self::DescriptorHostPending { .. } => "descriptor-host-pending",
            Self::DescriptorHostAccepted { .. } => "descriptor-host-accepted",
            Self::DescriptorClientConnected { .. } => "descriptor-client-connected",
        }
    }
}

pub struct RealOnlineSessionController {
    mode: RealOnlineSessionMode,
    player_slot: Option<u8>,
    next_sequence: u32,
    next_tick: u64,
}

#[allow(
    dead_code,
    reason = "real online controller is exercised by tests until desktop async UI ownership calls it"
)]
impl RealOnlineSessionController {
    /// Connect a localhost split Quinn session and apply joined UX state.
    ///
    /// # Errors
    ///
    /// Returns an error when Quinn endpoint setup, connect/accept, or join handshake fails.
    pub async fn connect_localhost(
        game: &mut GameState,
    ) -> Result<Self, crate::multiplayer::QuinnOnlineSessionError> {
        let client_config = crate::multiplayer::default_local_client_runtime();
        let session = crate::multiplayer::connect_split_localhost_quinn_session(
            crate::multiplayer::HostRuntimeConfig::default(),
            client_config,
            crate::multiplayer::SimulationTick::new(300),
        )
        .await?;
        let controller = Self {
            mode: RealOnlineSessionMode::CombinedLocalhost(session),
            player_slot: Some(1),
            next_sequence: 30,
            next_tick: 301,
        };
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_joined_session(
            controller.player_slot,
        ));
        Ok(controller)
    }

    pub fn host_descriptor_file_pending(
        game: &mut GameState,
        path: &Path,
    ) -> Result<Self, crate::multiplayer::QuinnOnlineSessionError> {
        let listener = crate::multiplayer::QuinnHostListener::bind_localhost(
            crate::multiplayer::QuinnEndpointConfig {
                bind_addr: game.online_host_bind_addr,
            },
        )?;
        let mut descriptor = listener.connection_descriptor()?;
        if game.online_host_advertise_addr.port() != 0 {
            descriptor.host_addr = game.online_host_advertise_addr;
        }
        let json = serde_json::to_string(&descriptor).map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Accept(error.to_string())
        })?;
        std::fs::write(path, json).map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Accept(error.to_string())
        })?;
        let controller = Self {
            mode: RealOnlineSessionMode::DescriptorHostPending {
                listener,
                descriptor_path: path.to_path_buf(),
                descriptor,
            },
            player_slot: Some(1),
            next_sequence: 30,
            next_tick: 301,
        };
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_host_descriptor_ready(
            Some(1),
            path,
        ));
        Ok(controller)
    }

    pub async fn connect_descriptor_client(
        game: &mut GameState,
        path: &Path,
    ) -> Result<Self, crate::multiplayer::QuinnOnlineSessionError> {
        let json = std::fs::read_to_string(path).map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                "descriptor file {} could not be read: {error}",
                path.display()
            ))
        })?;
        let descriptor: crate::multiplayer::QuinnHostConnectionDescriptor =
            serde_json::from_str(&json).map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "descriptor file {} could not be parsed: {error}",
                    path.display()
                ))
            })?;
        let connector = crate::multiplayer::QuinnClientConnector::bind_from_host_descriptor(
            crate::multiplayer::QuinnEndpointConfig::localhost_ephemeral(),
            &descriptor,
        )?;
        let packet_io = connector
            .connect_packet_io(descriptor.host_addr, &descriptor.server_name)
            .await?;
        let client_config = crate::multiplayer::default_local_client_runtime();
        let mut client_runtime = crate::multiplayer::ClientSessionRuntime::new(client_config);
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                client_runtime.connect_request(),
            ))
            .await?;
        let accepted_packet = packet_io.receive_reliable_packet().await?;
        let accepted_message = accepted_packet.decode_supported().map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                "unsupported protocol version: expected {}, actual {}",
                error.expected, error.actual
            ))
        })?;
        client_runtime.handle_message(accepted_message);
        let controller = Self {
            mode: RealOnlineSessionMode::DescriptorClientConnected {
                client_runtime,
                connector,
                packet_io,
                descriptor_path: path.to_path_buf(),
                descriptor,
            },
            player_slot: Some(2),
            next_sequence: 30,
            next_tick: 301,
        };
        game.apply_real_online_session_ux(
            RealOnlineSessionUxSnapshot::from_descriptor_client_connected(
                controller.player_slot,
                path,
            ),
        );
        Ok(controller)
    }

    pub async fn accept_descriptor_client(
        &mut self,
        game: &mut GameState,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostPending {
            listener,
            descriptor_path,
            descriptor,
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "pending descriptor host listener",
                ),
            );
        };
        let host_io = listener.accept_packet_io().await?;
        let mut host_runtime = crate::multiplayer::HostSessionRuntime::new(
            crate::multiplayer::HostRuntimeConfig::default(),
            crate::multiplayer::SimulationTick::new(300),
        );
        let join_packet = host_io.receive_reliable_packet().await?;
        let join_message = join_packet.decode_supported().map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                "unsupported protocol version: expected {}, actual {}",
                error.expected, error.actual
            ))
        })?;
        let join_response = match join_message {
            crate::multiplayer::ProtocolMessage::JoinRequest {
                client_id,
                session_token: None,
            } => host_runtime.accept_client(
                client_id,
                crate::multiplayer::PlayerId::new(2),
                crate::multiplayer::SimulationTick::new(300),
            ),
            crate::multiplayer::ProtocolMessage::ReconnectRequest {
                client_id,
                session_token,
            }
            | crate::multiplayer::ProtocolMessage::JoinRequest {
                client_id,
                session_token: Some(session_token),
            } => host_runtime.reconnect_client(
                client_id,
                session_token,
                crate::multiplayer::SimulationTick::new(300),
            ),
            other => {
                return Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other));
            }
        }
        .ok_or(crate::multiplayer::QuinnOnlineSessionError::JoinRejected)?;
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                join_response,
            ))
            .await?;
        let descriptor_path = descriptor_path.clone();
        let descriptor = descriptor.clone();
        self.mode = RealOnlineSessionMode::DescriptorHostAccepted {
            host_runtime,
            host_io,
            descriptor_path,
            descriptor,
        };
        game.apply_real_online_session_ux(
            RealOnlineSessionUxSnapshot::from_descriptor_host_accepted(self.player_slot),
        );
        Ok(())
    }

    pub async fn drive_descriptor_host_outbound_tick(
        &mut self,
        game: &mut GameState,
        input: crate::multiplayer::QuinnSessionTickInput,
    ) -> Result<
        crate::multiplayer::QuinnSessionTickSummary,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let command_summary = self
            .descriptor_host_try_receive_command_packet(Duration::from_millis(1))
            .await?;
        self.descriptor_host_send_player_identity(&game.online_player_name)
            .await?;
        self.descriptor_host_send_ready_state(game.online_local_ready)
            .await?;
        self.descriptor_host_try_receive_ready_state(game, Duration::from_millis(1))
            .await?;
        if game.run_mode == RunMode::Playing
            && game.online_local_ready
            && game.online_remote_player_ready
        {
            self.descriptor_host_send_start_session().await?;
        }
        let snapshot_replicated = if let Some(snapshot) = input.snapshot {
            self.descriptor_host_send_snapshot(snapshot).await?;
            true
        } else {
            false
        };
        let delta_replicated = if let Some((tick, payload)) = input.delta {
            self.descriptor_host_send_world_delta(tick, payload).await?;
            true
        } else {
            false
        };
        let summary = crate::multiplayer::QuinnSessionTickSummary {
            command_summary,
            snapshot_replicated,
            delta_replicated,
            terrain_chunk_response: None,
            correction_summary: None,
        };
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_tick_summary(
            &summary,
            self.player_slot,
        ));
        Ok(summary)
    }

    pub async fn drive_descriptor_client_outbound_tick(
        &mut self,
        game: &mut GameState,
        input: crate::multiplayer::QuinnSessionTickInput,
    ) -> Result<
        crate::multiplayer::QuinnSessionTickSummary,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let received_messages = self
            .descriptor_client_try_receive_pending_messages(game, Duration::from_millis(1))
            .await?;
        let summary = crate::multiplayer::QuinnSessionTickSummary {
            command_summary: None,
            snapshot_replicated: false,
            delta_replicated: false,
            terrain_chunk_response: None,
            correction_summary: None,
        };
        if game.online_session_state == OnlineSessionUxState::Shutdown {
            return Ok(summary);
        }
        let command_sent = if let Some(packet) = input.command_packet {
            self.descriptor_client_send_command_packet_unacknowledged(packet)
                .await?;
            true
        } else {
            false
        };
        self.descriptor_client_send_player_identity(&game.online_player_name)
            .await?;
        self.descriptor_client_send_ready_state(game.online_local_ready)
            .await?;
        if command_sent {
            game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
                state: OnlineSessionUxState::Connected,
                host_owns_save: false,
                player_slot: self.player_slot,
                status_message: format!(
                    "Descriptor client sent live command tick; received {received_messages} pending host messages."
                ),
            });
        }
        Ok(summary)
    }

    pub async fn descriptor_host_send_player_identity(
        &mut self,
        name: &str,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::PlayerIdentity {
                    player_id: crate::multiplayer::PlayerId::new(1),
                    name: name.to_owned(),
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_send_ready_state(
        &mut self,
        ready: bool,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::ReadyState {
                    player_id: crate::multiplayer::PlayerId::new(1),
                    ready,
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_try_receive_ready_state(
        &mut self,
        game: &mut GameState,
        timeout: Duration,
    ) -> Result<bool, crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        let mut received_any = false;
        for _ in 0..4 {
            let received =
                match tokio::time::timeout(timeout, host_io.receive_reliable_packet()).await {
                    Ok(Ok(packet)) => packet,
                    Ok(Err(error)) => return Err(error.into()),
                    Err(_) => break,
                };
            match received.decode_supported().map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "unsupported protocol version: expected {}, actual {}",
                    error.expected, error.actual
                ))
            })? {
                crate::multiplayer::ProtocolMessage::PlayerIdentity { name, .. } => {
                    game.online_remote_player_name = Some(name);
                    game.online_remote_player_connected = true;
                    received_any = true;
                }
                crate::multiplayer::ProtocolMessage::ReadyState { ready, .. } => {
                    game.online_remote_player_ready = ready;
                    game.online_remote_player_connected = true;
                    received_any = true;
                }
                other => {
                    return Err(
                        crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other),
                    );
                }
            }
        }
        Ok(received_any)
    }

    pub async fn descriptor_host_send_start_session(
        &mut self,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::StartSession {
                    authoritative_tick: crate::multiplayer::SimulationTick::new(0),
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_send_session_ended(
        &mut self,
        reason: &str,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::SessionEnded {
                    reason: reason.to_owned(),
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_client_send_player_identity(
        &mut self,
        name: &str,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorClientConnected { packet_io, .. } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::PlayerIdentity {
                    player_id: crate::multiplayer::PlayerId::new(2),
                    name: name.to_owned(),
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_client_send_ready_state(
        &mut self,
        ready: bool,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorClientConnected { packet_io, .. } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::ReadyState {
                    player_id: crate::multiplayer::PlayerId::new(2),
                    ready,
                },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_try_receive_command_packet(
        &mut self,
        timeout: Duration,
    ) -> Result<
        Option<crate::multiplayer::CommandPacketExchangeSummary>,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let RealOnlineSessionMode::DescriptorHostAccepted {
            host_runtime,
            host_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        let received = match tokio::time::timeout(timeout, host_io.receive_datagram_packet()).await
        {
            Ok(Ok(packet)) => packet,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Ok(None),
        };
        let packet = match received.decode_supported().map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                "unsupported protocol version: expected {}, actual {}",
                error.expected, error.actual
            ))
        })? {
            crate::multiplayer::ProtocolMessage::CommandPacket(packet) => packet,
            other => {
                return Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other));
            }
        };
        let (responses, summary) = host_runtime.apply_command_packet_exchange(&packet);
        for response in responses {
            host_io
                .send_packet(crate::multiplayer::VersionedProtocolPacket::new(response))
                .await?;
        }
        Ok(Some(summary))
    }

    pub async fn descriptor_client_try_receive_pending_messages(
        &mut self,
        game: &mut GameState,
        timeout: Duration,
    ) -> Result<usize, crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorClientConnected {
            client_runtime,
            packet_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        let mut received_count = 0;
        for _ in 0..8 {
            match tokio::time::timeout(timeout, packet_io.receive_reliable_packet()).await {
                Ok(Ok(packet)) => {
                    let message = packet.decode_supported().map_err(|error| {
                        crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                            "unsupported protocol version: expected {}, actual {}",
                            error.expected, error.actual
                        ))
                    })?;
                    match message {
                        crate::multiplayer::ProtocolMessage::PlayerIdentity { name, .. } => {
                            game.online_remote_player_name = Some(name);
                            game.online_remote_player_connected = true;
                        }
                        crate::multiplayer::ProtocolMessage::ReadyState { ready, .. } => {
                            game.online_remote_player_ready = ready;
                            game.online_remote_player_connected = true;
                        }
                        crate::multiplayer::ProtocolMessage::StartSession { .. } => {
                            game.online_remote_player_connected = true;
                            game.online_remote_player_ready = true;
                            game.online_session_state = OnlineSessionUxState::Connected;
                            "Host started online gameplay session."
                                .clone_into(&mut game.online_session_status_message);
                            game.run_mode = RunMode::Playing;
                            game.modal = None;
                            game.message.clone_from(&game.online_session_status_message);
                        }
                        crate::multiplayer::ProtocolMessage::SessionEnded { reason } => {
                            game.online_remote_player_connected = false;
                            game.online_remote_player_ready = false;
                            game.online_session_state = OnlineSessionUxState::Shutdown;
                            game.modal = None;
                            game.online_session_status_message =
                                format!("Online session ended by host: {reason}");
                            game.message.clone_from(&game.online_session_status_message);
                        }
                        other => client_runtime.handle_message(other),
                    }
                    received_count += 1;
                }
                Ok(Err(error)) => {
                    game.online_remote_player_connected = false;
                    game.online_remote_player_ready = false;
                    game.online_session_state = OnlineSessionUxState::Shutdown;
                    game.modal = None;
                    game.online_session_status_message = format!(
                        "Online session ended by host: reliable channel closed ({error:?})"
                    );
                    game.message.clone_from(&game.online_session_status_message);
                    return Ok(received_count);
                }
                Err(_) => break,
            }
        }
        if game.online_session_state == OnlineSessionUxState::Shutdown {
            return Ok(received_count);
        }
        match tokio::time::timeout(timeout, packet_io.receive_datagram_packet()).await {
            Ok(Ok(packet)) => {
                let message = packet.decode_supported().map_err(|error| {
                    crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                        "unsupported protocol version: expected {}, actual {}",
                        error.expected, error.actual
                    ))
                })?;
                match message {
                    crate::multiplayer::ProtocolMessage::SnapshotKeyframe { .. }
                    | crate::multiplayer::ProtocolMessage::WorldDelta { .. } => {
                        client_runtime.handle_message(message);
                        received_count += 1;
                    }
                    other => {
                        return Err(
                            crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other),
                        );
                    }
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {}
        }
        Ok(received_count)
    }

    pub async fn descriptor_client_send_command_packet_unacknowledged(
        &mut self,
        packet: crate::multiplayer::CommandPacket,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorClientConnected { packet_io, .. } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::CommandPacket(packet),
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_send_snapshot(
        &mut self,
        snapshot: crate::multiplayer::NetworkWorldSnapshot,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::SnapshotKeyframe { snapshot },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_host_send_world_delta(
        &mut self,
        tick: crate::multiplayer::SimulationTick,
        payload: crate::multiplayer::NetworkDeltaPayload,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::WorldDelta { tick, payload },
            ))
            .await?;
        Ok(())
    }

    pub async fn descriptor_client_receive_replication(
        &mut self,
    ) -> Result<crate::multiplayer::ProtocolMessage, crate::multiplayer::QuinnOnlineSessionError>
    {
        let RealOnlineSessionMode::DescriptorClientConnected {
            client_runtime,
            packet_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        let message = packet_io
            .receive_datagram_packet()
            .await?
            .decode_supported()
            .map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "unsupported protocol version: expected {}, actual {}",
                    error.expected, error.actual
                ))
            })?;
        match message {
            crate::multiplayer::ProtocolMessage::SnapshotKeyframe { .. }
            | crate::multiplayer::ProtocolMessage::WorldDelta { .. } => {
                client_runtime.handle_message(message.clone());
                Ok(message)
            }
            other => Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other)),
        }
    }

    pub async fn descriptor_host_answer_terrain_request(
        &mut self,
        response_revision: u64,
    ) -> Result<crate::multiplayer::ProtocolMessage, crate::multiplayer::QuinnOnlineSessionError>
    {
        let RealOnlineSessionMode::DescriptorHostAccepted { host_io, .. } = &mut self.mode else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        let request = host_io
            .receive_reliable_packet()
            .await?
            .decode_supported()
            .map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "unsupported protocol version: expected {}, actual {}",
                    error.expected, error.actual
                ))
            })?;
        let response = match request {
            crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
                chunk_x,
                chunk_y,
                known_revision: _,
            } => crate::multiplayer::ProtocolMessage::TerrainChunkResponse {
                chunk_x,
                chunk_y,
                revision: response_revision,
            },
            other => {
                return Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other));
            }
        };
        host_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                response.clone(),
            ))
            .await?;
        Ok(response)
    }

    pub async fn descriptor_client_request_terrain_chunk(
        &mut self,
        chunk_x: i32,
        chunk_y: i32,
        known_revision: u64,
    ) -> Result<crate::multiplayer::ProtocolMessage, crate::multiplayer::QuinnOnlineSessionError>
    {
        let RealOnlineSessionMode::DescriptorClientConnected {
            client_runtime,
            packet_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
                    chunk_x,
                    chunk_y,
                    known_revision,
                },
            ))
            .await?;
        let response = packet_io
            .receive_reliable_packet()
            .await?
            .decode_supported()
            .map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "unsupported protocol version: expected {}, actual {}",
                    error.expected, error.actual
                ))
            })?;
        match response {
            crate::multiplayer::ProtocolMessage::TerrainChunkResponse { .. } => {
                client_runtime.handle_message(response.clone());
                Ok(response)
            }
            other => Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other)),
        }
    }

    pub async fn descriptor_host_receive_command_packet(
        &mut self,
    ) -> Result<
        crate::multiplayer::CommandPacketExchangeSummary,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let RealOnlineSessionMode::DescriptorHostAccepted {
            host_runtime,
            host_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "accepted descriptor host packet io",
                ),
            );
        };
        let received = host_io.receive_datagram_packet().await?;
        let packet = match received.decode_supported().map_err(|error| {
            crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                "unsupported protocol version: expected {}, actual {}",
                error.expected, error.actual
            ))
        })? {
            crate::multiplayer::ProtocolMessage::CommandPacket(packet) => packet,
            other => {
                return Err(crate::multiplayer::QuinnOnlineSessionError::UnexpectedMessage(other));
            }
        };
        let (responses, summary) = host_runtime.apply_command_packet_exchange(&packet);
        for response in responses {
            host_io
                .send_packet(crate::multiplayer::VersionedProtocolPacket::new(response))
                .await?;
        }
        Ok(summary)
    }

    pub async fn descriptor_client_send_command_packet(
        &mut self,
        packet: crate::multiplayer::CommandPacket,
        expected_responses: usize,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        let RealOnlineSessionMode::DescriptorClientConnected {
            client_runtime,
            packet_io,
            ..
        } = &mut self.mode
        else {
            return Err(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "connected descriptor client packet io",
                ),
            );
        };
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::CommandPacket(packet),
            ))
            .await?;
        for _ in 0..expected_responses {
            let response = packet_io.receive_reliable_packet().await?;
            let message = response.decode_supported().map_err(|error| {
                crate::multiplayer::QuinnOnlineSessionError::Connect(format!(
                    "unsupported protocol version: expected {}, actual {}",
                    error.expected, error.actual
                ))
            })?;
            client_runtime.handle_message(message);
        }
        Ok(())
    }

    #[must_use]
    pub const fn mode_label(&self) -> &'static str {
        self.mode.label()
    }

    /// Drive one real network tick from a caller-provided payload and apply online UX state.
    ///
    /// # Errors
    ///
    /// Returns an error when the real Quinn tick driver fails.
    pub async fn drive_tick_input(
        &mut self,
        game: &mut GameState,
        input: crate::multiplayer::QuinnSessionTickInput,
    ) -> Result<
        crate::multiplayer::QuinnSessionTickTelemetry,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let telemetry = self
            .mode
            .session_mut()
            .ok_or(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "active online session",
                ),
            )?
            .drive_tick_with_telemetry(input)
            .await?;
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_tick_summary(
            &telemetry.summary,
            self.player_slot,
        ));
        Ok(telemetry)
    }

    /// Drive one real network telemetry tick and apply online UX state.
    ///
    /// # Errors
    ///
    /// Returns an error when the real Quinn tick driver fails.
    pub async fn drive_telemetry_tick(
        &mut self,
        game: &mut GameState,
    ) -> Result<
        crate::multiplayer::QuinnSessionTickTelemetry,
        crate::multiplayer::QuinnOnlineSessionError,
    > {
        let player_id = self
            .mode
            .session_mut()
            .ok_or(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "active online session",
                ),
            )?
            .joined_player_id()
            .unwrap_or(crate::multiplayer::LOCAL_PLAYER_ID);
        let client_id = self
            .mode
            .session_mut()
            .ok_or(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "active online session",
                ),
            )?
            .client_runtime
            .config
            .client_id;
        let sequence = self.next_sequence;
        let tick = self.next_tick;
        let snapshot_tick = crate::multiplayer::SimulationTick::new(tick + 1);
        let live_snapshot = live_player_network_snapshot(game, player_id, snapshot_tick);
        let live_player = live_snapshot
            .players
            .first()
            .cloned()
            .expect("live player snapshot contains local player");
        let terrain_request = live_player_terrain_request(game);
        let telemetry = self
            .mode
            .session_mut()
            .ok_or(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "active online session",
                ),
            )?
            .drive_tick_with_telemetry(crate::multiplayer::QuinnSessionTickInput {
                command_packet: Some(crate::multiplayer::CommandPacket {
                    client_id,
                    commands: vec![crate::multiplayer::SequencedPlayerCommand {
                        player_id,
                        sequence: crate::multiplayer::InputSequence::new(sequence),
                        target_tick: crate::multiplayer::SimulationTick::new(tick),
                        command: live_player_command(&game.player),
                    }],
                }),
                snapshot: Some(live_snapshot),
                delta: Some((
                    crate::multiplayer::SimulationTick::new(tick + 2),
                    crate::multiplayer::NetworkDeltaPayload::Players {
                        players: vec![player_id],
                    },
                )),
                terrain_chunk_request: Some(terrain_request),
                correction_probe: Some((
                    live_player.x + 24.0,
                    live_player.y,
                    live_player,
                    crate::multiplayer::SimulationTick::new(tick + 3),
                )),
            })
            .await?;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_tick = self.next_tick.saturating_add(4);
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_tick_summary(
            &telemetry.summary,
            self.player_slot,
        ));
        Ok(telemetry)
    }

    /// Reconnect the owned real Quinn session and apply reconnect UX state.
    ///
    /// # Errors
    ///
    /// Returns an error when reconnect setup or handshake fails.
    pub async fn reconnect(
        &mut self,
        game: &mut GameState,
        token: crate::multiplayer::SessionToken,
    ) -> Result<(), crate::multiplayer::QuinnOnlineSessionError> {
        self.mode
            .session_mut()
            .ok_or(
                crate::multiplayer::QuinnOnlineSessionError::MissingEndpoint(
                    "active online session",
                ),
            )?
            .reconnect_with_token(token)
            .await?;
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_reconnect(
            self.player_slot,
        ));
        Ok(())
    }
}

fn live_player_command(player: &Player) -> crate::multiplayer::PlayerCommand {
    crate::multiplayer::PlayerCommand::Movement {
        horizontal: player.velocity_x.clamp(-1.0, 1.0),
        thrust: player.velocity_y < -0.01,
        drill_down: player.velocity_y > 0.01,
    }
}

fn live_player_network_snapshot(
    game: &GameState,
    player_id: crate::multiplayer::PlayerId,
    tick: crate::multiplayer::SimulationTick,
) -> crate::multiplayer::NetworkWorldSnapshot {
    crate::multiplayer::NetworkWorldSnapshot {
        tick,
        players: vec![crate::multiplayer::NetworkPlayerSnapshot {
            player_id,
            x: game.player.x,
            y: game.player.y,
            velocity_x: game.player.velocity_x,
            velocity_y: game.player.velocity_y,
            fuel: game.player.fuel,
            hull: game.player.hull,
            credits: game.player.credits,
            cargo_used: game.player.cargo.values().copied().sum(),
            scanner_cooldown_seconds: 0.0,
        }],
    }
}

fn live_player_terrain_request(game: &GameState) -> (i32, i32, u64, u64) {
    (
        game.player.x.floor() as i32,
        game.player.y.floor() as i32,
        game.update_ticks,
        u64::from(game.total_resources_refined),
    )
}

#[allow(
    dead_code,
    reason = "real online UX bridge is exercised by tests until desktop async UI ownership calls it"
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealOnlineSessionUxSnapshot {
    pub state: OnlineSessionUxState,
    pub host_owns_save: bool,
    pub player_slot: Option<u8>,
    pub status_message: String,
}

#[allow(
    dead_code,
    reason = "real online UX bridge is exercised by tests until desktop async UI ownership calls it"
)]
impl RealOnlineSessionUxSnapshot {
    #[must_use]
    pub fn from_joined_session(player_slot: Option<u8>) -> Self {
        Self {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot,
            status_message: "Connected through real localhost Quinn session.".to_owned(),
        }
    }

    #[must_use]
    pub fn from_host_descriptor_ready(player_slot: Option<u8>, path: &Path) -> Self {
        Self {
            state: OnlineSessionUxState::Hosting,
            host_owns_save: true,
            player_slot,
            status_message: format!(
                "Host descriptor ready at {}; waiting for remote miner to join.",
                path.display()
            ),
        }
    }

    #[must_use]
    pub fn from_descriptor_host_accepted(player_slot: Option<u8>) -> Self {
        Self {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot,
            status_message: "Remote miner joined descriptor host; gameplay sync is ready to start."
                .to_owned(),
        }
    }

    #[must_use]
    pub fn from_descriptor_client_connected(player_slot: Option<u8>, path: &Path) -> Self {
        Self {
            state: OnlineSessionUxState::Connected,
            host_owns_save: false,
            player_slot,
            status_message: format!(
                "Connected to host descriptor {}; waiting for gameplay sync.",
                path.display()
            ),
        }
    }

    #[must_use]
    pub fn from_reconnect(player_slot: Option<u8>) -> Self {
        Self {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot,
            status_message: "Reconnected through real localhost Quinn session.".to_owned(),
        }
    }

    #[must_use]
    pub fn from_tick_summary(summary: &QuinnSessionTickSummary, player_slot: Option<u8>) -> Self {
        let state = if summary.advanced_authoritative_runtime() {
            OnlineSessionUxState::Connected
        } else {
            OnlineSessionUxState::Hosting
        };
        Self {
            state,
            host_owns_save: true,
            player_slot,
            status_message: format!(
                "Real Quinn tick: command={}, snapshot={}, delta={}, chunk={}, correction={}",
                summary.command_summary.is_some(),
                summary.snapshot_replicated,
                summary.delta_replicated,
                summary.terrain_chunk_response.is_some(),
                summary.correction_summary.is_some()
            ),
        }
    }

    #[must_use]
    pub fn from_correction(
        summary: SocketDrivenCorrectionSummary,
        player_slot: Option<u8>,
    ) -> Self {
        let state = if summary.exercised_socket_correction() {
            OnlineSessionUxState::Connected
        } else {
            OnlineSessionUxState::Error
        };
        Self {
            state,
            host_owns_save: true,
            player_slot,
            status_message: format!(
                "Authoritative correction over real Quinn: plan={:?}, snap={}",
                summary.correction_plan, summary.snap_applied
            ),
        }
    }
}

const fn default_online_session_state() -> OnlineSessionUxState {
    OnlineSessionUxState::Idle
}

fn default_online_player_name() -> String {
    "Player".to_owned()
}

fn default_online_descriptor_path() -> PathBuf {
    PathBuf::from("drillgame-online-host.json")
}

fn alternate_online_descriptor_path() -> PathBuf {
    PathBuf::from("/tmp/drillgame-online-host.json")
}

fn join_online_descriptor_path() -> PathBuf {
    PathBuf::from("/tmp/drillgame-online-join.json")
}

fn default_online_host_bind_addr() -> SocketAddr {
    "0.0.0.0:4242"
        .parse()
        .expect("default online host bind address parses")
}

fn default_online_host_advertise_addr() -> SocketAddr {
    "127.0.0.1:4242"
        .parse()
        .expect("default online host advertise address parses")
}

fn lan_online_host_bind_addr() -> SocketAddr {
    "0.0.0.0:5252"
        .parse()
        .expect("LAN online host bind address parses")
}

fn lan_online_host_advertise_addr() -> SocketAddr {
    "192.168.1.10:5252"
        .parse()
        .expect("LAN online host advertise address parses")
}

fn localhost_ephemeral_online_host_bind_addr() -> SocketAddr {
    "127.0.0.1:0"
        .parse()
        .expect("localhost ephemeral online host bind address parses")
}

fn localhost_ephemeral_online_host_advertise_addr() -> SocketAddr {
    "127.0.0.1:0"
        .parse()
        .expect("localhost ephemeral online host advertise address parses")
}

fn default_online_client_bind_addr() -> SocketAddr {
    "0.0.0.0:0"
        .parse()
        .expect("default online client bind address parses")
}

const fn default_online_gameplay_ticks() -> u32 {
    60
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ModalScreen {
    Fuel,
    FuelConfirm,
    Repair,
    RepairConfirm,
    Depot,
    Headquarters,
    DepotReceiptHistory,
    Shop,
    ShopConfirm,
    Bank,
    Explosives,
    Salvage,
    Options,
    SaveSlots,
    LoadSlots,
    ExitConfirm,
    UnsavedExitConfirm,
    Map,
    Help,
    TownDevelopment,
    ExpeditionBoard,
    ResearchLog,
    OnlineMultiplayer,
    Crafting,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PauseOption {
    Resume,
    Save,
    Load,
    Options,
    ExitToDesktop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TitleOption {
    Resume,
    NewGame,
    OnlineMultiplayer,
    LocalMultiplayer,
    LoadSlot,
    Options,
    Exit,
}

impl TitleOption {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume Saved Session",
            Self::NewGame => "New Game",
            Self::OnlineMultiplayer => "Online Multiplayer",
            Self::LocalMultiplayer => "Local Split-Screen",
            Self::LoadSlot => "Load Slot",
            Self::Options => "Options",
            Self::Exit => "Exit",
        }
    }
}

impl PauseOption {
    pub const ALL: [Self; 5] = [
        Self::Resume,
        Self::Save,
        Self::Load,
        Self::Options,
        Self::ExitToDesktop,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume",
            Self::Save => "Save Game",
            Self::Load => "Load Game",
            Self::Options => "Options",
            Self::ExitToDesktop => "Exit to Desktop",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DustParticle {
    pub x: f32,
    pub y: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum InfrastructureKind {
    SignalRelay,
    SurveyDrone,
    CargoLift,
    TunnelSupport,
    PumpStation,
    OreProcessor,
}

impl InfrastructureKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SignalRelay => "Signal Relay",
            Self::SurveyDrone => "Survey Drone",
            Self::CargoLift => "Cargo Lift",
            Self::TunnelSupport => "Tunnel Support",
            Self::PumpStation => "Pump Station",
            Self::OreProcessor => "Ore Processor",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct PlacedInfrastructure {
    pub kind: InfrastructureKind,
    pub position: TilePosition,
    #[serde(default = "default_infrastructure_durability")]
    pub durability: u8,
}

const fn default_infrastructure_durability() -> u8 {
    100
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct HazardCloud {
    pub x: f32,
    pub y: f32,
    pub life: f32,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct PlacedBomb {
    pub x: f32,
    pub y: f32,
    pub timer_seconds: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct ScanMarker {
    pub position: TilePosition,
    pub kind: TileKind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SideContractKind {
    #[default]
    Cargo,
    DepthSurvey,
    HazardScan,
    Rush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeKind {
    ReinforcedBulkhead,
    AuxiliaryTank,
    ExpandedSorter,
    SignalRelayKit,
    SurveyDroneKit,
    CargoLiftKit,
    TunnelSupportKit,
    PumpStationKit,
    OreProcessorKit,
}

impl RecipeKind {
    pub const ALL: [Self; 9] = [
        Self::ReinforcedBulkhead,
        Self::AuxiliaryTank,
        Self::ExpandedSorter,
        Self::SignalRelayKit,
        Self::SurveyDroneKit,
        Self::CargoLiftKit,
        Self::TunnelSupportKit,
        Self::PumpStationKit,
        Self::OreProcessorKit,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReinforcedBulkhead => "Reinforced Bulkhead",
            Self::AuxiliaryTank => "Auxiliary Tank",
            Self::ExpandedSorter => "Expanded Sorter",
            Self::SignalRelayKit => "Signal Relay Kit",
            Self::SurveyDroneKit => "Survey Drone Kit",
            Self::CargoLiftKit => "Cargo Lift Kit",
            Self::TunnelSupportKit => "Tunnel Support Kit",
            Self::PumpStationKit => "Pump Station Kit",
            Self::OreProcessorKit => "Ore Processor Kit",
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ReinforcedBulkhead => "+15 max hull rig part",
            Self::AuxiliaryTank => "+20 fuel capacity rig part",
            Self::ExpandedSorter => "+4 cargo capacity rig part",
            Self::SignalRelayKit => "crafted infrastructure item",
            Self::SurveyDroneKit => "reveals nearby map over time",
            Self::CargoLiftKit => "sends cargo upward from a station",
            Self::TunnelSupportKit => "protects nearby tunnel from collapses",
            Self::PumpStationKit => "suppresses nearby gas and heat hazards",
            Self::OreProcessorKit => "refines cheap ore into strategic materials",
        }
    }

    #[must_use]
    pub const fn cost(self) -> &'static [(StrategicResourceKind, u32)] {
        match self {
            Self::ReinforcedBulkhead => &[(StrategicResourceKind::AncientAlloy, 2)],
            Self::AuxiliaryTank => &[
                (StrategicResourceKind::AncientAlloy, 1),
                (StrategicResourceKind::CrystalLens, 1),
            ],
            Self::ExpandedSorter => &[
                (StrategicResourceKind::AncientAlloy, 1),
                (StrategicResourceKind::CoreShard, 1),
            ],
            Self::SignalRelayKit => &[(StrategicResourceKind::CoreShard, 2)],
            Self::SurveyDroneKit => &[
                (StrategicResourceKind::CrystalLens, 1),
                (StrategicResourceKind::CoreShard, 1),
            ],
            Self::CargoLiftKit => &[
                (StrategicResourceKind::AncientAlloy, 2),
                (StrategicResourceKind::CoreShard, 1),
            ],
            Self::TunnelSupportKit => &[(StrategicResourceKind::AncientAlloy, 1)],
            Self::PumpStationKit => &[
                (StrategicResourceKind::AncientAlloy, 1),
                (StrategicResourceKind::CrystalLens, 1),
                (StrategicResourceKind::CoreShard, 1),
            ],
            Self::OreProcessorKit => &[
                (StrategicResourceKind::AncientAlloy, 2),
                (StrategicResourceKind::CrystalLens, 1),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum NpcStoryRecord {
    ValeIntro,
    IonaSilverWarning,
    KadeRelicSignal,
    ValeThermalWarning,
    KadeStarCoreSignal,
    ValeStarCoreSecured,
}

impl NpcStoryRecord {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::ValeIntro => "Vale: Profitable Shaft",
            Self::IonaSilverWarning => "Iona: Silver Strata",
            Self::KadeRelicSignal => "Kade: Relic Signals",
            Self::ValeThermalWarning => "Vale: Thermal Readings",
            Self::KadeStarCoreSignal => "Kade: Star Core Harmonics",
            Self::ValeStarCoreSecured => "Vale: Star Core Secured",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
pub enum CollectionRewardKind {
    Minerals,
    Artifacts,
    Hazards,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CollectionLog {
    #[serde(default)]
    pub minerals: std::collections::BTreeSet<MineralKind>,
    #[serde(default)]
    pub artifacts: std::collections::BTreeSet<ArtifactKind>,
    #[serde(default)]
    pub hazards: std::collections::BTreeSet<TileKind>,
    #[serde(default)]
    pub strata: std::collections::BTreeSet<i32>,
    #[serde(default)]
    pub rewards_claimed: std::collections::BTreeSet<CollectionRewardKind>,
    #[serde(default)]
    pub story_records: std::collections::BTreeSet<NpcStoryRecord>,
}

impl CollectionLog {
    fn discover_tile(&mut self, tile: TileKind) {
        match tile {
            TileKind::Ore(mineral) => {
                self.minerals.insert(mineral);
            }
            TileKind::Artifact(artifact) => {
                self.artifacts.insert(artifact);
            }
            TileKind::Lava
            | TileKind::Gas
            | TileKind::ExplosivePocket
            | TileKind::PressurePocket
            | TileKind::MagmaVent => {
                self.hazards.insert(tile);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WorldEventKind {
    MarketCrash,
    MarketBoom,
    RareBuyer,
    FuelShortage,
    RepairBacklog,
    HeatWave,
    CollapseSurge,
    DeepPressureStorm,
    GasBloom,
    Earthquake,
    MeteorShower,
    RivalClaims,
    AncientMachine,
}

impl WorldEventKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MarketCrash => "Market crash",
            Self::MarketBoom => "Market boom",
            Self::RareBuyer => "Rare buyer",
            Self::FuelShortage => "Fuel shortage",
            Self::RepairBacklog => "Repair backlog",
            Self::HeatWave => "Heat wave",
            Self::CollapseSurge => "Collapse surge",
            Self::DeepPressureStorm => "Deep pressure storm",
            Self::GasBloom => "Gas bloom",
            Self::Earthquake => "Earthquake",
            Self::MeteorShower => "Meteor shower",
            Self::RivalClaims => "Rival claims",
            Self::AncientMachine => "Ancient machine",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct ActiveWorldEvent {
    pub kind: WorldEventKind,
    pub days_remaining: u32,
    pub severity: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExpeditionObjectiveKind {
    #[default]
    ReachDepth,
    DeliverCargo,
    ScanHazards,
    BuildPumpStations,
    RecoverProbe,
    MineVein,
    ScanAnomaly,
    RescueMiner,
    DeliverExplosives,
    StabilizeCollapse,
    RetrieveArtifact,
    ReachSignal,
    NoDamageReturn,
    FastReturn,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Expedition {
    pub kind: ExpeditionObjectiveKind,
    pub target: TileKind,
    pub required: u32,
    pub reward: u32,
    pub expires_day: u32,
}

impl Expedition {
    #[must_use]
    pub fn title(self) -> String {
        match self.kind {
            ExpeditionObjectiveKind::ReachDepth => format!("Survey {}m claim", self.required),
            ExpeditionObjectiveKind::DeliverCargo => {
                format!("Deliver {} x{}", self.target.name(), self.required)
            }
            ExpeditionObjectiveKind::ScanHazards => format!("Map {} hazards", self.required),
            ExpeditionObjectiveKind::BuildPumpStations => {
                format!("Install {} pump station(s)", self.required)
            }
            ExpeditionObjectiveKind::RecoverProbe => "Recover a lost probe".to_owned(),
            ExpeditionObjectiveKind::MineVein => {
                format!("Mine {} vein x{}", self.target.name(), self.required)
            }
            ExpeditionObjectiveKind::ScanAnomaly => format!("Scan {} anomaly", self.target.name()),
            ExpeditionObjectiveKind::RescueMiner => "Rescue trapped miner signal".to_owned(),
            ExpeditionObjectiveKind::DeliverExplosives => {
                format!("Deliver {} charges underground", self.required)
            }
            ExpeditionObjectiveKind::StabilizeCollapse => "Stabilize collapse zone".to_owned(),
            ExpeditionObjectiveKind::RetrieveArtifact => {
                format!("Retrieve ancient {}", self.target.name())
            }
            ExpeditionObjectiveKind::ReachSignal => {
                "Reach temporary signal before expiry".to_owned()
            }
            ExpeditionObjectiveKind::NoDamageReturn => "Return with no hull damage".to_owned(),
            ExpeditionObjectiveKind::FastReturn => "Return before deadline".to_owned(),
        }
    }
    #[must_use]
    pub const fn risk_label(self) -> &'static str {
        match self.kind {
            ExpeditionObjectiveKind::ReachDepth if self.required >= 120 => "extreme",
            ExpeditionObjectiveKind::ReachDepth if self.required >= 90 => "high",
            ExpeditionObjectiveKind::DeliverCargo if self.required >= 3 => "medium",
            ExpeditionObjectiveKind::ScanHazards
            | ExpeditionObjectiveKind::BuildPumpStations
            | ExpeditionObjectiveKind::RecoverProbe
            | ExpeditionObjectiveKind::DeliverExplosives
            | ExpeditionObjectiveKind::StabilizeCollapse
            | ExpeditionObjectiveKind::RescueMiner => "medium",
            ExpeditionObjectiveKind::ReachSignal
            | ExpeditionObjectiveKind::NoDamageReturn
            | ExpeditionObjectiveKind::FastReturn => "high",
            _ => "low",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SideContract {
    pub kind: SideContractKind,
    pub target: TileKind,
    pub required: u32,
    #[serde(default)]
    pub expires_day: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ServiceAnimation {
    Fuel,
    Repair,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct FallingBoulder {
    pub x: f32,
    pub y: f32,
    pub velocity_y: f32,
    pub warning_seconds: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct SparkParticle {
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SoundCue {
    Drill,
    Sell,
    Upgrade,
    Damage,
    Milestone,
    Rescue,
    Explosion,
    Ui,
}

#[derive(Clone, Debug, Default)]
pub struct VisualChanges {
    pub full_terrain_refresh: bool,
    pub changed_tiles: Vec<TilePosition>,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "game state tracks several orthogonal UI/progression flags"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameState {
    pub terrain: Terrain,
    pub player: Player,
    #[serde(default = "current_save_version")]
    pub save_version: u32,
    pub message: String,
    #[serde(default = "default_online_session_state")]
    pub online_session_state: OnlineSessionUxState,
    #[serde(default)]
    pub online_session_status_message: String,
    #[serde(default)]
    pub online_host_owns_save: bool,
    #[serde(default)]
    pub online_player_slot: Option<u8>,
    #[serde(default)]
    pub online_session_limitations: Vec<String>,
    #[serde(default = "default_online_player_name")]
    pub online_player_name: String,
    #[serde(default)]
    pub online_remote_player_name: Option<String>,
    #[serde(default)]
    pub online_remote_player_ready: bool,
    #[serde(default)]
    pub online_remote_player_connected: bool,
    #[serde(default = "default_online_descriptor_path")]
    pub online_descriptor_path: PathBuf,
    #[serde(default = "default_online_host_bind_addr")]
    pub online_host_bind_addr: SocketAddr,
    #[serde(default = "default_online_host_advertise_addr")]
    pub online_host_advertise_addr: SocketAddr,
    #[serde(default = "default_online_client_bind_addr")]
    pub online_client_bind_addr: SocketAddr,
    #[serde(default = "default_online_gameplay_ticks")]
    pub online_gameplay_ticks: u32,
    #[serde(default)]
    pub online_diagnostic_controller_mode: String,
    #[serde(default)]
    pub online_diagnostic_last_tick: String,
    #[serde(default)]
    pub online_local_ready: bool,
    #[serde(default, skip)]
    pub online_network_task_request: Option<OnlineNetworkTaskRequest>,
    #[serde(default)]
    pub local_multiplayer_requested: bool,
    #[serde(default)]
    pub local_multiplayer_active: bool,
    #[serde(default)]
    pub local_multiplayer_player_slots: u8,
    #[serde(default)]
    pub local_multiplayer_status_message: String,
    pub current_zone: Option<SurfaceZone>,
    #[serde(default)]
    pub interior_zone: Option<SurfaceZone>,
    #[serde(default)]
    pub interior_x: f32,
    #[serde(default)]
    pub interior_facing: f32,
    pub contracts: ContractLog,
    pub run_mode: RunMode,
    pub modal: Option<ModalScreen>,
    pub selected_menu_item: usize,
    #[serde(default)]
    pub selected_title_item: usize,
    #[serde(default)]
    pub selected_pause_item: usize,
    pub show_details: bool,
    #[serde(default)]
    pub request_exit: bool,
    #[serde(default)]
    pub won_game: bool,
    #[serde(default)]
    pub deep_claim_status: DeepClaimStatus,
    #[serde(default)]
    pub town_development: TownDevelopment,
    #[serde(default)]
    pub collection_log: CollectionLog,
    #[serde(default)]
    pub escape_sequence_seconds: f32,
    #[serde(default)]
    pub explored_tiles: Vec<bool>,
    #[serde(default)]
    pub last_depot_receipt: String,
    #[serde(default)]
    pub depot_receipts: Vec<String>,
    pub deepest_tile_reached: i32,
    #[serde(default)]
    pub total_earnings: u32,
    #[serde(default)]
    pub total_resources_refined: u32,
    #[serde(default)]
    pub best_return_depth: i32,
    #[serde(default)]
    pub most_valuable_cargo_run: u32,
    #[serde(default)]
    pub expeditions_completed: u32,
    #[serde(default)]
    pub infrastructure_built: u32,
    #[serde(default)]
    pub last_run_summary: String,
    #[serde(default)]
    pub fastest_star_core_seconds: Option<f32>,
    #[serde(default)]
    pub legendary_blueprints: std::collections::BTreeSet<LegendaryBlueprint>,
    #[serde(default)]
    pub rig_part_inventory: std::collections::BTreeSet<RigPartKind>,
    #[serde(default)]
    pub equipped_rig_parts: std::collections::BTreeMap<RigSlot, RigPartKind>,
    #[serde(default)]
    pub challenge_badges: std::collections::BTreeSet<ChallengeBadge>,
    #[serde(default)]
    pub cosmetic_skins: std::collections::BTreeSet<CosmeticRigSkin>,
    #[serde(default)]
    pub rescue_count: u32,
    #[serde(default)]
    pub artifacts_found: u32,
    #[serde(default)]
    pub trip_best_depth: i32,
    #[serde(default)]
    pub trip_seconds: f32,
    #[serde(default)]
    pub deep_instability: f32,
    #[serde(default)]
    pub deep_reward_milestone: i32,
    #[serde(default)]
    pub return_streak: u32,
    #[serde(default)]
    pub play_seconds: f32,
    #[serde(default)]
    pub last_delta_seconds: f32,
    #[serde(default)]
    pub update_ticks: u64,
    pub next_milestone_tile: i32,
    #[serde(default)]
    pub current_layer_band: i32,
    pub game_over: bool,
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    #[serde(default = "default_fullscreen")]
    pub fullscreen: bool,
    #[serde(default)]
    pub last_rescue_x: Option<f32>,
    #[serde(default)]
    pub last_rescue_y: Option<f32>,
    #[serde(default)]
    pub last_rescue_summary: String,
    #[serde(default)]
    pub lost_cargo_x: Option<f32>,
    #[serde(default)]
    pub lost_cargo_y: Option<f32>,
    #[serde(default)]
    pub lost_cargo_count: u32,
    #[serde(default)]
    pub lost_minerals: std::collections::BTreeMap<MineralKind, u32>,
    #[serde(default)]
    pub lost_artifacts: std::collections::BTreeMap<ArtifactKind, u32>,
    pub camera_x: f32,
    pub camera_y: f32,
    #[serde(default)]
    pub camera_intro_seconds: f32,
    pub drill_flash_seconds: f32,
    #[serde(default)]
    pub active_drill: Option<DrillState>,
    pub dust_particles: Vec<DustParticle>,
    #[serde(default)]
    pub hazard_clouds: Vec<HazardCloud>,
    #[serde(default)]
    pub placed_bombs: Vec<PlacedBomb>,
    #[serde(default)]
    pub infrastructure: Vec<PlacedInfrastructure>,
    #[serde(default)]
    pub service_animation: Option<ServiceAnimation>,
    #[serde(default)]
    pub service_animation_seconds: f32,
    #[serde(default)]
    pub market_salt: u32,
    #[serde(default)]
    pub market_history: Vec<u32>,
    #[serde(default)]
    pub mineral_market_history: std::collections::BTreeMap<MineralKind, Vec<u32>>,
    #[serde(default)]
    pub scanner_pulse_seconds: f32,
    #[serde(default)]
    pub scanner_cooldown_seconds: f32,
    #[serde(default)]
    pub town_event_day: u32,
    #[serde(default)]
    pub town_event: String,
    #[serde(default)]
    pub active_world_events: Vec<ActiveWorldEvent>,
    #[serde(default)]
    pub scan_markers: Vec<ScanMarker>,
    #[serde(default)]
    pub collapse_warnings: Vec<TilePosition>,
    #[serde(default)]
    pub side_contract_active: bool,
    #[serde(default)]
    pub side_contract_kind: SideContractKind,
    #[serde(default)]
    pub side_contract_target: Option<TileKind>,
    #[serde(default)]
    pub side_contract_required: u32,
    #[serde(default)]
    pub active_side_contracts: Vec<SideContract>,
    #[serde(default)]
    pub expedition_offers: Vec<Expedition>,
    #[serde(default)]
    pub active_expeditions: Vec<Expedition>,
    #[serde(default)]
    pub falling_boulders: Vec<FallingBoulder>,
    #[serde(default)]
    pub spark_particles: Vec<SparkParticle>,
    #[serde(default)]
    pub camera_shake_seconds: f32,
    #[serde(default)]
    pub camera_shake_strength: f32,
    #[serde(default)]
    pub screen_flash_seconds: f32,
    pub sound_cues: Vec<SoundCue>,
    #[serde(default)]
    pub settings_dirty: bool,
    #[serde(default)]
    pub save_dirty: bool,
    #[serde(skip)]
    pub visual_changes: VisualChanges,
}

impl GameState {
    #[must_use]
    pub fn clone_for_save(&self) -> Self {
        let mut saved = self.clone();
        saved.run_mode = RunMode::Playing;
        saved.interior_zone = None;
        saved.modal = None;
        saved.request_exit = false;
        saved.show_details = false;
        saved.save_dirty = false;
        saved.active_drill = None;
        saved.dust_particles.clear();
        saved.hazard_clouds.clear();
        saved.falling_boulders.clear();
        saved.spark_particles.clear();
        saved.camera_shake_seconds = 0.0;
        saved.camera_shake_strength = 0.0;
        saved.camera_intro_seconds = 0.0;
        saved.screen_flash_seconds = 0.0;
        saved.sound_cues.clear();
        saved.settings_dirty = false;
        saved.visual_changes = VisualChanges::default();
        saved.last_delta_seconds = 0.0;
        saved
    }

    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "new game state initializes all saved systems"
    )]
    pub fn new() -> Self {
        Self {
            terrain: Terrain::new_seeded(WORLD_WIDTH, WORLD_HEIGHT, WORLD_SEED),
            player: Player::new(PLAYER_SPAWN_X, PLAYER_SPAWN_Y),
            save_version: current_save_version(),
            message: "Mine ore, sell cargo, and buy upgrades. Press E at surface buildings."
                .to_owned(),
            online_session_state: OnlineSessionUxState::Idle,
            online_session_status_message: String::new(),
            online_host_owns_save: false,
            online_player_slot: None,
            online_session_limitations: Self::online_session_limitations(),
            online_player_name: default_online_player_name(),
            online_remote_player_name: None,
            online_remote_player_ready: false,
            online_remote_player_connected: false,
            online_descriptor_path: default_online_descriptor_path(),
            online_host_bind_addr: default_online_host_bind_addr(),
            online_host_advertise_addr: default_online_host_advertise_addr(),
            online_client_bind_addr: default_online_client_bind_addr(),
            online_gameplay_ticks: default_online_gameplay_ticks(),
            online_diagnostic_controller_mode: String::new(),
            online_diagnostic_last_tick: String::new(),
            online_local_ready: false,
            online_network_task_request: None,
            local_multiplayer_requested: false,
            local_multiplayer_active: false,
            local_multiplayer_player_slots: 1,
            local_multiplayer_status_message: String::new(),
            current_zone: None,
            interior_zone: None,
            interior_x: 88.0,
            interior_facing: 1.0,
            contracts: ContractLog::new(),
            run_mode: RunMode::Title,
            modal: None,
            selected_menu_item: 0,
            selected_title_item: 0,
            selected_pause_item: 0,
            show_details: false,
            request_exit: false,
            won_game: false,
            deep_claim_status: DeepClaimStatus::Locked,
            town_development: TownDevelopment::default(),
            collection_log: CollectionLog::default(),
            escape_sequence_seconds: 0.0,
            explored_tiles: vec![false; (WORLD_WIDTH * WORLD_HEIGHT) as usize],
            last_depot_receipt: String::new(),
            depot_receipts: Vec::new(),
            deepest_tile_reached: 0,
            total_earnings: 0,
            total_resources_refined: 0,
            best_return_depth: 0,
            most_valuable_cargo_run: 0,
            expeditions_completed: 0,
            infrastructure_built: 0,
            last_run_summary: String::new(),
            fastest_star_core_seconds: None,
            legendary_blueprints: std::collections::BTreeSet::new(),
            rig_part_inventory: std::collections::BTreeSet::from([
                RigPartKind::TitanDrill,
                RigPartKind::NeedleDrill,
                RigPartKind::ResonanceDrill,
                RigPartKind::LightweightEngine,
                RigPartKind::HaulerEngine,
                RigPartKind::BurstEngine,
                RigPartKind::ThermalHull,
                RigPartKind::ImpactHull,
                RigPartKind::PressureHull,
                RigPartKind::ProspectorScanner,
                RigPartKind::HazardScanner,
                RigPartKind::RelicScanner,
                RigPartKind::CargoBalloon,
                RigPartKind::ArmoredCargoBay,
                RigPartKind::SortedCargoRack,
            ]),
            equipped_rig_parts: std::collections::BTreeMap::new(),
            challenge_badges: std::collections::BTreeSet::new(),
            cosmetic_skins: std::collections::BTreeSet::new(),
            rescue_count: 0,
            artifacts_found: 0,
            trip_best_depth: 0,
            trip_seconds: 0.0,
            deep_instability: 0.0,
            deep_reward_milestone: 0,
            return_streak: 0,
            play_seconds: 0.0,
            last_delta_seconds: 0.0,
            update_ticks: 0,
            next_milestone_tile: 20,
            current_layer_band: 0,
            game_over: false,
            master_volume: default_master_volume(),
            fullscreen: default_fullscreen(),
            last_rescue_x: None,
            last_rescue_y: None,
            last_rescue_summary: String::new(),
            lost_cargo_x: None,
            lost_cargo_y: None,
            lost_cargo_count: 0,
            lost_minerals: std::collections::BTreeMap::new(),
            lost_artifacts: std::collections::BTreeMap::new(),
            camera_x: initial_camera_x(),
            camera_y: initial_camera_y(),
            camera_intro_seconds: CAMERA_INTRO_SECONDS,
            drill_flash_seconds: 0.0,
            active_drill: None,
            dust_particles: Vec::new(),
            hazard_clouds: Vec::new(),
            placed_bombs: Vec::new(),
            infrastructure: Vec::new(),
            service_animation: None,
            service_animation_seconds: 0.0,
            market_salt: 0,
            market_history: vec![market_factor(0, 0)],
            mineral_market_history: initial_mineral_market_history(0, 0),
            scanner_pulse_seconds: 0.0,
            scanner_cooldown_seconds: 0.0,
            town_event_day: 0,
            town_event: "Normal market conditions.".to_owned(),
            active_world_events: Vec::new(),
            scan_markers: Vec::new(),
            collapse_warnings: Vec::new(),
            side_contract_active: false,
            side_contract_kind: SideContractKind::Cargo,
            side_contract_target: None,
            side_contract_required: 0,
            active_side_contracts: Vec::new(),
            expedition_offers: Vec::new(),
            active_expeditions: Vec::new(),
            falling_boulders: Vec::new(),
            spark_particles: Vec::new(),
            camera_shake_seconds: 0.0,
            camera_shake_strength: 0.0,
            screen_flash_seconds: 0.0,
            sound_cues: Vec::new(),
            settings_dirty: false,
            save_dirty: false,
            visual_changes: VisualChanges {
                full_terrain_refresh: true,
                changed_tiles: Vec::new(),
            },
        }
    }

    #[must_use]
    pub const fn visual_changes(&self) -> &VisualChanges {
        &self.visual_changes
    }

    pub const fn mark_full_terrain_refresh(&mut self) {
        self.visual_changes.full_terrain_refresh = true;
    }

    fn mark_tile_visual_changed(&mut self, position: TilePosition) {
        self.visual_changes.changed_tiles.push(position);
    }

    fn mark_exploration_visual_changed(&mut self, position: TilePosition) {
        for y in position.y - EXPLORATION_VISUAL_CHANGE_RADIUS_TILES
            ..=position.y + EXPLORATION_VISUAL_CHANGE_RADIUS_TILES
        {
            for x in position.x - EXPLORATION_VISUAL_CHANGE_RADIUS_TILES
                ..=position.x + EXPLORATION_VISUAL_CHANGE_RADIUS_TILES
            {
                self.mark_tile_visual_changed(TilePosition { x, y });
            }
        }
    }

    fn mark_tiles_visual_changed<I>(&mut self, positions: I)
    where
        I: IntoIterator<Item = TilePosition>,
    {
        self.visual_changes.changed_tiles.extend(positions);
    }

    pub fn migrate_after_load(&mut self) {
        self.terrain.ensure_depth(WORLD_HEIGHT);
        let expected_tiles = (self.terrain.width() * self.terrain.height()) as usize;
        if self.explored_tiles.len() != expected_tiles {
            self.explored_tiles = vec![false; expected_tiles];
        }
        self.request_exit = false;
        self.save_dirty = false;
        self.visual_changes = VisualChanges {
            full_terrain_refresh: true,
            changed_tiles: Vec::new(),
        };
        self.contracts.migrate_after_load();
    }

    #[allow(
        clippy::too_many_lines,
        reason = "top-level mode dispatcher keeps frame order explicit"
    )]
    pub fn update(&mut self, input: PlayerInput, delta_seconds: f32) {
        self.last_delta_seconds = delta_seconds;
        self.update_ticks = self.update_ticks.saturating_add(1);
        self.sound_cues.clear();
        self.show_details = input.details;
        self.handle_save_load(input);
        if self.handle_exit_modal(input) {
            return;
        }
        if input.exit_requested {
            self.request_exit_or_prompt();
            return;
        }
        if self.run_mode != RunMode::Title && input_changes_game(input) {
            self.save_dirty = true;
        }
        if input.map {
            self.modal = if self.modal == Some(ModalScreen::Map) {
                None
            } else {
                Some(ModalScreen::Map)
            };
        }
        if input.help {
            self.modal = if self.modal == Some(ModalScreen::Help) {
                None
            } else {
                Some(ModalScreen::Help)
            };
        }
        if input.volume_up {
            self.master_volume = (self.master_volume + 0.1).min(1.0);
            self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
            self.settings_dirty = true;
            self.sound_cues.push(SoundCue::Ui);
        }
        if input.volume_down {
            self.master_volume = (self.master_volume - 0.1).max(0.0);
            self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
            self.settings_dirty = true;
            self.sound_cues.push(SoundCue::Ui);
        }
        if input.fullscreen {
            self.fullscreen = !self.fullscreen;
            self.message = if self.fullscreen {
                "Fullscreen preference saved. Restart/toggle window integration pending.".to_owned()
            } else {
                "Windowed preference saved.".to_owned()
            };
            self.settings_dirty = true;
            self.sound_cues.push(SoundCue::Ui);
        }
        self.update_particles(delta_seconds);
        self.update_placed_bombs(delta_seconds);
        self.update_service_animation(delta_seconds);
        self.update_scanner_timers(delta_seconds);
        self.update_boulders(delta_seconds);
        self.update_world_event_hazards();
        self.camera_shake_seconds = (self.camera_shake_seconds - delta_seconds).max(0.0);
        self.screen_flash_seconds = (self.screen_flash_seconds - delta_seconds).max(0.0);
        self.update_hazards(delta_seconds);
        self.recover_lost_cargo_if_near();
        self.reveal_near_player();
        self.reveal_scanner_area();
        self.update_collection_rewards();
        self.update_survey_drones();
        self.update_persistent_ore_prediction();
        self.drill_flash_seconds = (self.drill_flash_seconds - delta_seconds).max(0.0);
        if matches!(self.run_mode, RunMode::Playing | RunMode::Interior)
            && !self.game_over
            && !self.won_game
        {
            self.play_seconds += delta_seconds;
        }

        match self.run_mode {
            RunMode::Title => {
                if self.handle_modal(input) {
                    return;
                }
                self.handle_title_menu(input);
                return;
            }
            RunMode::Paused => {
                self.handle_pause_menu(input);
                return;
            }
            RunMode::Playing | RunMode::Interior => {}
        }

        if self.run_mode == RunMode::Interior {
            self.handle_interior(input, delta_seconds);
            return;
        }

        if self.game_over {
            self.handle_rescue(input);
            self.update_camera(delta_seconds);
            return;
        }

        self.current_zone = surface_zone_at(self.player.x, self.player.y);
        self.update_npc_story_records();
        if self.handle_modal(input) {
            self.update_camera(delta_seconds);
            return;
        }

        if input.pause || input.cancel {
            self.run_mode = RunMode::Paused;
            return;
        }

        self.handle_interaction(input);
        self.handle_scanner(input);
        self.handle_bomb(input);
        self.handle_infrastructure_placement(input);
        self.apply_movement(input, delta_seconds);
        self.update_drilling(input, delta_seconds);
        self.apply_depth_pressure(delta_seconds);
        self.apply_lava_damage(delta_seconds);
        self.update_depth_milestones();
        self.update_deep_run_pressure(delta_seconds);
        self.update_escape_sequence(delta_seconds);
        self.update_layer_band();
        self.award_return_bonus();
        self.update_warning_messages();
        self.update_status_messages();
        self.check_failure();
        self.update_camera(delta_seconds);
    }

    fn handle_title_menu(&mut self, input: PlayerInput) {
        let options = Self::title_options();
        if input.menu_up {
            self.selected_title_item = self.selected_title_item.saturating_sub(1);
        }
        if input.menu_down {
            self.selected_title_item = (self.selected_title_item + 1).min(options.len() - 1);
        }
        self.selected_title_item = self.selected_title_item.min(options.len() - 1);

        if input.cancel {
            self.request_exit = true;
            return;
        }
        if !input.confirm {
            return;
        }

        match options[self.selected_title_item] {
            TitleOption::Resume => self.load_latest_into_self(),
            TitleOption::NewGame => self.start_new_game(),
            TitleOption::OnlineMultiplayer => {
                self.modal = Some(ModalScreen::OnlineMultiplayer);
                self.selected_menu_item = 0;
            }
            TitleOption::LocalMultiplayer => self.start_local_multiplayer_request(),
            TitleOption::LoadSlot => {
                self.modal = Some(ModalScreen::LoadSlots);
                self.selected_menu_item = 0;
            }
            TitleOption::Options => {
                self.modal = Some(ModalScreen::Options);
                self.selected_menu_item = 0;
            }
            TitleOption::Exit => self.request_exit = true,
        }
    }

    #[must_use]
    pub fn title_options() -> Vec<TitleOption> {
        if saves_exist() {
            vec![
                TitleOption::Resume,
                TitleOption::NewGame,
                TitleOption::LocalMultiplayer,
                TitleOption::OnlineMultiplayer,
                TitleOption::LoadSlot,
                TitleOption::Options,
                TitleOption::Exit,
            ]
        } else {
            vec![
                TitleOption::NewGame,
                TitleOption::LocalMultiplayer,
                TitleOption::OnlineMultiplayer,
                TitleOption::Options,
                TitleOption::Exit,
            ]
        }
    }

    #[allow(
        dead_code,
        reason = "online task request drain is exercised by tests until desktop event-loop ownership calls it"
    )]
    pub fn take_online_network_task_request(&mut self) -> Option<OnlineNetworkTaskRequest> {
        mem::take(&mut self.online_network_task_request)
    }

    #[allow(
        dead_code,
        reason = "online task reducer is exercised by tests until desktop event-loop ownership calls it"
    )]
    pub fn apply_online_network_task_result(&mut self, result: OnlineNetworkTaskResult) {
        match result {
            OnlineNetworkTaskResult::Hosted(snapshot)
            | OnlineNetworkTaskResult::JoinedDescriptor(snapshot) => {
                self.apply_real_online_session_ux(snapshot);
            }
            OnlineNetworkTaskResult::Connected(snapshot)
            | OnlineNetworkTaskResult::Reconnected(snapshot) => {
                self.apply_real_online_session_ux(snapshot);
                self.enter_online_playing_session();
            }
            OnlineNetworkTaskResult::Failed(message) => {
                self.online_session_state = OnlineSessionUxState::Error;
                self.online_network_task_request = None;
                self.online_local_ready = false;
                self.online_remote_player_ready = false;
                self.online_remote_player_connected = false;
                self.clear_online_diagnostics();
                self.online_session_status_message = Self::online_failure_status_message(&message);
                self.message = self.online_session_status_message.clone();
            }
            OnlineNetworkTaskResult::Shutdown => {
                self.online_session_state = OnlineSessionUxState::Shutdown;
                self.online_network_task_request = None;
                self.online_local_ready = false;
                self.online_remote_player_ready = false;
                self.online_remote_player_connected = false;
                self.modal = None;
                self.clear_online_diagnostics();
                "Online session shutdown acknowledged."
                    .clone_into(&mut self.online_session_status_message);
                self.message = self.online_session_status_message.clone();
            }
        }
    }

    #[allow(
        dead_code,
        reason = "real online UX bridge is exercised by tests until desktop async UI ownership calls it"
    )]
    pub fn apply_real_online_session_ux(&mut self, snapshot: RealOnlineSessionUxSnapshot) {
        self.online_session_state = snapshot.state;
        self.online_host_owns_save = snapshot.host_owns_save;
        self.online_player_slot = snapshot.player_slot;
        self.online_remote_player_connected = snapshot.state == OnlineSessionUxState::Connected;
        if self.online_remote_player_connected && self.online_remote_player_name.is_none() {
            self.online_remote_player_name = Some("Remote miner".to_owned());
        }
        self.online_session_status_message = snapshot.status_message;
        self.message = self.online_session_status_message.clone();
    }

    pub fn apply_online_diagnostics(
        &mut self,
        controller_mode: impl Into<String>,
        last_tick: impl Into<String>,
    ) {
        self.online_diagnostic_controller_mode = controller_mode.into();
        self.online_diagnostic_last_tick = last_tick.into();
    }

    pub fn clear_online_diagnostics(&mut self) {
        self.online_diagnostic_controller_mode.clear();
        self.online_diagnostic_last_tick.clear();
    }

    #[must_use]
    pub fn online_failure_status_message(error: &str) -> String {
        let normalized = error.to_ascii_lowercase();
        if normalized.contains("version") {
            return "Connection error: game version/protocol mismatch. Update both players to the same build.".to_owned();
        }
        if normalized.contains("certificate") || normalized.contains("cert") {
            return "Connection error: host descriptor certificate could not be trusted. Regenerate and re-share the descriptor.".to_owned();
        }
        if normalized.contains("descriptor")
            || normalized.contains("json")
            || normalized.contains("parse")
        {
            return "Connection error: host descriptor could not be read. Check the descriptor file/path and ask the host to share it again.".to_owned();
        }
        if normalized.contains("timeout") || normalized.contains("timed out") {
            return "Connection timed out: verify the host is running, the advertised UDP port is open, and both machines are on the expected LAN/VPN.".to_owned();
        }
        if normalized.contains("refused") || normalized.contains("unreachable") {
            return "Connection refused or unreachable: verify host address, firewall, and UDP port forwarding/LAN routing.".to_owned();
        }
        if normalized.contains("reconnect") || normalized.contains("session token") {
            return "Reconnect failed: the previous online session is no longer available. Rejoin from the host descriptor.".to_owned();
        }
        if normalized.contains("shutdown") || normalized.contains("closed") {
            return "Online session ended: the host closed the session.".to_owned();
        }
        format!("Connection error: {error}")
    }

    const fn can_write_local_save(&self) -> bool {
        !matches!(
            self.online_session_state,
            OnlineSessionUxState::Hosting
                | OnlineSessionUxState::Joining
                | OnlineSessionUxState::Connected
                | OnlineSessionUxState::Reconnecting
        ) || self.online_host_owns_save
    }

    fn block_joined_client_save(&mut self) -> bool {
        if self.can_write_local_save() {
            return false;
        }
        "Save blocked: host owns the online session save.".clone_into(&mut self.message);
        true
    }

    fn enter_online_playing_session(&mut self) {
        self.run_mode = RunMode::Playing;
        self.modal = None;
        self.selected_menu_item = 0;
        self.local_multiplayer_requested = false;
        self.local_multiplayer_active = false;
        self.online_local_ready = true;
        if self.online_session_status_message.is_empty() {
            "Online session connected; entering gameplay."
                .clone_into(&mut self.online_session_status_message);
        }
        self.message = self.online_session_status_message.clone();
    }

    #[must_use]
    pub fn online_multiplayer_status_lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(10);
        lines.push(
            "Transport: Quinn/QUIC real socket IO enabled for direct-connect host/join/reconnect."
                .to_owned(),
        );
        if let Some(request) = &self.online_network_task_request {
            lines.push(format!("Pending network task: {request:?}"));
        } else {
            lines.push("Pending network task: none".to_owned());
        }
        lines.push(format!(
            "State: {:?} | Role: {} | Ready: {} | Host owns save: {} | Slot: {}",
            self.online_session_state,
            self.online_role_label(),
            if self.online_local_ready { "yes" } else { "no" },
            self.online_host_owns_save,
            self.online_player_slot
                .map_or_else(|| "unassigned".to_owned(), |slot| slot.to_string())
        ));
        lines.push(format!(
            "Role guidance: {}",
            self.online_role_guidance_line()
        ));
        lines.push(format!(
            "Descriptor file: {} | inspect before join: yes | share after host publish: yes",
            self.online_descriptor_path.display()
        ));
        lines.push(self.online_session_status_message.clone());
        lines.push(format!(
            "Online diagnostics: controller={}, last tick={}",
            if self.online_diagnostic_controller_mode.is_empty() {
                "none"
            } else {
                self.online_diagnostic_controller_mode.as_str()
            },
            if self.online_diagnostic_last_tick.is_empty() {
                "none"
            } else {
                self.online_diagnostic_last_tick.as_str()
            }
        ));
        lines.push(self.online_save_policy_line());
        lines.extend(self.online_lobby_participant_lines());
        lines.extend(self.online_direct_connect_setup_lines());
        lines.extend(Self::online_session_limitations());
        lines
    }

    #[must_use]
    pub const fn online_role_guidance_line(&self) -> &'static str {
        if self.online_host_owns_save {
            "You are the authoritative host: keep this app running, share the descriptor, and write saves from here."
        } else if self.online_player_slot.is_some() {
            "You are a joined client: play through the host, do not write local saves, and reconnect with host approval."
        } else {
            "Choose Host to own the session/save or Join to connect with a descriptor from the host."
        }
    }

    #[must_use]
    pub fn online_save_policy_line(&self) -> String {
        if self.can_write_local_save() {
            "Save policy: local save writes allowed for this player/session.".to_owned()
        } else {
            "Save policy: host owns the online save; joined clients cannot write local saves."
                .to_owned()
        }
    }

    #[must_use]
    pub fn online_lobby_participant_lines(&self) -> Vec<String> {
        let local_slot = self
            .online_player_slot
            .map_or_else(|| "unassigned".to_owned(), |slot| slot.to_string());
        let remote_name = self
            .online_remote_player_name
            .as_deref()
            .unwrap_or("Waiting for player");
        let remote_slot = match self.online_player_slot {
            Some(1) => "2",
            Some(2) => "1",
            _ => "unassigned",
        };
        vec![
            format!(
                "Local player: {} | slot {} | role {} | ready {} | connected {}",
                self.online_player_name,
                local_slot,
                self.online_role_label(),
                if self.online_local_ready { "yes" } else { "no" },
                if self.online_session_state == OnlineSessionUxState::Connected {
                    "yes"
                } else {
                    "pending"
                }
            ),
            format!(
                "Remote player: {remote_name} | slot {remote_slot} | role {} | ready {} | connected {}",
                if self.online_host_owns_save {
                    "client"
                } else {
                    "host"
                },
                if self.online_remote_player_ready {
                    "yes"
                } else {
                    "unknown"
                },
                if self.online_remote_player_connected {
                    "yes"
                } else {
                    "pending"
                }
            ),
        ]
    }

    #[must_use]
    pub const fn online_role_label(&self) -> &'static str {
        if self.online_host_owns_save {
            "host"
        } else if self.online_player_slot.is_some() {
            "client"
        } else {
            "unassigned"
        }
    }

    #[must_use]
    pub fn online_direct_connect_setup_lines(&self) -> Vec<String> {
        let descriptor = self.online_descriptor_path.display();
        vec![
            format!(
                "Direct-connect config: descriptor `{descriptor}`, host bind {}, advertise {}, client bind {}, ticks {}.",
                self.online_host_bind_addr,
                self.online_host_advertise_addr,
                self.online_client_bind_addr,
                self.online_gameplay_ticks
            ),
            "Host flow: share the generated descriptor with the joining player after hosting starts.".to_owned(),
            format!(
                "Host CLI helper: drillgame --online-host-gameplay-descriptor-file-on-addr {descriptor} {} {} {}",
                self.online_host_bind_addr,
                self.online_host_advertise_addr,
                self.online_gameplay_ticks
            ),
            format!(
                "Join CLI helper: drillgame --online-join-gameplay-descriptor-file-on-addr {descriptor} {} {}",
                self.online_client_bind_addr, self.online_gameplay_ticks
            ),
            format!(
                "QA helper: drillgame --online-lan-qa-checklist-md {descriptor} {} {} {} {}",
                self.online_host_bind_addr,
                self.online_host_advertise_addr,
                self.online_client_bind_addr,
                self.online_gameplay_ticks
            ),
        ]
    }

    #[must_use]
    pub fn online_session_limitations() -> Vec<String> {
        vec![
            "Real localhost Quinn socket IO is available for host/join/tick/reconnect coverage; direct-address game-window UX remains desktop-first.".to_owned(),
            "NAT traversal, matchmaking, invites, and host migration are intentionally unsupported.".to_owned(),
        ]
    }

    fn request_online_gameplay_start(&mut self) {
        if self.online_session_state != OnlineSessionUxState::Connected {
            "Start blocked: connect host and client before entering online gameplay."
                .clone_into(&mut self.online_session_status_message);
        } else if !self.online_local_ready {
            "Start blocked: toggle local ready before entering online gameplay."
                .clone_into(&mut self.online_session_status_message);
        } else if !self.online_remote_player_connected {
            "Start blocked: waiting for the remote player connection."
                .clone_into(&mut self.online_session_status_message);
        } else if !self.online_remote_player_ready {
            "Start blocked: waiting for the remote player to toggle ready."
                .clone_into(&mut self.online_session_status_message);
        } else {
            "Starting online gameplay from connected direct-connect session."
                .clone_into(&mut self.online_session_status_message);
            self.enter_online_playing_session();
        }
    }

    fn close_online_multiplayer_menu(&mut self) {
        self.online_network_task_request = None;
        if self.online_session_state != OnlineSessionUxState::Connected {
            self.online_local_ready = false;
            self.online_remote_player_ready = false;
            self.online_remote_player_connected = false;
        }
        self.online_session_state = match self.online_session_state {
            OnlineSessionUxState::Connected => OnlineSessionUxState::Connected,
            OnlineSessionUxState::Shutdown => OnlineSessionUxState::Shutdown,
            _ => OnlineSessionUxState::Idle,
        };
        self.modal = None;
        "Closed online multiplayer menu; no network task queued."
            .clone_into(&mut self.online_session_status_message);
        self.message = self.online_session_status_message.clone();
    }

    fn cycle_online_descriptor_path(&mut self) {
        self.online_descriptor_path =
            if self.online_descriptor_path == default_online_descriptor_path() {
                alternate_online_descriptor_path()
            } else if self.online_descriptor_path == alternate_online_descriptor_path() {
                join_online_descriptor_path()
            } else {
                default_online_descriptor_path()
            };
        self.online_session_status_message = format!(
            "Descriptor path selected: {}",
            self.online_descriptor_path.display()
        );
    }

    fn inspect_online_descriptor_path(&mut self) {
        match std::fs::read_to_string(&self.online_descriptor_path)
            .map_err(|error| format!("descriptor read failed: {error}"))
            .and_then(|contents| {
                serde_json::from_str::<crate::multiplayer::QuinnHostConnectionDescriptor>(&contents)
                    .map_err(|error| format!("descriptor parse failed: {error}"))
            }) {
            Ok(descriptor) => {
                self.online_session_status_message = format!(
                    "Descriptor OK: path={}, host={}, server={}, cert={} bytes",
                    self.online_descriptor_path.display(),
                    descriptor.host_addr,
                    descriptor.server_name,
                    descriptor.certificate_der.len()
                );
            }
            Err(error) => {
                self.online_session_state = OnlineSessionUxState::Error;
                self.online_session_status_message = format!(
                    "Descriptor inspect failed for {}: {error}",
                    self.online_descriptor_path.display()
                );
            }
        }
    }

    fn cycle_online_host_address_preset(&mut self) {
        if self.online_host_bind_addr == default_online_host_bind_addr()
            && self.online_host_advertise_addr == default_online_host_advertise_addr()
        {
            self.online_host_bind_addr = lan_online_host_bind_addr();
            self.online_host_advertise_addr = lan_online_host_advertise_addr();
        } else if self.online_host_bind_addr == lan_online_host_bind_addr()
            && self.online_host_advertise_addr == lan_online_host_advertise_addr()
        {
            self.online_host_bind_addr = localhost_ephemeral_online_host_bind_addr();
            self.online_host_advertise_addr = localhost_ephemeral_online_host_advertise_addr();
        } else {
            self.online_host_bind_addr = default_online_host_bind_addr();
            self.online_host_advertise_addr = default_online_host_advertise_addr();
        }
        self.online_session_status_message = format!(
            "Host address preset selected: bind {}, advertise {}",
            self.online_host_bind_addr, self.online_host_advertise_addr
        );
    }

    fn cycle_online_gameplay_ticks(&mut self) {
        self.online_gameplay_ticks = match self.online_gameplay_ticks {
            0..=60 => 120,
            61..=120 => 300,
            _ => 60,
        };
        self.online_session_status_message = format!(
            "Gameplay smoke tick count selected: {} ticks",
            self.online_gameplay_ticks
        );
    }

    fn confirm_online_multiplayer(&mut self) {
        match self.selected_menu_item {
            0 => {
                self.online_session_state = OnlineSessionUxState::Hosting;
                self.online_network_task_request =
                    Some(OnlineNetworkTaskRequest::HostDescriptorFile {
                        path: self.online_descriptor_path.clone(),
                    });
                self.online_host_owns_save = true;
                self.online_player_slot = Some(1);
                self.online_local_ready = false;
                self.online_remote_player_name = None;
                self.online_remote_player_ready = false;
                self.online_remote_player_connected = false;
                self.online_session_status_message = format!(
                    "Hosting direct-connect descriptor at {}. Host owns save/session authority.",
                    self.online_descriptor_path.display()
                );
            }
            1 => {
                self.online_session_state = OnlineSessionUxState::Joining;
                self.online_network_task_request =
                    Some(OnlineNetworkTaskRequest::JoinDescriptorFile {
                        path: self.online_descriptor_path.clone(),
                    });
                self.online_host_owns_save = false;
                self.online_player_slot = Some(2);
                self.online_local_ready = false;
                self.online_remote_player_name = Some("Host miner".to_owned());
                self.online_remote_player_ready = false;
                self.online_remote_player_connected = false;
                self.online_session_status_message = format!(
                    "Joining with descriptor {}. Waiting for host assignment.",
                    self.online_descriptor_path.display()
                );
            }
            2 => {
                self.online_session_state = OnlineSessionUxState::Reconnecting;
                self.online_network_task_request =
                    Some(OnlineNetworkTaskRequest::ReconnectDirectConnect);
                "Reconnect requested with previous session token."
                    .clone_into(&mut self.online_session_status_message);
            }
            3 => {
                self.cycle_online_descriptor_path();
            }
            4 => {
                self.inspect_online_descriptor_path();
            }
            5 => {
                self.cycle_online_host_address_preset();
            }
            6 => {
                self.cycle_online_gameplay_ticks();
            }
            7 => {
                self.online_session_state = OnlineSessionUxState::Timeout;
                "Connection timed out; retry or go back."
                    .clone_into(&mut self.online_session_status_message);
            }
            8 => {
                self.online_session_state = OnlineSessionUxState::Error;
                "Connection error: direct Quinn connection task failed."
                    .clone_into(&mut self.online_session_status_message);
            }
            9 => {
                self.online_session_state = OnlineSessionUxState::Shutdown;
                self.online_network_task_request = Some(OnlineNetworkTaskRequest::Shutdown);
                self.online_local_ready = false;
                self.modal = None;
                "Online session shutdown requested."
                    .clone_into(&mut self.online_session_status_message);
            }
            10 => {
                self.online_local_ready = !self.online_local_ready;
                if self.online_local_ready {
                    "Local player ready for online session start."
                } else {
                    "Local player not ready."
                }
                .clone_into(&mut self.online_session_status_message);
            }
            11 => {
                self.request_online_gameplay_start();
            }
            _ => {
                self.close_online_multiplayer_menu();
                self.sound_cues.push(SoundCue::Ui);
                return;
            }
        }
        self.message = self.online_session_status_message.clone();
        self.sound_cues.push(SoundCue::Ui);
    }

    fn start_new_game(&mut self) {
        let master_volume = self.master_volume;
        let fullscreen = self.fullscreen;
        *self = Self::new();
        self.master_volume = master_volume;
        self.fullscreen = fullscreen;
        self.run_mode = RunMode::Playing;
        self.save_dirty = true;
        self.sound_cues.push(SoundCue::Milestone);
        "Welcome to the dig site. Visit the depot for contracts.".clone_into(&mut self.message);
    }

    fn start_local_multiplayer_request(&mut self) {
        self.start_new_game();
        self.local_multiplayer_requested = true;
        "Local split-screen starting: Player 1 uses WASD, Player 2 uses arrow keys."
            .clone_into(&mut self.local_multiplayer_status_message);
        self.message = self.local_multiplayer_status_message.clone();
    }

    pub fn take_local_multiplayer_request(&mut self) -> bool {
        mem::take(&mut self.local_multiplayer_requested)
    }

    pub fn mark_local_multiplayer_active(&mut self, player_slots: u8) {
        self.local_multiplayer_active = true;
        self.local_multiplayer_player_slots = player_slots;
        self.local_multiplayer_status_message = format!(
            "Local split-screen active with {player_slots} players: Player 1 WASD, Player 2 arrow keys."
        );
        self.message = self.local_multiplayer_status_message.clone();
    }

    fn load_latest_into_self(&mut self) {
        match load_latest_game() {
            Ok(mut loaded) => {
                loaded.master_volume = self.master_volume;
                loaded.fullscreen = self.fullscreen;
                loaded.migrate_loaded_state();
                loaded.mark_full_terrain_refresh();
                *self = loaded;
                self.save_dirty = false;
                "Resumed saved session.".clone_into(&mut self.message);
            }
            Err(error) => self.message = format!("Resume failed: {error}"),
        }
    }

    const fn request_exit_or_prompt(&mut self) {
        if self.save_dirty {
            self.modal = Some(ModalScreen::UnsavedExitConfirm);
            self.selected_menu_item = 0;
        } else {
            self.modal = Some(ModalScreen::ExitConfirm);
        }
    }

    fn handle_exit_modal(&mut self, input: PlayerInput) -> bool {
        match self.modal {
            Some(ModalScreen::ExitConfirm) => {
                if input.cancel {
                    self.modal = None;
                } else if input.confirm {
                    self.request_exit = true;
                }
                true
            }
            Some(ModalScreen::UnsavedExitConfirm) => {
                if input.cancel {
                    self.modal = None;
                    return true;
                }
                if input.menu_up {
                    self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
                }
                if input.menu_down {
                    self.selected_menu_item = (self.selected_menu_item + 1).min(2);
                }
                if input.confirm {
                    match self.selected_menu_item {
                        0 => {
                            if self.block_joined_client_save() {
                                return true;
                            }
                            match save_game(self) {
                                Ok(()) => {
                                    self.save_dirty = false;
                                    self.request_exit = true;
                                }
                                Err(error) => {
                                    self.message = format!("Save before exit failed: {error}");
                                }
                            }
                        }
                        1 => self.request_exit = true,
                        _ => self.modal = None,
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn reveal_near_player(&mut self) {
        let center_x = (self.player.x / TILE_SIZE).floor() as i32;
        let center_y = (self.player.y / TILE_SIZE).floor() as i32;
        for y in center_y - 3..=center_y + 3 {
            for x in center_x - 4..=center_x + 4 {
                let position = TilePosition { x, y };
                if let Some(index) = self.tile_index(position)
                    && !self.explored_tiles[index]
                {
                    self.explored_tiles[index] = true;
                    self.mark_exploration_visual_changed(position);
                }
            }
        }
    }

    fn reveal_scanner_area(&mut self) {
        if self.player.scanner_level == 0 {
            return;
        }
        let center_x = (self.player.x / TILE_SIZE).floor() as i32;
        let center_y = (self.player.y / TILE_SIZE).floor() as i32;
        let radius = 3
            + i32::from(self.player.scanner_level) * 2
            + i32::from(self.town_development.scanner_lab_level);
        for y in center_y - radius..=center_y + radius {
            for x in center_x - radius..=center_x + radius {
                if (x - center_x).abs() + (y - center_y).abs() <= radius {
                    let position = TilePosition { x, y };
                    if let Some(index) = self.tile_index(position)
                        && !self.explored_tiles[index]
                    {
                        self.explored_tiles[index] = true;
                        self.mark_exploration_visual_changed(position);
                    }
                    if let Some(tile) = self.terrain.tile(position) {
                        self.collection_log.discover_tile(tile.kind);
                        let can_mark = scanner_can_mark(tile.kind, self.player.scanner_level)
                            || (self.has_equipped_part(RigPartKind::ProspectorScanner)
                                && matches!(tile.kind, TileKind::Ore(_)))
                            || (self.has_equipped_part(RigPartKind::HazardScanner)
                                && matches!(
                                    tile.kind,
                                    TileKind::Gas
                                        | TileKind::Lava
                                        | TileKind::MagmaVent
                                        | TileKind::ExplosivePocket
                                        | TileKind::PressurePocket
                                ))
                            || (self.has_equipped_part(RigPartKind::RelicScanner)
                                && matches!(tile.kind, TileKind::Artifact(_)));
                        if can_mark
                            && !self
                                .scan_markers
                                .iter()
                                .any(|marker| marker.position == position)
                        {
                            self.scan_markers.push(ScanMarker {
                                position,
                                kind: tile.kind,
                            });
                        }
                    }
                }
            }
        }
    }

    fn update_persistent_ore_prediction(&mut self) {
        if self.town_development.scanner_lab_level < 2 || !self.update_ticks.is_multiple_of(90) {
            return;
        }
        let center_x = (self.player.x / TILE_SIZE).floor() as i32;
        let center_y = (self.player.y / TILE_SIZE).floor() as i32;
        let depth_bias = 8 + i32::from(self.town_development.scanner_lab_level) * 4;
        let radius = 4 + i32::from(self.town_development.scanner_lab_level);
        for y in center_y..=center_y + depth_bias {
            for x in center_x - radius..=center_x + radius {
                let position = TilePosition { x, y };
                let Some(tile) = self.terrain.tile(position) else {
                    continue;
                };
                if !matches!(tile.kind, TileKind::Ore(_) | TileKind::Artifact(_)) {
                    continue;
                }
                if self
                    .scan_markers
                    .iter()
                    .any(|marker| marker.position == position)
                {
                    continue;
                }
                self.scan_markers.push(ScanMarker {
                    position,
                    kind: tile.kind,
                });
                if self.scan_markers.len() > 80 {
                    self.scan_markers.remove(0);
                }
                return;
            }
        }
    }

    fn update_survey_drones(&mut self) {
        if !self.update_ticks.is_multiple_of(30) {
            return;
        }
        let drones = self
            .infrastructure
            .iter()
            .filter(|item| item.kind == InfrastructureKind::SurveyDrone)
            .copied()
            .collect::<Vec<_>>();
        for drone in drones {
            let radius = 3 + i32::from(self.town_development.scanner_lab_level);
            for y in drone.position.y - radius..=drone.position.y + radius {
                for x in drone.position.x - radius..=drone.position.x + radius {
                    if (x - drone.position.x).abs() + (y - drone.position.y).abs() > radius {
                        continue;
                    }
                    let position = TilePosition { x, y };
                    if let Some(index) = self.tile_index(position)
                        && !self.explored_tiles[index]
                    {
                        self.explored_tiles[index] = true;
                        self.mark_exploration_visual_changed(position);
                    }
                    if let Some(tile) = self.terrain.tile(position) {
                        self.collection_log.discover_tile(tile.kind);
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn is_explored(&self, position: TilePosition) -> bool {
        self.tile_index(position)
            .and_then(|index| self.explored_tiles.get(index))
            .copied()
            .unwrap_or(false)
    }

    #[must_use]
    pub fn expedition_progress(&self, expedition: Expedition) -> u32 {
        match expedition.kind {
            ExpeditionObjectiveKind::ReachDepth => {
                (self.deepest_tile_reached as u32).min(expedition.required)
            }
            ExpeditionObjectiveKind::DeliverCargo => match expedition.target {
                TileKind::Ore(mineral) => self
                    .player
                    .cargo
                    .get(&mineral)
                    .copied()
                    .unwrap_or(0)
                    .min(expedition.required),
                TileKind::Artifact(artifact) => self
                    .player
                    .artifacts
                    .get(&artifact)
                    .copied()
                    .unwrap_or(0)
                    .min(expedition.required),
                _ => 0,
            },
            ExpeditionObjectiveKind::ScanHazards => {
                self.scan_markers
                    .iter()
                    .filter(|marker| marker.kind == expedition.target)
                    .count()
                    .min(expedition.required as usize) as u32
            }
            ExpeditionObjectiveKind::BuildPumpStations => {
                self.infrastructure
                    .iter()
                    .filter(|item| item.kind == InfrastructureKind::PumpStation)
                    .count()
                    .min(expedition.required as usize) as u32
            }
            ExpeditionObjectiveKind::RecoverProbe
            | ExpeditionObjectiveKind::ScanAnomaly
            | ExpeditionObjectiveKind::RescueMiner
            | ExpeditionObjectiveKind::StabilizeCollapse
            | ExpeditionObjectiveKind::ReachSignal => u32::from(
                self.deepest_tile_reached
                    >= i32::try_from(60 + expedition.required * 10).unwrap_or(i32::MAX),
            ),
            ExpeditionObjectiveKind::MineVein => self.player.cargo_used().min(expedition.required),
            ExpeditionObjectiveKind::DeliverExplosives => {
                self.player.bombs.min(expedition.required)
            }
            ExpeditionObjectiveKind::RetrieveArtifact => self
                .player
                .artifacts
                .values()
                .sum::<u32>()
                .min(expedition.required),
            ExpeditionObjectiveKind::NoDamageReturn => {
                u32::from(self.player.hull >= self.player.max_hull())
            }
            ExpeditionObjectiveKind::FastReturn => {
                u32::from(self.town_event_day <= expedition.expires_day)
            }
        }
    }

    #[must_use]
    pub fn expedition_status_line(&self, expedition: Expedition) -> String {
        format!(
            "{} {}/{} | {} cr | {} risk | day {}",
            expedition.title(),
            self.expedition_progress(expedition),
            expedition.required,
            expedition.reward,
            expedition.risk_label(),
            expedition.expires_day
        )
    }

    #[must_use]
    pub fn warning_summary(&self) -> String {
        let mut warnings = Vec::new();
        if self.player.fuel <= self.player.fuel_capacity * 0.2 {
            warnings.push("fuel");
        }
        if self.player.hull <= self.player.max_hull() * 0.35 {
            warnings.push("hull");
        }
        if self.player.tile_position(TILE_SIZE).y >= 60 {
            warnings.push("heat");
        }
        if self.deep_instability >= 60.0 {
            warnings.push("instability");
        }
        if self.player.tile_position(TILE_SIZE).y >= 80 && self.signal_relay_count() == 0 {
            warnings.push("signal");
        }
        if warnings.is_empty() {
            "Warnings: nominal".to_owned()
        } else {
            format!("Warnings: {}", warnings.join(", "))
        }
    }

    #[must_use]
    pub const fn reputation_rank(&self) -> &'static str {
        match self.town_development.reputation {
            0..=2 => "Prospector",
            3..=6 => "Claim Lead",
            7..=11 => "Charter Operator",
            _ => "Deep Claim Founder",
        }
    }

    #[must_use]
    pub const fn advanced_permit_status(&self) -> &'static str {
        if self.town_development.reputation >= 8 && self.best_return_depth >= 80 {
            "Advanced permits unlocked"
        } else {
            "Advanced permits need rank 8 + 80m return"
        }
    }

    #[must_use]
    pub fn has_world_event(&self, kind: WorldEventKind) -> bool {
        self.active_world_events
            .iter()
            .any(|event| event.kind == kind && event.days_remaining > 0)
    }

    #[must_use]
    pub fn world_event_severity(&self, kind: WorldEventKind) -> u32 {
        self.active_world_events
            .iter()
            .filter(|event| event.kind == kind && event.days_remaining > 0)
            .map(|event| event.severity)
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn active_world_event_summary(&self) -> String {
        if self.active_world_events.is_empty() {
            return "No active world events.".to_owned();
        }
        self.active_world_events
            .iter()
            .map(|event| {
                format!(
                    "{} S{} ({}d)",
                    event.kind.label(),
                    event.severity,
                    event.days_remaining
                )
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[must_use]
    pub fn mineral_market_value(&self, mineral: MineralKind) -> u32 {
        let mut factor =
            self.mineral_market_factor(mineral) + u32::from(self.town_development.depot_level) * 3;
        if self.has_world_event(WorldEventKind::MarketBoom) {
            factor += 25;
        }
        if self.has_world_event(WorldEventKind::RareBuyer)
            && matches!(
                mineral,
                MineralKind::Diamond | MineralKind::Mythril | MineralKind::Uranium
            )
        {
            factor += 45;
        }
        if self.has_world_event(WorldEventKind::MarketCrash) {
            factor = factor.saturating_sub(25).max(45);
        }
        (mineral.value() * factor) / 100
    }

    #[must_use]
    pub const fn mineral_market_factor(&self, mineral: MineralKind) -> u32 {
        market_factor_for(self.market_salt, self.town_event_day, mineral)
    }

    #[must_use]
    pub fn previous_mineral_market_factor(&self, mineral: MineralKind) -> Option<u32> {
        self.mineral_market_history
            .get(&mineral)
            .and_then(|history| history.iter().rev().nth(1).copied())
    }

    const fn tile_index(&self, position: TilePosition) -> Option<usize> {
        if position.x < 0
            || position.y < 0
            || position.x >= self.terrain.width()
            || position.y >= self.terrain.height()
        {
            return None;
        }
        Some((position.y * self.terrain.width() + position.x) as usize)
    }

    fn handle_pause_menu(&mut self, input: PlayerInput) {
        if self.modal == Some(ModalScreen::ExitConfirm)
            || self.modal == Some(ModalScreen::UnsavedExitConfirm)
        {
            self.handle_exit_modal(input);
            return;
        }

        if input.menu_up {
            self.selected_pause_item = self.selected_pause_item.saturating_sub(1);
        }
        if input.menu_down {
            self.selected_pause_item =
                (self.selected_pause_item + 1).min(PauseOption::ALL.len() - 1);
        }

        if input.pause || input.cancel {
            self.run_mode = RunMode::Playing;
            return;
        }

        if !input.confirm {
            return;
        }

        match PauseOption::ALL[self.selected_pause_item] {
            PauseOption::Resume => self.run_mode = RunMode::Playing,
            PauseOption::Save => {
                self.modal = Some(ModalScreen::SaveSlots);
                self.selected_menu_item = 0;
            }
            PauseOption::Load => {
                self.modal = Some(ModalScreen::LoadSlots);
                self.selected_menu_item = 0;
            }
            PauseOption::Options => {
                self.modal = Some(ModalScreen::Options);
                self.selected_menu_item = 0;
            }
            PauseOption::ExitToDesktop => self.request_exit_or_prompt(),
        }
    }

    fn handle_save_load(&mut self, input: PlayerInput) {
        if input.save {
            if self.block_joined_client_save() {
                return;
            }
            match save_game(self) {
                Ok(()) => {
                    self.save_dirty = false;
                    "Game saved to drillgame-save.json.".clone_into(&mut self.message);
                }
                Err(error) => self.message = format!("Save failed: {error}"),
            }
        }

        if input.load {
            self.load_into_self();
        }
    }

    fn load_into_self(&mut self) {
        if !save_exists() {
            "No save file found.".clone_into(&mut self.message);
            return;
        }

        match load_game() {
            Ok(mut loaded) => {
                "Game loaded.".clone_into(&mut loaded.message);
                loaded.migrate_loaded_state();
                loaded.mark_full_terrain_refresh();
                *self = loaded;
                self.save_dirty = false;
            }
            Err(error) => self.message = format!("Load failed: {error}"),
        }
    }

    fn migrate_loaded_state(&mut self) {
        if self.save_version < current_save_version() {
            self.contracts.migrate_after_load();
            self.scan_markers
                .retain(|marker| scanner_can_mark(marker.kind, self.player.scanner_level));
            if self.side_contract_active && self.side_contract_required == 0 {
                self.side_contract_active = false;
            }
            if self.side_contract_active
                && self.active_side_contracts.is_empty()
                && let Some(target) = self.side_contract_target
            {
                self.active_side_contracts.push(SideContract {
                    kind: self.side_contract_kind,
                    target,
                    required: self.side_contract_required.max(1),
                    expires_day: None,
                });
            }
            if self.won_game {
                self.deep_claim_status = DeepClaimStatus::Unlocked;
            }
            if self.mineral_market_history.is_empty() {
                self.mineral_market_history =
                    initial_mineral_market_history(self.market_salt, self.town_event_day);
            }
            self.save_version = current_save_version();
        }
    }

    fn handle_interaction(&mut self, input: PlayerInput) {
        if !input.interact {
            return;
        }

        if self.try_use_ore_processor() {
            return;
        }
        if self.try_use_cargo_lift() {
            return;
        }

        if let Some(zone) = self.current_zone {
            self.enter_interior(zone);
        } else {
            "No surface service here.".clone_into(&mut self.message);
        }
    }

    fn try_use_ore_processor(&mut self) -> bool {
        let position = self.player.tile_position(TILE_SIZE);
        let Some(processor_index) = self.infrastructure.iter().position(|item| {
            item.kind == InfrastructureKind::OreProcessor
                && (item.position.x - position.x).abs() <= 1
                && (item.position.y - position.y).abs() <= 1
        }) else {
            return false;
        };
        let recipes = [
            (MineralKind::Copper, StrategicResourceKind::AncientAlloy),
            (MineralKind::Iron, StrategicResourceKind::AncientAlloy),
            (MineralKind::Silver, StrategicResourceKind::CrystalLens),
        ];
        for (mineral, material) in recipes {
            let Some(count) = self.player.cargo.get_mut(&mineral) else {
                continue;
            };
            if *count < 5 {
                continue;
            }
            *count -= 5;
            self.player.cargo.retain(|_, count| *count > 0);
            self.player.add_material(material, 1);
            self.total_resources_refined = self.total_resources_refined.saturating_add(1);
            if let Some(processor) = self.infrastructure.get_mut(processor_index) {
                processor.durability = processor.durability.saturating_sub(8);
            }
            self.message = format!(
                "Ore processor refined 5 {} into 1 {}.",
                mineral.name(),
                material.name()
            );
            self.sound_cues.push(SoundCue::Upgrade);
            return true;
        }
        "Ore processor needs 5 Copper, Iron, or Silver cargo.".clone_into(&mut self.message);
        true
    }

    fn try_use_cargo_lift(&mut self) -> bool {
        let position = self.player.tile_position(TILE_SIZE);
        let Some(lift_index) = self.infrastructure.iter().position(|item| {
            item.kind == InfrastructureKind::CargoLift
                && (item.position.x - position.x).abs() <= 1
                && (item.position.y - position.y).abs() <= 1
        }) else {
            return false;
        };
        if self.player.cargo.is_empty() {
            if self.fast_travel_between_cargo_lifts(lift_index) {
                return true;
            }
            "Cargo lift ready, but mineral cargo is empty.".clone_into(&mut self.message);
            return true;
        }
        let capacity = 8_u32;
        let mut remaining = capacity;
        let mut value = 0;
        for mineral in all_minerals() {
            if remaining == 0 {
                break;
            }
            let Some(count) = self.player.cargo.get_mut(&mineral) else {
                continue;
            };
            let sent = (*count).min(remaining);
            *count -= sent;
            remaining -= sent;
            value += self.mineral_market_value(mineral) * sent;
        }
        self.player.cargo.retain(|_, count| *count > 0);
        if value == 0 {
            "Cargo lift found no mineral cargo to send.".clone_into(&mut self.message);
            return true;
        }
        if let Some(lift) = self.infrastructure.get_mut(lift_index) {
            lift.durability = lift.durability.saturating_sub(5);
        }
        self.player.credits += value;
        self.message = format!(
            "Cargo lift sent {} unit(s) upward for {value} credits.",
            capacity - remaining
        );
        self.sound_cues.push(SoundCue::Sell);
        true
    }

    fn fast_travel_between_cargo_lifts(&mut self, current_lift_index: usize) -> bool {
        if self.town_development.reputation < 6 {
            return false;
        }
        let Some(current) = self.infrastructure.get(current_lift_index) else {
            return false;
        };
        let destination = self
            .infrastructure
            .iter()
            .enumerate()
            .filter(|(index, item)| {
                *index != current_lift_index && item.kind == InfrastructureKind::CargoLift
            })
            .min_by_key(|(_, item)| item.position.y.abs_diff(current.position.y));
        let Some((_, lift)) = destination else {
            return false;
        };
        self.player.x = lift.position.x as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        self.player.y = lift.position.y as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        "Cargo lift network moved rig to the nearest linked station.".clone_into(&mut self.message);
        self.sound_cues.push(SoundCue::Milestone);
        true
    }

    fn enter_interior(&mut self, zone: SurfaceZone) {
        self.run_mode = RunMode::Interior;
        self.interior_zone = Some(zone);
        self.interior_x = 82.0;
        self.interior_facing = 1.0;
        self.modal = None;
        self.selected_menu_item = 0;
        self.sound_cues.push(SoundCue::Ui);
        self.message = format!(
            "Entered {}. Walk to a counter and press E; door exits.",
            surface_zone_label(zone)
        );
    }

    fn handle_interior(&mut self, input: PlayerInput, delta_seconds: f32) {
        if self.handle_modal(input) {
            return;
        }
        if input.pause {
            self.run_mode = RunMode::Paused;
            return;
        }
        let movement = input.horizontal;
        if movement.abs() > f32::EPSILON {
            self.interior_facing = movement.signum();
        }
        self.interior_x = (self.interior_x + movement * 185.0 * delta_seconds).clamp(42.0, 598.0);
        if input.cancel || (input.interact && self.interior_x < 74.0) {
            self.exit_interior();
            return;
        }
        if input.interact {
            self.open_interior_hotspot();
        }
    }

    fn exit_interior(&mut self) {
        self.run_mode = RunMode::Playing;
        self.interior_zone = None;
        self.modal = None;
        self.sound_cues.push(SoundCue::Ui);
        "Back outside.".clone_into(&mut self.message);
    }

    fn open_interior_hotspot(&mut self) {
        let Some(zone) = self.interior_zone else {
            return;
        };
        if (self.interior_x - interior_service_x(zone)).abs() > 70.0 {
            "Walk to the service counter or the exit door.".clone_into(&mut self.message);
            return;
        }
        match zone {
            SurfaceZone::Fuel => self.modal = Some(ModalScreen::Fuel),
            SurfaceZone::Repair => self.modal = Some(ModalScreen::Repair),
            SurfaceZone::Depot => {
                self.modal = Some(ModalScreen::Depot);
                self.selected_menu_item = 1;
            }
            SurfaceZone::Headquarters => self.modal = Some(ModalScreen::Headquarters),
            SurfaceZone::Shop => self.modal = Some(ModalScreen::Shop),
            SurfaceZone::Bank => self.modal = Some(ModalScreen::Bank),
            SurfaceZone::Explosives => self.modal = Some(ModalScreen::Explosives),
            SurfaceZone::Salvage => self.modal = Some(ModalScreen::Salvage),
        }
        self.sound_cues.push(SoundCue::Ui);
    }

    fn handle_modal(&mut self, input: PlayerInput) -> bool {
        let Some(modal) = self.modal else {
            return false;
        };

        if input.cancel {
            if modal == ModalScreen::OnlineMultiplayer {
                self.close_online_multiplayer_menu();
            } else {
                self.modal = None;
            }
            return true;
        }

        if input.menu_up {
            self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
        }
        if input.menu_down {
            let max_item = match modal {
                ModalScreen::Depot => 4,
                ModalScreen::Fuel
                | ModalScreen::Repair
                | ModalScreen::Options
                | ModalScreen::Bank => 2,
                ModalScreen::Explosives => 3,
                ModalScreen::Salvage => 5,
                ModalScreen::Headquarters => {
                    if self.deep_claim_status == DeepClaimStatus::Unlocked {
                        6
                    } else {
                        2
                    }
                }
                ModalScreen::Crafting => RecipeKind::ALL.len() - 1,
                ModalScreen::TownDevelopment => TownBuilding::ALL.len() - 1,
                ModalScreen::ExpeditionBoard => self
                    .expedition_offers
                    .len()
                    .saturating_add(self.active_expeditions.len())
                    .saturating_sub(1),
                ModalScreen::SaveSlots | ModalScreen::LoadSlots => save_slot_count() - 1,
                ModalScreen::OnlineMultiplayer => 12,
                _ => 0,
            };
            self.selected_menu_item = (self.selected_menu_item + 1).min(max_item);
        }

        if matches!(modal, ModalScreen::Shop) {
            if input.menu_left {
                self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
            }
            if input.menu_right {
                self.selected_menu_item =
                    (self.selected_menu_item + 1).min(upgrade_offers(&self.player).len() - 1);
            }
        }

        if let Some(index) = input.selected_upgrade {
            self.selected_menu_item = index.min(upgrade_offers(&self.player).len() - 1);
        }

        if input.confirm {
            match modal {
                ModalScreen::Fuel => self.modal = Some(ModalScreen::FuelConfirm),
                ModalScreen::FuelConfirm => self.confirm_refuel(),
                ModalScreen::Repair => self.modal = Some(ModalScreen::RepairConfirm),
                ModalScreen::RepairConfirm => self.confirm_repair(),
                ModalScreen::Depot => self.confirm_depot(),
                ModalScreen::Headquarters => self.confirm_headquarters(),
                ModalScreen::DepotReceiptHistory => self.modal = Some(ModalScreen::Depot),
                ModalScreen::Shop => self.modal = Some(ModalScreen::ShopConfirm),
                ModalScreen::ShopConfirm => self.try_buy_upgrade(self.selected_menu_item),
                ModalScreen::Bank => self.confirm_bank_menu(),
                ModalScreen::Explosives => self.confirm_explosives_menu(),
                ModalScreen::Salvage => self.confirm_salvage_menu(),
                ModalScreen::TownDevelopment => self.confirm_town_development(),
                ModalScreen::ExpeditionBoard => self.accept_expedition_offer(),
                ModalScreen::Crafting => self.confirm_crafting(),
                ModalScreen::Options => self.confirm_options(),
                ModalScreen::SaveSlots => self.save_slot(self.selected_menu_item),
                ModalScreen::LoadSlots => self.load_slot(self.selected_menu_item),
                ModalScreen::ExitConfirm
                | ModalScreen::UnsavedExitConfirm
                | ModalScreen::Map
                | ModalScreen::Help
                | ModalScreen::ResearchLog => {}
                ModalScreen::OnlineMultiplayer => self.confirm_online_multiplayer(),
            }
        }

        true
    }

    fn confirm_options(&mut self) {
        match self.selected_menu_item {
            0 => {
                self.master_volume = (self.master_volume + 0.1).min(1.0);
                self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
            }
            1 => {
                self.master_volume = (self.master_volume - 0.1).max(0.0);
                self.message = format!("Volume: {:.0}%", self.master_volume * 100.0);
            }
            _ => {
                self.fullscreen = !self.fullscreen;
                self.message = if self.fullscreen {
                    "Fullscreen preference enabled; F11 toggles immediately.".to_owned()
                } else {
                    "Windowed preference enabled; F11 toggles immediately.".to_owned()
                };
            }
        }
        self.settings_dirty = true;
        self.sound_cues.push(SoundCue::Ui);
    }

    fn save_slot(&mut self, slot: usize) {
        if self.block_joined_client_save() {
            self.modal = Some(ModalScreen::SaveSlots);
            return;
        }
        match save_game_slot(self, slot) {
            Ok(()) => {
                self.save_dirty = false;
                self.message = format!("Saved to slot {}.", slot + 1);
            }
            Err(error) => self.message = format!("Save slot failed: {error}"),
        }
        self.modal = Some(ModalScreen::SaveSlots);
    }

    fn load_slot(&mut self, slot: usize) {
        match load_game_slot(slot) {
            Ok(mut loaded) => {
                loaded.master_volume = self.master_volume;
                loaded.fullscreen = self.fullscreen;
                loaded.migrate_loaded_state();
                loaded.mark_full_terrain_refresh();
                *self = loaded;
                self.save_dirty = false;
                self.message = format!("Loaded slot {}.", slot + 1);
            }
            Err(error) => self.message = format!("Load slot failed: {error}"),
        }
    }

    const fn selected_service_fraction(&self) -> f32 {
        match self.selected_menu_item {
            0 => 0.25,
            1 => 0.5,
            _ => 1.0,
        }
    }

    fn confirm_refuel(&mut self) {
        let fraction = self.selected_service_fraction();
        let cost = refuel_amount(&mut self.player, fraction);
        self.message = if cost == 0 {
            "Fuel already full or no credits available.".to_owned()
        } else {
            format!("Fuel topped up for {cost} credits.")
        };
        if cost > 0 {
            if self.has_world_event(WorldEventKind::FuelShortage) {
                let surcharge = (cost / 4).min(self.player.credits);
                self.player.credits -= surcharge;
                self.message = format!(
                    "Fuel topped up for {cost} credits. Fuel shortage surcharge: {surcharge}."
                );
            } else if self.town_event_day.is_multiple_of(5) {
                let refund = cost / 5;
                self.player.credits += refund;
                self.message = format!("Fuel topped up for {cost} credits. Sale refund: {refund}.");
            }
            self.sound_cues.push(SoundCue::Upgrade);
            self.service_animation = Some(ServiceAnimation::Fuel);
            self.service_animation_seconds = 1.4;
        }
        self.modal = Some(ModalScreen::Fuel);
    }

    fn confirm_repair(&mut self) {
        let fraction = self.selected_service_fraction();
        let cost = repair_amount(&mut self.player, fraction);
        self.message = if cost == 0 {
            "Hull already repaired or no credits available.".to_owned()
        } else {
            format!("Hull repaired for {cost} credits.")
        };
        if cost > 0 {
            if self.town_development.mechanic_level > 0 {
                let refund =
                    (cost * u32::from(self.town_development.mechanic_level)).min(cost) / 10;
                self.player.credits = self.player.credits.saturating_add(refund);
                self.message =
                    format!("Hull repaired for {cost} credits. Mechanic upgrade rebate: {refund}.");
            }
            if self.has_world_event(WorldEventKind::RepairBacklog) {
                let surcharge = (cost / 4).min(self.player.credits);
                self.player.credits -= surcharge;
                self.message = format!(
                    "Hull repaired for {cost} credits. Repair backlog event surcharge: {surcharge}."
                );
            } else if self.town_event_day % 5 == 2 {
                let surcharge = (cost / 10).min(self.player.credits);
                self.player.credits -= surcharge;
                self.message = format!(
                    "Hull repaired for {cost} credits. Repair backlog surcharge: {surcharge}."
                );
            }
            self.sound_cues.push(SoundCue::Upgrade);
            self.service_animation = Some(ServiceAnimation::Repair);
            self.service_animation_seconds = 1.4;
        }
        self.modal = Some(ModalScreen::Repair);
    }

    fn confirm_headquarters(&mut self) {
        match self.selected_menu_item {
            0 => self.confirm_complete_contract(),
            1 => {
                self.message = hq_story_message(self);
                self.sound_cues.push(SoundCue::Milestone);
            }
            2 => self.confirm_finance(),
            3 if self.deep_claim_status == DeepClaimStatus::Unlocked => {
                self.modal = Some(ModalScreen::TownDevelopment);
                self.selected_menu_item = 0;
            }
            4 if self.deep_claim_status == DeepClaimStatus::Unlocked => {
                self.refresh_expedition_offers();
                self.modal = Some(ModalScreen::ExpeditionBoard);
                self.selected_menu_item = 0;
            }
            5 if self.deep_claim_status == DeepClaimStatus::Unlocked => {
                self.modal = Some(ModalScreen::ResearchLog);
                self.selected_menu_item = 0;
            }
            _ if self.deep_claim_status == DeepClaimStatus::Unlocked => {
                self.modal = Some(ModalScreen::Crafting);
                self.selected_menu_item = 0;
            }
            _ => self.confirm_finance(),
        }
    }

    fn confirm_crafting(&mut self) {
        let recipe = RecipeKind::ALL[self.selected_menu_item.min(RecipeKind::ALL.len() - 1)];
        for (material, required) in recipe.cost() {
            if self.player.materials.get(material).copied().unwrap_or(0) < *required {
                self.message =
                    format!("Need {required} {} for {}.", material.name(), recipe.name());
                return;
            }
        }
        for (material, required) in recipe.cost() {
            if let Some(count) = self.player.materials.get_mut(material) {
                *count = count.saturating_sub(*required);
            }
        }
        self.player.materials.retain(|_, count| *count > 0);
        match recipe {
            RecipeKind::ReinforcedBulkhead => {
                self.player.crafted_bulkheads = self.player.crafted_bulkheads.saturating_add(1);
                self.player.hull = self.player.max_hull();
            }
            RecipeKind::AuxiliaryTank => {
                self.player.fuel_capacity += 20.0;
                self.player.fuel = self.player.fuel_capacity;
            }
            RecipeKind::ExpandedSorter => {
                self.player.crafted_sorters = self.player.crafted_sorters.saturating_add(1);
                self.player.cargo_capacity = self.player.cargo_capacity.saturating_add(4);
            }
            RecipeKind::SignalRelayKit => {
                self.player.signal_relay_kits = self.player.signal_relay_kits.saturating_add(1);
            }
            RecipeKind::SurveyDroneKit => {
                self.player.survey_drone_kits = self.player.survey_drone_kits.saturating_add(1);
            }
            RecipeKind::CargoLiftKit => {
                self.player.cargo_lift_kits = self.player.cargo_lift_kits.saturating_add(1);
            }
            RecipeKind::TunnelSupportKit => {
                self.player.tunnel_support_kits = self.player.tunnel_support_kits.saturating_add(1);
            }
            RecipeKind::PumpStationKit => {
                self.player.pump_station_kits = self.player.pump_station_kits.saturating_add(1);
            }
            RecipeKind::OreProcessorKit => {
                self.player.ore_processor_kits = self.player.ore_processor_kits.saturating_add(1);
            }
        }
        self.message = format!("Crafted {}: {}.", recipe.name(), recipe.description());
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn confirm_town_development(&mut self) {
        let building = TownBuilding::ALL[self.selected_menu_item.min(TownBuilding::ALL.len() - 1)];
        let cost = self.town_development.upgrade_cost(building);
        let material_gate = self.town_development.level(building) >= 1;
        if material_gate
            && self
                .player
                .materials
                .get(&StrategicResourceKind::AncientAlloy)
                .copied()
                .unwrap_or(0)
                == 0
        {
            self.message = format!(
                "{} level {} upgrade also needs 1 Ancient Alloy from Deep Claim ore.",
                building.name(),
                self.town_development.level(building) + 1
            );
            return;
        }
        if self.player.credits < cost {
            self.message = format!("{} upgrade costs {cost} credits.", building.name());
            return;
        }
        self.player.credits -= cost;
        if material_gate {
            if let Some(count) = self
                .player
                .materials
                .get_mut(&StrategicResourceKind::AncientAlloy)
            {
                *count = count.saturating_sub(1);
            }
            self.player.materials.retain(|_, count| *count > 0);
        }
        *self.town_development.level_mut(building) += 1;
        self.town_development.reputation = self.town_development.reputation.saturating_add(1);
        self.message = format!(
            "{} upgraded to level {}. Deep Claim reputation increased.",
            building.name(),
            self.town_development.level(building)
        );
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn refresh_expedition_offers(&mut self) {
        if self.deep_claim_status != DeepClaimStatus::Unlocked {
            self.expedition_offers.clear();
            return;
        }
        if !self.expedition_offers.is_empty() {
            return;
        }
        let day = self.town_event_day;
        self.expedition_offers = vec![
            Expedition {
                kind: ExpeditionObjectiveKind::ReachDepth,
                target: TileKind::Stone,
                required: (self.deepest_tile_reached.max(80) as u32 + 15).min(140),
                reward: 320 + day * 12,
                expires_day: day + 4,
            },
            Expedition {
                kind: ExpeditionObjectiveKind::DeliverCargo,
                target: TileKind::Ore(MineralKind::Platinum),
                required: 2,
                reward: 420 + day * 14,
                expires_day: day + 3,
            },
            Expedition {
                kind: ExpeditionObjectiveKind::ScanHazards,
                target: TileKind::Gas,
                required: 4,
                reward: 360 + day * 10,
                expires_day: day + 3,
            },
            Expedition {
                kind: ExpeditionObjectiveKind::BuildPumpStations,
                target: TileKind::MagmaVent,
                required: 2,
                reward: 460 + day * 12,
                expires_day: day + 5,
            },
        ];
        let rotating_kind = match day % 10 {
            0 => ExpeditionObjectiveKind::RecoverProbe,
            1 => ExpeditionObjectiveKind::MineVein,
            2 => ExpeditionObjectiveKind::ScanAnomaly,
            3 => ExpeditionObjectiveKind::RescueMiner,
            4 => ExpeditionObjectiveKind::DeliverExplosives,
            5 => ExpeditionObjectiveKind::StabilizeCollapse,
            6 => ExpeditionObjectiveKind::RetrieveArtifact,
            7 => ExpeditionObjectiveKind::ReachSignal,
            8 => ExpeditionObjectiveKind::NoDamageReturn,
            _ => ExpeditionObjectiveKind::FastReturn,
        };
        self.expedition_offers.push(Expedition {
            kind: rotating_kind,
            target: TileKind::Artifact(ArtifactKind::BuriedIdol),
            required: 1 + day % 3,
            reward: 520 + day * 16,
            expires_day: day + 2,
        });
        if self.town_development.reputation >= 8 && self.best_return_depth >= 80 {
            self.expedition_offers.push(Expedition {
                kind: ExpeditionObjectiveKind::ReachDepth,
                target: TileKind::Artifact(ArtifactKind::OldCircuit),
                required: 150,
                reward: 950 + day * 20,
                expires_day: day + 2,
            });
        }
    }

    fn accept_expedition_offer(&mut self) {
        if self.expedition_offers.is_empty() {
            self.refresh_expedition_offers();
        }
        if self.selected_menu_item >= self.expedition_offers.len() {
            self.abandon_selected_expedition();
            return;
        }
        if self.active_expeditions.len() >= 3 {
            "Expedition board limit reached: complete one before accepting more."
                .clone_into(&mut self.message);
            return;
        }
        let index = self
            .selected_menu_item
            .min(self.expedition_offers.len().saturating_sub(1));
        let Some(expedition) = self.expedition_offers.get(index).copied() else {
            "No expedition offer is available.".clone_into(&mut self.message);
            return;
        };
        self.active_expeditions.push(expedition);
        self.expedition_offers.remove(index);
        self.message = format!("Accepted expedition: {}.", expedition.title());
        self.sound_cues.push(SoundCue::Ui);
    }

    fn abandon_selected_expedition(&mut self) {
        let active_index = self
            .selected_menu_item
            .saturating_sub(self.expedition_offers.len());
        let Some(expedition) = self.active_expeditions.get(active_index).copied() else {
            "Select an expedition offer to accept or an active expedition to abandon."
                .clone_into(&mut self.message);
            return;
        };
        self.active_expeditions.remove(active_index);
        self.message = format!("Abandoned expedition: {}.", expedition.title());
        self.sound_cues.push(SoundCue::Ui);
    }

    fn try_complete_expeditions(&mut self) {
        let mut reward = 0;
        let mut completed = 0;
        let mut completed_expeditions = Vec::new();
        let snapshot = self.clone();
        self.active_expeditions.retain(|expedition| {
            if expedition.expires_day < snapshot.town_event_day {
                return false;
            }
            if expedition_satisfied(*expedition, &snapshot) {
                reward += expedition.reward;
                completed += 1;
                completed_expeditions.push(*expedition);
                false
            } else {
                true
            }
        });
        for expedition in completed_expeditions {
            match expedition.kind {
                ExpeditionObjectiveKind::RetrieveArtifact => {
                    self.legendary_blueprints
                        .insert(LegendaryBlueprint::StarPlating);
                    self.rig_part_inventory.insert(RigPartKind::ResonanceDrill);
                }
                ExpeditionObjectiveKind::ScanAnomaly | ExpeditionObjectiveKind::ScanHazards => {
                    self.player
                        .add_material(StrategicResourceKind::CrystalLens, expedition.required);
                }
                ExpeditionObjectiveKind::RecoverProbe | ExpeditionObjectiveKind::ReachSignal => {
                    self.rig_part_inventory.insert(RigPartKind::HazardScanner);
                }
                ExpeditionObjectiveKind::MineVein => {
                    self.player
                        .add_material(StrategicResourceKind::AncientAlloy, 1);
                }
                _ => {}
            }
            consume_expedition_delivery(expedition, &mut self.player);
        }
        if reward > 0 {
            self.player.credits = self.player.credits.saturating_add(reward);
            self.total_earnings = self.total_earnings.saturating_add(reward);
            self.expeditions_completed = self.expeditions_completed.saturating_add(completed);
            self.town_development.reputation =
                self.town_development.reputation.saturating_add(completed);
            self.message = format!("Completed {completed} expedition(s) for {reward} credits.");
            self.sound_cues.push(SoundCue::Milestone);
        }
    }

    fn confirm_bank_menu(&mut self) {
        match self.selected_menu_item {
            0 => self.confirm_finance(),
            1 => self.buy_insurance(),
            _ => self.start_side_contract(),
        }
    }

    fn confirm_explosives_menu(&mut self) {
        match self.selected_menu_item {
            0 => self.buy_explosive_shack_pack(3, 55),
            1 => self.buy_explosive_shack_pack(7, 120),
            2 => self.buy_mining_rocket_pack(),
            _ => {
                self.player.bombs = self.player.bombs.saturating_add(1);
                "Nix comped one test charge. Try not to test it indoors."
                    .clone_into(&mut self.message);
            }
        }
    }

    fn confirm_salvage_menu(&mut self) {
        match self.selected_menu_item {
            0 => self.salvage_recover_lost_cargo(),
            1 => self.salvage_patch_hull(),
            2 => self.salvage_launch_drone(),
            3 => self.salvage_recover_wrecked_part(),
            4 => self.salvage_clear_collapse_zone(),
            _ => self.salvage_sell_scrap_tip(),
        }
    }

    fn buy_insurance(&mut self) {
        if self.player.insured {
            "Already insured for the next rescue.".clone_into(&mut self.message);
            return;
        }
        let max_tier = 1_u8.saturating_add(self.town_development.bank_level).min(4);
        let next_tier = self.player.insurance_tier.saturating_add(1).min(max_tier);
        if self.player.insurance_tier >= max_tier {
            self.message = format!(
                "Bank level {} only supports tier {max_tier} insurance.",
                self.town_development.bank_level
            );
            return;
        }
        let bank_discount = u32::from(self.town_development.bank_level).saturating_mul(15);
        let cost = (70 + u32::from(next_tier) * 55).saturating_sub(bank_discount);
        if self.player.credits < cost {
            self.message = format!("Tier {next_tier} insurance costs {cost} credits.");
            return;
        }
        self.player.credits -= cost;
        self.player.insured = true;
        self.player.insurance_tier = next_tier;
        self.message = format!(
            "Ledger sold tier {next_tier} rescue insurance. Higher tiers reduce fees and cargo loss."
        );
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn start_side_contract(&mut self) {
        if self.active_side_contracts.len() >= 3 {
            "Bank board only allows three active side contracts.".clone_into(&mut self.message);
            return;
        }
        self.side_contract_active = true;
        self.side_contract_kind = match self.town_event_day % 4 {
            0 => SideContractKind::Cargo,
            1 => SideContractKind::DepthSurvey,
            2 => SideContractKind::HazardScan,
            _ => SideContractKind::Rush,
        };
        self.side_contract_target = Some(match self.side_contract_kind {
            SideContractKind::Cargo => TileKind::Ore(MineralKind::Gold),
            SideContractKind::DepthSurvey => TileKind::Ore(MineralKind::Platinum),
            SideContractKind::HazardScan => TileKind::Gas,
            SideContractKind::Rush => TileKind::Ore(MineralKind::Ruby),
        });
        self.side_contract_required = match self.side_contract_kind {
            SideContractKind::Cargo => 2,
            SideContractKind::DepthSurvey => 65,
            SideContractKind::HazardScan => 3,
            SideContractKind::Rush => 1,
        };
        self.message = match self.side_contract_kind {
            SideContractKind::Cargo => format!(
                "Side contract posted: deliver {} x{} for bonus pay.",
                self.side_contract_target.map_or("sample", TileKind::name),
                self.side_contract_required
            ),
            SideContractKind::DepthSurvey => format!(
                "Side contract posted: reach {}m and report to depot.",
                self.side_contract_required
            ),
            SideContractKind::HazardScan => format!(
                "Side contract posted: scan {} hazards and report to depot.",
                self.side_contract_required
            ),
            SideContractKind::Rush => format!(
                "Rush contract posted: deliver {} x{} before day {}.",
                self.side_contract_target.map_or("sample", TileKind::name),
                self.side_contract_required,
                self.town_event_day + 2
            ),
        };
        if let Some(target) = self.side_contract_target {
            self.active_side_contracts.push(SideContract {
                kind: self.side_contract_kind,
                target,
                required: self.side_contract_required,
                expires_day: (self.side_contract_kind == SideContractKind::Rush)
                    .then_some(self.town_event_day + 2),
            });
        }
    }

    fn confirm_finance(&mut self) {
        if self.player.loan_debt == 0 {
            let advance = 250 + u32::from(self.town_development.bank_level) * 150;
            let risk_premium = 50 + u32::from(self.town_development.bank_level) * 25;
            self.player.credits = self.player.credits.saturating_add(advance);
            self.player.loan_debt = advance.saturating_add(risk_premium);
            self.message = format!(
                "HQ finance issued a {advance} credit advance. Risk-adjusted payoff: {} credits.",
                self.player.loan_debt
            );
        } else {
            let payment = self.player.loan_debt.min(self.player.credits);
            self.player.credits -= payment;
            self.player.loan_debt -= payment;
            self.message = format!(
                "Paid {payment} credits toward HQ debt. Remaining: {}.",
                self.player.loan_debt
            );
        }
        self.sound_cues.push(SoundCue::Sell);
    }

    fn buy_explosive_shack_pack(&mut self, count: u32, cost: u32) {
        if self.player.credits < cost {
            self.message = format!("Explosive Shack: bomb bundle costs {cost} credits.");
            return;
        }
        self.player.credits -= cost;
        let bonus = u32::from(self.town_development.explosives_shack_level / 2);
        let delivered = count + bonus;
        self.player.bombs += delivered;
        self.sound_cues.push(SoundCue::Upgrade);
        self.message = format!("Nix sold you {delivered} timed charges. Don't hug them.");
    }

    fn buy_mining_rocket_pack(&mut self) {
        if self.town_development.explosives_shack_level < 4 {
            "Mining rockets unlock at Explosives Shack level 4.".clone_into(&mut self.message);
            return;
        }
        if self.player.credits < 180 {
            "Mining rocket bundle costs 180 credits.".clone_into(&mut self.message);
            return;
        }
        self.player.credits -= 180;
        self.player.bombs = self.player.bombs.saturating_add(4);
        self.sound_cues.push(SoundCue::Upgrade);
        "Nix packed 4 mining rockets as high-yield shaped charges.".clone_into(&mut self.message);
    }

    fn salvage_recover_lost_cargo(&mut self) {
        let recovered = self.lost_cargo_count;
        if recovered == 0 {
            "No lost cargo beacon is active.".clone_into(&mut self.message);
            return;
        }
        let discount = u32::from(self.town_development.salvage_yard_level) * 3;
        let fee = (recovered * 12)
            .saturating_sub(recovered * discount)
            .min(self.player.credits);
        self.player.credits -= fee;
        for (mineral, count) in std::mem::take(&mut self.lost_minerals) {
            *self.player.cargo.entry(mineral).or_default() += count;
        }
        for (artifact, count) in std::mem::take(&mut self.lost_artifacts) {
            *self.player.artifacts.entry(artifact).or_default() += count;
        }
        self.lost_cargo_count = 0;
        self.lost_cargo_x = None;
        self.lost_cargo_y = None;
        self.message = format!("Mara recovered {recovered} lost cargo markers for {fee} credits.");
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn salvage_patch_hull(&mut self) {
        let patch = (self.player.max_hull() * 0.12).ceil();
        self.player.hull = (self.player.hull + patch).min(self.player.max_hull());
        self.message = format!("Salvage Yard patch job restored {patch:.0} hull.");
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn salvage_launch_drone(&mut self) {
        if self.town_development.salvage_yard_level < 2 {
            "Salvage drones unlock at Salvage Yard level 2.".clone_into(&mut self.message);
            return;
        }
        let Some(x) = self.lost_cargo_x else {
            "No rescue beacon for a salvage drone to follow.".clone_into(&mut self.message);
            return;
        };
        let y = self.lost_cargo_y.unwrap_or(10.0 * TILE_SIZE);
        self.infrastructure.push(PlacedInfrastructure {
            kind: InfrastructureKind::SurveyDrone,
            position: TilePosition {
                x: (x / TILE_SIZE).floor() as i32,
                y: (y / TILE_SIZE).floor() as i32,
            },
            durability: default_infrastructure_durability(),
        });
        self.salvage_recover_lost_cargo();
        "Salvage drone deployed to the rescue beacon and recovered marked cargo."
            .clone_into(&mut self.message);
    }

    fn salvage_recover_wrecked_part(&mut self) {
        if self.town_development.salvage_yard_level < 3 {
            "Wrecked rig part recovery unlocks at Salvage Yard level 3."
                .clone_into(&mut self.message);
            return;
        }
        let part = match self.rescue_count % 4 {
            0 => RigPartKind::ImpactHull,
            1 => RigPartKind::ArmoredCargoBay,
            2 => RigPartKind::HazardScanner,
            _ => RigPartKind::NeedleDrill,
        };
        self.rig_part_inventory.insert(part);
        self.sound_cues.push(SoundCue::Upgrade);
        self.message = format!("Mara recovered a wrecked {} rig part.", part.title());
    }

    fn salvage_clear_collapse_zone(&mut self) {
        if self.town_development.salvage_yard_level < 4 {
            "Collapse-zone contracts unlock at Salvage Yard level 4.".clone_into(&mut self.message);
            return;
        }
        let cleared = self.collapse_warnings.len();
        self.collapse_warnings.clear();
        let payout = u32::try_from(cleared)
            .unwrap_or(u32::MAX)
            .saturating_mul(45);
        self.player.credits = self.player.credits.saturating_add(payout);
        self.total_earnings = self.total_earnings.saturating_add(payout);
        self.sound_cues.push(SoundCue::Milestone);
        self.message = format!("Cleared {cleared} collapse-zone warning(s) for {payout} credits.");
    }

    fn salvage_sell_scrap_tip(&mut self) {
        self.player.credits = self.player.credits.saturating_add(35);
        "Mara bought scrap telemetry for 35 credits.".clone_into(&mut self.message);
        self.sound_cues.push(SoundCue::Sell);
    }

    fn try_complete_side_contract(&mut self) {
        if !self.active_side_contracts.is_empty() {
            let mut completed_index = None;
            let mut completed_reward = 0;
            for (index, contract) in self.active_side_contracts.iter().enumerate() {
                if side_contract_satisfied(*contract, self) {
                    completed_index = Some(index);
                    completed_reward = 420 + contract.required.min(10) * 80;
                    break;
                }
            }
            if let Some(index) = completed_index {
                let contract = self.active_side_contracts.remove(index);
                if matches!(
                    contract.kind,
                    SideContractKind::Cargo | SideContractKind::Rush
                ) {
                    consume_side_contract_cargo(contract, &mut self.player);
                }
                self.player.credits += completed_reward;
                self.total_earnings += completed_reward;
                self.side_contract_active = !self.active_side_contracts.is_empty();
                self.message =
                    format!("Side contract fulfilled: {completed_reward} credits bonus.");
                self.sound_cues.push(SoundCue::Sell);
                return;
            }
        }
        if !self.side_contract_active {
            return;
        }
        let Some(target) = self.side_contract_target else {
            return;
        };
        let satisfied = match self.side_contract_kind {
            SideContractKind::Cargo | SideContractKind::Rush => match target {
                TileKind::Ore(mineral) => {
                    self.player.cargo.get(&mineral).copied().unwrap_or(0)
                        >= self.side_contract_required
                }
                TileKind::Artifact(artifact) => {
                    self.player.artifacts.get(&artifact).copied().unwrap_or(0)
                        >= self.side_contract_required
                }
                _ => false,
            },
            SideContractKind::DepthSurvey => {
                u32::try_from(self.deepest_tile_reached).unwrap_or(0) >= self.side_contract_required
            }
            SideContractKind::HazardScan => {
                self.scan_markers
                    .iter()
                    .filter(|marker| {
                        matches!(
                            marker.kind,
                            TileKind::Gas
                                | TileKind::Lava
                                | TileKind::MagmaVent
                                | TileKind::ExplosivePocket
                                | TileKind::PressurePocket
                        )
                    })
                    .count()
                    >= usize::try_from(self.side_contract_required).unwrap_or(usize::MAX)
            }
        };
        if !satisfied {
            return;
        }
        if self.side_contract_kind == SideContractKind::Cargo {
            match target {
                TileKind::Ore(mineral) => consume_side_count(
                    &mut self.player.cargo,
                    &mineral,
                    self.side_contract_required,
                ),
                TileKind::Artifact(artifact) => consume_side_count(
                    &mut self.player.artifacts,
                    &artifact,
                    self.side_contract_required,
                ),
                _ => {}
            }
        }
        let reward = 420 + self.side_contract_required * 80;
        self.player.credits += reward;
        self.total_earnings += reward;
        self.side_contract_active = false;
        self.message = format!("Side contract fulfilled: {reward} credits bonus.");
        self.sound_cues.push(SoundCue::Sell);
    }

    fn confirm_depot(&mut self) {
        match self.selected_menu_item {
            0 => {
                self.try_complete_expeditions();
                self.try_complete_side_contract();
                self.confirm_complete_contract();
            }
            1 => self.confirm_sell_cargo(),
            2 => self.auto_sort_low_grade_cargo(),
            3 => self.sell_scan_data(),
            _ => self.modal = Some(ModalScreen::DepotReceiptHistory),
        }
    }

    fn confirm_complete_contract(&mut self) {
        if let Some(completion) = self.contracts.try_complete(&mut self.player) {
            self.sound_cues.push(SoundCue::Sell);
            let escrow_bonus = completion.reward * u32::from(self.town_development.bank_level) / 20;
            if escrow_bonus > 0 {
                self.player.credits = self.player.credits.saturating_add(escrow_bonus);
            }
            self.total_earnings += completion.reward.saturating_add(escrow_bonus);
            if completion.finished_story {
                self.won_game = true;
                self.deep_claim_status = DeepClaimStatus::Unlocked;
                self.fastest_star_core_seconds = self
                    .fastest_star_core_seconds
                    .map_or(Some(self.play_seconds), |best| {
                        Some(best.min(self.play_seconds))
                    });
                self.message = format!(
                    "{} complete! Star Core secured. Deep Claim charter unlocked. Bonus: {} credits + {escrow_bonus} escrow.",
                    completion.completed_title, completion.reward
                );
            } else {
                let story = ContractLog::story_for_completed(self.contracts.completed);
                self.message = format!(
                    "{} complete! Bonus paid: {} credits + {escrow_bonus} escrow. {story}",
                    completion.completed_title, completion.reward
                );
            }
        } else {
            "Contract target not ready.".clone_into(&mut self.message);
        }
    }

    fn confirm_sell_cargo(&mut self) {
        self.last_depot_receipt.clear();
        for (mineral, count) in &self.player.cargo {
            let _ = writeln!(
                &mut self.last_depot_receipt,
                "{} x{} = {} cr",
                mineral.name(),
                count,
                mineral.value() * count
            );
        }
        for (artifact, count) in &self.player.artifacts {
            let _ = writeln!(
                &mut self.last_depot_receipt,
                "{} x{} = {} cr",
                artifact.name(),
                count,
                artifact.value() * count
            );
        }

        let depot_bonus = u32::from(self.town_development.depot_level) * 3;
        let mut adjusted = self
            .player
            .cargo
            .iter()
            .map(|(mineral, count)| {
                mineral.value() * count * (self.mineral_market_factor(*mineral) + depot_bonus) / 100
            })
            .sum::<u32>()
            + self
                .player
                .artifacts
                .iter()
                .map(|(artifact, count)| artifact.value() * count)
                .sum::<u32>();
        let cargo_count = self.player.cargo_used();
        let bulk_bonus = self.town_development.depot_level >= 2 && cargo_count >= 8;
        if bulk_bonus {
            adjusted += adjusted / 10;
        }
        let diverse_bonus = self.has_equipped_part(RigPartKind::SortedCargoRack)
            && self
                .player
                .cargo
                .len()
                .saturating_add(self.player.artifacts.len())
                >= 4;
        if diverse_bonus {
            adjusted += adjusted / 8;
        }
        let payout = sell_cargo(&mut self.player);
        if adjusted != payout {
            self.player.credits = self
                .player
                .credits
                .saturating_sub(payout)
                .saturating_add(adjusted);
        }
        self.market_salt = self.market_salt.wrapping_add(1);
        if adjusted > 0 {
            self.total_earnings += adjusted;
            let _ = writeln!(
                &mut self.last_depot_receipt,
                "MARKET mineral pricing applied{}{}",
                if bulk_bonus { " + bulk-sale bonus" } else { "" },
                if diverse_bonus {
                    " + sorted-rack diversity bonus"
                } else {
                    ""
                }
            );
            let _ = writeln!(&mut self.last_depot_receipt, "TOTAL = {adjusted} cr");
            self.depot_receipts.push(self.last_depot_receipt.clone());
            if self.depot_receipts.len() > 5 {
                self.depot_receipts.remove(0);
            }
        }
        if adjusted == 0 {
            "No cargo to sell.".clone_into(&mut self.message);
        } else {
            self.sound_cues.push(SoundCue::Sell);
            self.message = if diverse_bonus {
                format!("Sold cargo for {adjusted} credits with sorted-rack diversity bonus.")
            } else if bulk_bonus {
                format!("Sold cargo for {adjusted} credits with depot bulk-sale bonus.")
            } else {
                format!("Sold cargo for {adjusted} credits at current mineral markets.")
            };
        }
    }

    fn sell_scan_data(&mut self) {
        if self.town_development.scanner_lab_level == 0 {
            "Scanner Lab must be funded before scan data has a buyer."
                .clone_into(&mut self.message);
            return;
        }
        let marker_count = u32::try_from(self.scan_markers.len()).unwrap_or(u32::MAX);
        if marker_count == 0 {
            "No scan markers to sell. Pulse the scanner or deploy survey drones first."
                .clone_into(&mut self.message);
            return;
        }
        let payout =
            marker_count.saturating_mul(6 + u32::from(self.town_development.scanner_lab_level) * 4);
        self.scan_markers.clear();
        self.player.credits = self.player.credits.saturating_add(payout);
        self.total_earnings = self.total_earnings.saturating_add(payout);
        self.sound_cues.push(SoundCue::Sell);
        self.message =
            format!("Scanner Lab bought {marker_count} mapped contact(s) for {payout} credits.");
    }

    fn auto_sort_low_grade_cargo(&mut self) {
        if self.town_development.depot_level < 3 {
            "Depot auto-sort rules unlock at Depot level 3.".clone_into(&mut self.message);
            return;
        }
        let low_grade = [MineralKind::Copper, MineralKind::Iron, MineralKind::Silver];
        let mut units = 0;
        let mut payout = 0;
        for mineral in low_grade {
            let Some(count) = self.player.cargo.remove(&mineral) else {
                continue;
            };
            units += count;
            payout += self.mineral_market_value(mineral).saturating_mul(count);
        }
        if payout == 0 {
            "Auto-sort found no low-grade mineral cargo. Rare cargo preserved."
                .clone_into(&mut self.message);
            return;
        }
        self.player.credits = self.player.credits.saturating_add(payout);
        self.total_earnings = self.total_earnings.saturating_add(payout);
        self.last_depot_receipt = format!(
            "AUTO-SORT sold {units} low-grade unit(s) for {payout} cr. Rare minerals and artifacts preserved."
        );
        self.depot_receipts.push(self.last_depot_receipt.clone());
        if self.depot_receipts.len() > 5 {
            self.depot_receipts.remove(0);
        }
        self.sound_cues.push(SoundCue::Sell);
        self.message = format!(
            "Depot auto-sort sold {units} low-grade unit(s) for {payout} credits; rare cargo kept onboard."
        );
    }

    fn try_buy_upgrade(&mut self, index: usize) {
        if self.current_zone != Some(SurfaceZone::Shop) {
            return;
        }

        match buy_upgrade(&mut self.player, index) {
            Ok(offer) => {
                self.sound_cues.push(SoundCue::Upgrade);
                self.message = format!(
                    "Bought {}.",
                    upgrade_tier_name(offer.kind, offer.level.saturating_sub(1))
                );
            }
            Err(PurchaseError::InvalidSelection) => {
                "Unknown upgrade selection.".clone_into(&mut self.message);
            }
            Err(PurchaseError::MaxLevel) => {
                "That upgrade is already maxed.".clone_into(&mut self.message);
            }
            Err(PurchaseError::NotEnoughCredits) => {
                "Not enough credits for that upgrade.".clone_into(&mut self.message);
            }
        }
        self.modal = Some(ModalScreen::Shop);
    }

    fn update_world_event_hazards(&mut self) {
        if self.run_mode != RunMode::Playing || self.game_over || self.player.y < TILE_SIZE * 8.0 {
            return;
        }
        if self.has_world_event(WorldEventKind::CollapseSurge)
            && self.update_ticks.is_multiple_of(180)
        {
            self.spawn_cave_in();
            "Collapse surge: falling rock detected nearby.".clone_into(&mut self.message);
        }
        if self.has_world_event(WorldEventKind::GasBloom) && self.update_ticks.is_multiple_of(240) {
            self.hazard_clouds.push(HazardCloud {
                x: self.player.x,
                y: self.player.y + TILE_SIZE,
                life: 6.0,
                radius: 12.0,
            });
            "Gas bloom: corrosive vapor is spreading through open tunnels."
                .clone_into(&mut self.message);
        }
    }

    fn update_npc_story_records(&mut self) {
        if self.current_zone != Some(SurfaceZone::Headquarters) {
            return;
        }
        let record = if self.won_game {
            NpcStoryRecord::ValeStarCoreSecured
        } else {
            match self.deepest_tile_reached {
                0..=19 => NpcStoryRecord::ValeIntro,
                20..=39 => NpcStoryRecord::IonaSilverWarning,
                40..=59 => NpcStoryRecord::KadeRelicSignal,
                60..=79 => NpcStoryRecord::ValeThermalWarning,
                _ => NpcStoryRecord::KadeStarCoreSignal,
            }
        };
        self.collection_log.story_records.insert(record);
    }

    fn update_collection_rewards(&mut self) {
        if self.collection_log.minerals.len() >= 10
            && self
                .collection_log
                .rewards_claimed
                .insert(CollectionRewardKind::Minerals)
        {
            self.player.credits = self.player.credits.saturating_add(500);
            self.player
                .add_material(StrategicResourceKind::CrystalLens, 2);
            "Mineral encyclopedia complete: awarded 500 credits and 2 Crystal Lenses."
                .clone_into(&mut self.message);
            self.sound_cues.push(SoundCue::Milestone);
        }
        if self.collection_log.artifacts.len() >= 4
            && self
                .collection_log
                .rewards_claimed
                .insert(CollectionRewardKind::Artifacts)
        {
            self.player.credits = self.player.credits.saturating_add(900);
            self.player
                .add_material(StrategicResourceKind::CoreShard, 2);
            "Artifact museum complete: awarded 900 credits and 2 Core Shards."
                .clone_into(&mut self.message);
            self.sound_cues.push(SoundCue::Milestone);
        }
        if self.collection_log.hazards.len() >= 5
            && self
                .collection_log
                .rewards_claimed
                .insert(CollectionRewardKind::Hazards)
        {
            self.town_development.scanner_lab_level =
                self.town_development.scanner_lab_level.saturating_add(1);
            "Hazard research complete: scanner lab upgraded.".clone_into(&mut self.message);
            self.sound_cues.push(SoundCue::Milestone);
        }
    }

    fn handle_scanner(&mut self, input: PlayerInput) {
        if !input.scan {
            return;
        }
        if self.has_world_event(WorldEventKind::DeepPressureStorm) {
            "Deep pressure storm is scrambling scanner returns today."
                .clone_into(&mut self.message);
            return;
        }
        if self.player.scanner_level == 0 {
            "No scanner installed. Buy one at the upgrade shop.".clone_into(&mut self.message);
            return;
        }
        if self.scanner_cooldown_seconds > 0.0 {
            self.message = format!("Scanner recharging: {:.1}s.", self.scanner_cooldown_seconds);
            return;
        }
        self.scanner_pulse_seconds = 1.2;
        self.scanner_cooldown_seconds = (7.0
            - f32::from(self.player.scanner_level)
            - f32::from(self.town_development.scanner_lab_level) * 0.35)
            .max(1.5);
        self.reveal_scanner_area();
        self.sound_cues.push(SoundCue::Ui);
        "Scanner pulse mapped ore, hazards, and artifacts nearby.".clone_into(&mut self.message);
    }

    fn update_scanner_timers(&mut self, delta_seconds: f32) {
        self.scanner_pulse_seconds = (self.scanner_pulse_seconds - delta_seconds).max(0.0);
        self.scanner_cooldown_seconds = (self.scanner_cooldown_seconds - delta_seconds).max(0.0);
    }

    fn handle_bomb(&mut self, input: PlayerInput) {
        if !input.bomb {
            return;
        }
        if self.player.bombs == 0 {
            "No bombs. Buy bomb packs at the upgrade shop.".clone_into(&mut self.message);
            return;
        }
        self.player.bombs -= 1;
        let remote_timer = if self.town_development.explosives_shack_level >= 3 {
            0.8
        } else {
            2.4
        };
        self.placed_bombs.push(PlacedBomb {
            x: self.player.x,
            y: self.player.y + TILE_SIZE * 0.4,
            timer_seconds: remote_timer,
        });
        self.sound_cues.push(SoundCue::Ui);
        self.message = format!(
            "Bomb armed: {remote_timer:.1} seconds. {} bombs left. Clear out!",
            self.player.bombs
        );
    }

    fn handle_infrastructure_placement(&mut self, input: PlayerInput) {
        if input.place_relay {
            self.place_infrastructure_kit(
                InfrastructureKind::SignalRelay,
                "No signal relay kits. Craft one at HQ first.",
            );
        }
        if input.place_drone {
            self.place_infrastructure_kit(
                InfrastructureKind::SurveyDrone,
                "No survey drone kits. Craft one at HQ first.",
            );
        }
        if input.place_lift {
            self.place_infrastructure_kit(
                InfrastructureKind::CargoLift,
                "No cargo lift kits. Craft one at HQ first.",
            );
        }
        if input.place_support {
            self.place_infrastructure_kit(
                InfrastructureKind::TunnelSupport,
                "No tunnel support kits. Craft one at HQ first.",
            );
        }
        if input.place_pump {
            self.place_infrastructure_kit(
                InfrastructureKind::PumpStation,
                "No pump station kits. Craft one at HQ first.",
            );
        }
        if input.place_processor {
            self.place_infrastructure_kit(
                InfrastructureKind::OreProcessor,
                "No ore processor kits. Craft one at HQ first.",
            );
        }
    }

    fn place_infrastructure_kit(&mut self, kind: InfrastructureKind, empty_message: &str) {
        let kits = match kind {
            InfrastructureKind::SignalRelay => self.player.signal_relay_kits,
            InfrastructureKind::SurveyDrone => self.player.survey_drone_kits,
            InfrastructureKind::CargoLift => self.player.cargo_lift_kits,
            InfrastructureKind::TunnelSupport => self.player.tunnel_support_kits,
            InfrastructureKind::PumpStation => self.player.pump_station_kits,
            InfrastructureKind::OreProcessor => self.player.ore_processor_kits,
        };
        if kits == 0 {
            empty_message.clone_into(&mut self.message);
            return;
        }
        let position = self.player.tile_position(TILE_SIZE);
        if position.y < 8 {
            format!("{} must be placed underground.", kind.name()).clone_into(&mut self.message);
            return;
        }
        if self
            .infrastructure
            .iter()
            .any(|item| item.position == position)
        {
            "Infrastructure already occupies this tile.".clone_into(&mut self.message);
            return;
        }
        match kind {
            InfrastructureKind::SignalRelay => {
                self.player.signal_relay_kits = self.player.signal_relay_kits.saturating_sub(1);
            }
            InfrastructureKind::SurveyDrone => {
                self.player.survey_drone_kits = self.player.survey_drone_kits.saturating_sub(1);
            }
            InfrastructureKind::CargoLift => {
                self.player.cargo_lift_kits = self.player.cargo_lift_kits.saturating_sub(1);
            }
            InfrastructureKind::TunnelSupport => {
                self.player.tunnel_support_kits = self.player.tunnel_support_kits.saturating_sub(1);
            }
            InfrastructureKind::PumpStation => {
                self.player.pump_station_kits = self.player.pump_station_kits.saturating_sub(1);
            }
            InfrastructureKind::OreProcessor => {
                self.player.ore_processor_kits = self.player.ore_processor_kits.saturating_sub(1);
            }
        }
        self.infrastructure.push(PlacedInfrastructure {
            kind,
            position,
            durability: default_infrastructure_durability(),
        });
        self.infrastructure_built = self.infrastructure_built.saturating_add(1);
        self.message = match kind {
            InfrastructureKind::SignalRelay => {
                "Placed Signal Relay. Deep rescue signal improved.".to_owned()
            }
            InfrastructureKind::SurveyDrone => {
                "Placed Survey Drone. Nearby map will reveal over time.".to_owned()
            }
            InfrastructureKind::CargoLift => {
                "Placed Cargo Lift. Press E on it to send cargo upward.".to_owned()
            }
            InfrastructureKind::TunnelSupport => {
                "Placed Tunnel Support. Nearby collapse warnings will be suppressed.".to_owned()
            }
            InfrastructureKind::PumpStation => {
                "Placed Pump Station. Nearby gas and heat hazards are suppressed.".to_owned()
            }
            InfrastructureKind::OreProcessor => {
                "Placed Ore Processor. Press E nearby to refine cheap ore.".to_owned()
            }
        };
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn effective_cargo_capacity(&self) -> u32 {
        let mut capacity = self.player.cargo_capacity;
        if self.has_equipped_part(RigPartKind::CargoBalloon) {
            capacity = capacity.saturating_add(20);
        }
        if self.has_equipped_part(RigPartKind::ArmoredCargoBay) {
            capacity = capacity.saturating_sub(4).max(1);
        }
        capacity
    }

    fn add_mined_cargo(&mut self, mineral: MineralKind) -> bool {
        if self.effective_cargo_capacity() == self.player.cargo_capacity {
            return self.player.add_cargo(mineral);
        }
        if self.player.cargo_used() >= self.effective_cargo_capacity() {
            return false;
        }
        *self.player.cargo.entry(mineral).or_default() += 1;
        true
    }

    fn add_mined_artifact(&mut self, artifact: ArtifactKind) -> bool {
        if self.effective_cargo_capacity() == self.player.cargo_capacity {
            return self.player.add_artifact(artifact);
        }
        if self.player.cargo_used() >= self.effective_cargo_capacity() {
            return false;
        }
        *self.player.artifacts.entry(artifact).or_default() += 1;
        true
    }

    fn has_equipped_part(&self, part: RigPartKind) -> bool {
        self.equipped_rig_parts
            .values()
            .any(|equipped| *equipped == part)
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;
        let grounded = self.is_grounded();
        let cargo_ratio =
            self.player.cargo_used() as f32 / self.effective_cargo_capacity().max(1) as f32;
        let cargo_pressure = if self.has_equipped_part(RigPartKind::HaulerEngine) {
            0.08
        } else if self.has_equipped_part(RigPartKind::LightweightEngine) {
            0.28
        } else {
            0.18
        };
        let cargo_penalty = 1.0 - cargo_ratio.min(1.0) * cargo_pressure;
        let mut engine_multiplier =
            (1.0 + f32::from(self.player.engine_level.saturating_sub(1)) * 0.28) * cargo_penalty;
        if self.has_equipped_part(RigPartKind::LightweightEngine) {
            engine_multiplier *= 1.12;
        }
        if self.has_equipped_part(RigPartKind::HaulerEngine) {
            engine_multiplier *= 0.92;
        }
        if self.has_equipped_part(RigPartKind::BurstEngine) {
            engine_multiplier *= 1.25;
        }
        if self.has_equipped_part(RigPartKind::CargoBalloon) {
            engine_multiplier *= 0.88;
        }
        let horizontal_acceleration = if grounded {
            HORIZONTAL_ACCELERATION * 1.35
        } else {
            HORIZONTAL_ACCELERATION * 0.65
        };

        self.player.velocity_x +=
            input.horizontal * horizontal_acceleration * engine_multiplier * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * engine_multiplier * delta_seconds;
            let mut efficiency =
                1.0 - f32::from(self.player.fuel_tank_level.saturating_sub(1)) * 0.06;
            if self.has_equipped_part(RigPartKind::BurstEngine) {
                efficiency *= 1.22;
            }
            if self.has_equipped_part(RigPartKind::TitanDrill) {
                efficiency *= 1.08;
            }
            self.player.fuel =
                (self.player.fuel - FUEL_BURN_PER_SECOND * efficiency * delta_seconds).max(0.0);
        }

        self.player.velocity_y += GRAVITY * delta_seconds;
        let drag = if grounded { 0.78 } else { DRAG };
        self.player.velocity_x *= drag.powf(delta_seconds * 60.0);
        self.player.velocity_x = self.player.velocity_x.clamp(
            -MAX_HORIZONTAL_SPEED * engine_multiplier,
            MAX_HORIZONTAL_SPEED * engine_multiplier,
        );
        self.player.velocity_y = self
            .player
            .velocity_y
            .clamp(-MAX_FALL_SPEED, MAX_FALL_SPEED);

        self.move_axis(self.player.velocity_x * delta_seconds, 0.0);
        self.move_axis(0.0, self.player.velocity_y * delta_seconds);
    }

    fn move_axis(&mut self, delta_x: f32, delta_y: f32) {
        let next_x = self.player.x + delta_x;
        let next_y = self.player.y + delta_y;

        if self.collides(next_x, next_y) {
            if delta_x != 0.0 {
                if self.player.velocity_x.abs() > SAFE_LANDING_SPEED * 0.75 {
                    self.apply_bump_damage(self.player.velocity_x.abs());
                }
                self.player.velocity_x = 0.0;
            }
            if delta_y > 0.0 {
                self.apply_landing_damage();
            }
            if delta_y != 0.0 {
                self.player.velocity_y = 0.0;
            }
            return;
        }

        self.player.x = next_x.clamp(0.0, (self.terrain.width() as f32 - 1.0) * TILE_SIZE);
        self.player.y = next_y.clamp(
            MIN_PLAYER_Y,
            (self.terrain.height() as f32 - 1.0) * TILE_SIZE,
        );
    }

    fn apply_landing_damage(&mut self) {
        if self.player.velocity_y <= SAFE_LANDING_SPEED {
            return;
        }

        let raw_damage = (self.player.velocity_y - SAFE_LANDING_SPEED) * CRASH_DAMAGE_SCALE;
        let damage = self.mechanic_crash_damage(raw_damage);
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        self.shake_camera(0.28, 7.0);
        self.spawn_sparks();
        self.message = format!("Hard landing! Hull took {damage:.0} damage.");
    }

    fn mechanic_crash_damage(&self, damage: f32) -> f32 {
        let mut mitigation = (f32::from(self.town_development.mechanic_level) * 0.08).min(0.32);
        if self.has_equipped_part(RigPartKind::ImpactHull) {
            mitigation += 0.22;
        }
        if self.has_equipped_part(RigPartKind::ThermalHull) {
            mitigation -= 0.10;
        }
        let mitigation = mitigation.clamp(0.0, 0.55);
        damage * (1.0 - mitigation)
    }

    fn apply_bump_damage(&mut self, speed: f32) {
        let raw_damage = (speed - SAFE_LANDING_SPEED * 0.75) * CRASH_DAMAGE_SCALE * 0.5;
        let damage = self.mechanic_crash_damage(raw_damage);
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        self.shake_camera(0.2, 5.0);
        self.spawn_sparks();
        self.message = format!("Hull scraped the wall for {damage:.0} damage.");
    }

    fn collides(&self, x: f32, y: f32) -> bool {
        collision_points(x, y)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    fn is_grounded(&self) -> bool {
        collision_points(self.player.x, self.player.y + 2.0)
            .iter()
            .any(|position| self.terrain.is_solid_at(*position))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "drilling update coordinates input, physics, terrain, feedback, and collection in one frame step"
    )]
    fn update_drilling(&mut self, input: PlayerInput, delta_seconds: f32) {
        let Some((target, direction)) = mine_target(&self.player, input) else {
            self.active_drill = None;
            return;
        };

        if direction != DrillDirection::Down
            && (!self.is_grounded() || self.player.velocity_y.abs() > 80.0)
        {
            self.active_drill = None;
            "Side drilling requires stable ground contact.".clone_into(&mut self.message);
            return;
        }

        if self.player.fuel <= 0.0 {
            self.active_drill = None;
            "Out of fuel. Reach a fuel station or await rescue.".clone_into(&mut self.message);
            return;
        }

        let Some(tile) = self.terrain.tile(target) else {
            self.active_drill = None;
            return;
        };
        if tile.kind == TileKind::Air {
            self.active_drill = None;
            return;
        }
        let effective_drill_strength = if self.has_equipped_part(RigPartKind::TitanDrill)
            || self.has_equipped_part(RigPartKind::ResonanceDrill)
        {
            self.player.drill_strength.saturating_add(1)
        } else {
            self.player.drill_strength
        };
        if self
            .terrain
            .hardness_at(target)
            .is_some_and(|hardness| hardness > effective_drill_strength)
        {
            self.active_drill = None;
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.16, 4.0);
            self.spawn_sparks();
            "That layer is too hard. Upgrade your drill.".clone_into(&mut self.message);
            return;
        }

        let mut seconds_per_chip =
            drill_seconds_per_chip(tile.kind, effective_drill_strength, direction);
        if self.has_equipped_part(RigPartKind::NeedleDrill) {
            seconds_per_chip *= match tile.kind {
                TileKind::Dirt | TileKind::Clay => 0.72,
                TileKind::Stone | TileKind::HardRock => 1.25,
                _ => 0.95,
            };
        }
        if self.has_equipped_part(RigPartKind::TitanDrill) {
            seconds_per_chip *= 1.10;
        }
        if self.has_equipped_part(RigPartKind::ResonanceDrill)
            && matches!(
                deep_stratum_at_depth(target.y),
                Some(DeepStratum::CrystalFaults | DeepStratum::AncientMachineLayer)
            )
        {
            seconds_per_chip *= 0.65;
        }
        let reset = self
            .active_drill
            .is_none_or(|state| state.target != target || state.direction != direction);
        if reset {
            self.active_drill = Some(DrillState {
                target,
                direction,
                progress: 0.0,
                initial_durability: tile.durability.max(1),
                seconds_per_chip,
                sound_timer: 0.0,
                dust_timer: 0.0,
            });
        }

        self.player.fuel = (self.player.fuel - DRILL_FUEL_COST * 1.25 * delta_seconds).max(0.0);
        self.creep_into_drill(direction, delta_seconds);

        let mut should_chip = false;
        let mut should_spawn_dust = false;
        if let Some(state) = &mut self.active_drill {
            state.seconds_per_chip = seconds_per_chip;
            state.progress += delta_seconds / seconds_per_chip;
            state.sound_timer -= delta_seconds;
            state.dust_timer -= delta_seconds;
            if state.sound_timer <= 0.0 {
                self.sound_cues.push(SoundCue::Drill);
                state.sound_timer = 0.13;
            }
            if state.dust_timer <= 0.0 {
                should_spawn_dust = true;
                state.dust_timer = 0.09;
            }
            should_chip = state.progress >= 1.0;
            self.drill_flash_seconds = 0.09;
            let chipped = state.initial_durability.saturating_sub(tile.durability);
            let total_progress = ((f32::from(chipped) + state.progress.min(1.0))
                / f32::from(state.initial_durability.max(1)))
            .clamp(0.0, 1.0);
            self.message = format!(
                "Drilling {}... {:.0}%",
                tile.kind.name(),
                total_progress * 100.0
            );
        }
        if should_spawn_dust {
            self.spawn_dust();
        }

        if should_chip {
            if let Some(state) = &mut self.active_drill {
                state.progress -= 1.0;
            }
            let mine_result = self.terrain.chip(target);
            if !matches!(mine_result, MineResult::Blocked | MineResult::TooDangerous) {
                self.mark_tile_visual_changed(target);
            }
            match mine_result {
                MineResult::Blocked => self.active_drill = None,
                MineResult::TooDangerous => {
                    self.active_drill = None;
                    self.player.hull = (self.player.hull - 8.0).max(0.0);
                    self.sound_cues.push(SoundCue::Damage);
                    self.screen_flash_seconds = 0.1;
                    let warning = if self
                        .terrain
                        .tile(target)
                        .is_some_and(|tile| tile.kind == TileKind::MagmaVent)
                    {
                        "Magma vent! Hull scorched and heat rising."
                    } else {
                        "Lava pocket! Hull scorched."
                    };
                    warning.clone_into(&mut self.message);
                }
                MineResult::Exploded => {
                    self.active_drill = None;
                    self.trigger_gas_explosion();
                }
                MineResult::Blast => {
                    self.active_drill = None;
                    self.trigger_explosive_pocket();
                }
                MineResult::Chipped => {}
                MineResult::Mined(mined) => {
                    self.active_drill = None;
                    self.collect_mined_tile(mined, target);
                }
            }
        }
    }

    fn creep_into_drill(&mut self, direction: DrillDirection, delta_seconds: f32) {
        let creep = 32.0 * delta_seconds;
        match direction {
            DrillDirection::Down => self.move_axis(0.0, creep),
            DrillDirection::Left => self.move_axis(-creep * 0.65, 0.0),
            DrillDirection::Right => self.move_axis(creep * 0.65, 0.0),
        }
    }

    fn trigger_gas_explosion(&mut self) {
        let protected = self.is_pump_protected(self.player.tile_position(TILE_SIZE));
        self.player.fuel = (self.player.fuel - DRILL_FUEL_COST).max(0.0);
        self.player.velocity_x *= -0.25;
        self.player.velocity_y = -90.0;
        self.sound_cues.push(SoundCue::Damage);
        self.drill_flash_seconds = 0.2;
        self.screen_flash_seconds = 0.12;
        if !protected {
            self.hazard_clouds.push(HazardCloud {
                x: self.player.x,
                y: self.player.y + TILE_SIZE,
                life: 8.0,
                radius: 10.0,
            });
        }
        for _ in 0..5 {
            self.spawn_dust();
        }
        if protected {
            "Pump station vented nearby gas before it became corrosive."
                .clone_into(&mut self.message);
        } else {
            "Gas pocket venting! Clear the green leak before it turns corrosive."
                .clone_into(&mut self.message);
        }
    }

    fn trigger_explosive_pocket(&mut self) {
        self.player.fuel = (self.player.fuel - DRILL_FUEL_COST * 2.0).max(0.0);
        self.player.hull = (self.player.hull - 24.0).max(0.0);
        self.player.velocity_x *= -0.7;
        self.player.velocity_y = -260.0;
        self.sound_cues.push(SoundCue::Damage);
        self.drill_flash_seconds = 0.35;
        self.screen_flash_seconds = 0.22;
        self.shake_camera(0.45, 14.0);
        for _ in 0..12 {
            self.spawn_dust();
            self.spawn_sparks();
        }
        "Explosive pocket detonated! Hull damaged and tunnel destabilized."
            .clone_into(&mut self.message);
        self.spawn_cave_in();
    }

    fn collect_mined_tile(&mut self, mined: TileKind, target: TilePosition) {
        self.scan_markers.retain(|marker| marker.position != target);
        self.player.fuel -= DRILL_FUEL_COST;
        self.sound_cues.push(SoundCue::Drill);
        self.spawn_dust();
        self.drill_flash_seconds = 0.12;

        self.collection_log.discover_tile(mined);

        if let TileKind::Ore(mineral) = mined {
            if self.add_mined_cargo(mineral) {
                self.message = format!("Loaded {} ore worth {}.", mineral.name(), mineral.value());
                if self.deep_claim_status == DeepClaimStatus::Unlocked
                    && target.y >= 70
                    && let Some(material) = deep_claim_material_for(mineral, target)
                {
                    self.player.add_material(material, 1);
                    let _ = write!(self.message, " Found {}.", material.name());
                }
            } else {
                "Cargo full. Return to depot to sell.".clone_into(&mut self.message);
            }
        } else if let TileKind::Artifact(artifact) = mined {
            if self.add_mined_artifact(artifact) {
                self.artifacts_found += 1;
                if artifact == ArtifactKind::StarCore {
                    self.escape_sequence_seconds = 120.0;
                    self.shake_camera(1.0, 10.0);
                    "Star Core extracted! Core fracture cascade started: return to HQ before the mine collapses."
                        .clone_into(&mut self.message);
                } else {
                    self.message = format!(
                        "Recovered {} artifact worth {}.",
                        artifact.name(),
                        artifact.value()
                    );
                }
            } else {
                "Cargo full. Return to depot to sell.".clone_into(&mut self.message);
            }
        } else if mined == TileKind::PressurePocket {
            self.player.velocity_y = -360.0;
            self.player.velocity_x *= 1.4;
            self.player.hull = (self.player.hull - 10.0).max(0.0);
            self.shake_camera(0.3, 9.0);
            self.sound_cues.push(SoundCue::Damage);
            "Pressure pocket ruptured! The blast shoved the rig upward."
                .clone_into(&mut self.message);
        } else {
            "Tunnel opened.".clone_into(&mut self.message);
        }
        if matches!(mined, TileKind::Stone | TileKind::HardRock)
            && falling_rock_roll(target, self.terrain.seed())
        {
            self.falling_boulders.push(FallingBoulder {
                x: target.x as f32 * TILE_SIZE + TILE_SIZE * 0.5,
                y: (target.y as f32 - 1.0) * TILE_SIZE,
                velocity_y: 0.0,
                warning_seconds: BOULDER_WARNING_SECONDS,
                life: 3.6,
            });
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.18, 4.0);
            self.message.push_str(" Unstable rock falling!");
        }
    }

    fn spawn_cave_in(&mut self) {
        for offset in -1_i32..=1 {
            self.falling_boulders.push(FallingBoulder {
                x: self.player.x + offset as f32 * TILE_SIZE,
                y: self.player.y - TILE_SIZE * 2.0,
                velocity_y: 0.0,
                warning_seconds: 0.45 + offset.unsigned_abs() as f32 * 0.15,
                life: 4.0,
            });
        }
    }

    fn update_service_animation(&mut self, delta_seconds: f32) {
        self.service_animation_seconds = (self.service_animation_seconds - delta_seconds).max(0.0);
        if self.service_animation_seconds == 0.0 {
            self.service_animation = None;
        }
    }

    fn update_placed_bombs(&mut self, delta_seconds: f32) {
        let mut detonations = Vec::new();
        for bomb in &mut self.placed_bombs {
            bomb.timer_seconds -= delta_seconds;
            if bomb.timer_seconds <= 0.0 {
                detonations.push(TilePosition {
                    x: (bomb.x / TILE_SIZE).floor() as i32,
                    y: (bomb.y / TILE_SIZE).floor() as i32,
                });
            }
        }
        self.placed_bombs.retain(|bomb| bomb.timer_seconds > 0.0);
        for center in detonations {
            let radius = if self.town_development.explosives_shack_level >= 4 {
                3
            } else {
                2
            };
            self.detonate_bomb(center, radius);
        }
    }

    fn detonate_bomb(&mut self, center: TilePosition, radius: i32) {
        let blast = self.terrain.blast_radius(center, radius);
        self.mark_tiles_visual_changed(blast.changed_tiles);
        let cleared = blast.cleared;
        self.sound_cues.push(SoundCue::Explosion);
        self.screen_flash_seconds = self.screen_flash_seconds.max(0.22);
        self.shake_camera(0.45, 13.0);
        for _ in 0..14 {
            self.spawn_dust();
            self.spawn_sparks();
        }
        let distance = ((self.player.x / TILE_SIZE - center.x as f32).abs()
            + (self.player.y / TILE_SIZE - center.y as f32).abs())
        .max(0.0);
        if distance <= radius as f32 + 1.0 {
            self.player.hull = (self.player.hull - 22.0).max(0.0);
        }
        if self.town_development.explosives_shack_level >= 5 {
            return;
        }
        self.chain_react_near(center, radius + 2);
        self.message =
            format!("Bomb detonated. Cleared {cleared} tiles and rattled nearby pockets.");
        self.reveal_near_player();
    }

    fn chain_react_near(&mut self, center: TilePosition, radius: i32) {
        for y in center.y - radius..=center.y + radius {
            for x in center.x - radius..=center.x + radius {
                if (x - center.x).abs() + (y - center.y).abs() > radius {
                    continue;
                }
                let position = TilePosition { x, y };
                if matches!(
                    self.terrain.tile(position).map(|tile| tile.kind),
                    Some(TileKind::Gas | TileKind::ExplosivePocket | TileKind::PressurePocket)
                ) {
                    let blast = self.terrain.blast_radius(position, 1);
                    self.mark_tiles_visual_changed(blast.changed_tiles);
                    self.hazard_clouds.push(HazardCloud {
                        x: x as f32 * TILE_SIZE,
                        y: y as f32 * TILE_SIZE,
                        life: 6.0,
                        radius: 18.0,
                    });
                }
            }
        }
    }

    fn update_particles(&mut self, delta_seconds: f32) {
        for particle in &mut self.dust_particles {
            particle.life -= delta_seconds;
            particle.y -= 18.0 * delta_seconds;
        }
        self.dust_particles.retain(|particle| particle.life > 0.0);
        for spark in &mut self.spark_particles {
            spark.life -= delta_seconds;
            spark.x += spark.velocity_x * delta_seconds;
            spark.y += spark.velocity_y * delta_seconds;
            spark.velocity_y += 180.0 * delta_seconds;
        }
        self.spark_particles.retain(|particle| particle.life > 0.0);
    }

    fn update_boulders(&mut self, delta_seconds: f32) {
        for boulder in &mut self.falling_boulders {
            boulder.life -= delta_seconds;
            if boulder.warning_seconds > 0.0 {
                boulder.warning_seconds -= delta_seconds;
                continue;
            }
            boulder.velocity_y = (boulder.velocity_y + GRAVITY * 0.8 * delta_seconds).min(520.0);
            boulder.y += boulder.velocity_y * delta_seconds;
        }

        let mut hit_player = false;
        self.falling_boulders.retain(|boulder| {
            if boulder.warning_seconds > 0.0 {
                return true;
            }
            let dx = self.player.x - boulder.x;
            let dy = self.player.y - boulder.y;
            let hit = dx.hypot(dy) <= PLAYER_RADIUS + 8.0;
            hit_player |= hit;
            !hit
        });
        if hit_player {
            self.player.hull = (self.player.hull - BOULDER_DAMAGE).max(0.0);
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.35, 9.0);
            self.spawn_sparks();
            "Falling boulder slammed the rig!".clone_into(&mut self.message);
        }

        self.falling_boulders.retain(|boulder| {
            boulder.life > 0.0
                && boulder.y < (self.terrain.height() as f32 - 1.0) * TILE_SIZE
                && !self.terrain.is_solid_at(TilePosition {
                    x: (boulder.x / TILE_SIZE).floor() as i32,
                    y: (boulder.y / TILE_SIZE).floor() as i32,
                })
        });
    }

    fn update_hazards(&mut self, delta_seconds: f32) {
        for cloud in &mut self.hazard_clouds {
            cloud.life -= delta_seconds;
            cloud.radius += 8.0 * delta_seconds;
        }
        self.hazard_clouds.retain(|cloud| cloud.life > 0.0);

        let in_gas = self.hazard_clouds.iter().any(|cloud| {
            if cloud.life >= 6.0 {
                return false;
            }
            let dx = self.player.x - cloud.x;
            let dy = self.player.y - cloud.y;
            dx.hypot(dy) <= cloud.radius
        });
        if in_gas {
            self.player.hull = (self.player.hull - 4.0 * delta_seconds).max(0.0);
            "Corrosive gas cloud eating hull plating!".clone_into(&mut self.message);
        }
    }

    #[allow(
        clippy::missing_const_for_fn,
        reason = "uses f32 max for camera shake state"
    )]
    fn shake_camera(&mut self, seconds: f32, strength: f32) {
        self.camera_shake_seconds = self.camera_shake_seconds.max(seconds);
        self.camera_shake_strength = self.camera_shake_strength.max(strength);
    }

    fn spawn_sparks(&mut self) {
        for index in 0..8 {
            let side = if index % 2 == 0 { -1.0 } else { 1.0 };
            self.spark_particles.push(SparkParticle {
                x: self.player.x + side * 8.0,
                y: self.player.y,
                velocity_x: side * (45.0 + index as f32 * 8.0),
                velocity_y: -80.0 + index as f32 * 12.0,
                life: 0.45,
            });
        }
    }

    fn spawn_dust(&mut self) {
        let base_x = self.player.x;
        let base_y = self.player.y + 18.0;
        self.dust_particles.push(DustParticle {
            x: base_x - 7.0,
            y: base_y,
            life: 0.35,
        });
        self.dust_particles.push(DustParticle {
            x: base_x + 7.0,
            y: base_y + 2.0,
            life: 0.28,
        });
    }

    fn update_depth_milestones(&mut self) {
        let current_tile = (self.player.y / TILE_SIZE).floor() as i32;
        self.deepest_tile_reached = self.deepest_tile_reached.max(current_tile);
        if current_tile > 6 {
            self.trip_best_depth = self.trip_best_depth.max(current_tile);
        }
        if self.deepest_tile_reached < self.next_milestone_tile {
            return;
        }

        let reward = u32::try_from(self.next_milestone_tile).unwrap_or(0) * 2;
        self.player.credits += reward;
        self.total_earnings += reward;
        self.sound_cues.push(SoundCue::Milestone);
        let unlock = match self.next_milestone_tile {
            20 => "Silver seams now appear in useful quantities.",
            40 => "Gold and relic pockets are becoming common.",
            60 => "Emerald, ruby, and heat hazards intensify below.",
            _ => "Diamond traces and Star Core readings strengthen below.",
        };
        self.message = format!(
            "Depth milestone reached: {}m. Survey bonus: {reward} credits. {unlock}",
            self.next_milestone_tile - 5
        );
        self.next_milestone_tile += 20;
    }

    fn apply_depth_pressure(&mut self, delta_seconds: f32) {
        let safe_depth = HEAT_START_DEPTH
            + f32::from(self.player.radiator_level.saturating_sub(1)) * 12.0 * TILE_SIZE;
        if self.player.y <= safe_depth {
            return;
        }

        let depth_factor = ((self.player.y - safe_depth) / (12.0 * TILE_SIZE)).max(1.0);
        let mut heat_multiplier =
            1.0 + self.world_event_severity(WorldEventKind::HeatWave) as f32 * 0.18;
        if self.has_equipped_part(RigPartKind::ThermalHull) {
            heat_multiplier *= 0.55;
        }
        if self.has_equipped_part(RigPartKind::ImpactHull) {
            heat_multiplier *= 1.18;
        }
        if self.has_equipped_part(RigPartKind::PressureHull) && self.player.y >= 90.0 * TILE_SIZE {
            heat_multiplier *= 0.72;
        }
        let damage = HEAT_DAMAGE_PER_SECOND * depth_factor * heat_multiplier * delta_seconds;
        self.player.hull = (self.player.hull - damage).max(0.0);
        "Depth pressure overheating hull. Upgrade radiator.".clone_into(&mut self.message);
    }

    fn apply_lava_damage(&mut self, delta_seconds: f32) {
        let near_lava = (-2..=2).any(|dy| {
            (-2..=2).any(|dx| {
                let position = TilePosition {
                    x: (self.player.x / TILE_SIZE) as i32 + dx,
                    y: (self.player.y / TILE_SIZE) as i32 + dy,
                };
                self.terrain.is_lava_at(position)
            })
        });
        if !near_lava {
            return;
        }

        let player_tile = self.player.tile_position(TILE_SIZE);
        let mut heat_multiplier =
            1.0 + self.world_event_severity(WorldEventKind::HeatWave) as f32 * 0.18;
        if self.has_equipped_part(RigPartKind::ThermalHull) {
            heat_multiplier *= 0.55;
        }
        if self.has_equipped_part(RigPartKind::ImpactHull) {
            heat_multiplier *= 1.18;
        }
        if self.has_equipped_part(RigPartKind::PressureHull) && self.player.y >= 90.0 * TILE_SIZE {
            heat_multiplier *= 0.72;
        }
        let damage = if self.is_pump_protected(player_tile) {
            2.5 * heat_multiplier * delta_seconds
        } else {
            9.0 * heat_multiplier * delta_seconds
        };
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        "Lava heat is burning the hull!".clone_into(&mut self.message);
    }

    fn update_camera(&mut self, delta_seconds: f32) {
        let (target_x, target_y) = target_camera_offset(self);
        if self.camera_intro_seconds > 0.0 {
            self.camera_intro_seconds = (self.camera_intro_seconds - delta_seconds).max(0.0);
            let progress = 1.0 - self.camera_intro_seconds / CAMERA_INTRO_SECONDS;
            let eased = 1.0 - (1.0 - progress).powi(3);
            self.camera_x = target_x;
            self.camera_y = target_y - CAMERA_INTRO_DROP_DISTANCE * (1.0 - eased);
            return;
        }

        let blend = (delta_seconds * CAMERA_SMOOTHING).clamp(0.0, 1.0);
        self.camera_x += (target_x - self.camera_x) * blend;
        self.camera_y += (target_y - self.camera_y) * blend;
    }

    fn update_warning_messages(&mut self) {
        let low_fuel = self.player.fuel <= self.player.fuel_capacity * 0.18;
        let low_hull = self.player.hull <= self.player.max_hull() * 0.25;
        match (low_fuel, low_hull) {
            (true, true) => "CRITICAL: low fuel and damaged hull. Return to surface!"
                .clone_into(&mut self.message),
            (true, false) => "Warning: fuel reserves low.".clone_into(&mut self.message),
            (false, true) => "Warning: hull integrity low.".clone_into(&mut self.message),
            (false, false) => {}
        }
    }

    fn update_deep_run_pressure(&mut self, delta_seconds: f32) {
        let depth = self.player.tile_position(TILE_SIZE).y;
        if depth < 80 || self.current_zone.is_some() {
            self.trip_seconds = 0.0;
            self.deep_instability = (self.deep_instability - delta_seconds * 2.0).max(0.0);
            return;
        }
        self.trip_seconds += delta_seconds;
        let rare_cargo = self.rare_cargo_count() as f32;
        self.deep_instability += delta_seconds
            * ((depth - 70) as f32 * 0.006 + self.trip_seconds * 0.0006 + rare_cargo * 0.02);
        if depth >= 105 {
            self.player.fuel = (self.player.fuel - delta_seconds * 0.25).max(0.0);
            self.scanner_cooldown_seconds = self.scanner_cooldown_seconds.max(1.2);
        }
        if rare_cargo > 0.0 && self.update_ticks.is_multiple_of(240) {
            self.hazard_clouds.push(HazardCloud {
                x: self.player.x,
                y: self.player.y + TILE_SIZE,
                life: 5.0,
                radius: 10.0 + rare_cargo.min(4.0),
            });
            "Rare cargo is attracting unstable deep vapors.".clone_into(&mut self.message);
        }
        if self.deep_instability >= 100.0 {
            self.deep_instability = 45.0;
            self.spawn_cave_in();
            self.player.hull = (self.player.hull - 4.0).max(0.0);
            "Claim instability spiked: cave-in and hull stress detected."
                .clone_into(&mut self.message);
        }
        self.maybe_spawn_deep_reward(depth);
    }

    fn rare_cargo_count(&self) -> u32 {
        let mineral_count: u32 = self
            .player
            .cargo
            .iter()
            .filter(|(mineral, _)| {
                matches!(
                    mineral,
                    MineralKind::Diamond
                        | MineralKind::Platinum
                        | MineralKind::Uranium
                        | MineralKind::Mythril
                )
            })
            .map(|(_, count)| *count)
            .sum();
        let artifact_count: u32 = self.player.artifacts.values().copied().sum();
        mineral_count + artifact_count
    }

    fn maybe_spawn_deep_reward(&mut self, depth: i32) {
        let milestone = depth / 20;
        if depth < 90 || milestone <= self.deep_reward_milestone {
            return;
        }
        self.deep_reward_milestone = milestone;
        let position = TilePosition {
            x: self
                .player
                .tile_position(TILE_SIZE)
                .x
                .saturating_add(2)
                .clamp(1, self.terrain.width() - 2),
            y: depth.clamp(10, self.terrain.height() - 2),
        };
        let reward_tile = if depth >= 120 {
            let material = match depth {
                180.. => StrategicResourceKind::RadiantFossil,
                160..=179 => StrategicResourceKind::VoidGlass,
                140..=159 => StrategicResourceKind::MachineRelic,
                _ => StrategicResourceKind::PressurePearl,
            };
            self.player.add_material(material, 1);
            self.player
                .add_material(StrategicResourceKind::CoreShard, 1);
            self.award_legendary_blueprint(depth);
            TileKind::Artifact(ArtifactKind::OldCircuit)
        } else {
            TileKind::Ore(MineralKind::Mythril)
        };
        if self.terrain.set_kind(position, reward_tile) {
            self.mark_tile_visual_changed(position);
        }
        self.message = if depth >= 120 {
            "Deep reward signal: unique relic exposed and Core Shard recovered.".to_owned()
        } else {
            "Deep reward signal: huge mythril vein migrated nearby.".to_owned()
        };
    }

    fn award_legendary_blueprint(&mut self, depth: i32) {
        let blueprint = if depth >= 160 {
            LegendaryBlueprint::RelicSorter
        } else if depth >= 140 {
            LegendaryBlueprint::VoidTank
        } else {
            LegendaryBlueprint::StarPlating
        };
        if !self.legendary_blueprints.insert(blueprint) {
            return;
        }
        match blueprint {
            LegendaryBlueprint::StarPlating => {
                self.rig_part_inventory.insert(RigPartKind::PressureHull);
                self.equipped_rig_parts
                    .insert(RigSlot::HullPlating, RigPartKind::PressureHull);
                self.player.crafted_bulkheads = self.player.crafted_bulkheads.saturating_add(1);
                self.player.hull = self.player.max_hull();
            }
            LegendaryBlueprint::VoidTank => {
                self.rig_part_inventory.insert(RigPartKind::HaulerEngine);
                self.equipped_rig_parts
                    .insert(RigSlot::Engine, RigPartKind::HaulerEngine);
                self.player.fuel_capacity += 20.0;
                self.player.fuel = self.player.fuel_capacity;
            }
            LegendaryBlueprint::RelicSorter => {
                self.rig_part_inventory.insert(RigPartKind::RelicScanner);
                self.rig_part_inventory.insert(RigPartKind::CargoBalloon);
                self.equipped_rig_parts
                    .insert(RigSlot::ScannerModule, RigPartKind::RelicScanner);
                self.equipped_rig_parts
                    .insert(RigSlot::CargoModule, RigPartKind::CargoBalloon);
                self.player.cargo_capacity = self.player.cargo_capacity.saturating_add(4);
            }
        }
        self.message = format!(
            "Legendary blueprint recovered: {} special rig part installed.",
            blueprint.title()
        );
        self.sound_cues.push(SoundCue::Milestone);
    }

    fn update_escape_sequence(&mut self, delta_seconds: f32) {
        if self.escape_sequence_seconds <= 0.0 || self.won_game {
            return;
        }
        self.escape_sequence_seconds = (self.escape_sequence_seconds - delta_seconds).max(0.0);
        if self.current_zone == Some(SurfaceZone::Headquarters) {
            self.escape_sequence_seconds = 0.0;
            return;
        }
        if self.update_ticks.is_multiple_of(45) {
            self.spawn_cave_in();
            self.shake_camera(
                0.25,
                6.0 + (120.0 - self.escape_sequence_seconds).max(0.0) * 0.05,
            );
        }
        if self.update_ticks.is_multiple_of(90) {
            self.seal_escape_tunnel();
        } else if self.update_ticks % 90 == 75 {
            self.warn_escape_tunnel_collapse();
        }
        if self.update_ticks.is_multiple_of(120) {
            "CORE CASCADE: tunnels are sealing. Climb now!".clone_into(&mut self.message);
        }
        if self.escape_sequence_seconds == 0.0 {
            self.player.hull = 0.0;
            self.game_over = true;
            "The mine collapsed around the Star Core. Emergency rescue required."
                .clone_into(&mut self.message);
        }
    }

    fn warn_escape_tunnel_collapse(&mut self) {
        self.collapse_warnings.clear();
        let px = (self.player.x / TILE_SIZE).floor() as i32;
        let py = (self.player.y / TILE_SIZE).floor() as i32 + 5;
        for dx in -2..=2 {
            let position = TilePosition { x: px + dx, y: py };
            if position.y > 7
                && !self.terrain.is_solid_at(position)
                && !self.is_tunnel_supported(position)
            {
                self.collapse_warnings.push(position);
            }
        }
        if !self.collapse_warnings.is_empty() {
            "Ceiling stress warning: marked tunnel will seal next.".clone_into(&mut self.message);
        }
    }

    fn seal_escape_tunnel(&mut self) {
        let px = (self.player.x / TILE_SIZE).floor() as i32;
        let py = (self.player.y / TILE_SIZE).floor() as i32 + 5;
        for dx in -2..=2 {
            let position = TilePosition { x: px + dx, y: py };
            if position.y <= 7
                || self.terrain.is_solid_at(position)
                || self.is_tunnel_supported(position)
            {
                continue;
            }
            if self.terrain.set_kind(position, TileKind::Stone) {
                self.mark_tile_visual_changed(position);
                self.scan_markers
                    .retain(|marker| marker.position != position);
            }
        }
        self.collapse_warnings.clear();
    }

    fn update_layer_band(&mut self) {
        let band = self.deepest_tile_reached / 20;
        if band <= self.current_layer_band {
            return;
        }
        self.current_layer_band = band;
        self.collection_log.strata.insert(band);
        let layer = match band {
            1 => "Clay Belt",
            2 => "Silver Caverns",
            3 => "Thermal Strata",
            _ => "Core Fracture Zone",
        };
        self.message = format!("Entering {layer}. Hazards and ore density increased.");
    }

    fn award_challenge_badges(&mut self, cargo_value: u32, risk_rating: &str) {
        let mut unlocked = Vec::new();
        if self.trip_best_depth >= 100 && self.challenge_badges.insert(ChallengeBadge::DeepReturn) {
            self.cosmetic_skins.insert(CosmeticRigSkin::BronzeTrim);
            unlocked.push(ChallengeBadge::DeepReturn.title());
        }
        if matches!(risk_rating, "High" | "Extreme")
            && self.challenge_badges.insert(ChallengeBadge::HighRiskReturn)
        {
            self.cosmetic_skins.insert(CosmeticRigSkin::HazardStripes);
            unlocked.push(ChallengeBadge::HighRiskReturn.title());
        }
        if cargo_value >= 1_500 && self.challenge_badges.insert(ChallengeBadge::ValuableHaul) {
            self.cosmetic_skins.insert(CosmeticRigSkin::StarChrome);
            unlocked.push(ChallengeBadge::ValuableHaul.title());
        }
        if !unlocked.is_empty() {
            self.last_run_summary.push_str(" Badges: ");
            self.last_run_summary.push_str(&unlocked.join(", "));
            self.last_run_summary.push('.');
            self.message = self.last_run_summary.clone();
            self.sound_cues.push(SoundCue::Milestone);
        }
    }

    fn current_cargo_value(&self) -> u32 {
        let minerals = self
            .player
            .cargo
            .iter()
            .map(|(mineral, count)| self.mineral_market_value(*mineral).saturating_mul(*count))
            .sum::<u32>();
        let artifacts = self
            .player
            .artifacts
            .iter()
            .map(|(artifact, count)| artifact.value().saturating_mul(*count))
            .sum::<u32>();
        minerals.saturating_add(artifacts)
    }

    fn award_return_bonus(&mut self) {
        if self.current_zone.is_none() || self.trip_best_depth < 15 {
            return;
        }
        self.return_streak += 1;
        if self.player.loan_debt > 0 {
            let risk_interest = 12 + self.player.loan_debt / 200;
            self.player.loan_debt = self.player.loan_debt.saturating_add(risk_interest);
        }
        let depth = u32::try_from(self.trip_best_depth).unwrap_or(0);
        let cargo_value = self.current_cargo_value();
        self.best_return_depth = self.best_return_depth.max(self.trip_best_depth);
        self.most_valuable_cargo_run = self.most_valuable_cargo_run.max(cargo_value);
        let reward = (depth / 4).saturating_mul(self.return_streak.min(5));
        self.player.credits += reward;
        self.total_earnings += reward;
        let risk_rating = if self.deep_instability >= 75.0 {
            "Extreme"
        } else if self.deep_instability >= 40.0 {
            "High"
        } else if self.trip_best_depth >= 80 {
            "Moderate"
        } else {
            "Low"
        };
        self.last_run_summary = format!(
            "Run summary: depth {}m | return bonus {reward} | cargo value {cargo_value} | hull damage {:.0}% | discoveries {} | risk {risk_rating} | streak x{}.",
            self.trip_best_depth,
            (1.0 - self.player.hull / self.player.max_hull()).max(0.0) * 100.0,
            self.collection_log.minerals.len() + self.collection_log.artifacts.len(),
            self.return_streak
        );
        self.message = self.last_run_summary.clone();
        self.award_challenge_badges(cargo_value, risk_rating);
        self.trip_best_depth = 0;
        self.trip_seconds = 0.0;
        self.deep_instability = 0.0;
        self.deep_reward_milestone = 0;
        self.advance_town_event();
    }

    fn tick_world_events(&mut self) {
        for event in &mut self.active_world_events {
            event.days_remaining = event.days_remaining.saturating_sub(1);
        }
        self.active_world_events
            .retain(|event| event.days_remaining > 0);
    }

    fn schedule_world_event(&mut self) {
        if !self.town_event_day.is_multiple_of(3) {
            return;
        }
        let kind = match (self.market_salt + self.town_event_day) % 13 {
            0 => WorldEventKind::MarketCrash,
            1 => WorldEventKind::MarketBoom,
            2 => WorldEventKind::RareBuyer,
            3 => WorldEventKind::FuelShortage,
            4 => WorldEventKind::RepairBacklog,
            5 => WorldEventKind::HeatWave,
            6 => WorldEventKind::CollapseSurge,
            7 => WorldEventKind::DeepPressureStorm,
            8 => WorldEventKind::GasBloom,
            9 => WorldEventKind::Earthquake,
            10 => WorldEventKind::MeteorShower,
            11 => WorldEventKind::RivalClaims,
            _ => WorldEventKind::AncientMachine,
        };
        if self.has_world_event(kind) {
            return;
        }
        let severity = 1 + (self.town_event_day + self.market_salt) % 3;
        let days_remaining = 2 + severity.min(2);
        self.active_world_events.push(ActiveWorldEvent {
            kind,
            days_remaining,
            severity,
        });
        self.apply_world_event_start(kind, severity);
        self.message = format!(
            "World event: {} severity {severity} for {days_remaining} days.",
            kind.label()
        );
    }

    fn apply_world_event_start(&mut self, kind: WorldEventKind, severity: u32) {
        match kind {
            WorldEventKind::Earthquake => {
                self.spawn_cave_in();
                self.apply_seismic_pump_strain();
                self.migrate_ore_veins(severity);
                self.shake_camera(0.6, 12.0 + severity as f32 * 4.0);
            }
            WorldEventKind::GasBloom => {
                self.hazard_clouds.push(HazardCloud {
                    x: self.player.x,
                    y: self.player.y + TILE_SIZE,
                    life: 8.0 + severity as f32,
                    radius: 12.0 + severity as f32 * 2.0,
                });
            }
            WorldEventKind::MeteorShower => {
                self.start_side_contract();
                self.contracts.active.reward = self.contracts.active.reward.saturating_add(75);
            }
            WorldEventKind::RivalClaims => {
                self.expedition_offers.clear();
                self.refresh_expedition_offers();
            }
            WorldEventKind::AncientMachine => self.awaken_ancient_machine(severity),
            WorldEventKind::MarketCrash
            | WorldEventKind::MarketBoom
            | WorldEventKind::RareBuyer
            | WorldEventKind::FuelShortage
            | WorldEventKind::RepairBacklog
            | WorldEventKind::HeatWave
            | WorldEventKind::CollapseSurge
            | WorldEventKind::DeepPressureStorm => {}
        }
    }

    fn migrate_ore_veins(&mut self, severity: u32) {
        let center_x = (self.player.x / TILE_SIZE).floor() as i32;
        let base_y = (self.deepest_tile_reached + 8).clamp(12, self.terrain.height() - 2);
        let minerals = [MineralKind::Gold, MineralKind::Ruby, MineralKind::Platinum];
        let mut changed = 0_u32;
        for index in 0..(3 + severity) {
            let position = TilePosition {
                x: (center_x + i32::try_from(index).unwrap_or(0) * 3 - 5)
                    .clamp(1, self.terrain.width() - 2),
                y: (base_y + i32::try_from(index % 3).unwrap_or(0) - 1)
                    .clamp(8, self.terrain.height() - 2),
            };
            if !self.terrain.is_solid_at(position) {
                continue;
            }
            let mineral = minerals[usize::try_from(index).unwrap_or(0) % minerals.len()];
            if self.terrain.set_kind(position, TileKind::Ore(mineral)) {
                self.mark_tile_visual_changed(position);
                changed += 1;
            }
        }
        if changed > 0 {
            self.message = format!("Earthquake exposed {changed} shifted ore vein(s).");
        }
    }

    fn awaken_ancient_machine(&mut self, severity: u32) {
        self.player
            .add_material(StrategicResourceKind::CoreShard, severity.max(1));
        self.scanner_cooldown_seconds = self.scanner_cooldown_seconds.max(6.0);
        self.sound_cues.push(SoundCue::Milestone);
        self.shake_camera(0.35, 8.0 + severity as f32 * 2.0);
        self.message = format!(
            "Ancient machine awakened: recovered {} Core Shard(s), scanner interference rising.",
            severity.max(1)
        );
    }

    fn advance_town_event(&mut self) {
        self.town_event_day = self.town_event_day.saturating_add(1);
        self.tick_world_events();
        self.schedule_world_event();
        self.town_event = match self.town_event_day % 5 {
            0 => "Fuel sale: mechanics whisper about cheaper surface fuel.".to_owned(),
            1 => "Gold boom: depot buyers are bidding aggressively.".to_owned(),
            2 => "Repair backlog: Iona says don't dent anything expensive today.".to_owned(),
            3 => "Cave instability warning: HQ predicts more falling rock.".to_owned(),
            _ => "Explosive Shack overstock: Nix is pushing bomb bundles.".to_owned(),
        };
        self.market_history
            .push(market_factor(self.market_salt, self.town_event_day));
        if self.market_history.len() > 7 {
            self.market_history.remove(0);
        }
        if self.town_development.bank_level > 0 {
            let dividend = u32::from(self.town_development.bank_level) * 12;
            self.player.credits = self.player.credits.saturating_add(dividend);
            self.total_earnings = self.total_earnings.saturating_add(dividend);
            if self.player.loan_debt > 0 {
                self.message = format!(
                    "Bank office dividend: {dividend} credits. Debt remaining: {}.",
                    self.player.loan_debt
                );
            }
        }
        self.apply_infrastructure_maintenance();
        self.apply_seismic_pump_strain();
        for mineral in all_minerals() {
            let history = self.mineral_market_history.entry(mineral).or_default();
            history.push(market_factor_for(
                self.market_salt,
                self.town_event_day,
                mineral,
            ));
            if history.len() > 7 {
                history.remove(0);
            }
        }
    }

    fn apply_seismic_pump_strain(&mut self) {
        if self.town_event_day % 5 != 3 {
            return;
        }
        let mut damaged = 0_u32;
        let mut failed = 0_u32;
        for item in &mut self.infrastructure {
            if item.kind != InfrastructureKind::PumpStation {
                continue;
            }
            damaged += 1;
            let before = item.durability;
            item.durability = item.durability.saturating_sub(35);
            if before > 0 && item.durability == 0 {
                failed += 1;
            }
        }
        if damaged == 0 {
            return;
        }
        self.infrastructure.retain(|item| item.durability > 0);
        self.message = if failed == 0 {
            format!("Seismic tremor strained {damaged} pump station(s).")
        } else {
            format!("Seismic tremor strained {damaged} pump station(s); {failed} failed.")
        };
    }

    fn apply_infrastructure_maintenance(&mut self) {
        let cost = self.infrastructure.len() as u32 * 3;
        if cost == 0 {
            return;
        }
        let paid = self.player.credits.min(cost);
        self.player.credits -= paid;
        if paid < cost {
            let lost = self
                .infrastructure
                .len()
                .saturating_sub((paid / 3) as usize);
            for _ in 0..lost {
                self.infrastructure.pop();
            }
            self.message = format!(
                "Infrastructure maintenance shortfall: paid {paid}/{cost} credits, {lost} relays failed."
            );
        } else {
            self.message = format!("Paid {cost} credits to maintain signal relay network.");
        }
        self.apply_infrastructure_wear();
    }

    fn apply_infrastructure_wear(&mut self) {
        let hazard_positions = self
            .infrastructure
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let near_hazard = (-2..=2).any(|dy| {
                    (-2..=2).any(|dx| {
                        self.terrain
                            .tile(TilePosition {
                                x: item.position.x + dx,
                                y: item.position.y + dy,
                            })
                            .is_some_and(|tile| {
                                matches!(
                                    tile.kind,
                                    TileKind::Gas
                                        | TileKind::Lava
                                        | TileKind::MagmaVent
                                        | TileKind::ExplosivePocket
                                        | TileKind::PressurePocket
                                )
                            })
                    })
                });
                near_hazard.then_some(index)
            })
            .collect::<Vec<_>>();
        for index in hazard_positions {
            if let Some(item) = self.infrastructure.get_mut(index) {
                item.durability = item.durability.saturating_sub(20);
            }
        }
        let before = self.infrastructure.len();
        self.infrastructure.retain(|item| item.durability > 0);
        let lost = before.saturating_sub(self.infrastructure.len());
        if lost > 0 {
            self.message = format!("{lost} infrastructure unit(s) failed in dangerous ground.");
        }
    }

    fn update_status_messages(&mut self) {
        if self.message.starts_with("Warning:") || self.message.starts_with("CRITICAL:") {
            return;
        }
        if self.player.fuel <= self.player.fuel_capacity * 0.15 && self.player.y > 6.0 * TILE_SIZE {
            "CRITICAL: fuel reserve low. Return to the fuel station now."
                .clone_into(&mut self.message);
            return;
        }
        if self.player.cargo_used() >= self.player.cargo_capacity {
            "Warning: cargo hold full. Return to the depot or leave valuables behind."
                .clone_into(&mut self.message);
            return;
        }
        if let Some(zone) = self.current_zone {
            self.message = match zone {
                SurfaceZone::Fuel => {
                    "Fuel Station: press E to buy fuel (1 credit/unit).".to_owned()
                }
                SurfaceZone::Repair => {
                    "Repair Garage: press E to repair hull (2 credits/unit).".to_owned()
                }
                SurfaceZone::Depot => {
                    "Ore Depot: press E to sell cargo or review receipts.".to_owned()
                }
                SurfaceZone::Headquarters => depot_prompt(self),
                SurfaceZone::Shop => shop_prompt(&self.player),
                SurfaceZone::Bank => "Bank: press E for loan/debt service.".to_owned(),
                SurfaceZone::Explosives => {
                    "Explosive Shack: press E to buy 3 timed charges for 55 credits.".to_owned()
                }
                SurfaceZone::Salvage => {
                    "Salvage Yard: press E for cargo beacon or hull patch.".to_owned()
                }
            };
        }
    }

    fn check_failure(&mut self) {
        if self.player.hull <= 0.0 {
            self.game_over = true;
            "Hull destroyed! Press E for emergency rescue.".clone_into(&mut self.message);
        } else if self.player.fuel <= 0.0 && self.player.y > 6.0 * TILE_SIZE {
            self.game_over = true;
            "Out of fuel underground! Press E for emergency rescue.".clone_into(&mut self.message);
        }
    }

    #[must_use]
    pub fn signal_relay_count(&self) -> usize {
        self.infrastructure
            .iter()
            .filter(|item| item.kind == InfrastructureKind::SignalRelay)
            .count()
    }

    #[must_use]
    pub fn is_tunnel_supported(&self, position: TilePosition) -> bool {
        self.infrastructure.iter().any(|item| {
            item.kind == InfrastructureKind::TunnelSupport
                && (item.position.x - position.x).abs() <= 3
                && (item.position.y - position.y).abs() <= 3
        })
    }

    #[must_use]
    pub fn is_pump_protected(&self, position: TilePosition) -> bool {
        self.infrastructure.iter().any(|item| {
            item.kind == InfrastructureKind::PumpStation
                && (item.position.x - position.x).abs() <= 4
                && (item.position.y - position.y).abs() <= 4
        })
    }

    fn recover_lost_cargo_if_near(&mut self) {
        let (Some(x), Some(y)) = (self.lost_cargo_x, self.lost_cargo_y) else {
            return;
        };
        if self.lost_cargo_count == 0 {
            return;
        }
        let dx = self.player.x - x;
        let dy = self.player.y - y;
        if dx.hypot(dy) > TILE_SIZE * 0.9 || !self.player.has_cargo_space() {
            return;
        }
        let recovered =
            (self.player.cargo_capacity - self.player.cargo_used()).min(self.lost_cargo_count);
        if recovered == 0 {
            return;
        }
        *self
            .player
            .cargo
            .entry(crate::terrain::MineralKind::Iron)
            .or_default() += recovered;
        self.lost_cargo_count -= recovered;
        self.message = format!("Recovered {recovered} lost cargo crates from rescue site.");
        if self.lost_cargo_count == 0 {
            self.lost_cargo_x = None;
            self.lost_cargo_y = None;
        }
    }

    fn handle_rescue(&mut self, input: PlayerInput) {
        if !input.interact {
            return;
        }

        let base_fee = rescue_fee(self.player.y);
        let relay_count = self.signal_relay_count() as u32;
        let relay_discount_percent = relay_count.saturating_mul(10).min(50);
        let relayed_fee = base_fee.saturating_mul(100 - relay_discount_percent) / 100;
        let fee_divisor = if self.player.insured {
            u32::from(self.player.insurance_tier).saturating_add(1)
        } else {
            1
        };
        let fee = (relayed_fee / fee_divisor).min(self.player.credits);
        self.player.credits -= fee;
        self.rescue_count += 1;
        let before_minerals = self.player.cargo.clone();
        let before_artifacts = self.player.artifacts.clone();
        let armored_cargo = self.has_equipped_part(RigPartKind::ArmoredCargoBay);
        let mut lost_items = if self.player.insured && self.player.insurance_tier >= 2 {
            0
        } else if self.player.insured || armored_cargo {
            drop_quarter_cargo(&mut self.player)
        } else {
            drop_half_cargo(&mut self.player)
        };
        if self.player.y >= 70.0 * TILE_SIZE
            && relay_count == 0
            && (!self.player.insured || self.player.insurance_tier < 4)
        {
            lost_items = lost_items.saturating_add(drop_quarter_cargo(&mut self.player));
        }
        self.player.insured = false;
        self.last_rescue_x = Some(self.player.x);
        self.last_rescue_y = Some(self.player.y);
        if lost_items > 0 {
            self.lost_cargo_x = Some(self.player.x);
            self.lost_cargo_y = Some(self.player.y);
            self.lost_cargo_count = lost_items;
            self.lost_minerals = cargo_difference(&before_minerals, &self.player.cargo);
            self.lost_artifacts = cargo_difference(&before_artifacts, &self.player.artifacts);
        }
        self.last_rescue_summary =
            format!("Fee: {fee} credits. Cargo lost: {lost_items}. Relays online: {relay_count}.");
        self.depot_receipts.push(format!(
            "RESCUE INVOICE\nDepth: {}m\nFee: {fee} cr\nRelay discount: {relay_discount_percent}%\nCargo lost: {lost_items}",
            (self.player.y / TILE_SIZE).floor() as i32
        ));
        if self.depot_receipts.len() > 5 {
            self.depot_receipts.remove(0);
        }
        self.player.x = 12.0 * TILE_SIZE;
        self.player.y = 4.0 * TILE_SIZE;
        self.player.velocity_x = 0.0;
        self.player.velocity_y = 0.0;
        self.player.fuel = self.player.fuel_capacity * 0.5;
        self.player.hull = self.player.max_hull() * 0.5;
        self.game_over = false;
        self.sound_cues.push(SoundCue::Rescue);
        self.message = format!("Emergency rescue completed. {}", self.last_rescue_summary);
    }
    pub const fn take_settings_dirty(&mut self) -> bool {
        let dirty = self.settings_dirty;
        self.settings_dirty = false;
        dirty
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

fn hq_story_message(game: &GameState) -> String {
    if game.won_game {
        return "Director Vale: The Star Core is secure. Deep Claim operations are open: build relays, chase expeditions, catalogue strata, and push for legendary blueprints.".to_owned();
    }
    match game.deepest_tile_reached {
        0..=19 => "Director Vale: Bring us contract cargo and prove this shaft is profitable.".to_owned(),
        20..=39 => "Mechanic Iona: Silver strata ahead. Upgrade before chasing deep seams.".to_owned(),
        40..=59 => "Surveyor Kade: Relic signals are stronger. Gas pockets are no longer rumors.".to_owned(),
        60..=79 => "Director Vale: Thermal readings are ugly. Radiators and hull plating are survival gear.".to_owned(),
        _ => "Surveyor Kade: Star Core harmonics are below. Expect vents, blasts, and cave-ins.".to_owned(),
    }
}

fn rescue_fee(player_y: f32) -> u32 {
    50 + ((player_y / TILE_SIZE).max(0.0) as u32 * 3)
}

fn cargo_difference<K: Copy + Ord>(
    before: &std::collections::BTreeMap<K, u32>,
    after: &std::collections::BTreeMap<K, u32>,
) -> std::collections::BTreeMap<K, u32> {
    let mut difference = std::collections::BTreeMap::new();
    for (key, before_count) in before {
        let after_count = after.get(key).copied().unwrap_or(0);
        if *before_count > after_count {
            difference.insert(*key, *before_count - after_count);
        }
    }
    difference
}

fn drop_half_cargo(player: &mut Player) -> u32 {
    let mut lost = 0;
    for count in player.cargo.values_mut() {
        let dropped = (*count).div_ceil(2);
        *count -= dropped;
        lost += dropped;
    }
    player.cargo.retain(|_, count| *count > 0);

    for count in player.artifacts.values_mut() {
        let dropped = (*count).div_ceil(2);
        *count -= dropped;
        lost += dropped;
    }
    player.artifacts.retain(|_, count| *count > 0);
    lost
}

fn drop_quarter_cargo(player: &mut Player) -> u32 {
    let mut lost = 0;
    for count in player.cargo.values_mut() {
        let dropped = (*count).div_ceil(4);
        *count -= dropped;
        lost += dropped;
    }
    player.cargo.retain(|_, count| *count > 0);

    for count in player.artifacts.values_mut() {
        let dropped = (*count).div_ceil(4);
        *count -= dropped;
        lost += dropped;
    }
    player.artifacts.retain(|_, count| *count > 0);
    lost
}

fn drill_seconds_per_chip(kind: TileKind, drill_strength: u8, direction: DrillDirection) -> f32 {
    let base = match kind {
        TileKind::Air => 0.0,
        TileKind::Dirt => 0.09,
        TileKind::Clay => 0.12,
        TileKind::Stone => 0.15,
        TileKind::HardRock | TileKind::Foundation => 0.19,
        TileKind::Lava
        | TileKind::Gas
        | TileKind::ExplosivePocket
        | TileKind::PressurePocket
        | TileKind::MagmaVent => 0.08,
        TileKind::Ore(_) => 0.16,
        TileKind::Artifact(_) => 0.21,
    };
    let drill_bonus = 1.0 + f32::from(drill_strength.saturating_sub(1)) * 0.4;
    let direction_penalty = if direction == DrillDirection::Down {
        1.0
    } else {
        1.45
    };
    (base * direction_penalty / drill_bonus).max(0.045)
}

fn mine_target(player: &Player, input: PlayerInput) -> Option<(TilePosition, DrillDirection)> {
    if !input.drill_down && input.horizontal == 0.0 {
        return None;
    }

    let current_tile = player.tile_position(TILE_SIZE);
    Some(if input.drill_down {
        (
            TilePosition {
                x: current_tile.x,
                y: current_tile.y + 1,
            },
            DrillDirection::Down,
        )
    } else {
        let facing = facing_direction(input.horizontal);
        (
            TilePosition {
                x: current_tile.x + facing,
                y: current_tile.y,
            },
            if facing < 0 {
                DrillDirection::Left
            } else {
                DrillDirection::Right
            },
        )
    })
}

const fn deep_claim_material_for(
    mineral: MineralKind,
    position: TilePosition,
) -> Option<StrategicResourceKind> {
    let roll = (position.x.unsigned_abs() + position.y as u32 + mineral.value()).wrapping_rem(6);
    match (mineral, roll) {
        (MineralKind::Mythril | MineralKind::Uranium, 0 | 1) => {
            Some(StrategicResourceKind::CoreShard)
        }
        (MineralKind::Diamond | MineralKind::Platinum, 0) => {
            Some(StrategicResourceKind::CrystalLens)
        }
        (MineralKind::Ruby | MineralKind::Emerald | MineralKind::Gold, 0) => {
            Some(StrategicResourceKind::AncientAlloy)
        }
        _ => None,
    }
}

fn consume_expedition_delivery(expedition: Expedition, player: &mut Player) {
    if expedition.kind != ExpeditionObjectiveKind::DeliverCargo {
        return;
    }
    match expedition.target {
        TileKind::Ore(mineral) => {
            if let Some(count) = player.cargo.get_mut(&mineral) {
                *count = count.saturating_sub(expedition.required);
            }
            player.cargo.retain(|_, count| *count > 0);
        }
        TileKind::Artifact(artifact) => {
            if let Some(count) = player.artifacts.get_mut(&artifact) {
                *count = count.saturating_sub(expedition.required);
            }
            player.artifacts.retain(|_, count| *count > 0);
        }
        _ => {}
    }
}

fn expedition_satisfied(expedition: Expedition, game: &GameState) -> bool {
    match expedition.kind {
        ExpeditionObjectiveKind::ReachDepth => {
            game.deepest_tile_reached as u32 >= expedition.required
        }
        ExpeditionObjectiveKind::DeliverCargo => match expedition.target {
            TileKind::Ore(mineral) => {
                game.player.cargo.get(&mineral).copied().unwrap_or(0) >= expedition.required
            }
            TileKind::Artifact(artifact) => {
                game.player.artifacts.get(&artifact).copied().unwrap_or(0) >= expedition.required
            }
            _ => false,
        },
        ExpeditionObjectiveKind::ScanHazards => {
            game.scan_markers
                .iter()
                .filter(|marker| marker.kind == expedition.target)
                .count()
                >= expedition.required as usize
        }
        ExpeditionObjectiveKind::BuildPumpStations => {
            game.infrastructure
                .iter()
                .filter(|item| item.kind == InfrastructureKind::PumpStation)
                .count()
                >= expedition.required as usize
        }
        ExpeditionObjectiveKind::RecoverProbe
        | ExpeditionObjectiveKind::MineVein
        | ExpeditionObjectiveKind::ScanAnomaly
        | ExpeditionObjectiveKind::RescueMiner
        | ExpeditionObjectiveKind::DeliverExplosives
        | ExpeditionObjectiveKind::StabilizeCollapse
        | ExpeditionObjectiveKind::RetrieveArtifact
        | ExpeditionObjectiveKind::ReachSignal
        | ExpeditionObjectiveKind::NoDamageReturn
        | ExpeditionObjectiveKind::FastReturn => {
            game.expedition_progress(expedition) >= expedition.required
        }
    }
}

fn side_contract_satisfied(contract: SideContract, game: &GameState) -> bool {
    if contract
        .expires_day
        .is_some_and(|expires_day| game.town_event_day > expires_day)
    {
        return false;
    }
    match contract.kind {
        SideContractKind::Cargo | SideContractKind::Rush => match contract.target {
            TileKind::Ore(mineral) => {
                game.player.cargo.get(&mineral).copied().unwrap_or(0) >= contract.required
            }
            TileKind::Artifact(artifact) => {
                game.player.artifacts.get(&artifact).copied().unwrap_or(0) >= contract.required
            }
            _ => false,
        },
        SideContractKind::DepthSurvey => {
            u32::try_from(game.deepest_tile_reached).unwrap_or(0) >= contract.required
        }
        SideContractKind::HazardScan => {
            game.scan_markers
                .iter()
                .filter(|marker| {
                    matches!(
                        marker.kind,
                        TileKind::Gas
                            | TileKind::Lava
                            | TileKind::MagmaVent
                            | TileKind::ExplosivePocket
                            | TileKind::PressurePocket
                    )
                })
                .count()
                >= usize::try_from(contract.required).unwrap_or(usize::MAX)
        }
    }
}

fn consume_side_contract_cargo(contract: SideContract, player: &mut Player) {
    match contract.target {
        TileKind::Ore(mineral) => {
            consume_side_count(&mut player.cargo, &mineral, contract.required);
        }
        TileKind::Artifact(artifact) => {
            consume_side_count(&mut player.artifacts, &artifact, contract.required);
        }
        _ => {}
    }
}

fn consume_side_count<K: Ord>(items: &mut std::collections::BTreeMap<K, u32>, key: &K, count: u32) {
    let Some(available) = items.get_mut(key) else {
        return;
    };
    *available = available.saturating_sub(count);
    if *available == 0 {
        items.remove(key);
    }
}

const fn scanner_can_mark(kind: TileKind, scanner_level: u8) -> bool {
    match kind {
        TileKind::Ore(_) => scanner_level >= 1,
        TileKind::Gas
        | TileKind::Lava
        | TileKind::MagmaVent
        | TileKind::ExplosivePocket
        | TileKind::PressurePocket => scanner_level >= 2,
        TileKind::Artifact(_) => scanner_level >= 3,
        _ => false,
    }
}

const fn current_save_version() -> u32 {
    2
}

const fn all_minerals() -> [MineralKind; 10] {
    [
        MineralKind::Copper,
        MineralKind::Iron,
        MineralKind::Silver,
        MineralKind::Gold,
        MineralKind::Emerald,
        MineralKind::Ruby,
        MineralKind::Diamond,
        MineralKind::Platinum,
        MineralKind::Uranium,
        MineralKind::Mythril,
    ]
}

fn initial_mineral_market_history(
    salt: u32,
    event_day: u32,
) -> std::collections::BTreeMap<MineralKind, Vec<u32>> {
    all_minerals()
        .into_iter()
        .map(|mineral| (mineral, vec![market_factor_for(salt, event_day, mineral)]))
        .collect()
}

const fn market_factor_for(salt: u32, event_day: u32, mineral: MineralKind) -> u32 {
    let mineral_salt = salt.wrapping_add(mineral.value().wrapping_mul(13));
    let base = market_factor(mineral_salt, event_day);
    match (event_day % 5, mineral) {
        (1, MineralKind::Gold | MineralKind::Silver | MineralKind::Platinum) => base + 16,
        (0, MineralKind::Copper | MineralKind::Iron) => base + 8,
        (4, MineralKind::Ruby | MineralKind::Emerald | MineralKind::Diamond) => base + 10,
        _ => base,
    }
}

const fn market_factor(salt: u32, event_day: u32) -> u32 {
    let base = 85 + salt.wrapping_mul(37).wrapping_add(11) % 41;
    if event_day % 5 == 1 { base + 20 } else { base }
}

const fn surface_zone_label(zone: SurfaceZone) -> &'static str {
    match zone {
        SurfaceZone::Fuel => "Fuel Station",
        SurfaceZone::Repair => "Repair Garage",
        SurfaceZone::Depot => "Ore Depot",
        SurfaceZone::Headquarters => "HQ",
        SurfaceZone::Shop => "Upgrade Shop",
        SurfaceZone::Bank => "Bank",
        SurfaceZone::Explosives => "Explosive Shack",
        SurfaceZone::Salvage => "Salvage Yard",
    }
}

const fn interior_service_x(zone: SurfaceZone) -> f32 {
    match zone {
        SurfaceZone::Fuel => 430.0,
        SurfaceZone::Repair => 405.0,
        SurfaceZone::Depot => 455.0,
        SurfaceZone::Headquarters => 390.0,
        SurfaceZone::Shop => 450.0,
        SurfaceZone::Bank => 380.0,
        SurfaceZone::Explosives => 431.0,
        SurfaceZone::Salvage => 410.0,
    }
}

fn surface_zone_at(x: f32, y: f32) -> Option<SurfaceZone> {
    if y > 5.5 * TILE_SIZE {
        return None;
    }

    surface_building_at_tile((x / TILE_SIZE).floor() as i32).map(|building| building.zone)
}

fn depot_prompt(game: &GameState) -> String {
    let contract = &game.contracts.active;
    format!(
        "Depot: E completes contract ({}/{}) {} for {} cr, otherwise sells cargo.",
        contract.progress(&game.player),
        contract.required,
        contract.target.name(),
        contract.reward
    )
}

fn shop_prompt(player: &Player) -> String {
    let offers = upgrade_offers(player);
    let mut prompt = String::from("Upgrade Shop: ");
    for (index, offer) in offers.iter().enumerate() {
        let label = if offer.level >= crate::economy::MAX_UPGRADE_LEVEL {
            "MAX".to_owned()
        } else {
            offer.cost.to_string()
        };
        let _ = write!(prompt, "{}:{}({label}) ", index + 1, offer.name);
    }
    prompt
}

const fn falling_rock_roll(position: TilePosition, seed: u64) -> bool {
    let value = seed
        ^ ((position.x as u64).wrapping_mul(0x9E37))
        ^ ((position.y as u64).wrapping_mul(0x85EB));
    value.is_multiple_of(BOULDER_SPAWN_CHANCE)
}

const fn initial_camera_x() -> f32 {
    PLAYER_SPAWN_X - 1280.0 / 2.0
}

const fn initial_camera_y() -> f32 {
    PLAYER_SPAWN_Y - 720.0 / 2.0 - CAMERA_INTRO_DROP_DISTANCE
}

fn target_camera_offset(game: &GameState) -> (f32, f32) {
    let screen_width = 1280.0;
    let screen_height = 720.0;
    let max_x = game.terrain.width() as f32 * TILE_SIZE - screen_width;
    let max_y = game.terrain.height() as f32 * TILE_SIZE - screen_height;

    (
        (game.player.x - screen_width / 2.0).clamp(0.0, max_x),
        (game.player.y - screen_height / 2.0).clamp(MIN_PLAYER_Y, max_y),
    )
}

fn collision_points(x: f32, y: f32) -> [TilePosition; 4] {
    [
        point_to_tile(x - PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y - PLAYER_RADIUS),
        point_to_tile(x - PLAYER_RADIUS, y + PLAYER_RADIUS),
        point_to_tile(x + PLAYER_RADIUS, y + PLAYER_RADIUS),
    ]
}

fn point_to_tile(x: f32, y: f32) -> TilePosition {
    TilePosition {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

fn facing_direction(horizontal: f32) -> i32 {
    if horizontal < 0.0 { -1 } else { 1 }
}

fn input_changes_game(input: PlayerInput) -> bool {
    input.horizontal.abs() > f32::EPSILON
        || input.thrust
        || input.drill_down
        || input.interact
        || input.confirm
        || input.bomb
        || input.scan
        || input.place_relay
        || input.place_drone
        || input.place_lift
        || input.place_support
        || input.place_pump
        || input.place_processor
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_menu_exposes_local_split_screen_entrypoint() {
        assert!(GameState::title_options().contains(&TitleOption::LocalMultiplayer));
    }

    #[test]
    fn selecting_local_split_screen_starts_game_and_requests_session_activation() {
        let mut game = GameState::new();
        let options = GameState::title_options();
        game.selected_title_item = options
            .iter()
            .position(|option| *option == TitleOption::LocalMultiplayer)
            .expect("local split-screen option exists");

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );

        assert_eq!(game.run_mode, RunMode::Playing);
        assert!(game.take_local_multiplayer_request());
        assert!(game.message.contains("Player 2"));
    }

    #[test]
    fn joined_online_client_cannot_write_local_saves() {
        let mut game = GameState::new();
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: false,
            player_slot: Some(2),
            status_message: "Joined host-owned online session.".to_owned(),
        });

        assert!(!game.can_write_local_save());

        game.save_dirty = true;
        game.handle_save_load(PlayerInput {
            save: true,
            ..PlayerInput::default()
        });
        assert!(game.save_dirty);
        assert!(game.message.contains("host owns"));

        game.save_slot(0);
        assert!(matches!(game.modal, Some(ModalScreen::SaveSlots)));
        assert!(game.message.contains("host owns"));

        game.modal = Some(ModalScreen::UnsavedExitConfirm);
        game.selected_menu_item = 0;
        game.request_exit = false;
        assert!(game.handle_exit_modal(PlayerInput {
            confirm: true,
            ..PlayerInput::default()
        }));
        assert!(!game.request_exit);
        assert!(game.message.contains("host owns"));
    }

    #[test]
    fn host_owned_online_session_can_write_local_save_policy() {
        let mut game = GameState::new();
        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot: Some(1),
            status_message: "Hosting online session.".to_owned(),
        });

        assert!(game.can_write_local_save());
        assert!(!game.block_joined_client_save());
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn descriptor_host_and_client_complete_join_handshake() {
        let unique_path = std::env::temp_dir().join(format!(
            "drillgame-descriptor-handshake-{}.json",
            std::process::id()
        ));
        let _ignored = std::fs::remove_file(&unique_path);
        let mut host_game = GameState::new();
        host_game.online_host_bind_addr = "127.0.0.1:0".parse().expect("bind addr parses");
        host_game.online_host_advertise_addr =
            "127.0.0.1:0".parse().expect("advertise addr parses");
        let mut host_controller =
            RealOnlineSessionController::host_descriptor_file_pending(&mut host_game, &unique_path)
                .expect("host descriptor writes");
        let descriptor_json = std::fs::read_to_string(&unique_path).expect("descriptor written");
        let descriptor: crate::multiplayer::QuinnHostConnectionDescriptor =
            serde_json::from_str(&descriptor_json).expect("descriptor parses");
        host_game.online_host_advertise_addr = descriptor.host_addr;

        let mut accept_game = GameState::new();
        let mut client_game = GameState::new();
        let (accept_result, client_result) = tokio::join!(
            host_controller.accept_descriptor_client(&mut accept_game),
            RealOnlineSessionController::connect_descriptor_client(&mut client_game, &unique_path),
        );

        accept_result.expect("host accepts descriptor client");
        let client_controller = client_result.expect("client joins descriptor host");
        assert_eq!(host_controller.mode_label(), "descriptor-host-accepted");
        assert_eq!(
            client_controller.mode_label(),
            "descriptor-client-connected"
        );
        assert_eq!(
            accept_game.online_session_state,
            OnlineSessionUxState::Connected
        );
        assert_eq!(
            client_game.online_session_state,
            OnlineSessionUxState::Connected
        );
        assert!(accept_game.message.contains("Remote miner joined"));
        assert!(client_game.message.contains("Connected to host descriptor"));

        let command_packet = crate::multiplayer::CommandPacket {
            client_id: crate::multiplayer::LOCAL_CLIENT_ID,
            commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: crate::multiplayer::PlayerId::new(2),
                sequence: crate::multiplayer::InputSequence::new(1),
                target_tick: crate::multiplayer::SimulationTick::new(301),
                command: crate::multiplayer::PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: true,
                    drill_down: false,
                },
            }],
        };
        let (host_summary, client_command_result) = tokio::join!(
            host_controller.descriptor_host_receive_command_packet(),
            async {
                let mut client_controller = client_controller;
                client_controller
                    .descriptor_client_send_command_packet(command_packet, 1)
                    .await
                    .map(|()| client_controller)
            },
        );
        let host_summary = host_summary.expect("host receives descriptor command packet");
        let client_controller = client_command_result.expect("client receives command ack");
        assert!(host_summary.all_accepted());
        assert_eq!(host_summary.acknowledged, 1);
        assert_eq!(
            client_controller.mode_label(),
            "descriptor-client-connected"
        );
        let mut client_controller = client_controller;

        let snapshot = live_player_network_snapshot(
            &client_game,
            crate::multiplayer::PlayerId::new(2),
            crate::multiplayer::SimulationTick::new(302),
        );
        host_controller
            .descriptor_host_send_snapshot(snapshot)
            .await
            .expect("host sends descriptor snapshot");
        let snapshot_message = client_controller
            .descriptor_client_receive_replication()
            .await
            .expect("client receives descriptor snapshot");
        assert!(matches!(
            snapshot_message,
            crate::multiplayer::ProtocolMessage::SnapshotKeyframe { .. }
        ));

        host_controller
            .descriptor_host_send_world_delta(
                crate::multiplayer::SimulationTick::new(303),
                crate::multiplayer::NetworkDeltaPayload::Players {
                    players: vec![crate::multiplayer::PlayerId::new(2)],
                },
            )
            .await
            .expect("host sends descriptor delta");
        let delta_message = client_controller
            .descriptor_client_receive_replication()
            .await
            .expect("client receives descriptor delta");
        assert!(matches!(
            delta_message,
            crate::multiplayer::ProtocolMessage::WorldDelta { .. }
        ));

        let (terrain_response, client_response) = tokio::join!(
            host_controller.descriptor_host_answer_terrain_request(9),
            client_controller.descriptor_client_request_terrain_chunk(4, 5, 8),
        );
        let expected_response = crate::multiplayer::ProtocolMessage::TerrainChunkResponse {
            chunk_x: 4,
            chunk_y: 5,
            revision: 9,
        };
        assert_eq!(
            terrain_response.expect("host responds to terrain request"),
            expected_response
        );
        assert_eq!(
            client_response.expect("client receives terrain response"),
            expected_response
        );
        let _ignored = std::fs::remove_file(unique_path);
    }

    #[tokio::test]
    async fn real_online_session_controller_applies_join_tick_and_reconnect_ux() {
        let mut game = GameState::new();
        game.player.x = 42.75;
        game.player.y = 17.25;
        game.player.velocity_x = 0.5;
        game.player.velocity_y = -0.25;
        game.player.fuel = 88.0;
        game.player.hull = 77.0;
        game.player.credits = 123;
        game.update_ticks = 9;
        game.total_resources_refined = 4;
        let mut controller = RealOnlineSessionController::connect_localhost(&mut game)
            .await
            .expect("real session connects");

        assert_eq!(controller.mode_label(), "combined-localhost");
        assert_eq!(game.online_session_state, OnlineSessionUxState::Connected);
        assert!(
            game.message
                .contains("Connected through real localhost Quinn")
        );

        let telemetry = controller
            .drive_telemetry_tick(&mut game)
            .await
            .expect("telemetry tick runs");
        assert!(telemetry.local_smoke_passed());
        assert_eq!(
            telemetry.summary.terrain_chunk_response,
            Some(crate::multiplayer::ProtocolMessage::TerrainChunkResponse {
                chunk_x: 42,
                chunk_y: 17,
                revision: 4,
            })
        );
        assert!(matches!(
            telemetry
                .summary
                .correction_summary
                .as_ref()
                .map(|summary| (summary.correction_plan, summary.snap_applied)),
            Some((crate::session::CorrectionPlan::Snap, true))
        ));
        assert_eq!(game.online_session_state, OnlineSessionUxState::Connected);
        assert!(game.message.contains("Real Quinn tick"));

        controller
            .reconnect(&mut game, crate::multiplayer::SessionToken::new(301))
            .await
            .expect("session reconnects");
        assert_eq!(game.online_session_state, OnlineSessionUxState::Connected);
        assert!(
            game.message
                .contains("Reconnected through real localhost Quinn")
        );
    }

    #[test]
    fn real_online_session_ux_snapshot_updates_game_state_from_tick_driver() {
        let mut game = GameState::new();
        let tick_summary = QuinnSessionTickSummary {
            command_summary: Some(crate::multiplayer::CommandPacketExchangeSummary {
                client_id: crate::multiplayer::ClientId::new(1),
                acknowledged: 1,
                rejected: 0,
                authoritative_tick: crate::multiplayer::SimulationTick::new(5),
            }),
            snapshot_replicated: true,
            delta_replicated: true,
            terrain_chunk_response: Some(
                crate::multiplayer::ProtocolMessage::TerrainChunkResponse {
                    chunk_x: 1,
                    chunk_y: 2,
                    revision: 3,
                },
            ),
            correction_summary: Some(SocketDrivenCorrectionSummary {
                snapshot_replicated: true,
                authoritative_tick: crate::multiplayer::SimulationTick::new(6),
                correction_plan: crate::session::CorrectionPlan::Smooth,
                presentation_x: 1.0,
                presentation_y: 2.0,
                snap_applied: false,
            }),
        };

        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot::from_tick_summary(
            &tick_summary,
            Some(2),
        ));

        assert_eq!(game.online_session_state, OnlineSessionUxState::Connected);
        assert!(game.online_host_owns_save);
        assert_eq!(game.online_player_slot, Some(2));
        assert!(game.message.contains("Real Quinn tick"));
    }

    #[test]
    fn online_limitations_report_real_local_quinn_socket_progress() {
        let limitations = GameState::online_session_limitations();

        assert!(
            limitations
                .iter()
                .any(|limitation| limitation.contains("Real localhost Quinn socket IO"))
        );
    }

    #[test]
    fn online_multiplayer_modal_queues_real_network_task_requests() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);

        game.selected_menu_item = 0;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.take_online_network_task_request(),
            Some(OnlineNetworkTaskRequest::HostDescriptorFile {
                path: default_online_descriptor_path()
            })
        );

        game.selected_menu_item = 1;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.take_online_network_task_request(),
            Some(OnlineNetworkTaskRequest::JoinDescriptorFile {
                path: default_online_descriptor_path()
            })
        );

        game.selected_menu_item = 2;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.take_online_network_task_request(),
            Some(OnlineNetworkTaskRequest::ReconnectDirectConnect)
        );
    }

    #[test]
    fn online_multiplayer_status_lines_report_real_lifecycle_state() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 1;
        game.confirm_online_multiplayer();

        let pending_lines = game.online_multiplayer_status_lines();
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Quinn/QUIC real socket IO enabled"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("JoinDescriptorFile"))
        );
        assert!(pending_lines.iter().any(|line| line.contains("Joining")));
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Role: client"))
        );
        assert!(pending_lines.iter().any(|line| line.contains("Ready: no")));
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Local player: Player"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("joined client: play through the host"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Descriptor file:"))
        );
        assert!(
            pending_lines.iter().any(
                |line| line.contains("Remote player: Host miner") && line.contains("role host")
            )
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("host owns the online save"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Online diagnostics"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("Direct-connect config"))
        );
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("online-lan-qa-checklist-md"))
        );
        assert!(
            !pending_lines
                .iter()
                .any(|line| line.contains("not enabled yet"))
        );

        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: false,
            player_slot: Some(2),
            status_message: "Connected through real localhost Quinn as player 2.".to_owned(),
        });
        let connected_lines = game.online_multiplayer_status_lines();
        assert!(
            connected_lines
                .iter()
                .any(|line| line.contains("Connected"))
        );
        assert!(connected_lines.iter().any(|line| line.contains("Slot: 2")));
        assert!(
            connected_lines
                .iter()
                .any(|line| line.contains("Host owns save: false"))
        );
    }

    #[test]
    fn online_direct_connect_setup_lines_use_game_state_configuration() {
        let mut game = GameState::new();
        game.online_descriptor_path = PathBuf::from("/tmp/custom-host.json");
        game.online_host_bind_addr = "0.0.0.0:5252".parse().expect("host bind parses");
        game.online_host_advertise_addr = "192.0.2.25:5252".parse().expect("host advertise parses");
        game.online_client_bind_addr = "0.0.0.0:0".parse().expect("client bind parses");
        game.online_gameplay_ticks = 90;

        let lines = game.online_direct_connect_setup_lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/tmp/custom-host.json"))
        );
        assert!(lines.iter().any(|line| line.contains("192.0.2.25:5252")));
        assert!(lines.iter().any(|line| line.contains("90")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("online-host-gameplay-descriptor-file-on-addr"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("online-join-gameplay-descriptor-file-on-addr"))
        );
    }

    #[test]
    fn online_failure_status_messages_are_player_actionable() {
        assert!(
            GameState::online_failure_status_message("protocol version mismatch")
                .contains("same build")
        );
        assert!(
            GameState::online_failure_status_message("certificate verify failed")
                .contains("Regenerate")
        );
        assert!(
            GameState::online_failure_status_message("descriptor JSON parse error")
                .contains("descriptor file")
        );
        assert!(
            GameState::online_failure_status_message("connection timed out").contains("UDP port")
        );
        assert!(
            GameState::online_failure_status_message("connection refused").contains("firewall")
        );
        assert!(
            GameState::online_failure_status_message("session token reconnect failed")
                .contains("Rejoin")
        );
    }

    #[test]
    fn online_network_task_results_reduce_into_game_state() {
        let mut game = GameState::new();

        game.apply_online_network_task_result(OnlineNetworkTaskResult::Connected(
            RealOnlineSessionUxSnapshot::from_joined_session(Some(1)),
        ));
        assert_eq!(game.online_session_state, OnlineSessionUxState::Connected);
        assert_eq!(game.run_mode, RunMode::Playing);
        assert_eq!(game.modal, None);
        assert_eq!(game.selected_menu_item, 0);

        game.apply_online_network_task_result(OnlineNetworkTaskResult::Failed(
            "direct Quinn connection task failed".to_owned(),
        ));
        assert_eq!(game.online_session_state, OnlineSessionUxState::Error);
        assert_eq!(game.online_network_task_request, None);
        assert!(game.message.contains("direct Quinn connection task failed"));

        game.apply_online_network_task_result(OnlineNetworkTaskResult::Shutdown);
        assert_eq!(game.online_session_state, OnlineSessionUxState::Shutdown);
    }

    #[test]
    fn online_multiplayer_modal_drives_host_join_reconnect_error_and_shutdown_ux_state() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);

        game.selected_menu_item = 0;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Hosting);
        assert!(game.online_host_owns_save);
        assert_eq!(game.online_player_slot, Some(1));

        game.selected_menu_item = 1;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Joining);
        assert!(!game.online_host_owns_save);
        assert_eq!(game.online_player_slot, Some(2));

        game.selected_menu_item = 2;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.online_session_state,
            OnlineSessionUxState::Reconnecting
        );

        game.selected_menu_item = 7;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Timeout);

        game.selected_menu_item = 8;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Error);

        game.selected_menu_item = 9;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Shutdown);
        assert_eq!(
            game.online_network_task_request,
            Some(OnlineNetworkTaskRequest::Shutdown)
        );
        assert_eq!(game.modal, None);
        assert!(game.message.contains("shutdown"));
        assert!(!game.online_session_limitations.is_empty());
    }

    #[test]
    fn online_multiplayer_cancel_clears_queued_network_request() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 0;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.online_network_task_request,
            Some(OnlineNetworkTaskRequest::HostDescriptorFile {
                path: default_online_descriptor_path()
            })
        );

        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.update(
            PlayerInput {
                cancel: true,
                ..PlayerInput::default()
            },
            0.0,
        );

        assert_eq!(game.online_network_task_request, None);
        assert_eq!(game.online_session_state, OnlineSessionUxState::Idle);
        assert_eq!(game.modal, None);
        assert!(game.message.contains("no network task queued"));
    }

    #[test]
    fn online_save_policy_line_reports_host_owned_save_rules() {
        let mut game = GameState::new();
        assert!(game.online_save_policy_line().contains("allowed"));

        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: false,
            player_slot: Some(2),
            status_message: "Connected as joined client.".to_owned(),
        });
        assert!(
            game.online_save_policy_line()
                .contains("joined clients cannot write")
        );

        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot: Some(1),
            status_message: "Connected as host.".to_owned(),
        });
        assert!(game.online_save_policy_line().contains("allowed"));
    }

    #[test]
    fn online_shutdown_preserves_dirty_save_state() {
        let mut game = GameState::new();
        game.save_dirty = true;
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 9;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.save_dirty);
        assert_eq!(game.modal, None);
        assert_eq!(
            game.online_network_task_request,
            Some(OnlineNetworkTaskRequest::Shutdown)
        );

        game.apply_online_network_task_result(OnlineNetworkTaskResult::Shutdown);
        assert!(game.save_dirty);
        assert_eq!(game.online_network_task_request, None);
        assert_eq!(game.modal, None);
    }

    #[test]
    fn online_lobby_participant_lines_report_names_slots_ready_and_connection() {
        let mut game = GameState::new();
        game.online_player_name = "Ada".to_owned();
        game.online_remote_player_name = Some("Bert".to_owned());
        game.online_player_slot = Some(1);
        game.online_host_owns_save = true;
        game.online_local_ready = true;
        game.online_remote_player_ready = true;
        game.online_remote_player_connected = true;
        game.online_session_state = OnlineSessionUxState::Connected;

        let lines = game.online_lobby_participant_lines();
        assert!(lines.iter().any(|line| line.contains("Ada")));
        assert!(lines.iter().any(|line| line.contains("slot 1")));
        assert!(lines.iter().any(|line| line.contains("role host")));
        assert!(lines.iter().any(|line| line.contains("Bert")));
        assert!(lines.iter().any(|line| line.contains("slot 2")));
        assert!(lines.iter().any(|line| line.contains("connected yes")));
    }

    #[test]
    fn online_multiplayer_cycles_descriptor_path_for_host_join_ui() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 3;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.online_descriptor_path,
            alternate_online_descriptor_path()
        );
        assert!(game.message.contains("Descriptor path selected"));

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_descriptor_path, join_online_descriptor_path());

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.online_descriptor_path,
            default_online_descriptor_path()
        );
    }

    #[test]
    fn online_multiplayer_inspects_descriptor_file_from_modal() {
        let unique_path = std::env::temp_dir().join(format!(
            "drillgame-inspect-descriptor-{}.json",
            std::process::id()
        ));
        let descriptor = crate::multiplayer::QuinnHostConnectionDescriptor {
            host_addr: "127.0.0.1:4242".parse().expect("host addr parses"),
            server_name: "localhost".to_owned(),
            certificate_der: vec![1, 2, 3, 4],
        };
        std::fs::write(
            &unique_path,
            serde_json::to_string(&descriptor).expect("descriptor serializes"),
        )
        .expect("descriptor writes");

        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.online_descriptor_path = unique_path.clone();
        game.selected_menu_item = 4;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.message.contains("Descriptor OK"));
        assert!(game.message.contains("127.0.0.1:4242"));
        assert!(game.message.contains("cert=4 bytes"));

        std::fs::write(&unique_path, "not json").expect("bad descriptor writes");
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_session_state, OnlineSessionUxState::Error);
        assert!(game.message.contains("Descriptor inspect failed"));
        let _ignored = std::fs::remove_file(unique_path);
    }

    #[test]
    fn online_multiplayer_cycles_host_address_and_tick_presets() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 5;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_host_bind_addr, lan_online_host_bind_addr());
        assert_eq!(
            game.online_host_advertise_addr,
            lan_online_host_advertise_addr()
        );
        assert!(game.message.contains("Host address preset selected"));

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(
            game.online_host_bind_addr,
            localhost_ephemeral_online_host_bind_addr()
        );
        assert_eq!(
            game.online_host_advertise_addr,
            localhost_ephemeral_online_host_advertise_addr()
        );

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_host_bind_addr, default_online_host_bind_addr());
        assert_eq!(
            game.online_host_advertise_addr,
            default_online_host_advertise_addr()
        );

        game.selected_menu_item = 6;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_gameplay_ticks, 120);
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.online_gameplay_ticks, 300);
        assert!(game.message.contains("Gameplay smoke tick count selected"));
    }

    #[test]
    fn online_multiplayer_start_gameplay_gates_on_connection_and_ready() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 11;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.modal.is_some());
        assert!(game.message.contains("Start blocked: connect"));

        game.apply_real_online_session_ux(RealOnlineSessionUxSnapshot {
            state: OnlineSessionUxState::Connected,
            host_owns_save: true,
            player_slot: Some(1),
            status_message: "Descriptor client accepted.".to_owned(),
        });
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 11;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.modal.is_some());
        assert!(game.message.contains("toggle local ready"));

        game.selected_menu_item = 10;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        game.selected_menu_item = 11;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.modal.is_some());
        assert!(game.message.contains("remote player to toggle ready"));

        game.online_remote_player_ready = true;
        game.selected_menu_item = 11;
        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert_eq!(game.run_mode, RunMode::Playing);
        assert_eq!(game.modal, None);
        assert!(game.message.contains("Starting online gameplay"));
    }

    #[test]
    fn online_multiplayer_toggle_ready_updates_lobby_status() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::OnlineMultiplayer);
        game.selected_menu_item = 10;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(game.online_local_ready);
        assert!(game.message.contains("ready"));
        assert!(
            game.online_multiplayer_status_lines()
                .iter()
                .any(|line| line.contains("Ready: yes"))
        );

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );
        assert!(!game.online_local_ready);
        assert!(game.message.contains("not ready"));
    }

    #[test]
    fn title_menu_exposes_online_multiplayer_entrypoint() {
        assert!(GameState::title_options().contains(&TitleOption::OnlineMultiplayer));
    }

    #[test]
    fn selecting_online_multiplayer_opens_online_modal() {
        let mut game = GameState::new();
        let options = GameState::title_options();
        game.selected_title_item = options
            .iter()
            .position(|option| *option == TitleOption::OnlineMultiplayer)
            .expect("online multiplayer option exists");

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.0,
        );

        assert_eq!(game.modal, Some(ModalScreen::OnlineMultiplayer));
    }

    #[test]
    fn hq_briefing_changes_with_depth() {
        let mut game = GameState::new();
        game.deepest_tile_reached = 65;
        assert!(hq_story_message(&game).contains("Thermal"));
    }

    #[test]
    fn return_bonus_resets_trip_depth_and_pays() {
        let mut game = GameState::new();
        game.current_zone = Some(SurfaceZone::Depot);
        game.trip_best_depth = 24;
        let initial_credits = game.player.credits;
        game.award_return_bonus();
        assert_eq!(game.trip_best_depth, 0);
        assert_eq!(game.return_streak, 1);
        assert!(game.player.credits > initial_credits);
    }

    #[test]
    fn lost_cargo_recovers_when_player_returns_to_site() {
        let mut game = GameState::new();
        game.lost_cargo_x = Some(game.player.x);
        game.lost_cargo_y = Some(game.player.y);
        game.lost_cargo_count = 2;
        game.recover_lost_cargo_if_near();
        assert_eq!(game.lost_cargo_count, 0);
        assert_eq!(game.player.cargo_used(), 2);
    }

    #[test]
    fn options_changes_mark_settings_dirty() {
        let mut game = GameState::new();
        game.selected_menu_item = 0;
        game.confirm_options();
        assert!(game.take_settings_dirty());
        assert!(!game.take_settings_dirty());
    }

    #[test]
    fn explosive_pocket_sets_flash_and_cave_in() {
        let mut game = GameState::new();
        game.trigger_explosive_pocket();
        assert!(game.screen_flash_seconds > 0.0);
        assert!(!game.falling_boulders.is_empty());
    }

    #[test]
    fn entering_surface_zone_opens_walkable_interior() {
        let mut game = GameState::new();
        game.enter_interior(SurfaceZone::Fuel);
        assert_eq!(game.run_mode, RunMode::Interior);
        assert_eq!(game.interior_zone, Some(SurfaceZone::Fuel));
        assert!(game.modal.is_none());
    }

    #[test]
    fn interior_counter_opens_existing_service_modal() {
        let mut game = GameState::new();
        game.enter_interior(SurfaceZone::Shop);
        game.interior_x = interior_service_x(SurfaceZone::Shop);
        game.open_interior_hotspot();
        assert_eq!(game.modal, Some(ModalScreen::Shop));
    }

    #[test]
    fn camera_can_follow_player_above_surface() {
        let mut game = GameState::new();
        game.player.y = MIN_PLAYER_Y;
        let (_, target_y) = target_camera_offset(&game);
        assert!(target_y < 0.0);
    }

    #[test]
    fn vertical_movement_allows_limited_sky_flight() {
        let mut game = GameState::new();
        game.player.y = 2.0;
        game.move_axis(0.0, MIN_PLAYER_Y * 2.0);
        assert!((game.player.y - MIN_PLAYER_Y).abs() < f32::EPSILON);
    }

    #[test]
    fn new_game_starts_camera_intro_above_player() {
        let game = GameState::new();
        let (target_x, target_y) = target_camera_offset(&game);

        assert!(game.camera_intro_seconds > 0.0);
        assert!((game.camera_x - target_x).abs() < f32::EPSILON);
        assert!(game.camera_y < target_y);
    }

    #[test]
    fn saved_game_disables_camera_intro() {
        let game = GameState::new();
        let saved = game.clone_for_save();

        assert!(saved.camera_intro_seconds <= f32::EPSILON);
    }

    #[test]
    fn camera_intro_drops_toward_target() {
        let mut game = GameState::new();
        let (_, target_y) = target_camera_offset(&game);
        let initial_y = game.camera_y;

        game.update_camera(CAMERA_INTRO_SECONDS * 0.5);

        assert!(game.camera_y > initial_y);
        assert!(game.camera_y < target_y);
    }

    #[test]
    fn revealing_exploration_marks_nearby_tiles_for_redraw() {
        let mut game = GameState::new();
        game.visual_changes.changed_tiles.clear();
        let position = TilePosition { x: 20, y: 20 };

        game.mark_exploration_visual_changed(position);

        assert!(game.visual_changes.changed_tiles.contains(&position));
        assert!(game.visual_changes.changed_tiles.contains(&TilePosition {
            x: position.x + EXPLORATION_VISUAL_CHANGE_RADIUS_TILES,
            y: position.y
        }));
    }

    #[test]
    fn movement_regression_updates_position_and_burns_fuel() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        let initial_x = game.player.x;
        let initial_fuel = game.player.fuel;

        game.update(
            PlayerInput {
                horizontal: 1.0,
                thrust: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert!(game.player.x > initial_x);
        assert!(game.player.fuel < initial_fuel);
    }

    #[test]
    fn drilling_regression_mines_target_tile_and_adds_cargo() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.x = 10.0 * TILE_SIZE;
        game.player.y = 10.0 * TILE_SIZE;
        game.terrain.set_kind(
            TilePosition { x: 10, y: 11 },
            TileKind::Ore(MineralKind::Copper),
        );

        for _ in 0..12 {
            game.update(
                PlayerInput {
                    drill_down: true,
                    ..PlayerInput::default()
                },
                0.1,
            );
        }

        assert!(matches!(
            game.terrain
                .tile(TilePosition { x: 10, y: 11 })
                .map(|tile| tile.kind),
            Some(TileKind::Air)
        ));
        assert!(game.player.cargo_used() > 0);
    }

    #[test]
    fn cargo_and_economy_regression_sells_loaded_ore() {
        let mut game = GameState::new();
        let initial_credits = game.player.credits;
        assert!(game.player.add_cargo(MineralKind::Copper));

        let earnings = sell_cargo(&mut game.player);

        assert!(earnings > 0);
        assert!(game.player.credits > initial_credits);
        assert_eq!(game.player.cargo_used(), 0);
    }

    #[test]
    fn bombs_regression_arm_and_detonate() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.bombs = 1;
        game.player.y = MIN_PLAYER_Y;
        game.town_development.explosives_shack_level = 3;

        game.update(
            PlayerInput {
                bomb: true,
                ..PlayerInput::default()
            },
            0.1,
        );
        assert_eq!(game.placed_bombs.len(), 1);
        game.update(PlayerInput::default(), 1.0);

        assert!(game.placed_bombs.is_empty());
        assert!(
            game.sound_cues
                .iter()
                .any(|cue| matches!(cue, SoundCue::Explosion))
        );
    }

    #[test]
    fn rescue_regression_returns_player_to_surface_and_records_fee() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.game_over = true;
        game.player.y = 40.0 * TILE_SIZE;
        game.player.credits = 1_000;
        let initial_credits = game.player.credits;

        game.update(
            PlayerInput {
                interact: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert!(!game.game_over);
        assert!(game.player.y <= 4.0 * TILE_SIZE);
        assert!(game.player.credits < initial_credits);
        assert_eq!(game.rescue_count, 1);
    }

    #[test]
    fn damage_and_repair_regression_restores_hull_for_credits() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.hull = 50.0;
        game.player.credits = 500;
        game.modal = Some(ModalScreen::RepairConfirm);
        let initial_credits = game.player.credits;

        game.update(
            PlayerInput {
                confirm: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert!(game.player.hull > 50.0);
        assert!(game.player.credits < initial_credits);
        assert!(game.message.contains("Hull repaired"));
    }

    #[test]
    fn ui_transition_regression_pauses_from_playing() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;

        game.update(
            PlayerInput {
                pause: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert_eq!(game.run_mode, RunMode::Paused);
    }

    #[test]
    fn scanner_regression_pulses_and_enters_cooldown() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.scanner_level = 1;

        game.update(
            PlayerInput {
                scan: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert!(game.scanner_pulse_seconds > 0.0);
        assert!(game.scanner_cooldown_seconds > 0.0);
        assert!(game.message.contains("Scanner pulse"));
    }

    #[test]
    fn infrastructure_regression_places_signal_relay() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.y = 12.0 * TILE_SIZE;
        game.player.signal_relay_kits = 1;

        game.update(
            PlayerInput {
                place_relay: true,
                ..PlayerInput::default()
            },
            0.1,
        );

        assert_eq!(game.infrastructure.len(), 1);
        assert_eq!(game.infrastructure[0].kind, InfrastructureKind::SignalRelay);
        assert_eq!(game.player.signal_relay_kits, 0);
    }
}
