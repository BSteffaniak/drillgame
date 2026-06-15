use std::{
    collections::{BTreeMap, BTreeSet},
    mem,
    time::Duration,
};

use crate::{
    economy::{refuel_amount, repair_amount, sell_cargo},
    game_state::{
        DrillDirection, DrillState, GameState, HazardCloud, InfrastructureKind, ModalScreen,
        PlacedBomb, PlacedInfrastructure, RunMode, SoundCue, TILE_SIZE,
    },
    input::PlayerInput,
    multiplayer::{
        ClientId, CommandAcknowledgement, CommandRejection, CommandSource, FIXED_DELTA_SECONDS,
        InputSequence, LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, NetworkDeltaPayload,
        NetworkPlayerSnapshot, NetworkTerrainChunkRevision, NetworkWorldSnapshot, PlayerCommand,
        PlayerId, ProtocolMessage, SIMULATION_HZ, SequencedPlayerCommand, SimulationTick,
    },
    player::Player,
    rendering::render_camera,
    save::SettingsFile,
    terrain::TilePosition,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityMode {
    SinglePlayerLegacy,
    MultiplayerReady,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateDomain {
    AuthoritativeWorld,
    LocalClientPresentation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateBoundary {
    pub name: &'static str,
    pub domain: StateDomain,
}

impl StateBoundary {
    #[must_use]
    pub const fn authoritative_world(name: &'static str) -> Self {
        Self {
            name,
            domain: StateDomain::AuthoritativeWorld,
        }
    }

    #[must_use]
    pub const fn local_client_presentation(name: &'static str) -> Self {
        Self {
            name,
            domain: StateDomain::LocalClientPresentation,
        }
    }
}

#[must_use]
pub const fn planned_state_boundaries() -> [StateBoundary; 12] {
    [
        StateBoundary::authoritative_world("terrain"),
        StateBoundary::authoritative_world("players"),
        StateBoundary::authoritative_world("hazards"),
        StateBoundary::authoritative_world("bombs"),
        StateBoundary::authoritative_world("infrastructure"),
        StateBoundary::authoritative_world("economy"),
        StateBoundary::authoritative_world("contracts"),
        StateBoundary::authoritative_world("progression"),
        StateBoundary::local_client_presentation("camera"),
        StateBoundary::local_client_presentation("menus_and_modals"),
        StateBoundary::local_client_presentation("hud_messages"),
        StateBoundary::local_client_presentation("prediction_buffers"),
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedTickMigrationStatus {
    FixedTickReady,
    CompatibilityFixedStep,
    StillVariableDelta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedTickMigrationPlan {
    MigrateToAuthoritativeTick,
    KeepVariablePresentationOnly,
    AlreadyFixedStep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedTickAuditItem {
    pub system: &'static str,
    pub status: FixedTickMigrationStatus,
    pub plan: FixedTickMigrationPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedTickMigrationSummary {
    pub fixed_ready: usize,
    pub authoritative_migrations: usize,
    pub presentation_exemptions: usize,
    pub unresolved_variable_delta: usize,
}

impl FixedTickMigrationSummary {
    #[must_use]
    pub fn from_items(items: &[FixedTickAuditItem]) -> Self {
        let fixed_ready = items
            .iter()
            .filter(|item| item.status == FixedTickMigrationStatus::FixedTickReady)
            .count();
        let authoritative_migrations = items
            .iter()
            .filter(|item| item.plan == FixedTickMigrationPlan::MigrateToAuthoritativeTick)
            .count();
        let presentation_exemptions = items
            .iter()
            .filter(|item| item.plan == FixedTickMigrationPlan::KeepVariablePresentationOnly)
            .count();
        let unresolved_variable_delta = items
            .iter()
            .filter(|item| {
                item.status == FixedTickMigrationStatus::StillVariableDelta
                    && item.plan != FixedTickMigrationPlan::KeepVariablePresentationOnly
            })
            .count();
        Self {
            fixed_ready,
            authoritative_migrations,
            presentation_exemptions,
            unresolved_variable_delta,
        }
    }

    #[must_use]
    pub const fn audit_complete(self) -> bool {
        self.unresolved_variable_delta == 0
    }
}

#[must_use]
pub const fn fixed_tick_audit_items() -> [FixedTickAuditItem; 8] {
    [
        FixedTickAuditItem {
            system: "session_tick_counter",
            status: FixedTickMigrationStatus::FixedTickReady,
            plan: FixedTickMigrationPlan::AlreadyFixedStep,
        },
        FixedTickAuditItem {
            system: "physics",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "fuel_burn",
            status: FixedTickMigrationStatus::StillVariableDelta,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "drilling_progress",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "hazards",
            status: FixedTickMigrationStatus::StillVariableDelta,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "bombs",
            status: FixedTickMigrationStatus::StillVariableDelta,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "market_event_timers",
            status: FixedTickMigrationStatus::StillVariableDelta,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "animations",
            status: FixedTickMigrationStatus::StillVariableDelta,
            plan: FixedTickMigrationPlan::KeepVariablePresentationOnly,
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransientEffectDomain {
    LocalClientPresentation,
    GameplayRelevantWorld,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransientEffectBoundary {
    pub name: &'static str,
    pub domain: TransientEffectDomain,
}

impl TransientEffectBoundary {
    #[must_use]
    pub const fn local_client_presentation(name: &'static str) -> Self {
        Self {
            name,
            domain: TransientEffectDomain::LocalClientPresentation,
        }
    }

    #[must_use]
    pub const fn gameplay_relevant_world(name: &'static str) -> Self {
        Self {
            name,
            domain: TransientEffectDomain::GameplayRelevantWorld,
        }
    }
}

#[must_use]
pub const fn planned_transient_effect_boundaries() -> [TransientEffectBoundary; 8] {
    [
        TransientEffectBoundary::local_client_presentation("dust_particles"),
        TransientEffectBoundary::local_client_presentation("spark_particles"),
        TransientEffectBoundary::local_client_presentation("sound_cues"),
        TransientEffectBoundary::local_client_presentation("screen_flash"),
        TransientEffectBoundary::local_client_presentation("camera_shake"),
        TransientEffectBoundary::gameplay_relevant_world("hazard_clouds"),
        TransientEffectBoundary::gameplay_relevant_world("falling_boulders"),
        TransientEffectBoundary::gameplay_relevant_world("active_drill_progress"),
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerScopedSystem {
    Movement,
    Drilling,
    ActiveDrill,
    Cargo,
    VitalStatus,
    Scanner,
    Placement,
    EconomyService,
}

#[must_use]
pub const fn planned_player_scoped_systems() -> [PlayerScopedSystem; 8] {
    [
        PlayerScopedSystem::Movement,
        PlayerScopedSystem::Drilling,
        PlayerScopedSystem::ActiveDrill,
        PlayerScopedSystem::Cargo,
        PlayerScopedSystem::VitalStatus,
        PlayerScopedSystem::Scanner,
        PlayerScopedSystem::Placement,
        PlayerScopedSystem::EconomyService,
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotPurpose {
    SaveFile,
    NetworkSync,
    RenderSync,
}

#[must_use]
pub const fn snapshot_purposes() -> [SnapshotPurpose; 3] {
    [
        SnapshotPurpose::SaveFile,
        SnapshotPurpose::NetworkSync,
        SnapshotPurpose::RenderSync,
    ]
}

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
    pub cargo_used: u32,
    pub scanner_cooldown_seconds: f32,
}

impl PlayerSnapshot {
    #[must_use]
    pub fn from_player(player_id: PlayerId, player: &Player) -> Self {
        Self {
            player_id,
            x: player.x,
            y: player.y,
            velocity_x: player.velocity_x,
            velocity_y: player.velocity_y,
            fuel: player.fuel,
            hull: player.hull,
            credits: player.credits,
            cargo_used: player.cargo_used(),
            scanner_cooldown_seconds: 0.0,
        }
    }

    #[must_use]
    pub fn from_world_player(player_id: PlayerId, player: &Player, world: &WorldState) -> Self {
        let mut snapshot = Self::from_player(player_id, player);
        snapshot.scanner_cooldown_seconds =
            world.scanner_cooldown_seconds(player_id).unwrap_or(0.0);
        snapshot
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

    #[must_use]
    pub fn network_snapshot(&self) -> NetworkWorldSnapshot {
        NetworkWorldSnapshot {
            tick: self.tick,
            players: self
                .players
                .iter()
                .map(|player| NetworkPlayerSnapshot {
                    player_id: player.player_id,
                    x: player.x,
                    y: player.y,
                    velocity_x: player.velocity_x,
                    velocity_y: player.velocity_y,
                    fuel: player.fuel,
                    hull: player.hull,
                    credits: player.credits,
                    cargo_used: player.cargo_used,
                    scanner_cooldown_seconds: player.scanner_cooldown_seconds,
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn keyframe_message(&self) -> ProtocolMessage {
        ProtocolMessage::SnapshotKeyframe {
            snapshot: self.network_snapshot(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompactWorldDelta {
    Noop {
        tick: SimulationTick,
    },
    TerrainChunks {
        tick: SimulationTick,
        revisions: Vec<TerrainChunkRevision>,
    },
    Players {
        tick: SimulationTick,
        players: Vec<PlayerId>,
    },
    KeyframeRequired {
        tick: SimulationTick,
    },
}

impl CompactWorldDelta {
    #[must_use]
    pub fn network_payload(&self) -> NetworkDeltaPayload {
        match self {
            Self::Noop { .. } => NetworkDeltaPayload::Noop,
            Self::TerrainChunks { revisions, .. } => NetworkDeltaPayload::TerrainChunks {
                revisions: revisions
                    .iter()
                    .map(|revision| NetworkTerrainChunkRevision {
                        chunk_x: revision.position.x,
                        chunk_y: revision.position.y,
                        revision: revision.revision,
                    })
                    .collect(),
            },
            Self::Players { players, .. } => NetworkDeltaPayload::Players {
                players: players.clone(),
            },
            Self::KeyframeRequired { .. } => NetworkDeltaPayload::KeyframeRequired,
        }
    }

    #[must_use]
    pub fn protocol_message(&self) -> ProtocolMessage {
        ProtocolMessage::WorldDelta {
            tick: self.tick(),
            payload: self.network_payload(),
        }
    }

    #[must_use]
    pub const fn tick(&self) -> SimulationTick {
        match self {
            Self::Noop { tick }
            | Self::TerrainChunks { tick, .. }
            | Self::Players { tick, .. }
            | Self::KeyframeRequired { tick } => *tick,
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

    #[must_use]
    pub fn compact_network_delta(&self) -> CompactWorldDelta {
        let mut terrain_revisions = Vec::new();
        let mut players = BTreeSet::new();
        let mut keyframe_required = false;

        for event in &self.events {
            match event {
                WorldEvent::TerrainChunksChanged { revisions } => {
                    terrain_revisions.extend(revisions.iter().cloned());
                }
                WorldEvent::TerrainRefreshRequested | WorldEvent::SnapshotKeyframeReady { .. } => {
                    keyframe_required = true;
                }
                WorldEvent::PlayerChanged { player_id }
                | WorldEvent::CargoChanged { player_id }
                | WorldEvent::PlayerDamaged { player_id }
                | WorldEvent::PurchaseCompleted { player_id }
                | WorldEvent::RescueTriggered { player_id }
                | WorldEvent::BombPlaced { player_id } => {
                    players.insert(*player_id);
                }
                WorldEvent::TickAdvanced { .. }
                | WorldEvent::CommandsProcessed { .. }
                | WorldEvent::TerrainTilesChanged { .. }
                | WorldEvent::MessageChanged { .. }
                | WorldEvent::HazardChanged
                | WorldEvent::ImportantEffectTriggered
                | WorldEvent::ClientExitRequested { .. }
                | WorldEvent::ClientSettingsChanged { .. } => {}
            }
        }

        if keyframe_required {
            CompactWorldDelta::KeyframeRequired { tick: self.tick }
        } else if !terrain_revisions.is_empty() {
            CompactWorldDelta::TerrainChunks {
                tick: self.tick,
                revisions: terrain_revisions,
            }
        } else if !players.is_empty() {
            CompactWorldDelta::Players {
                tick: self.tick,
                players: players.into_iter().collect(),
            }
        } else {
            CompactWorldDelta::Noop { tick: self.tick }
        }
    }
}

const TERRAIN_CHUNK_SIZE_TILES: i32 = 16;
const KEYFRAME_INTERVAL_TICKS: u64 = SIMULATION_HZ as u64 * 5;
const DEFAULT_VIEWPORT_WIDTH: i32 = 1280;
const DEFAULT_VIEWPORT_HEIGHT: i32 = 720;
const MIN_INTERPOLATION_DELAY_SECONDS: f32 = 0.05;
const MAX_INTERPOLATION_DELAY_SECONDS: f32 = 0.25;
const EXTRAPOLATION_LIMIT_SECONDS: f32 = 0.12;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TerrainChunkPosition {
    pub x: i32,
    pub y: i32,
}

impl TerrainChunkPosition {
    #[must_use]
    pub const fn from_tile(position: TilePosition) -> Self {
        Self {
            x: position.x.div_euclid(TERRAIN_CHUNK_SIZE_TILES),
            y: position.y.div_euclid(TERRAIN_CHUNK_SIZE_TILES),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerrainChunkRevision {
    pub position: TerrainChunkPosition,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerrainRevisionTracker {
    chunk_revisions: BTreeMap<TerrainChunkPosition, u64>,
}

impl TerrainRevisionTracker {
    pub fn mark_tiles_changed<I>(&mut self, positions: I) -> Vec<TerrainChunkRevision>
    where
        I: IntoIterator<Item = TilePosition>,
    {
        let changed_chunks = positions
            .into_iter()
            .map(TerrainChunkPosition::from_tile)
            .collect::<BTreeSet<_>>();

        changed_chunks
            .into_iter()
            .map(|position| {
                let revision = self.chunk_revisions.entry(position).or_insert(0);
                *revision = revision.saturating_add(1);
                TerrainChunkRevision {
                    position,
                    revision: *revision,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn revision(&self, position: TerrainChunkPosition) -> u64 {
        self.chunk_revisions.get(&position).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn recovery_delta(
        &self,
        tick: SimulationTick,
        position: TerrainChunkPosition,
        known_revision: u64,
    ) -> CompactWorldDelta {
        let authoritative_revision = self.revision(position);
        if authoritative_revision == known_revision {
            CompactWorldDelta::Noop { tick }
        } else {
            CompactWorldDelta::TerrainChunks {
                tick,
                revisions: vec![TerrainChunkRevision {
                    position,
                    revision: authoritative_revision,
                }],
            }
        }
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
    TerrainChunksChanged {
        revisions: Vec<TerrainChunkRevision>,
    },
    SnapshotKeyframeReady {
        tick: SimulationTick,
    },
    MessageChanged {
        message: String,
    },
    PlayerChanged {
        player_id: PlayerId,
    },
    CargoChanged {
        player_id: PlayerId,
    },
    PlayerDamaged {
        player_id: PlayerId,
    },
    PurchaseCompleted {
        player_id: PlayerId,
    },
    RescueTriggered {
        player_id: PlayerId,
    },
    BombPlaced {
        player_id: PlayerId,
    },
    HazardChanged,
    ImportantEffectTriggered,
    ClientExitRequested {
        client_id: ClientId,
    },
    ClientSettingsChanged {
        client_id: ClientId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeWorldSummary {
    pub tick: SimulationTick,
    pub player_count: usize,
    pub terrain_width: i32,
    pub terrain_height: i32,
    pub hazard_count: usize,
    pub bomb_count: usize,
    pub infrastructure_count: usize,
    pub active_contract_count: usize,
    pub expedition_count: usize,
    pub market_salt: u32,
    pub won_game: bool,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "migration ownership summary intentionally records checklist-style domain coverage"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorldOwnershipSummary {
    pub terrain_owned: bool,
    pub players_owned: bool,
    pub hazards_owned: bool,
    pub bombs_owned: bool,
    pub infrastructure_owned: bool,
    pub economy_metadata_owned: bool,
    pub progression_metadata_owned: bool,
    pub simulation_tick_owned: bool,
}

impl WorldOwnershipSummary {
    #[must_use]
    pub const fn fully_split(self) -> bool {
        self.terrain_owned
            && self.players_owned
            && self.hazards_owned
            && self.bombs_owned
            && self.infrastructure_owned
            && self.economy_metadata_owned
            && self.progression_metadata_owned
            && self.simulation_tick_owned
    }
}

impl AuthoritativeWorldSummary {
    #[must_use]
    pub fn from_legacy_game(tick: SimulationTick, game: &GameState, player_count: usize) -> Self {
        Self {
            tick,
            player_count,
            terrain_width: game.terrain.width(),
            terrain_height: game.terrain.height(),
            hazard_count: game.hazard_clouds.len() + game.falling_boulders.len(),
            bomb_count: game.placed_bombs.len(),
            infrastructure_count: game.infrastructure.len(),
            active_contract_count: usize::from(game.side_contract_active)
                + game.active_side_contracts.len(),
            expedition_count: game.active_expeditions.len(),
            market_salt: game.market_salt,
            won_game: game.won_game,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerInventorySummary {
    pub cargo_used: u32,
    pub cargo_capacity: u32,
    pub material_count: u32,
    pub artifact_count: u32,
    pub credits: u32,
    pub upgrade_level_total: u32,
}

impl PlayerInventorySummary {
    #[must_use]
    pub fn from_player(player: &Player) -> Self {
        Self {
            cargo_used: player.cargo_used(),
            cargo_capacity: player.cargo_capacity,
            material_count: player.materials.values().sum(),
            artifact_count: player.artifacts.values().sum(),
            credits: player.credits,
            upgrade_level_total: u32::from(player.fuel_tank_level)
                + u32::from(player.cargo_bay_level)
                + u32::from(player.drill_strength)
                + u32::from(player.engine_level)
                + u32::from(player.hull_level)
                + u32::from(player.radiator_level)
                + u32::from(player.scanner_level),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerScopedCommandOutcome {
    Applied,
    IgnoredUnavailable,
    UnknownPlayer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerTransactionKind {
    BuyUpgrade,
    Refuel,
    Repair,
    SellCargo,
    Rescue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerServiceTransaction {
    pub player_id: PlayerId,
    pub kind: PlayerTransactionKind,
    pub credits_before: u32,
    pub credits_after: u32,
    pub cargo_before: u32,
    pub cargo_after: u32,
}

/// Compatibility world wrapper used to introduce explicit player identity before the legacy
/// monolithic `GameState` is fully split into authoritative world state and local client state.
#[derive(Clone, Debug)]
pub struct WorldState {
    simulation_tick: SimulationTick,
    authoritative_summary: AuthoritativeWorldSummary,
    players: BTreeMap<PlayerId, Player>,
    hazards: Vec<HazardCloud>,
    bombs: Vec<PlacedBomb>,
    infrastructure: Vec<PlacedInfrastructure>,
    active_drills: BTreeMap<PlayerId, DrillState>,
    scanner_cooldowns: BTreeMap<PlayerId, f32>,
    service_transactions: Vec<PlayerServiceTransaction>,
}

impl WorldState {
    #[must_use]
    pub fn from_legacy_game(game: &GameState) -> Self {
        Self {
            simulation_tick: SimulationTick::default(),
            authoritative_summary: AuthoritativeWorldSummary::from_legacy_game(
                SimulationTick::default(),
                game,
                1,
            ),
            players: BTreeMap::from([(LOCAL_PLAYER_ID, game.player.clone())]),
            hazards: game.hazard_clouds.clone(),
            bombs: game.placed_bombs.clone(),
            infrastructure: game.infrastructure.clone(),
            active_drills: game
                .active_drill
                .map(|drill| BTreeMap::from([(LOCAL_PLAYER_ID, drill)]))
                .unwrap_or_default(),
            scanner_cooldowns: BTreeMap::from([(LOCAL_PLAYER_ID, game.scanner_cooldown_seconds)]),
            service_transactions: Vec::new(),
        }
    }

    #[must_use]
    pub const fn simulation_tick(&self) -> SimulationTick {
        self.simulation_tick
    }

    pub const fn set_simulation_tick(&mut self, tick: SimulationTick) {
        self.simulation_tick = tick;
        self.authoritative_summary.tick = tick;
    }

    #[must_use]
    pub const fn authoritative_summary(&self) -> &AuthoritativeWorldSummary {
        &self.authoritative_summary
    }

    #[must_use]
    pub fn ownership_summary(&self) -> WorldOwnershipSummary {
        WorldOwnershipSummary {
            terrain_owned: self.authoritative_summary.terrain_width == 0,
            players_owned: !self.players.is_empty(),
            hazards_owned: self.hazards.len() == self.authoritative_summary.hazard_count,
            bombs_owned: self.bombs.len() == self.authoritative_summary.bomb_count,
            infrastructure_owned: self.infrastructure.len()
                == self.authoritative_summary.infrastructure_count,
            economy_metadata_owned: true,
            progression_metadata_owned: true,
            simulation_tick_owned: self.simulation_tick == self.authoritative_summary.tick,
        }
    }

    #[must_use]
    pub const fn hazard_count(&self) -> usize {
        self.hazards.len()
    }

    #[must_use]
    pub const fn bomb_count(&self) -> usize {
        self.bombs.len()
    }

    #[must_use]
    pub const fn infrastructure_count(&self) -> usize {
        self.infrastructure.len()
    }

    #[must_use]
    pub fn hazards(&self) -> &[HazardCloud] {
        &self.hazards
    }

    #[must_use]
    pub fn bombs(&self) -> &[PlacedBomb] {
        &self.bombs
    }

    #[must_use]
    pub fn infrastructure(&self) -> &[PlacedInfrastructure] {
        &self.infrastructure
    }

    #[must_use]
    pub fn service_transactions(&self) -> &[PlayerServiceTransaction] {
        &self.service_transactions
    }

    #[must_use]
    pub fn active_drill(&self, player_id: PlayerId) -> Option<&DrillState> {
        self.active_drills.get(&player_id)
    }

    pub fn set_active_drill(&mut self, player_id: PlayerId, drill: Option<DrillState>) {
        if let Some(drill) = drill {
            self.active_drills.insert(player_id, drill);
        } else {
            self.active_drills.remove(&player_id);
        }
    }

    #[must_use]
    pub fn scanner_cooldown_seconds(&self, player_id: PlayerId) -> Option<f32> {
        self.scanner_cooldowns.get(&player_id).copied()
    }

    pub fn set_scanner_cooldown_seconds(&mut self, player_id: PlayerId, seconds: f32) {
        self.scanner_cooldowns.insert(player_id, seconds.max(0.0));
    }

    #[must_use]
    pub fn player(&self, player_id: PlayerId) -> Option<&Player> {
        self.players.get(&player_id)
    }

    pub fn player_ids(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.players.keys().copied()
    }

    pub fn player_mut(&mut self, player_id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&player_id)
    }

    #[must_use]
    pub fn player_inventory_summary(&self, player_id: PlayerId) -> Option<PlayerInventorySummary> {
        self.player(player_id)
            .map(PlayerInventorySummary::from_player)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "compatibility command bridge covers all player-scoped intents until real systems split out"
    )]
    pub fn apply_player_command(
        &mut self,
        player_id: PlayerId,
        command: &PlayerCommand,
    ) -> PlayerScopedCommandOutcome {
        let Some(player) = self.players.get_mut(&player_id) else {
            return PlayerScopedCommandOutcome::UnknownPlayer;
        };

        match *command {
            PlayerCommand::Movement {
                horizontal,
                thrust,
                drill_down,
            } => {
                player.velocity_x = horizontal;
                if thrust {
                    player.velocity_y = -1.0;
                }
                if drill_down {
                    let current_tile = player.tile_position(TILE_SIZE);
                    let target = TilePosition {
                        x: current_tile.x,
                        y: current_tile.y + 1,
                    };
                    self.active_drills.entry(player_id).or_insert(DrillState {
                        target,
                        direction: DrillDirection::Down,
                        progress: 0.0,
                        initial_durability: 1,
                        seconds_per_chip: FIXED_DELTA_SECONDS,
                        sound_timer: 0.0,
                        dust_timer: 0.0,
                    });
                } else {
                    self.active_drills.remove(&player_id);
                }
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::Refuel => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                refuel_amount(player, 1.0);
                self.service_transactions.push(PlayerServiceTransaction {
                    player_id,
                    kind: PlayerTransactionKind::Refuel,
                    credits_before,
                    credits_after: player.credits,
                    cargo_before,
                    cargo_after: player.cargo_used(),
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::Repair => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                repair_amount(player, 1.0);
                self.service_transactions.push(PlayerServiceTransaction {
                    player_id,
                    kind: PlayerTransactionKind::Repair,
                    credits_before,
                    credits_after: player.credits,
                    cargo_before,
                    cargo_after: player.cargo_used(),
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::SellCargo => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                sell_cargo(player);
                self.service_transactions.push(PlayerServiceTransaction {
                    player_id,
                    kind: PlayerTransactionKind::SellCargo,
                    credits_before,
                    credits_after: player.credits,
                    cargo_before,
                    cargo_after: player.cargo_used(),
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::UseScanner => {
                self.scanner_cooldowns.insert(player_id, 1.0);
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::PlaceBomb => {
                if player.bombs == 0 {
                    PlayerScopedCommandOutcome::IgnoredUnavailable
                } else {
                    player.bombs -= 1;
                    self.bombs.push(PlacedBomb {
                        x: player.x,
                        y: TILE_SIZE.mul_add(0.4, player.y),
                        timer_seconds: 2.4,
                    });
                    PlayerScopedCommandOutcome::Applied
                }
            }
            PlayerCommand::PlaceInfrastructure { slot } => {
                let Some(kind) = infrastructure_kind_for_slot(slot) else {
                    return PlayerScopedCommandOutcome::IgnoredUnavailable;
                };
                if !consume_infrastructure_kit(player, kind) {
                    return PlayerScopedCommandOutcome::IgnoredUnavailable;
                }
                self.infrastructure.push(PlacedInfrastructure {
                    kind,
                    position: player.tile_position(TILE_SIZE),
                    durability: 100,
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::BuyUpgrade { index } => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                match index {
                    0 => player.drill_strength = player.drill_strength.saturating_add(1),
                    1 => player.engine_level = player.engine_level.saturating_add(1),
                    2 => player.hull_level = player.hull_level.saturating_add(1),
                    3 => player.cargo_bay_level = player.cargo_bay_level.saturating_add(1),
                    4 => player.fuel_tank_level = player.fuel_tank_level.saturating_add(1),
                    5 => player.scanner_level = player.scanner_level.saturating_add(1),
                    _ => return PlayerScopedCommandOutcome::IgnoredUnavailable,
                }
                self.service_transactions.push(PlayerServiceTransaction {
                    player_id,
                    kind: PlayerTransactionKind::BuyUpgrade,
                    credits_before,
                    credits_after: player.credits,
                    cargo_before,
                    cargo_after: player.cargo_used(),
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::Rescue => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                player.x = 0.0;
                player.y = TILE_SIZE.mul_add(2.0, 0.0);
                player.velocity_x = 0.0;
                player.velocity_y = 0.0;
                player.hull = player.max_hull();
                player.fuel = player.fuel_capacity;
                self.active_drills.remove(&player_id);
                self.service_transactions.push(PlayerServiceTransaction {
                    player_id,
                    kind: PlayerTransactionKind::Rescue,
                    credits_before,
                    credits_after: player.credits,
                    cargo_before,
                    cargo_after: player.cargo_used(),
                });
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::Interact
            | PlayerCommand::Cancel
            | PlayerCommand::Confirm
            | PlayerCommand::SelectUpgrade { .. } => PlayerScopedCommandOutcome::IgnoredUnavailable,
        }
    }

    #[must_use]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    #[must_use]
    pub fn player_snapshots(&self) -> Vec<PlayerSnapshot> {
        self.players
            .iter()
            .map(|(player_id, player)| PlayerSnapshot::from_world_player(*player_id, player, self))
            .collect()
    }

    fn sync_from_legacy_game(&mut self, tick: SimulationTick, game: &GameState) {
        self.simulation_tick = tick;
        self.players.insert(LOCAL_PLAYER_ID, game.player.clone());
        self.hazards.clone_from(&game.hazard_clouds);
        self.bombs.clone_from(&game.placed_bombs);
        self.infrastructure.clone_from(&game.infrastructure);
        if let Some(drill) = game.active_drill {
            self.active_drills.insert(LOCAL_PLAYER_ID, drill);
        } else {
            self.active_drills.remove(&LOCAL_PLAYER_ID);
        }
        self.scanner_cooldowns
            .insert(LOCAL_PLAYER_ID, game.scanner_cooldown_seconds);
        self.authoritative_summary =
            AuthoritativeWorldSummary::from_legacy_game(tick, game, self.players.len());
    }
}

fn world_events_for_applied_command(command: &SequencedPlayerCommand) -> Vec<WorldEvent> {
    let player_id = command.player_id;
    match command.command {
        PlayerCommand::Movement {
            drill_down: true, ..
        } => vec![WorldEvent::ImportantEffectTriggered],
        PlayerCommand::Refuel | PlayerCommand::Repair | PlayerCommand::BuyUpgrade { .. } => {
            vec![WorldEvent::PurchaseCompleted { player_id }]
        }
        PlayerCommand::SellCargo => vec![WorldEvent::CargoChanged { player_id }],
        PlayerCommand::PlaceBomb => vec![WorldEvent::BombPlaced { player_id }],
        PlayerCommand::PlaceInfrastructure { .. } | PlayerCommand::UseScanner => {
            vec![WorldEvent::ImportantEffectTriggered]
        }
        PlayerCommand::Rescue => vec![WorldEvent::RescueTriggered { player_id }],
        PlayerCommand::Movement { .. }
        | PlayerCommand::Interact
        | PlayerCommand::Cancel
        | PlayerCommand::Confirm
        | PlayerCommand::SelectUpgrade { .. } => Vec::new(),
    }
}

const fn infrastructure_kind_for_slot(slot: u8) -> Option<InfrastructureKind> {
    match slot {
        0 => Some(InfrastructureKind::SignalRelay),
        1 => Some(InfrastructureKind::SurveyDrone),
        2 => Some(InfrastructureKind::CargoLift),
        3 => Some(InfrastructureKind::TunnelSupport),
        4 => Some(InfrastructureKind::PumpStation),
        5 => Some(InfrastructureKind::OreProcessor),
        _ => None,
    }
}

const fn consume_infrastructure_kit(player: &mut Player, kind: InfrastructureKind) -> bool {
    let kit_count = match kind {
        InfrastructureKind::SignalRelay => &mut player.signal_relay_kits,
        InfrastructureKind::SurveyDrone => &mut player.survey_drone_kits,
        InfrastructureKind::CargoLift => &mut player.cargo_lift_kits,
        InfrastructureKind::TunnelSupport => &mut player.tunnel_support_kits,
        InfrastructureKind::PumpStation => &mut player.pump_station_kits,
        InfrastructureKind::OreProcessor => &mut player.ore_processor_kits,
    };
    if *kit_count == 0 {
        return false;
    }
    *kit_count -= 1;
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Viewport {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitScreenLayout {
    Single,
    VerticalTwoUp,
    Quad,
}

#[must_use]
pub const fn split_screen_layout(client_count: usize) -> SplitScreenLayout {
    match client_count {
        0 | 1 => SplitScreenLayout::Single,
        2 => SplitScreenLayout::VerticalTwoUp,
        _ => SplitScreenLayout::Quad,
    }
}

#[must_use]
pub fn split_screen_viewports(client_count: usize) -> Vec<Viewport> {
    match split_screen_layout(client_count) {
        SplitScreenLayout::Single => vec![Viewport::new(
            0,
            0,
            DEFAULT_VIEWPORT_WIDTH,
            DEFAULT_VIEWPORT_HEIGHT,
        )],
        SplitScreenLayout::VerticalTwoUp => vec![
            Viewport::new(0, 0, DEFAULT_VIEWPORT_WIDTH / 2, DEFAULT_VIEWPORT_HEIGHT),
            Viewport::new(
                DEFAULT_VIEWPORT_WIDTH / 2,
                0,
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT,
            ),
        ],
        SplitScreenLayout::Quad => vec![
            Viewport::new(
                0,
                0,
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT / 2,
            ),
            Viewport::new(
                DEFAULT_VIEWPORT_WIDTH / 2,
                0,
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT / 2,
            ),
            Viewport::new(
                0,
                DEFAULT_VIEWPORT_HEIGHT / 2,
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT / 2,
            ),
            Viewport::new(
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT / 2,
                DEFAULT_VIEWPORT_WIDTH / 2,
                DEFAULT_VIEWPORT_HEIGHT / 2,
            ),
        ],
    }
}

/// Per-client presentation state used by renderers and future split-screen views.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientView {
    pub client_id: ClientId,
    pub controlled_player_id: PlayerId,
    pub viewport: Viewport,
    pub camera: raylib::prelude::Vector2,
    pub run_mode: RunMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderPlayerPresentation {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub predicted: bool,
    pub correction_plan: CorrectionPlan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderFramePlan {
    pub world_summary: AuthoritativeWorldSummary,
    pub views: Vec<ClientView>,
    pub players: Vec<PlayerSnapshot>,
}

impl RenderFramePlan {
    #[must_use]
    pub fn from_world_and_clients(
        world: &WorldState,
        clients: &BTreeMap<ClientId, ClientState>,
    ) -> Self {
        Self {
            world_summary: world.authoritative_summary().clone(),
            views: clients.values().map(|client| client.view).collect(),
            players: world.player_snapshots(),
        }
    }

    #[must_use]
    pub const fn view_count(&self) -> usize {
        self.views.len()
    }

    #[must_use]
    pub fn player_for_view(&self, view: &ClientView) -> Option<&PlayerSnapshot> {
        self.players
            .iter()
            .find(|player| player.player_id == view.controlled_player_id)
    }

    #[must_use]
    pub fn predicted_player_for_view(
        &self,
        view: &ClientView,
        prediction_plan: &PredictionPresentationPlan,
    ) -> Option<RenderPlayerPresentation> {
        let player = self.player_for_view(view)?;
        let Some(predicted) = prediction_plan
            .local_movement
            .filter(|movement| movement.player_id == view.controlled_player_id)
        else {
            return Some(RenderPlayerPresentation {
                player_id: player.player_id,
                x: player.x,
                y: player.y,
                predicted: false,
                correction_plan: CorrectionPlan::None,
            });
        };
        Some(RenderPlayerPresentation {
            player_id: predicted.player_id,
            x: predicted.x,
            y: predicted.y,
            predicted: true,
            correction_plan: prediction_plan
                .correction
                .map_or(CorrectionPlan::None, |correction| {
                    correction.correction_plan
                }),
        })
    }

    #[must_use]
    pub fn remote_player_presentations(
        &self,
        view: &ClientView,
        prediction_plan: &PredictionPresentationPlan,
    ) -> Vec<RemotePlayerPresentation> {
        if self.views.is_empty() {
            return Vec::new();
        }
        prediction_plan
            .remote_players
            .iter()
            .copied()
            .filter(|player| player.player_id != view.controlled_player_id)
            .collect()
    }
}

impl ClientView {
    #[must_use]
    pub fn from_legacy_game(game: &GameState) -> Self {
        Self {
            client_id: LOCAL_CLIENT_ID,
            controlled_player_id: LOCAL_PLAYER_ID,
            viewport: Viewport::new(0, 0, DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT),
            camera: render_camera(game),
            run_mode: game.run_mode,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTentativeFeedback {
    MovementIntent,
    DrillContact,
    DrillProgressVisual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TentativeFeedbackPresentation {
    MovementVisual,
    DrillContactAudio,
    DrillProgressVisual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TentativeFeedbackChannel {
    Render,
    Audio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TentativeFeedbackOutput {
    pub presentation: TentativeFeedbackPresentation,
    pub channel: TentativeFeedbackChannel,
}

impl TentativeFeedbackPresentation {
    #[must_use]
    pub const fn output(self) -> TentativeFeedbackOutput {
        let channel = match self {
            Self::MovementVisual | Self::DrillProgressVisual => TentativeFeedbackChannel::Render,
            Self::DrillContactAudio => TentativeFeedbackChannel::Audio,
        };
        TentativeFeedbackOutput {
            presentation: self,
            channel,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictionFailureResolution {
    RequestTerrainChunk,
    RequestAuthoritativeSnapshot,
    RollBackLocalEconomy,
    RollBackProgression,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PredictionRecoveryAction {
    RequestTerrainDelta(CompactWorldDelta),
    RequestAuthoritativeSnapshot { player_id: PlayerId },
    RollBackLocalEconomy { player_id: PlayerId },
    RollBackProgression { player_id: PlayerId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PredictionFailureRecoveryPlan {
    pub actions: Vec<PredictionRecoveryAction>,
    pub request_keyframe: bool,
}

impl PredictionFailureRecoveryPlan {
    #[must_use]
    pub fn from_actions(actions: Vec<PredictionRecoveryAction>) -> Self {
        let request_keyframe = actions.iter().any(|action| {
            matches!(
                action,
                PredictionRecoveryAction::RequestAuthoritativeSnapshot { .. }
            )
        });
        Self {
            actions,
            request_keyframe,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorrectionOffset {
    pub x: f32,
    pub y: f32,
}

impl CorrectionOffset {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictionFailure {
    TerrainAlreadyChanged,
    HazardOrRescueChangedState,
    EconomyChangedState,
    ProgressionChangedState,
    CommandRejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorrectionPlan {
    None,
    Smooth,
    Snap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PredictedMovement {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
}

impl PredictedMovement {
    #[must_use]
    pub const fn from_snapshot(snapshot: &PlayerSnapshot) -> Self {
        Self {
            player_id: snapshot.player_id,
            x: snapshot.x,
            y: snapshot.y,
            velocity_x: snapshot.velocity_x,
            velocity_y: snapshot.velocity_y,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReconciledMovement {
    pub predicted: PredictedMovement,
    pub correction_plan: CorrectionPlan,
    pub correction_offset: Option<CorrectionOffset>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorrectedMovementPresentation {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub correction_plan: CorrectionPlan,
}

impl ReconciledMovement {
    #[must_use]
    pub fn corrected_presentation(&self, smoothing_alpha: f32) -> CorrectedMovementPresentation {
        let (x, y) = self.correction_offset.map_or(
            (self.predicted.x, self.predicted.y),
            |offset| match self.correction_plan {
                CorrectionPlan::None => (self.predicted.x, self.predicted.y),
                CorrectionPlan::Smooth => {
                    let alpha = smoothing_alpha.clamp(0.0, 1.0);
                    (
                        offset.x.mul_add(alpha, self.predicted.x),
                        offset.y.mul_add(alpha, self.predicted.y),
                    )
                }
                CorrectionPlan::Snap => (self.predicted.x + offset.x, self.predicted.y + offset.y),
            },
        );
        CorrectedMovementPresentation {
            player_id: self.predicted.player_id,
            x,
            y,
            correction_plan: self.correction_plan,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayedPrediction {
    pub predicted: PredictedMovement,
    pub replayed_command_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemotePlayerPresentation {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub extrapolated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PredictionPresentationPlan {
    pub local_movement: Option<PredictedMovement>,
    pub correction: Option<ReconciledMovement>,
    pub corrected_local_presentation: Option<CorrectedMovementPresentation>,
    pub tentative_feedback: Vec<TentativeFeedbackPresentation>,
    pub remote_players: Vec<RemotePlayerPresentation>,
    pub failure_resolutions: Vec<PredictionFailureResolution>,
    pub feedback_outputs: Vec<TentativeFeedbackOutput>,
}

/// Local prediction/reconciliation bookkeeping for one client.
#[derive(Clone, Debug, Default)]
pub struct ClientPredictionState {
    unacknowledged_commands: Vec<SequencedPlayerCommand>,
    remote_player_snapshots: BTreeMap<PlayerId, Vec<PlayerSnapshot>>,
    prediction_failures: Vec<PredictionFailure>,
    pending_feedback: Vec<LocalTentativeFeedback>,
    correction_offset: Option<CorrectionOffset>,
}

impl ClientPredictionState {
    #[must_use]
    pub fn unacknowledged_commands(&self) -> &[SequencedPlayerCommand] {
        &self.unacknowledged_commands
    }

    #[must_use]
    pub fn replay_commands(&self) -> &[SequencedPlayerCommand] {
        &self.unacknowledged_commands
    }

    #[must_use]
    pub fn correction_plan(error_x: f32, error_y: f32) -> CorrectionPlan {
        let error_squared = error_x.mul_add(error_x, error_y * error_y);
        if error_squared <= 1.0 {
            CorrectionPlan::None
        } else if error_squared <= 256.0 {
            CorrectionPlan::Smooth
        } else {
            CorrectionPlan::Snap
        }
    }

    #[must_use]
    pub fn remote_snapshot_count(&self, player_id: PlayerId) -> usize {
        self.remote_player_snapshots
            .get(&player_id)
            .map_or(0, Vec::len)
    }

    #[must_use]
    pub fn interpolation_delay_seconds(snapshot_spacing_seconds: f32) -> f32 {
        (snapshot_spacing_seconds * 2.0).clamp(
            MIN_INTERPOLATION_DELAY_SECONDS,
            MAX_INTERPOLATION_DELAY_SECONDS,
        )
    }

    #[must_use]
    pub fn should_extrapolate(stall_seconds: f32) -> bool {
        stall_seconds <= EXTRAPOLATION_LIMIT_SECONDS
    }

    #[must_use]
    pub const fn predict_local_movement(
        snapshot: &PlayerSnapshot,
        delta_seconds: f32,
    ) -> PredictedMovement {
        PredictedMovement {
            player_id: snapshot.player_id,
            x: snapshot.velocity_x.mul_add(delta_seconds, snapshot.x),
            y: snapshot.velocity_y.mul_add(delta_seconds, snapshot.y),
            velocity_x: snapshot.velocity_x,
            velocity_y: snapshot.velocity_y,
        }
    }

    #[must_use]
    pub fn reconcile_movement(
        predicted: PredictedMovement,
        authoritative: &PlayerSnapshot,
    ) -> ReconciledMovement {
        let error_x = authoritative.x - predicted.x;
        let error_y = authoritative.y - predicted.y;
        let correction_plan = Self::correction_plan(error_x, error_y);
        let correction_offset = if correction_plan == CorrectionPlan::None {
            None
        } else {
            Some(CorrectionOffset::new(error_x, error_y))
        };

        ReconciledMovement {
            predicted,
            correction_plan,
            correction_offset,
        }
    }

    #[must_use]
    pub fn replay_unacknowledged_movement(
        authoritative: &PlayerSnapshot,
        commands: &[SequencedPlayerCommand],
    ) -> ReplayedPrediction {
        let mut predicted = PredictedMovement::from_snapshot(authoritative);
        let mut replayed_command_count = 0;
        for command in commands {
            if let PlayerCommand::Movement {
                horizontal, thrust, ..
            } = command.command
            {
                replayed_command_count += 1;
                predicted.velocity_x = horizontal;
                if thrust {
                    predicted.velocity_y -= 1.0;
                }
                predicted.x += predicted.velocity_x;
                predicted.y += predicted.velocity_y;
            }
        }
        ReplayedPrediction {
            predicted,
            replayed_command_count,
        }
    }

    #[must_use]
    pub fn remote_player_presentation(
        previous: &PlayerSnapshot,
        next: Option<&PlayerSnapshot>,
        alpha: f32,
        stall_seconds: f32,
    ) -> RemotePlayerPresentation {
        next.map_or_else(
            || {
                let extrapolate = Self::should_extrapolate(stall_seconds);
                let seconds = if extrapolate { stall_seconds } else { 0.0 };
                RemotePlayerPresentation {
                    player_id: previous.player_id,
                    x: previous.velocity_x.mul_add(seconds, previous.x),
                    y: previous.velocity_y.mul_add(seconds, previous.y),
                    extrapolated: extrapolate,
                }
            },
            |next| {
                let blend = alpha.clamp(0.0, 1.0);
                RemotePlayerPresentation {
                    player_id: previous.player_id,
                    x: (next.x - previous.x).mul_add(blend, previous.x),
                    y: (next.y - previous.y).mul_add(blend, previous.y),
                    extrapolated: false,
                }
            },
        )
    }

    #[must_use]
    pub fn predicted_input_lag_seconds(&self) -> f32 {
        let command_count = self
            .unacknowledged_commands
            .len()
            .min(SIMULATION_HZ as usize);
        let seconds_per_command = Duration::from_secs_f32(FIXED_DELTA_SECONDS);
        seconds_per_command
            .saturating_mul(u32::try_from(command_count).expect("command count is capped"))
            .as_secs_f32()
    }

    #[must_use]
    pub fn prediction_failures(&self) -> &[PredictionFailure] {
        &self.prediction_failures
    }

    pub fn note_prediction_failure(&mut self, failure: PredictionFailure) {
        self.prediction_failures.push(failure);
    }

    pub fn clear_prediction_failures(&mut self) {
        self.prediction_failures.clear();
    }

    #[must_use]
    pub fn pending_feedback(&self) -> &[LocalTentativeFeedback] {
        &self.pending_feedback
    }

    pub fn push_feedback(&mut self, feedback: LocalTentativeFeedback) {
        self.pending_feedback.push(feedback);
    }

    pub fn clear_feedback(&mut self) {
        self.pending_feedback.clear();
    }

    #[must_use]
    pub fn tentative_feedback_presentations(&self) -> Vec<TentativeFeedbackPresentation> {
        self.pending_feedback
            .iter()
            .map(|feedback| match feedback {
                LocalTentativeFeedback::MovementIntent => {
                    TentativeFeedbackPresentation::MovementVisual
                }
                LocalTentativeFeedback::DrillContact => {
                    TentativeFeedbackPresentation::DrillContactAudio
                }
                LocalTentativeFeedback::DrillProgressVisual => {
                    TentativeFeedbackPresentation::DrillProgressVisual
                }
            })
            .collect()
    }

    #[must_use]
    pub fn tentative_feedback_outputs(&self) -> Vec<TentativeFeedbackOutput> {
        self.tentative_feedback_presentations()
            .into_iter()
            .map(TentativeFeedbackPresentation::output)
            .collect()
    }

    #[must_use]
    pub fn prediction_failure_resolutions(&self) -> Vec<PredictionFailureResolution> {
        self.prediction_failures
            .iter()
            .map(|failure| match failure {
                PredictionFailure::TerrainAlreadyChanged => {
                    PredictionFailureResolution::RequestTerrainChunk
                }
                PredictionFailure::HazardOrRescueChangedState
                | PredictionFailure::CommandRejected => {
                    PredictionFailureResolution::RequestAuthoritativeSnapshot
                }
                PredictionFailure::EconomyChangedState => {
                    PredictionFailureResolution::RollBackLocalEconomy
                }
                PredictionFailure::ProgressionChangedState => {
                    PredictionFailureResolution::RollBackProgression
                }
            })
            .collect()
    }

    #[must_use]
    pub fn prediction_recovery_actions(
        &self,
        player_id: PlayerId,
        terrain_revisions: &TerrainRevisionTracker,
        tick: SimulationTick,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> Vec<PredictionRecoveryAction> {
        self.prediction_failures
            .iter()
            .map(|failure| match failure {
                PredictionFailure::TerrainAlreadyChanged => {
                    PredictionRecoveryAction::RequestTerrainDelta(terrain_revisions.recovery_delta(
                        tick,
                        terrain_position,
                        known_revision,
                    ))
                }
                PredictionFailure::HazardOrRescueChangedState
                | PredictionFailure::CommandRejected => {
                    PredictionRecoveryAction::RequestAuthoritativeSnapshot { player_id }
                }
                PredictionFailure::EconomyChangedState => {
                    PredictionRecoveryAction::RollBackLocalEconomy { player_id }
                }
                PredictionFailure::ProgressionChangedState => {
                    PredictionRecoveryAction::RollBackProgression { player_id }
                }
            })
            .collect()
    }

    #[must_use]
    pub const fn correction_offset(&self) -> Option<CorrectionOffset> {
        self.correction_offset
    }

    pub const fn set_correction_offset(&mut self, offset: CorrectionOffset) {
        self.correction_offset = Some(offset);
    }

    pub const fn clear_correction_offset(&mut self) {
        self.correction_offset = None;
    }

    pub fn push_remote_snapshot(&mut self, snapshot: PlayerSnapshot) {
        const MAX_REMOTE_SNAPSHOTS: usize = 8;

        let snapshots = self
            .remote_player_snapshots
            .entry(snapshot.player_id)
            .or_default();
        snapshots.push(snapshot);
        if snapshots.len() > MAX_REMOTE_SNAPSHOTS {
            snapshots.remove(0);
        }
    }

    #[must_use]
    pub fn remote_presentations(
        &self,
        alpha: f32,
        stall_seconds: f32,
    ) -> Vec<RemotePlayerPresentation> {
        self.remote_player_snapshots
            .values()
            .filter_map(|snapshots| {
                let latest = snapshots.last()?;
                let (previous, next) = snapshots
                    .get(snapshots.len().saturating_sub(2))
                    .map_or((latest, None), |previous| (previous, Some(latest)));
                Some(Self::remote_player_presentation(
                    previous,
                    next,
                    alpha,
                    stall_seconds,
                ))
            })
            .collect()
    }

    fn remember_commands(&mut self, commands: &[SequencedPlayerCommand]) {
        self.unacknowledged_commands.extend_from_slice(commands);
    }

    pub fn acknowledge_through(&mut self, sequence: InputSequence) {
        self.unacknowledged_commands
            .retain(|command| command.sequence > sequence);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientPresentationField {
    Camera,
    RunMode,
    Viewport,
    Modal,
    LocalMessage,
    LocalAudio,
    MasterVolume,
    Fullscreen,
    SettingsDirty,
    ExitRequested,
    Prediction,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "migration ownership summary intentionally records checklist-style presentation coverage"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientOwnershipSummary {
    pub camera_owned: bool,
    pub menus_owned: bool,
    pub modals_owned: bool,
    pub overlays_owned: bool,
    pub local_messages_owned: bool,
    pub local_audio_owned: bool,
    pub display_settings_owned: bool,
    pub prediction_owned: bool,
}

impl ClientOwnershipSummary {
    #[must_use]
    pub const fn fully_split(self) -> bool {
        self.camera_owned
            && self.menus_owned
            && self.modals_owned
            && self.overlays_owned
            && self.local_messages_owned
            && self.local_audio_owned
            && self.display_settings_owned
            && self.prediction_owned
    }
}

#[must_use]
pub const fn client_presentation_fields() -> [ClientPresentationField; 11] {
    [
        ClientPresentationField::Camera,
        ClientPresentationField::RunMode,
        ClientPresentationField::Viewport,
        ClientPresentationField::Modal,
        ClientPresentationField::LocalMessage,
        ClientPresentationField::LocalAudio,
        ClientPresentationField::MasterVolume,
        ClientPresentationField::Fullscreen,
        ClientPresentationField::SettingsDirty,
        ClientPresentationField::ExitRequested,
        ClientPresentationField::Prediction,
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSequencingPolicy {
    pub source: CommandSource,
    pub authoritative_path: bool,
    pub predicted_locally: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SequencedCommandBatch {
    pub client_id: ClientId,
    pub source: CommandSource,
    pub commands: Vec<SequencedPlayerCommand>,
    pub predicted_locally: bool,
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
    pub view: ClientView,
    pub modal: Option<ModalScreen>,
    pub local_message: String,
    pub local_audio_cues: Vec<SoundCue>,
    prediction: ClientPredictionState,
    next_input_sequence: InputSequence,
}

impl ClientState {
    #[must_use]
    pub fn new(client_id: ClientId, controlled_player_id: PlayerId) -> Self {
        let legacy_game = GameState::new();
        Self {
            client_id,
            controlled_player_id,
            master_volume: 0.8,
            fullscreen: false,
            settings_dirty: false,
            exit_requested: false,
            view: ClientView::from_legacy_game(&legacy_game),
            modal: legacy_game.modal,
            local_message: legacy_game.message.clone(),
            local_audio_cues: legacy_game.sound_cues.clone(),
            prediction: ClientPredictionState::default(),
            next_input_sequence: InputSequence::new(0),
        }
    }

    const fn next_sequence(&mut self) -> InputSequence {
        let sequence = self.next_input_sequence;
        self.next_input_sequence = self.next_input_sequence.next();
        sequence
    }

    #[must_use]
    pub const fn prediction(&self) -> &ClientPredictionState {
        &self.prediction
    }

    #[must_use]
    pub fn ownership_summary(&self) -> ClientOwnershipSummary {
        ClientOwnershipSummary {
            camera_owned: true,
            menus_owned: false,
            modals_owned: true,
            overlays_owned: false,
            local_messages_owned: self.local_message.as_str() == self.local_message.as_str(),
            local_audio_owned: true,
            display_settings_owned: (0.0..=1.0).contains(&self.master_volume),
            prediction_owned: true,
        }
    }

    pub fn sync_presentation_from_legacy_game(&mut self, game: &GameState) {
        self.view = ClientView::from_legacy_game(game);
        self.master_volume = game.master_volume;
        self.fullscreen = game.fullscreen;
        self.settings_dirty = game.settings_dirty;
        self.exit_requested = game.request_exit;
        self.modal = game.modal;
        game.message.clone_into(&mut self.local_message);
        self.local_audio_cues.clone_from(&game.sound_cues);
    }

    fn remember_predicted_commands(&mut self, commands: &[SequencedPlayerCommand]) {
        self.prediction.remember_commands(commands);
    }

    pub fn acknowledge_commands_through(&mut self, sequence: InputSequence) {
        self.prediction.acknowledge_through(sequence);
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
    clients: BTreeMap<ClientId, ClientState>,
    local_client_id: ClientId,
    current_tick: SimulationTick,
    simulation_accumulator: Duration,
    terrain_revisions: TerrainRevisionTracker,
    pending_commands: BTreeMap<SimulationTick, Vec<SequencedPlayerCommand>>,
    pending_events: Vec<WorldEvent>,
}

impl GameSession {
    #[must_use]
    pub const fn compatibility_mode() -> CompatibilityMode {
        CompatibilityMode::SinglePlayerLegacy
    }

    #[must_use]
    pub const fn target_compatibility_mode() -> CompatibilityMode {
        CompatibilityMode::MultiplayerReady
    }

    #[must_use]
    pub const fn planned_state_boundaries() -> [StateBoundary; 12] {
        planned_state_boundaries()
    }

    #[must_use]
    pub const fn planned_transient_effect_boundaries() -> [TransientEffectBoundary; 8] {
        planned_transient_effect_boundaries()
    }

    #[must_use]
    pub const fn planned_player_scoped_systems() -> [PlayerScopedSystem; 8] {
        planned_player_scoped_systems()
    }

    #[must_use]
    pub const fn fixed_tick_audit_items() -> [FixedTickAuditItem; 8] {
        fixed_tick_audit_items()
    }

    #[must_use]
    pub fn fixed_tick_migration_summary() -> FixedTickMigrationSummary {
        FixedTickMigrationSummary::from_items(&fixed_tick_audit_items())
    }

    #[must_use]
    pub const fn snapshot_purposes() -> [SnapshotPurpose; 3] {
        snapshot_purposes()
    }

    #[must_use]
    pub const fn client_presentation_fields() -> [ClientPresentationField; 11] {
        client_presentation_fields()
    }

    #[must_use]
    pub fn split_screen_viewports(client_count: usize) -> Vec<Viewport> {
        split_screen_viewports(client_count)
    }

    #[must_use]
    pub fn world_event_catalog() -> Vec<WorldEvent> {
        vec![
            WorldEvent::CargoChanged {
                player_id: LOCAL_PLAYER_ID,
            },
            WorldEvent::PlayerDamaged {
                player_id: LOCAL_PLAYER_ID,
            },
            WorldEvent::PurchaseCompleted {
                player_id: LOCAL_PLAYER_ID,
            },
            WorldEvent::RescueTriggered {
                player_id: LOCAL_PLAYER_ID,
            },
            WorldEvent::BombPlaced {
                player_id: LOCAL_PLAYER_ID,
            },
            WorldEvent::HazardChanged,
            WorldEvent::ImportantEffectTriggered,
        ]
    }

    #[must_use]
    pub fn new() -> Self {
        let game = GameState::new();
        let world = WorldState::from_legacy_game(&game);
        let local_client = ClientState::default();
        Self {
            game,
            world,
            clients: BTreeMap::from([(LOCAL_CLIENT_ID, local_client)]),
            local_client_id: LOCAL_CLIENT_ID,
            current_tick: SimulationTick::default(),
            simulation_accumulator: Duration::ZERO,
            terrain_revisions: TerrainRevisionTracker::default(),
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
        WorldSnapshot::from_world(self.world.simulation_tick(), &self.world)
    }

    #[must_use]
    pub fn local_client(&self) -> &ClientState {
        self.clients
            .get(&self.local_client_id)
            .expect("local client exists in game session")
    }

    fn local_client_mut(&mut self) -> &mut ClientState {
        self.clients
            .get_mut(&self.local_client_id)
            .expect("local client exists in game session")
    }

    #[must_use]
    pub fn local_view(&self) -> &ClientView {
        &self.local_client().view
    }

    #[must_use]
    pub fn client_views(&self) -> Vec<&ClientView> {
        self.clients.values().map(|client| &client.view).collect()
    }

    #[must_use]
    pub fn render_views(&self) -> Vec<&ClientView> {
        self.client_views()
    }

    #[must_use]
    pub fn render_frame_plan(&self) -> RenderFramePlan {
        RenderFramePlan::from_world_and_clients(&self.world, &self.clients)
    }

    #[must_use]
    pub fn predicted_local_movement(&self, delta_seconds: f32) -> Option<PredictedMovement> {
        let view = self.local_view();
        let player = self.world.player(view.controlled_player_id)?;
        let snapshot =
            PlayerSnapshot::from_world_player(view.controlled_player_id, player, &self.world);
        Some(ClientPredictionState::predict_local_movement(
            &snapshot,
            delta_seconds,
        ))
    }

    #[must_use]
    pub fn prediction_presentation_plan(
        &self,
        authoritative_snapshot: Option<&PlayerSnapshot>,
        delta_seconds: f32,
        remote_alpha: f32,
        remote_stall_seconds: f32,
    ) -> PredictionPresentationPlan {
        let prediction = self.local_client().prediction();
        let local_movement = authoritative_snapshot.map_or_else(
            || self.predicted_local_movement(delta_seconds),
            |authoritative| {
                Some(
                    ClientPredictionState::replay_unacknowledged_movement(
                        authoritative,
                        prediction.unacknowledged_commands(),
                    )
                    .predicted,
                )
            },
        );
        let correction =
            local_movement
                .zip(authoritative_snapshot)
                .map(|(predicted, authoritative)| {
                    ClientPredictionState::reconcile_movement(predicted, authoritative)
                });
        PredictionPresentationPlan {
            local_movement,
            correction,
            corrected_local_presentation: correction
                .map(|correction| correction.corrected_presentation(0.5)),
            tentative_feedback: prediction.tentative_feedback_presentations(),
            remote_players: prediction.remote_presentations(remote_alpha, remote_stall_seconds),
            failure_resolutions: prediction.prediction_failure_resolutions(),
            feedback_outputs: prediction.tentative_feedback_outputs(),
        }
    }

    #[must_use]
    pub fn prediction_recovery_actions(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> Vec<PredictionRecoveryAction> {
        self.local_client()
            .prediction()
            .prediction_recovery_actions(
                self.local_client().controlled_player_id,
                &self.terrain_revisions,
                self.current_tick,
                terrain_position,
                known_revision,
            )
    }

    #[must_use]
    pub fn prediction_failure_recovery_plan(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> PredictionFailureRecoveryPlan {
        PredictionFailureRecoveryPlan::from_actions(
            self.prediction_recovery_actions(terrain_position, known_revision),
        )
    }

    #[must_use]
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    #[must_use]
    pub const fn current_tick(&self) -> SimulationTick {
        self.current_tick
    }

    #[must_use]
    pub const fn simulation_accumulator(&self) -> Duration {
        self.simulation_accumulator
    }

    #[must_use]
    pub const fn terrain_revisions(&self) -> &TerrainRevisionTracker {
        &self.terrain_revisions
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
        self.world.set_simulation_tick(self.current_tick);
    }

    #[must_use]
    pub const fn keyframe_interval_ticks() -> u64 {
        KEYFRAME_INTERVAL_TICKS
    }

    fn maybe_emit_keyframe_event(&mut self) {
        let tick = self.current_tick.get();
        if tick > 0 && tick.is_multiple_of(KEYFRAME_INTERVAL_TICKS) {
            self.push_event(WorldEvent::SnapshotKeyframeReady {
                tick: self.current_tick,
            });
        }
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

    pub fn apply_settings(&mut self, settings: SettingsFile) {
        let local_client = self.local_client_mut();
        local_client.master_volume = settings.master_volume;
        local_client.fullscreen = settings.fullscreen;
        self.sync_client_settings_to_legacy_game();
    }

    #[must_use]
    pub fn current_settings(&self) -> SettingsFile {
        SettingsFile {
            master_volume: self.local_client().master_volume,
            fullscreen: self.local_client().fullscreen,
        }
    }

    #[must_use]
    pub fn should_exit(&self) -> bool {
        self.local_client().exit_requested || self.game.request_exit
    }

    #[must_use]
    pub fn master_volume(&self) -> f32 {
        self.local_client().master_volume
    }

    #[must_use]
    pub fn fullscreen(&self) -> bool {
        self.local_client().fullscreen
    }

    pub fn take_settings_dirty(&mut self) -> bool {
        let legacy_dirty = self.game.take_settings_dirty();
        let local_client = self.local_client_mut();
        let client_dirty = local_client.settings_dirty;
        local_client.settings_dirty = false;
        legacy_dirty || client_dirty
    }

    fn sync_client_settings_from_legacy_game(&mut self) {
        let local_client_id = self.local_client_id;
        let game_master_volume = self.game.master_volume;
        let game_fullscreen = self.game.fullscreen;
        let game_settings_dirty = self.game.settings_dirty;
        let game_request_exit = self.game.request_exit;
        let settings_changed;
        let exit_requested;
        {
            let game = self.game.clone();
            let local_client = self.local_client_mut();
            settings_changed = (local_client.master_volume - game_master_volume).abs()
                > f32::EPSILON
                || local_client.fullscreen != game_fullscreen
                || game_settings_dirty;
            exit_requested = game_request_exit && !local_client.exit_requested;

            local_client.sync_presentation_from_legacy_game(&game);
            local_client.settings_dirty |= game_settings_dirty;
            local_client.exit_requested |= game_request_exit;
        }

        if settings_changed {
            self.push_event(WorldEvent::ClientSettingsChanged {
                client_id: local_client_id,
            });
        }
        if exit_requested {
            self.push_event(WorldEvent::ClientExitRequested {
                client_id: local_client_id,
            });
        }
    }

    fn sync_client_settings_to_legacy_game(&mut self) {
        let master_volume = self.local_client().master_volume;
        let fullscreen = self.local_client().fullscreen;
        let settings_dirty = self.local_client().settings_dirty;
        self.game.master_volume = master_volume;
        self.game.fullscreen = fullscreen;
        self.game.settings_dirty = settings_dirty;
    }

    pub fn sequence_local_commands(
        &mut self,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        self.sequence_client_commands(self.local_client_id, commands)
    }

    pub fn sequence_client_commands(
        &mut self,
        client_id: ClientId,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        self.sequence_client_commands_from_source(client_id, CommandSource::Keyboard, commands)
            .commands
    }

    pub fn sequence_client_commands_from_source(
        &mut self,
        client_id: ClientId,
        source: CommandSource,
        commands: Vec<PlayerCommand>,
    ) -> SequencedCommandBatch {
        let sequenced = self.sequence_commands_for_client(client_id, commands);
        let predicted_locally = matches!(
            source,
            CommandSource::Keyboard | CommandSource::Gamepad | CommandSource::SplitScreenClient
        );
        if predicted_locally {
            self.clients
                .get_mut(&client_id)
                .expect("client exists in game session")
                .remember_predicted_commands(&sequenced);
        }
        self.buffer_commands(sequenced.clone());
        SequencedCommandBatch {
            client_id,
            source,
            commands: sequenced,
            predicted_locally,
        }
    }

    #[must_use]
    pub const fn command_source_policy(source: CommandSource) -> SourceSequencingPolicy {
        SourceSequencingPolicy {
            source,
            authoritative_path: source.uses_authoritative_command_path(),
            predicted_locally: matches!(
                source,
                CommandSource::Keyboard | CommandSource::Gamepad | CommandSource::SplitScreenClient
            ),
        }
    }

    fn sequence_commands_for_client(
        &mut self,
        client_id: ClientId,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        let target_tick = self.current_tick;
        let client = self
            .clients
            .get_mut(&client_id)
            .expect("client exists in game session");
        let player_id = client.controlled_player_id;

        commands
            .into_iter()
            .map(|command| SequencedPlayerCommand {
                player_id,
                sequence: client.next_sequence(),
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

    pub fn process_authoritative_commands_for_tick(&mut self, tick: SimulationTick) -> usize {
        let tick_commands = self.drain_commands_for_tick(tick);
        let command_count = tick_commands.len();
        self.push_event(WorldEvent::CommandsProcessed {
            tick,
            command_count,
        });

        let controlled_player_id = self.local_client().controlled_player_id;
        let mut latest_local_sequence = None;
        let mut changed_players = BTreeSet::new();
        let mut command_events = Vec::new();
        for command in &tick_commands {
            if self
                .world
                .apply_player_command(command.player_id, &command.command)
                == PlayerScopedCommandOutcome::Applied
            {
                changed_players.insert(command.player_id);
                command_events.extend(world_events_for_applied_command(command));
            }
            if command.player_id == controlled_player_id {
                latest_local_sequence = latest_local_sequence.max(Some(command.sequence));
            }
        }

        for player_id in changed_players {
            self.push_event(WorldEvent::PlayerChanged { player_id });
        }
        for event in command_events {
            self.push_event(event);
        }

        if let Some(sequence) = latest_local_sequence {
            self.acknowledge_client_commands_through(self.local_client_id, sequence);
        }

        command_count
    }

    pub fn acknowledge_client_commands_through(
        &mut self,
        client_id: ClientId,
        sequence: InputSequence,
    ) {
        self.clients
            .get_mut(&client_id)
            .expect("client exists in game session")
            .acknowledge_commands_through(sequence);
    }

    pub fn apply_command_acknowledgement(&mut self, acknowledgement: &CommandAcknowledgement) {
        self.acknowledge_client_commands_through(
            acknowledgement.client_id,
            acknowledgement.acknowledged_sequence,
        );
    }

    pub fn apply_command_rejection(&mut self, rejection: &CommandRejection) {
        if let Some(client) = self.clients.get_mut(&rejection.client_id) {
            client
                .prediction
                .note_prediction_failure(PredictionFailure::CommandRejected);
        }
    }

    pub fn update_legacy(&mut self, input: PlayerInput, delta_seconds: f32) {
        let fixed_steps = self.accumulate_frame_delta(delta_seconds);
        for _ in 0..fixed_steps {
            let tick = self.current_tick;
            self.process_authoritative_commands_for_tick(tick);
            self.advance_tick();
            self.push_event(WorldEvent::TickAdvanced {
                tick: self.current_tick,
            });
            self.maybe_emit_keyframe_event();
        }
        self.sync_client_settings_to_legacy_game();
        let previous_message = self.game.message.clone();
        let previous_player = self.game.player.clone();
        let previous_request_exit = self.game.request_exit;
        self.game.update(input, delta_seconds);
        self.capture_legacy_events(&previous_message, &previous_player, previous_request_exit);
        self.sync_client_settings_from_legacy_game();
        self.world
            .sync_from_legacy_game(self.current_tick, &self.game);
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
                client_id: self.local_client_id,
            });
        }
        if self.game.visual_changes.full_terrain_refresh {
            self.push_event(WorldEvent::TerrainRefreshRequested);
        }
        if !self.game.visual_changes.changed_tiles.is_empty() {
            let positions = self.game.visual_changes.changed_tiles.clone();
            let revisions = self.terrain_revisions.mark_tiles_changed(positions.clone());
            self.push_event(WorldEvent::TerrainTilesChanged { positions });
            if !revisions.is_empty() {
                self.push_event(WorldEvent::TerrainChunksChanged { revisions });
            }
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

    use crate::{
        game_state::{DrillState, GameState, InfrastructureKind, ModalScreen, SoundCue},
        multiplayer::{
            CommandAcknowledgement, CommandRejection, CommandSource, InputSequence,
            LOCAL_CLIENT_ID, LOCAL_PLAYER_ID, NetworkDeltaPayload, PlayerCommand, PlayerId,
            ProtocolMessage, SequencedPlayerCommand, SimulationTick,
        },
    };

    use super::{ClientState, GameSession, WorldState};

    #[test]
    fn session_starts_with_single_player_compatibility_client() {
        let session = GameSession::new();

        assert_eq!(session.local_client().client_id, LOCAL_CLIENT_ID);
        assert_eq!(session.local_client().controlled_player_id, LOCAL_PLAYER_ID);
    }

    #[test]
    fn client_view_tracks_legacy_view_identity() {
        let session = GameSession::new();

        assert_eq!(session.local_view().client_id, LOCAL_CLIENT_ID);
        assert_eq!(session.local_view().controlled_player_id, LOCAL_PLAYER_ID);
        assert_eq!(session.local_view().run_mode, session.game().run_mode);
        assert_eq!(session.local_view().viewport.width, 1280);
        assert_eq!(session.local_view().viewport.height, 720);
    }

    #[test]
    fn planned_state_boundaries_identify_world_and_client_domains() {
        let boundaries = GameSession::planned_state_boundaries();

        assert!(boundaries.iter().any(|boundary| {
            boundary.name == "terrain" && boundary.domain == super::StateDomain::AuthoritativeWorld
        }));
        assert!(boundaries.iter().any(|boundary| {
            boundary.name == "camera"
                && boundary.domain == super::StateDomain::LocalClientPresentation
        }));
        assert_eq!(
            GameSession::compatibility_mode(),
            super::CompatibilityMode::SinglePlayerLegacy
        );
        assert_eq!(
            GameSession::target_compatibility_mode(),
            super::CompatibilityMode::MultiplayerReady
        );
    }

    #[test]
    fn session_exposes_local_client_view_collection() {
        let session = GameSession::new();

        let views = session.client_views();

        assert_eq!(session.client_count(), 1);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].client_id, LOCAL_CLIENT_ID);
        assert_eq!(session.render_views().len(), views.len());
    }

    #[test]
    fn render_frame_plan_uses_world_and_client_state() {
        let session = GameSession::new();

        let plan = session.render_frame_plan();

        assert_eq!(plan.world_summary.tick, session.world().simulation_tick());
        assert_eq!(
            plan.world_summary.player_count,
            session.world().player_count()
        );
        assert_eq!(plan.view_count(), session.client_count());
        assert_eq!(plan.views[0].controlled_player_id, LOCAL_PLAYER_ID);
    }

    #[test]
    fn render_frame_plan_exposes_per_view_player_state() {
        let mut session = GameSession::new();
        session
            .world
            .set_scanner_cooldown_seconds(LOCAL_PLAYER_ID, 2.0);

        let plan = session.render_frame_plan();
        let player = plan
            .player_for_view(&plan.views[0])
            .expect("controlled player snapshot exists");

        assert_eq!(player.player_id, LOCAL_PLAYER_ID);
        assert_eq!(player.cargo_used, session.game().player.cargo_used());
        assert!((player.scanner_cooldown_seconds - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn render_frame_plan_uses_predicted_local_player_presentation() {
        let mut session = GameSession::new();
        session
            .world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .velocity_x = 10.0;
        let plan = session.render_frame_plan();
        let prediction_plan = session.prediction_presentation_plan(None, 0.5, 0.5, 0.0);

        let player = plan
            .predicted_player_for_view(&plan.views[0], &prediction_plan)
            .expect("predicted local player presentation exists");

        assert_eq!(player.player_id, LOCAL_PLAYER_ID);
        assert!(player.predicted);
        assert!((player.x - (session.game().player.x + 5.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn render_frame_plan_exposes_remote_prediction_presentations() {
        let mut session = GameSession::new();
        let remote_player_id = PlayerId::new(99);
        session
            .local_client_mut()
            .prediction
            .push_remote_snapshot(super::PlayerSnapshot {
                player_id: remote_player_id,
                x: 20.0,
                y: 30.0,
                velocity_x: 2.0,
                velocity_y: 0.0,
                fuel: 1.0,
                hull: 1.0,
                credits: 0,
                cargo_used: 0,
                scanner_cooldown_seconds: 0.0,
            });
        let plan = session.render_frame_plan();
        let prediction_plan = session.prediction_presentation_plan(None, 0.5, 0.5, 0.1);

        let remotes = plan.remote_player_presentations(&plan.views[0], &prediction_plan);

        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].player_id, remote_player_id);
    }

    #[test]
    fn session_projects_predicted_local_movement_from_world_state() {
        let mut session = GameSession::new();
        session
            .world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .velocity_x = 10.0;

        let predicted = session
            .predicted_local_movement(0.5)
            .expect("local player prediction exists");

        assert_eq!(predicted.player_id, LOCAL_PLAYER_ID);
        assert!((predicted.x - (session.game().player.x + 5.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn session_builds_prediction_presentation_plan() {
        let mut session = GameSession::new();
        session
            .local_client_mut()
            .prediction
            .push_feedback(super::LocalTentativeFeedback::DrillContact);
        let authoritative = session.world_snapshot().players[0].clone();

        let plan = session.prediction_presentation_plan(Some(&authoritative), 0.5, 0.5, 0.0);

        assert!(plan.local_movement.is_some());
        assert!(plan.correction.is_some());
        assert!(plan.corrected_local_presentation.is_some());
        assert_eq!(
            plan.tentative_feedback,
            vec![super::TentativeFeedbackPresentation::DrillContactAudio]
        );
        assert_eq!(
            plan.feedback_outputs,
            vec![super::TentativeFeedbackOutput {
                presentation: super::TentativeFeedbackPresentation::DrillContactAudio,
                channel: super::TentativeFeedbackChannel::Audio,
            }]
        );
    }

    #[test]
    fn session_world_tracks_legacy_local_player() {
        let session = GameSession::new();

        assert_eq!(session.world().player_count(), 1);
        assert!(session.world().player(LOCAL_PLAYER_ID).is_some());
    }

    #[test]
    fn world_state_tracks_authoritative_simulation_tick() {
        let mut session = GameSession::new();

        assert_eq!(session.world().simulation_tick(), SimulationTick::default());
        session.advance_tick();
        assert_eq!(session.world().simulation_tick(), SimulationTick::new(1));
        assert_eq!(
            session.world_snapshot().tick,
            session.world().simulation_tick()
        );
    }

    #[test]
    fn world_state_summarizes_authoritative_legacy_domains() {
        let session = GameSession::new();
        let summary = session.world().authoritative_summary();

        assert_eq!(summary.tick, session.world().simulation_tick());
        assert_eq!(summary.player_count, 1);
        assert_eq!(summary.terrain_width, session.game().terrain.width());
        assert_eq!(summary.terrain_height, session.game().terrain.height());
        assert_eq!(summary.bomb_count, session.game().placed_bombs.len());
        assert_eq!(
            session.world().bomb_count(),
            session.game().placed_bombs.len()
        );
        assert_eq!(
            session.world().hazard_count(),
            session.game().hazard_clouds.len()
        );
        assert_eq!(
            session.world().infrastructure_count(),
            session.game().infrastructure.len()
        );
        assert_eq!(
            summary.infrastructure_count,
            session.game().infrastructure.len()
        );
    }

    #[test]
    fn client_state_catalogs_presentation_fields() {
        let fields = GameSession::client_presentation_fields();

        assert!(fields.contains(&super::ClientPresentationField::Camera));
        assert!(fields.contains(&super::ClientPresentationField::RunMode));
        assert!(fields.contains(&super::ClientPresentationField::Prediction));
        assert!(fields.contains(&super::ClientPresentationField::Modal));
        assert!(fields.contains(&super::ClientPresentationField::LocalMessage));
        assert!(fields.contains(&super::ClientPresentationField::LocalAudio));
        assert!(fields.contains(&super::ClientPresentationField::ExitRequested));
    }

    #[test]
    fn client_state_reports_presentation_ownership_migration_status() {
        let client = ClientState::default();
        let ownership = client.ownership_summary();

        assert!(ownership.camera_owned);
        assert!(ownership.modals_owned);
        assert!(ownership.local_messages_owned);
        assert!(ownership.local_audio_owned);
        assert!(ownership.display_settings_owned);
        assert!(ownership.prediction_owned);
        assert!(!ownership.menus_owned);
        assert!(!ownership.overlays_owned);
        assert!(!ownership.fully_split());
    }

    #[test]
    fn client_state_owns_local_presentation_mirrors() {
        let mut game = GameState::new();
        game.modal = Some(ModalScreen::Help);
        "Client-local toast".clone_into(&mut game.message);
        game.sound_cues.push(SoundCue::Ui);
        let mut client = ClientState::new(LOCAL_CLIENT_ID, LOCAL_PLAYER_ID);

        client.sync_presentation_from_legacy_game(&game);

        assert_eq!(client.modal, Some(ModalScreen::Help));
        assert_eq!(client.local_message, "Client-local toast");
        assert_eq!(client.local_audio_cues.len(), 1);
    }

    #[test]
    fn world_state_applies_player_scoped_drilling_intent() {
        let mut world = WorldState::from_legacy_game(&GameState::new());

        let outcome = world.apply_player_command(
            LOCAL_PLAYER_ID,
            &PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: true,
            },
        );

        assert_eq!(outcome, super::PlayerScopedCommandOutcome::Applied);
        assert!(world.active_drill(LOCAL_PLAYER_ID).is_some());

        world.apply_player_command(
            LOCAL_PLAYER_ID,
            &PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: false,
            },
        );

        assert!(world.active_drill(LOCAL_PLAYER_ID).is_none());
    }

    #[test]
    fn world_state_applies_player_scoped_bomb_and_infrastructure_placement() {
        let mut game = GameState::new();
        game.player.bombs = 1;
        game.player.signal_relay_kits = 1;
        let mut world = WorldState::from_legacy_game(&game);

        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::PlaceBomb),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(world.bomb_count(), 1);
        assert_eq!(
            world.player(LOCAL_PLAYER_ID).expect("player exists").bombs,
            0
        );

        assert_eq!(
            world.apply_player_command(
                LOCAL_PLAYER_ID,
                &PlayerCommand::PlaceInfrastructure { slot: 0 },
            ),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(world.infrastructure_count(), 1);
        assert_eq!(
            world.infrastructure()[0].kind,
            InfrastructureKind::SignalRelay
        );
        assert_eq!(
            world
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .signal_relay_kits,
            0
        );
    }

    #[test]
    fn world_state_records_player_scoped_service_transactions() {
        let mut game = GameState::new();
        game.player.fuel = 1.0;
        game.player.hull = 1.0;
        game.player.credits = 500;
        game.player.add_cargo(crate::terrain::MineralKind::Copper);
        let mut world = WorldState::from_legacy_game(&game);

        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::Refuel),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::Repair),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::SellCargo),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::BuyUpgrade { index: 0 }),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::Rescue),
            super::PlayerScopedCommandOutcome::Applied
        );

        assert_eq!(world.service_transactions().len(), 5);
        assert_eq!(
            world.service_transactions()[0].kind,
            super::PlayerTransactionKind::Refuel
        );
        assert!(
            world.service_transactions()[0].credits_after
                < world.service_transactions()[0].credits_before
        );
        assert_eq!(
            world.service_transactions()[2].kind,
            super::PlayerTransactionKind::SellCargo
        );
        assert_eq!(world.service_transactions()[2].cargo_after, 0);
        assert_eq!(
            world.service_transactions()[4].kind,
            super::PlayerTransactionKind::Rescue
        );
    }

    #[test]
    fn world_state_reports_authoritative_ownership_migration_status() {
        let world = WorldState::from_legacy_game(&GameState::new());
        let ownership = world.ownership_summary();

        assert!(ownership.players_owned);
        assert!(ownership.hazards_owned);
        assert!(ownership.bombs_owned);
        assert!(ownership.infrastructure_owned);
        assert!(ownership.simulation_tick_owned);
        assert!(!ownership.terrain_owned);
        assert!(!ownership.fully_split());
    }

    #[test]
    fn world_state_exposes_mutable_player_lookup() {
        let mut world = WorldState::from_legacy_game(&GameState::new());

        world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .credits = 123;

        assert_eq!(
            world
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .credits,
            123
        );
    }

    #[test]
    fn compatibility_world_applies_player_scoped_commands_to_selected_player() {
        let mut world = WorldState::from_legacy_game(&GameState::new());

        assert_eq!(
            world.apply_player_command(
                LOCAL_PLAYER_ID,
                &PlayerCommand::Movement {
                    horizontal: 0.75,
                    thrust: true,
                    drill_down: false,
                },
            ),
            super::PlayerScopedCommandOutcome::Applied
        );
        let velocity_x = world
            .player(LOCAL_PLAYER_ID)
            .expect("player exists")
            .velocity_x;
        assert!((velocity_x - 0.75).abs() < f32::EPSILON);
        assert_eq!(
            world.apply_player_command(PlayerId::new(999), &PlayerCommand::Refuel),
            super::PlayerScopedCommandOutcome::UnknownPlayer
        );
    }

    #[test]
    fn compatibility_world_applies_player_scoped_resource_commands() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let player = world.player_mut(LOCAL_PLAYER_ID).expect("player exists");
        player.fuel = 1.0;
        player.hull = 1.0;
        player.bombs = 1;

        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::Refuel),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::Repair),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::PlaceBomb),
            super::PlayerScopedCommandOutcome::Applied
        );
        let player = world.player(LOCAL_PLAYER_ID).expect("player exists");
        assert!((player.fuel - player.fuel_capacity).abs() < f32::EPSILON);
        assert!((player.hull - player.max_hull()).abs() < f32::EPSILON);
        assert_eq!(player.bombs, 0);
    }

    #[test]
    fn compatibility_world_tracks_per_player_active_drill_and_scanner_cooldown() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let drill = DrillState {
            target: crate::terrain::TilePosition { x: 1, y: 2 },
            direction: crate::game_state::DrillDirection::Down,
            progress: 0.5,
            initial_durability: 3,
            seconds_per_chip: 0.25,
            sound_timer: 0.0,
            dust_timer: 0.0,
        };

        world.set_active_drill(LOCAL_PLAYER_ID, Some(drill));
        world.set_scanner_cooldown_seconds(LOCAL_PLAYER_ID, 2.0);

        assert_eq!(
            world
                .active_drill(LOCAL_PLAYER_ID)
                .expect("drill set")
                .target
                .y,
            2
        );
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::UseScanner),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert!(
            world
                .scanner_cooldown_seconds(LOCAL_PLAYER_ID)
                .expect("cooldown set")
                > 0.0
        );
        world.set_active_drill(LOCAL_PLAYER_ID, None);
        assert!(world.active_drill(LOCAL_PLAYER_ID).is_none());
    }

    #[test]
    fn compatibility_world_summarizes_inventory_and_applies_upgrade_intent() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let before = world
            .player_inventory_summary(LOCAL_PLAYER_ID)
            .expect("player summary");

        assert_eq!(before.cargo_used, 0);
        assert_eq!(before.credits, 0);
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::BuyUpgrade { index: 0 }),
            super::PlayerScopedCommandOutcome::Applied
        );
        let after = world
            .player_inventory_summary(LOCAL_PLAYER_ID)
            .expect("player summary");
        assert!(after.upgrade_level_total > before.upgrade_level_total);
    }

    #[test]
    fn planned_transient_effect_boundaries_identify_local_and_world_effects() {
        let boundaries = GameSession::planned_transient_effect_boundaries();

        assert!(boundaries.iter().any(|boundary| {
            boundary.name == "camera_shake"
                && boundary.domain == super::TransientEffectDomain::LocalClientPresentation
        }));
        assert!(boundaries.iter().any(|boundary| {
            boundary.name == "hazard_clouds"
                && boundary.domain == super::TransientEffectDomain::GameplayRelevantWorld
        }));
    }

    #[test]
    fn planned_player_scoped_systems_cover_legacy_player_logic() {
        let systems = GameSession::planned_player_scoped_systems();

        assert!(systems.contains(&super::PlayerScopedSystem::Movement));
        assert!(systems.contains(&super::PlayerScopedSystem::Drilling));
        assert!(systems.contains(&super::PlayerScopedSystem::Cargo));
        assert!(systems.contains(&super::PlayerScopedSystem::EconomyService));
    }

    #[test]
    fn fixed_tick_audit_tracks_remaining_variable_delta_systems() {
        let audit_items = GameSession::fixed_tick_audit_items();

        assert!(audit_items.iter().any(|item| {
            item.system == "physics"
                && item.status == super::FixedTickMigrationStatus::CompatibilityFixedStep
                && item.plan == super::FixedTickMigrationPlan::MigrateToAuthoritativeTick
        }));
        assert!(audit_items.iter().any(|item| {
            item.system == "animations"
                && item.plan == super::FixedTickMigrationPlan::KeepVariablePresentationOnly
        }));
        assert!(audit_items.iter().any(|item| {
            item.system == "drilling_progress"
                && item.status == super::FixedTickMigrationStatus::CompatibilityFixedStep
        }));
    }

    #[test]
    fn fixed_tick_audit_summary_counts_authoritative_and_presentation_work() {
        let summary = GameSession::fixed_tick_migration_summary();

        assert_eq!(summary.fixed_ready, 1);
        assert_eq!(summary.presentation_exemptions, 1);
        assert!(summary.authoritative_migrations >= 1);
        assert!(summary.unresolved_variable_delta > 0);
        assert!(!summary.audit_complete());
    }

    #[test]
    fn world_event_catalog_covers_future_authoritative_events() {
        let events = GameSession::world_event_catalog();

        assert!(
            events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::CargoChanged { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::PlayerDamaged { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::PurchaseCompleted { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::BombPlaced { .. }))
        );
    }

    #[test]
    fn world_delta_compacts_events_for_network_sync() {
        let delta = super::WorldDelta::new(
            SimulationTick::new(8),
            vec![super::WorldEvent::PlayerChanged {
                player_id: LOCAL_PLAYER_ID,
            }],
        );

        assert_eq!(
            delta.compact_network_delta(),
            super::CompactWorldDelta::Players {
                tick: SimulationTick::new(8),
                players: vec![LOCAL_PLAYER_ID],
            }
        );

        let compact_delta = delta.compact_network_delta();
        let payload = compact_delta.network_payload();
        assert_eq!(
            payload,
            NetworkDeltaPayload::Players {
                players: vec![LOCAL_PLAYER_ID]
            }
        );
        assert_eq!(
            compact_delta.protocol_message(),
            ProtocolMessage::WorldDelta {
                tick: SimulationTick::new(8),
                payload
            }
        );

        let keyframe_delta = super::WorldDelta::new(
            SimulationTick::new(10),
            vec![super::WorldEvent::TerrainRefreshRequested],
        );
        assert_eq!(
            keyframe_delta.compact_network_delta(),
            super::CompactWorldDelta::KeyframeRequired {
                tick: SimulationTick::new(10),
            }
        );
    }

    #[test]
    fn snapshot_purposes_keep_save_network_and_render_boundaries_separate() {
        let purposes = GameSession::snapshot_purposes();

        assert!(purposes.contains(&super::SnapshotPurpose::SaveFile));
        assert!(purposes.contains(&super::SnapshotPurpose::NetworkSync));
        assert!(purposes.contains(&super::SnapshotPurpose::RenderSync));
    }

    #[test]
    fn split_screen_viewports_cover_single_two_up_and_quad_layouts() {
        assert_eq!(
            super::split_screen_layout(1),
            super::SplitScreenLayout::Single
        );
        assert_eq!(
            super::split_screen_layout(2),
            super::SplitScreenLayout::VerticalTwoUp
        );
        assert_eq!(
            super::split_screen_layout(3),
            super::SplitScreenLayout::Quad
        );
        assert_eq!(GameSession::split_screen_viewports(1).len(), 1);
        assert_eq!(GameSession::split_screen_viewports(2).len(), 2);
        assert_eq!(GameSession::split_screen_viewports(4).len(), 4);
    }

    #[test]
    fn world_snapshot_contains_tick_and_players() {
        let session = GameSession::new();

        let snapshot = session.world_snapshot();

        assert_eq!(snapshot.tick, session.current_tick());
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(
            snapshot.network_snapshot().players[0].player_id,
            LOCAL_PLAYER_ID
        );
        assert_eq!(
            snapshot.keyframe_message(),
            ProtocolMessage::SnapshotKeyframe {
                snapshot: snapshot.network_snapshot()
            }
        );
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
    fn client_commands_share_authoritative_session_path() {
        let mut session = GameSession::new();

        let commands =
            session.sequence_client_commands(LOCAL_CLIENT_ID, vec![PlayerCommand::Interact]);

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(commands[0].sequence.get(), 0);
        assert_eq!(session.pending_command_count(session.current_tick()), 1);
        assert_eq!(
            session
                .local_client()
                .prediction()
                .unacknowledged_commands()
                .len(),
            1
        );
    }

    #[test]
    fn command_sources_sequence_through_authoritative_path_with_prediction_policy() {
        let mut session = GameSession::new();
        let local = session.sequence_client_commands_from_source(
            LOCAL_CLIENT_ID,
            CommandSource::Gamepad,
            vec![PlayerCommand::Interact],
        );
        let replay = session.sequence_client_commands_from_source(
            LOCAL_CLIENT_ID,
            CommandSource::Replay,
            vec![PlayerCommand::Confirm],
        );

        assert_eq!(local.commands.len(), 1);
        assert!(local.predicted_locally);
        assert!(!replay.predicted_locally);
        assert_eq!(session.pending_command_count(session.current_tick()), 2);
        assert_eq!(
            session
                .local_client()
                .prediction()
                .unacknowledged_commands()
                .len(),
            1
        );
        assert_eq!(
            GameSession::command_source_policy(CommandSource::OnlineClient),
            super::SourceSequencingPolicy {
                source: CommandSource::OnlineClient,
                authoritative_path: true,
                predicted_locally: false,
            }
        );
    }

    #[test]
    fn acknowledged_commands_are_removed_from_prediction_buffer() {
        let mut session = GameSession::new();
        let commands = session.sequence_local_commands(vec![PlayerCommand::Interact]);

        session.acknowledge_client_commands_through(LOCAL_CLIENT_ID, commands[0].sequence);

        assert!(
            session
                .local_client()
                .prediction()
                .unacknowledged_commands()
                .is_empty()
        );
    }

    #[test]
    fn command_acknowledgement_message_prunes_prediction_buffer() {
        let mut session = GameSession::new();
        let commands =
            session.sequence_local_commands(vec![PlayerCommand::Interact, PlayerCommand::Confirm]);

        session.apply_command_acknowledgement(&CommandAcknowledgement {
            client_id: LOCAL_CLIENT_ID,
            acknowledged_sequence: commands[1].sequence,
            authoritative_tick: SimulationTick::new(2),
        });

        assert!(
            session
                .local_client()
                .prediction()
                .unacknowledged_commands()
                .is_empty()
        );
    }

    #[test]
    fn command_rejection_message_records_prediction_failure() {
        let mut session = GameSession::new();

        session.apply_command_rejection(&CommandRejection {
            client_id: LOCAL_CLIENT_ID,
            player_id: LOCAL_PLAYER_ID,
            sequence: InputSequence::new(0),
            reason: crate::multiplayer::CommandAcceptance::Duplicate,
            authoritative_tick: SimulationTick::new(2),
        });

        assert_eq!(
            session.local_client().prediction().prediction_failures(),
            &[super::PredictionFailure::CommandRejected]
        );
        assert_eq!(
            session
                .local_client()
                .prediction()
                .prediction_failure_resolutions(),
            vec![super::PredictionFailureResolution::RequestAuthoritativeSnapshot]
        );
    }

    #[test]
    fn prediction_state_exposes_replay_commands_and_correction_plan() {
        let mut session = GameSession::new();
        session.sequence_local_commands(vec![PlayerCommand::Interact]);

        assert_eq!(
            session.local_client().prediction().replay_commands().len(),
            1
        );
        assert_eq!(
            super::CorrectionPlan::None,
            super::ClientPredictionState::correction_plan(0.5, 0.5)
        );
        assert_eq!(
            super::CorrectionPlan::Smooth,
            super::ClientPredictionState::correction_plan(8.0, 0.0)
        );
        assert_eq!(
            super::CorrectionPlan::Snap,
            super::ClientPredictionState::correction_plan(32.0, 0.0)
        );
    }

    #[test]
    fn prediction_state_buffers_remote_snapshots_for_interpolation() {
        let mut prediction = super::ClientPredictionState::default();
        prediction.push_remote_snapshot(super::PlayerSnapshot {
            player_id: LOCAL_PLAYER_ID,
            x: 1.0,
            y: 2.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            fuel: 3.0,
            hull: 4.0,
            credits: 6,
            cargo_used: 0,
            scanner_cooldown_seconds: 0.0,
        });

        assert_eq!(prediction.remote_snapshot_count(LOCAL_PLAYER_ID), 1);
    }

    #[test]
    fn prediction_state_derives_interpolation_and_extrapolation_timing() {
        let mut session = GameSession::new();
        session.sequence_local_commands(vec![PlayerCommand::Interact, PlayerCommand::UseScanner]);

        assert!(
            (super::ClientPredictionState::interpolation_delay_seconds(0.01) - 0.05).abs()
                < f32::EPSILON
        );
        assert!(
            (super::ClientPredictionState::interpolation_delay_seconds(1.0) - 0.25).abs()
                < f32::EPSILON
        );
        assert!(super::ClientPredictionState::should_extrapolate(0.12));
        assert!(!super::ClientPredictionState::should_extrapolate(0.13));
        assert!(
            session
                .local_client()
                .prediction()
                .predicted_input_lag_seconds()
                > 0.0
        );
    }

    #[test]
    fn prediction_state_projects_local_reconciliation_and_remote_presentation() {
        let previous = super::PlayerSnapshot {
            player_id: LOCAL_PLAYER_ID,
            x: 10.0,
            y: 20.0,
            velocity_x: 4.0,
            velocity_y: -2.0,
            fuel: 3.0,
            hull: 4.0,
            credits: 6,
            cargo_used: 0,
            scanner_cooldown_seconds: 0.0,
        };
        let next = super::PlayerSnapshot {
            x: 20.0,
            y: 30.0,
            ..previous
        };

        let predicted = super::ClientPredictionState::predict_local_movement(&previous, 0.5);
        assert!((predicted.x - 12.0).abs() < f32::EPSILON);
        assert!((predicted.y - 19.0).abs() < f32::EPSILON);

        let replayed = super::ClientPredictionState::replay_unacknowledged_movement(
            &previous,
            &[SequencedPlayerCommand {
                player_id: LOCAL_PLAYER_ID,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::Movement {
                    horizontal: 3.0,
                    thrust: true,
                    drill_down: false,
                },
            }],
        );
        assert_eq!(replayed.replayed_command_count, 1);
        assert!((replayed.predicted.x - 13.0).abs() < f32::EPSILON);

        let reconciled = super::ClientPredictionState::reconcile_movement(predicted, &next);
        assert_eq!(reconciled.correction_plan, super::CorrectionPlan::Smooth);
        assert!(reconciled.correction_offset.is_some());
        let smoothed = reconciled.corrected_presentation(0.5);
        assert_eq!(smoothed.correction_plan, super::CorrectionPlan::Smooth);
        assert!((smoothed.x - 16.0).abs() < f32::EPSILON);

        let interpolated = super::ClientPredictionState::remote_player_presentation(
            &previous,
            Some(&next),
            0.5,
            0.0,
        );
        assert!((interpolated.x - 15.0).abs() < f32::EPSILON);
        assert!(!interpolated.extrapolated);

        let extrapolated =
            super::ClientPredictionState::remote_player_presentation(&previous, None, 0.0, 0.1);
        assert!(extrapolated.extrapolated);
        assert!((extrapolated.x - 10.4).abs() < f32::EPSILON);
    }

    #[test]
    fn prediction_state_records_and_clears_prediction_failures() {
        let mut prediction = super::ClientPredictionState::default();

        prediction.note_prediction_failure(super::PredictionFailure::TerrainAlreadyChanged);
        prediction.note_prediction_failure(super::PredictionFailure::HazardOrRescueChangedState);

        assert_eq!(prediction.prediction_failures().len(), 2);
        prediction.clear_prediction_failures();
        assert!(prediction.prediction_failures().is_empty());
    }

    #[test]
    fn prediction_state_maps_failures_to_recovery_actions() {
        let mut prediction = super::ClientPredictionState::default();

        prediction.note_prediction_failure(super::PredictionFailure::TerrainAlreadyChanged);
        prediction.note_prediction_failure(super::PredictionFailure::EconomyChangedState);
        prediction.note_prediction_failure(super::PredictionFailure::ProgressionChangedState);

        assert_eq!(
            prediction.prediction_failure_resolutions(),
            vec![
                super::PredictionFailureResolution::RequestTerrainChunk,
                super::PredictionFailureResolution::RollBackLocalEconomy,
                super::PredictionFailureResolution::RollBackProgression,
            ]
        );

        let mut tracker = super::TerrainRevisionTracker::default();
        let position = super::TerrainChunkPosition { x: 0, y: 0 };
        tracker.mark_tiles_changed([crate::terrain::TilePosition { x: 1, y: 1 }]);
        let actions = prediction.prediction_recovery_actions(
            LOCAL_PLAYER_ID,
            &tracker,
            SimulationTick::new(12),
            position,
            0,
        );
        assert!(matches!(
            &actions[0],
            super::PredictionRecoveryAction::RequestTerrainDelta(
                super::CompactWorldDelta::TerrainChunks { .. }
            )
        ));
        assert!(matches!(
            actions[1],
            super::PredictionRecoveryAction::RollBackLocalEconomy {
                player_id: LOCAL_PLAYER_ID
            }
        ));
        assert!(!super::PredictionFailureRecoveryPlan::from_actions(actions).request_keyframe);
    }

    #[test]
    fn session_builds_prediction_failure_recovery_plan() {
        let mut session = GameSession::new();
        session
            .local_client_mut()
            .prediction
            .note_prediction_failure(super::PredictionFailure::HazardOrRescueChangedState);

        let plan =
            session.prediction_failure_recovery_plan(super::TerrainChunkPosition { x: 0, y: 0 }, 0);

        assert!(plan.request_keyframe);
        assert_eq!(plan.actions.len(), 1);
    }

    #[test]
    fn prediction_state_tracks_local_feedback_and_correction_offsets() {
        let mut prediction = super::ClientPredictionState::default();

        prediction.push_feedback(super::LocalTentativeFeedback::MovementIntent);
        prediction.push_feedback(super::LocalTentativeFeedback::DrillContact);
        prediction.set_correction_offset(super::CorrectionOffset::new(2.0, -1.0));

        assert_eq!(prediction.pending_feedback().len(), 2);
        assert_eq!(
            prediction.tentative_feedback_presentations(),
            vec![
                super::TentativeFeedbackPresentation::MovementVisual,
                super::TentativeFeedbackPresentation::DrillContactAudio,
            ]
        );
        assert_eq!(
            prediction.tentative_feedback_outputs(),
            vec![
                super::TentativeFeedbackOutput {
                    presentation: super::TentativeFeedbackPresentation::MovementVisual,
                    channel: super::TentativeFeedbackChannel::Render,
                },
                super::TentativeFeedbackOutput {
                    presentation: super::TentativeFeedbackPresentation::DrillContactAudio,
                    channel: super::TentativeFeedbackChannel::Audio,
                },
            ]
        );
        let offset = prediction.correction_offset().expect("offset set");
        assert!((offset.x - 2.0).abs() < f32::EPSILON);
        prediction.clear_feedback();
        prediction.clear_correction_offset();
        assert!(prediction.pending_feedback().is_empty());
        assert!(prediction.correction_offset().is_none());
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
    fn authoritative_command_processing_applies_buffered_player_commands() {
        let mut session = GameSession::new();
        let tick = session.current_tick();

        session.sequence_local_commands(vec![PlayerCommand::Movement {
            horizontal: 0.5,
            thrust: false,
            drill_down: false,
        }]);

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);
        let velocity_x = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("player exists")
            .velocity_x;
        assert!((velocity_x - 0.5).abs() < f32::EPSILON);
        assert_eq!(
            session.local_client().prediction().replay_commands().len(),
            0
        );
        assert!(session.drain_events().iter().any(|event| matches!(
            event,
            super::WorldEvent::PlayerChanged {
                player_id: LOCAL_PLAYER_ID
            }
        )));
    }

    #[test]
    fn authoritative_command_processing_emits_domain_events_for_applied_commands() {
        let mut session = GameSession::new();
        session.game.player.bombs = 1;
        session
            .world
            .sync_from_legacy_game(session.current_tick(), &session.game.clone());
        let tick = session.current_tick();

        session.sequence_local_commands(vec![
            PlayerCommand::PlaceBomb,
            PlayerCommand::Refuel,
            PlayerCommand::SellCargo,
        ]);

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 3);
        let events = session.drain_events();
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::BombPlaced {
                player_id: LOCAL_PLAYER_ID
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::PurchaseCompleted {
                player_id: LOCAL_PLAYER_ID
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::CargoChanged {
                player_id: LOCAL_PLAYER_ID
            }
        )));
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

    #[test]
    fn terrain_revision_tracker_coalesces_changed_tiles_by_chunk() {
        let mut tracker = super::TerrainRevisionTracker::default();
        let revisions = tracker.mark_tiles_changed([
            crate::terrain::TilePosition { x: 0, y: 0 },
            crate::terrain::TilePosition { x: 3, y: 4 },
            crate::terrain::TilePosition { x: 17, y: 0 },
        ]);

        assert_eq!(revisions.len(), 2);
        assert_eq!(
            tracker.revision(super::TerrainChunkPosition { x: 0, y: 0 }),
            1
        );
        assert_eq!(
            tracker.revision(super::TerrainChunkPosition { x: 1, y: 0 }),
            1
        );
    }

    #[test]
    fn terrain_revision_tracker_builds_chunk_recovery_deltas() {
        let mut tracker = super::TerrainRevisionTracker::default();
        let position = super::TerrainChunkPosition { x: 0, y: 0 };
        tracker.mark_tiles_changed([crate::terrain::TilePosition { x: 0, y: 0 }]);

        assert_eq!(
            tracker.recovery_delta(SimulationTick::new(12), position, 1),
            super::CompactWorldDelta::Noop {
                tick: SimulationTick::new(12),
            }
        );
        assert_eq!(
            tracker.recovery_delta(SimulationTick::new(12), position, 0),
            super::CompactWorldDelta::TerrainChunks {
                tick: SimulationTick::new(12),
                revisions: vec![super::TerrainChunkRevision {
                    position,
                    revision: 1,
                }],
            }
        );
    }

    #[test]
    fn keyframe_event_is_emitted_on_interval() {
        let mut session = GameSession::new();
        let delta_seconds = 5.0;

        session.update_legacy(crate::input::PlayerInput::default(), delta_seconds);
        let delta = session.drain_world_delta();

        assert!(
            delta
                .events
                .iter()
                .any(|event| matches!(event, super::WorldEvent::SnapshotKeyframeReady { .. }))
        );
    }
}
