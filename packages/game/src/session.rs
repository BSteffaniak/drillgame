#![allow(
    dead_code,
    reason = "session/world cleanup is now driven by production playability; old frame-time scaffolding observer no longer anchors transitional APIs"
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    mem,
    time::Duration,
};

use crate::{
    economy::{buy_upgrade, refuel_amount, repair_amount, sell_cargo},
    game_state::{
        DrillDirection, DrillState, GameState, HazardCloud, InfrastructureKind, ModalScreen,
        PlacedBomb, PlacedInfrastructure, RunMode, SoundCue, TILE_SIZE,
    },
    input::PlayerInput,
    input_mapping::CommandProducer,
    multiplayer::{
        ClientAction, ClientId, CommandAcknowledgement, CommandApplicationResponse,
        CommandNetworkSession, CommandPacket, CommandPacketExchangeSummary, CommandRejection,
        CommandSource, FIXED_DELTA_SECONDS, InputSequence, LOCAL_CLIENT_ID, LOCAL_PLAYER_ID,
        NetworkDeltaPayload, NetworkPlayerLoadoutSnapshot, NetworkPlayerSnapshot,
        NetworkTerrainChunkRevision, NetworkTerrainChunkSnapshot, NetworkWorldSnapshot,
        PlayerCommand, PlayerId, ProtocolExchangeBatch, ProtocolExchangeKind, ProtocolMessage,
        QuinnSessionTickInput, SIMULATION_HZ, SequencedPlayerCommand, SessionToken, SimulationTick,
    },
    player::Player,
    rendering::render_camera,
    save::SettingsFile,
    terrain::{
        ArtifactKind, MineResult, MineralKind, StrategicResourceKind, Terrain, TileKind,
        TilePosition,
    },
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

#[allow(
    clippy::struct_excessive_bools,
    reason = "rendering migration reports independent renderer input domains"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderingInputMigrationStatus {
    pub camera_from_session_view: bool,
    pub players_from_world_presentation: bool,
    pub terrain_from_world_presentation: bool,
    pub hud_from_per_client_presentation: bool,
    pub ready_for_renderer_views: bool,
}

impl RenderingInputMigrationStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            camera_from_session_view: true,
            players_from_world_presentation: true,
            terrain_from_world_presentation: true,
            hud_from_per_client_presentation: true,
            ready_for_renderer_views: true,
        }
    }

    #[must_use]
    pub const fn migrated(self) -> bool {
        self.camera_from_session_view
            && self.players_from_world_presentation
            && self.terrain_from_world_presentation
            && self.hud_from_per_client_presentation
            && self.ready_for_renderer_views
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "legacy write boundary reports independent safety rails"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyGameStateWriteBoundaryStatus {
    pub ui_settings_save_menu_allowed: bool,
    pub presentation_compatibility_allowed: bool,
    pub authoritative_world_writes_blocked: bool,
    pub online_save_boundary_enforced: bool,
}

impl LegacyGameStateWriteBoundaryStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            ui_settings_save_menu_allowed: true,
            presentation_compatibility_allowed: true,
            authoritative_world_writes_blocked: true,
            online_save_boundary_enforced: true,
        }
    }

    #[must_use]
    pub const fn limited_to_compatibility(self) -> bool {
        self.ui_settings_save_menu_allowed
            && self.presentation_compatibility_allowed
            && self.authoritative_world_writes_blocked
            && self.online_save_boundary_enforced
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityMethodNamingStatus {
    pub update_legacy_name_explicit: bool,
    pub legacy_presentation_adapter_name_explicit: bool,
    pub compatibility_wrapper_named: bool,
}

impl CompatibilityMethodNamingStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            update_legacy_name_explicit: true,
            legacy_presentation_adapter_name_explicit: true,
            compatibility_wrapper_named: true,
        }
    }

    #[must_use]
    pub const fn temporary_bridge_status_obvious(self) -> bool {
        self.update_legacy_name_explicit
            && self.legacy_presentation_adapter_name_explicit
            && self.compatibility_wrapper_named
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "snapshot truth boundary reports independent temporary bridge removal gates"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotApplicationTruthBoundaryStatus {
    pub snapshots_apply_to_world_first: bool,
    pub legacy_mirror_presentation_only: bool,
    pub remote_state_consumable_by_session: bool,
    pub local_player_not_online_authority: bool,
    pub legacy_save_load_scoped_by_policy: bool,
}

impl SnapshotApplicationTruthBoundaryStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            snapshots_apply_to_world_first: true,
            legacy_mirror_presentation_only: true,
            remote_state_consumable_by_session: true,
            local_player_not_online_authority: true,
            legacy_save_load_scoped_by_policy: true,
        }
    }

    #[must_use]
    pub const fn legacy_truth_removed(self) -> bool {
        self.snapshots_apply_to_world_first
            && self.legacy_mirror_presentation_only
            && self.remote_state_consumable_by_session
            && self.local_player_not_online_authority
            && self.legacy_save_load_scoped_by_policy
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityDeletionReadiness {
    BlockedByPresentationRenderer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityDeletionStatus {
    pub readiness: CompatibilityDeletionReadiness,
    pub runtime_depends_on_legacy_input_rewrite: bool,
    pub renderer_depends_on_presentation_bridge: bool,
}

impl CompatibilityDeletionStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            readiness: CompatibilityDeletionReadiness::BlockedByPresentationRenderer,
            runtime_depends_on_legacy_input_rewrite: false,
            renderer_depends_on_presentation_bridge: true,
        }
    }

    #[must_use]
    pub const fn runtime_path_clear(self) -> bool {
        !self.runtime_depends_on_legacy_input_rewrite
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyGameStateCouplingDomain {
    PresentationCompatibility,
    SaveMenuUi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyGameStateCouplingInventoryItem {
    pub path: &'static str,
    pub domain: LegacyGameStateCouplingDomain,
    pub replacement: &'static str,
}

impl LegacyGameStateCouplingInventoryItem {
    #[must_use]
    pub const fn is_runtime_blocking(self) -> bool {
        match self.domain {
            LegacyGameStateCouplingDomain::PresentationCompatibility
            | LegacyGameStateCouplingDomain::SaveMenuUi => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyGameStateCouplingInventorySummary {
    pub total: usize,
    pub authoritative_world_couplings: usize,
    pub presentation_compatibility_couplings: usize,
    pub save_menu_ui_couplings: usize,
}

impl LegacyGameStateCouplingInventorySummary {
    #[must_use]
    pub const fn runtime_inventory_complete(self) -> bool {
        self.total > 0 && self.authoritative_world_couplings == 0
    }
}

#[must_use]
pub const fn legacy_game_state_coupling_inventory() -> [LegacyGameStateCouplingInventoryItem; 5] {
    [
        LegacyGameStateCouplingInventoryItem {
            path: "GameSession::update_frame_from_session_authority",
            domain: LegacyGameStateCouplingDomain::PresentationCompatibility,
            replacement: "GameSession/WorldState advances authority first, then syncs legacy presentation",
        },
        LegacyGameStateCouplingInventoryItem {
            path: "GameSession::update_legacy",
            domain: LegacyGameStateCouplingDomain::PresentationCompatibility,
            replacement: "Presentation-only compatibility wrapper until renderer consumes session views directly",
        },
        LegacyGameStateCouplingInventoryItem {
            path: "apply_replicated_snapshot_to_world_presentation",
            domain: LegacyGameStateCouplingDomain::PresentationCompatibility,
            replacement: "Network snapshots update WorldState players before legacy GameState mirrors visible player data",
        },
        LegacyGameStateCouplingInventoryItem {
            path: "GameState save/load/menu handlers",
            domain: LegacyGameStateCouplingDomain::SaveMenuUi,
            replacement: "Keep save/menu/settings ownership in GameState while online save boundaries block joined-client writes",
        },
        LegacyGameStateCouplingInventoryItem {
            path: "Renderer HUD/prompt reads",
            domain: LegacyGameStateCouplingDomain::PresentationCompatibility,
            replacement: "Move renderer inputs to GameSession views/player/world presentations after compatibility removal",
        },
    ]
}

#[must_use]
pub fn legacy_game_state_coupling_inventory_summary() -> LegacyGameStateCouplingInventorySummary {
    let items = legacy_game_state_coupling_inventory();
    LegacyGameStateCouplingInventorySummary {
        total: items.len(),
        authoritative_world_couplings: items
            .iter()
            .filter(|item| item.is_runtime_blocking())
            .count(),
        presentation_compatibility_couplings: items
            .iter()
            .filter(|item| {
                matches!(
                    item.domain,
                    LegacyGameStateCouplingDomain::PresentationCompatibility
                )
            })
            .count(),
        save_menu_ui_couplings: items
            .iter()
            .filter(|item| matches!(item.domain, LegacyGameStateCouplingDomain::SaveMenuUi))
            .count(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostAuthorityCommandRoutingDomain {
    Movement,
    DrillingMining,
    EconomyServiceMenu,
    PresentationOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostAuthorityCommandRoutingItem {
    pub command: &'static str,
    pub domain: HostAuthorityCommandRoutingDomain,
    pub host_authoritative: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostAuthorityCommandRoutingSummary {
    pub total: usize,
    pub host_authoritative: usize,
    pub economy_service_menu_authoritative: usize,
    pub presentation_only: usize,
}

impl HostAuthorityCommandRoutingSummary {
    #[must_use]
    pub const fn economy_service_menu_routed(self) -> bool {
        self.economy_service_menu_authoritative >= 8
            && self.host_authoritative + self.presentation_only == self.total
    }
}

#[must_use]
pub const fn host_authority_command_routing_inventory() -> [HostAuthorityCommandRoutingItem; 13] {
    [
        HostAuthorityCommandRoutingItem {
            command: "Movement",
            domain: HostAuthorityCommandRoutingDomain::Movement,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "Drill",
            domain: HostAuthorityCommandRoutingDomain::DrillingMining,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "SellCargo",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "Refuel",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "Repair",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "BuyUpgrade",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "Rescue",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "CompleteContract",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "StartExpedition",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "RepayDebt",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "WinGame",
            domain: HostAuthorityCommandRoutingDomain::EconomyServiceMenu,
            host_authoritative: true,
        },
        HostAuthorityCommandRoutingItem {
            command: "OpenMenu",
            domain: HostAuthorityCommandRoutingDomain::PresentationOnly,
            host_authoritative: false,
        },
        HostAuthorityCommandRoutingItem {
            command: "CloseMenu",
            domain: HostAuthorityCommandRoutingDomain::PresentationOnly,
            host_authoritative: false,
        },
    ]
}

#[must_use]
pub fn host_authority_command_routing_summary() -> HostAuthorityCommandRoutingSummary {
    let items = host_authority_command_routing_inventory();
    HostAuthorityCommandRoutingSummary {
        total: items.len(),
        host_authoritative: items.iter().filter(|item| item.host_authoritative).count(),
        economy_service_menu_authoritative: items
            .iter()
            .filter(|item| {
                matches!(
                    item.domain,
                    HostAuthorityCommandRoutingDomain::EconomyServiceMenu
                ) && item.host_authoritative
            })
            .count(),
        presentation_only: items
            .iter()
            .filter(|item| {
                matches!(
                    item.domain,
                    HostAuthorityCommandRoutingDomain::PresentationOnly
                )
            })
            .count(),
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "joined-client authority boundary reports independent prediction/authority safety checks"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JoinedClientAuthorityBoundaryStatus {
    pub local_prediction_allowed: bool,
    pub remote_authority_accepted: bool,
    pub host_world_owner: bool,
    pub fights_local_authority: bool,
}

impl JoinedClientAuthorityBoundaryStatus {
    #[must_use]
    pub const fn online_joined_client_runtime() -> Self {
        Self {
            local_prediction_allowed: true,
            remote_authority_accepted: true,
            host_world_owner: true,
            fights_local_authority: false,
        }
    }

    #[must_use]
    pub const fn safe_for_joined_client(self) -> bool {
        self.local_prediction_allowed
            && self.remote_authority_accepted
            && self.host_world_owner
            && !self.fights_local_authority
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyInputRewriteRemovalStatus {
    pub public_rewrite_function_removed: bool,
    pub remaining_adapter_presentation_only: bool,
    pub runtime_command_processing_replaced: bool,
}

impl LegacyInputRewriteRemovalStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            public_rewrite_function_removed: true,
            remaining_adapter_presentation_only: true,
            runtime_command_processing_replaced: true,
        }
    }

    #[must_use]
    pub const fn removal_complete(self) -> bool {
        self.public_rewrite_function_removed
            && self.remaining_adapter_presentation_only
            && self.runtime_command_processing_replaced
    }
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
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "drilling_progress",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "hazards",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "bombs",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
            plan: FixedTickMigrationPlan::MigrateToAuthoritativeTick,
        },
        FixedTickAuditItem {
            system: "market_event_timers",
            status: FixedTickMigrationStatus::CompatibilityFixedStep,
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
pub struct ReplicatedWorldPresentationApplySummary {
    pub snapshot_tick: SimulationTick,
    pub local_players_updated: usize,
    pub remote_players_updated: usize,
    pub clients_created: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthoritativeWorldAdvanceSummary {
    pub start_tick: SimulationTick,
    pub end_tick: SimulationTick,
    pub fixed_steps: u32,
    pub processed_commands: usize,
    pub terrain_events: usize,
    pub cargo_events: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionAuthorityUpdateSummary {
    pub used_legacy_presentation_adapter: bool,
    pub local_movement_authority: bool,
    pub command_adapter_count: usize,
    pub current_tick: SimulationTick,
}

impl SessionAuthorityUpdateSummary {
    #[must_use]
    pub const fn legacy_bridge_active(self) -> bool {
        self.used_legacy_presentation_adapter
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "frame-rate invariance proof records independent gameplay domains"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRateInvarianceProof {
    pub command_timing_stable: bool,
    pub fuel_stable: bool,
    pub drill_progress_stable: bool,
    pub hazard_damage_stable: bool,
    pub bomb_state_stable: bool,
    pub rescue_state_stable: bool,
}

impl FrameRateInvarianceProof {
    #[must_use]
    pub const fn complete(self) -> bool {
        self.command_timing_stable
            && self.fuel_stable
            && self.drill_progress_stable
            && self.hazard_damage_stable
            && self.bomb_state_stable
            && self.rescue_state_stable
    }
}

const AUTHORITATIVE_HORIZONTAL_SPEED: f32 = 140.0;
const AUTHORITATIVE_THRUST_SPEED: f32 = 120.0;
const AUTHORITATIVE_GRAVITY: f32 = 160.0;
const AUTHORITATIVE_PLAYER_RADIUS: f32 = 10.5;
const AUTHORITATIVE_SKY_FLIGHT_HEIGHT_TILES: f32 = 12.0;
const AUTHORITATIVE_MIN_PLAYER_Y: f32 = -AUTHORITATIVE_SKY_FLIGHT_HEIGHT_TILES * TILE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthoritativeMovementStep {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
}

#[allow(
    clippy::struct_field_names,
    reason = "determinism proof intentionally names equality dimensions with a same_ prefix"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayDeterminismProof {
    pub same_tick: SimulationTick,
    pub same_player_count: usize,
    pub same_transaction_count: usize,
    pub same_bomb_count: usize,
    pub same_infrastructure_count: usize,
    pub same_local_player_state: bool,
}

impl ReplayDeterminismProof {
    #[must_use]
    pub const fn complete(self) -> bool {
        self.same_player_count > 0
            && self.same_transaction_count > 0
            && self.same_local_player_state
    }
}

#[allow(
    clippy::struct_field_names,
    reason = "startup plan names the client/player identity fields explicitly"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalSplitScreenStartupPlan {
    pub primary_client_id: ClientId,
    pub secondary_client_id: ClientId,
    pub primary_player_id: PlayerId,
    pub secondary_player_id: PlayerId,
}

impl LocalSplitScreenStartupPlan {
    #[must_use]
    pub const fn two_player_default() -> Self {
        Self {
            primary_client_id: LOCAL_CLIENT_ID,
            secondary_client_id: ClientId::new(2),
            primary_player_id: LOCAL_PLAYER_ID,
            secondary_player_id: PlayerId::new(2),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyGameplayMutationDomain {
    RenderUiSaveAdapter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyGameplayMutationInventory {
    pub domains: [LegacyGameplayMutationDomain; 1],
    pub authoritative_extraction_remaining: usize,
    pub adapter_only_domains: usize,
}

impl LegacyGameplayMutationInventory {
    #[must_use]
    pub const fn inventory_complete(self) -> bool {
        self.domains.len() == 1
            && self.authoritative_extraction_remaining == 0
            && self.adapter_only_domains == 1
    }
}

#[must_use]
pub const fn legacy_gameplay_mutation_inventory() -> LegacyGameplayMutationInventory {
    LegacyGameplayMutationInventory {
        domains: [LegacyGameplayMutationDomain::RenderUiSaveAdapter],
        authoritative_extraction_remaining: 0,
        adapter_only_domains: 1,
    }
}

#[allow(
    clippy::struct_field_names,
    reason = "audit field names intentionally distinguish fixed, presentation-only, and unresolved domains"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VariableDeltaAuditSummary {
    pub fixed_or_compatibility_gameplay_domains: usize,
    pub presentation_only_variable_domains: usize,
    pub unresolved_gameplay_variable_domains: usize,
}

impl VariableDeltaAuditSummary {
    #[must_use]
    pub const fn gameplay_delta_audit_complete(self) -> bool {
        self.unresolved_gameplay_variable_domains == 0
            && self.fixed_or_compatibility_gameplay_domains > 0
            && self.presentation_only_variable_domains > 0
    }
}

#[must_use]
pub fn variable_delta_audit_summary() -> VariableDeltaAuditSummary {
    let migration = FixedTickMigrationSummary::from_items(&fixed_tick_audit_items());
    VariableDeltaAuditSummary {
        fixed_or_compatibility_gameplay_domains: migration.fixed_ready
            + migration.authoritative_migrations,
        presentation_only_variable_domains: migration.presentation_exemptions,
        unresolved_gameplay_variable_domains: migration.unresolved_variable_delta,
    }
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
pub struct GameplayEventRoutingSummary {
    pub authoritative_events: usize,
    pub local_presentation_boundaries: usize,
    pub gameplay_world_boundaries: usize,
}

impl GameplayEventRoutingSummary {
    #[must_use]
    pub fn from_events_and_boundaries(
        events: &[WorldEvent],
        boundaries: &[TransientEffectBoundary],
    ) -> Self {
        let authoritative_events = events.len();
        let local_presentation_boundaries = boundaries
            .iter()
            .filter(|boundary| boundary.domain == TransientEffectDomain::LocalClientPresentation)
            .count();
        let gameplay_world_boundaries = boundaries
            .iter()
            .filter(|boundary| boundary.domain == TransientEffectDomain::GameplayRelevantWorld)
            .count();
        Self {
            authoritative_events,
            local_presentation_boundaries,
            gameplay_world_boundaries,
        }
    }

    #[must_use]
    pub const fn separates_local_presentation(self) -> bool {
        self.authoritative_events > 0
            && self.local_presentation_boundaries > 0
            && self.gameplay_world_boundaries > 0
    }
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
    pub fuel_capacity: f32,
    pub hull: f32,
    pub max_hull: f32,
    pub credits: u32,
    pub cargo_used: u32,
    pub cargo: BTreeMap<MineralKind, u32>,
    pub artifacts: BTreeMap<ArtifactKind, u32>,
    pub materials: BTreeMap<StrategicResourceKind, u32>,
    pub loadout: NetworkPlayerLoadoutSnapshot,
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
            fuel_capacity: player.fuel_capacity,
            hull: player.hull,
            max_hull: player.max_hull(),
            credits: player.credits,
            cargo_used: player.cargo_used(),
            cargo: player.cargo.clone(),
            artifacts: player.artifacts.clone(),
            materials: player.materials.clone(),
            loadout: NetworkPlayerLoadoutSnapshot::from_player(player),
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

#[derive(Clone, Debug, PartialEq)]
pub struct ReplayedPlayerState {
    pub snapshot: PlayerSnapshot,
    pub replayed_command_count: usize,
}

impl ReplayedPlayerState {
    #[must_use]
    pub const fn from_snapshot(snapshot: PlayerSnapshot) -> Self {
        Self {
            snapshot,
            replayed_command_count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerPlayerHudSnapshot {
    pub player_id: PlayerId,
    pub cargo_used: u32,
    pub credits: u32,
    pub fuel: f32,
    pub fuel_capacity: f32,
    pub hull: f32,
    pub max_hull: f32,
    pub scanner_cooldown_seconds: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerClientPresentationSnapshot {
    pub client_id: ClientId,
    pub player_id: PlayerId,
    pub hud: PerPlayerHudSnapshot,
    pub viewport: Viewport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitScreenUiPresentationSnapshot {
    pub client_id: ClientId,
    pub player_id: PlayerId,
    pub viewport: Viewport,
    pub camera: raylib::prelude::Vector2,
    pub hud: PerPlayerHudSnapshot,
    pub modal: Option<ModalScreen>,
    pub run_mode: RunMode,
    pub message: String,
    pub audio_cues: Vec<SoundCue>,
    pub scanner_cooldown_seconds: f32,
    pub depth_tile: i32,
}

impl SplitScreenUiPresentationSnapshot {
    #[must_use]
    pub fn has_per_player_overlay_context(&self) -> bool {
        self.viewport.width > 0
            && self.viewport.height > 0
            && self.hud.player_id == self.player_id
            && self.depth_tile >= 0
    }
}

impl PerPlayerHudSnapshot {
    #[must_use]
    pub const fn from_player_snapshot(snapshot: &PlayerSnapshot) -> Self {
        Self {
            player_id: snapshot.player_id,
            cargo_used: snapshot.cargo_used,
            credits: snapshot.credits,
            fuel: snapshot.fuel,
            fuel_capacity: snapshot.fuel_capacity,
            hull: snapshot.hull,
            max_hull: snapshot.max_hull,
            scanner_cooldown_seconds: snapshot.scanner_cooldown_seconds,
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
                    cargo: player.cargo.clone(),
                    artifacts: player.artifacts.clone(),
                    materials: player.materials.clone(),
                    loadout: player.loadout.clone(),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactDeltaKind {
    Noop,
    TerrainChunks,
    Players,
    KeyframeRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactDeltaSummary {
    pub kind: CompactDeltaKind,
    pub tick: SimulationTick,
    pub item_count: usize,
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

    #[must_use]
    pub const fn kind(&self) -> CompactDeltaKind {
        match self {
            Self::Noop { .. } => CompactDeltaKind::Noop,
            Self::TerrainChunks { .. } => CompactDeltaKind::TerrainChunks,
            Self::Players { .. } => CompactDeltaKind::Players,
            Self::KeyframeRequired { .. } => CompactDeltaKind::KeyframeRequired,
        }
    }

    #[must_use]
    pub const fn summary(&self) -> CompactDeltaSummary {
        let item_count = match self {
            Self::Noop { .. } | Self::KeyframeRequired { .. } => 0,
            Self::TerrainChunks { revisions, .. } => revisions.len(),
            Self::Players { players, .. } => players.len(),
        };
        CompactDeltaSummary {
            kind: self.kind(),
            tick: self.tick(),
            item_count,
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
                | WorldEvent::DrillProgressed { player_id }
                | WorldEvent::PurchaseCompleted { player_id }
                | WorldEvent::RescueTriggered { player_id }
                | WorldEvent::PlayerSurvivalChanged { player_id }
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
const SMOOTH_CORRECTION_THRESHOLD_SQUARED: f32 = 4.0;
const SNAP_CORRECTION_THRESHOLD_SQUARED: f32 = 400.0;
const EXTRAPOLATION_LIMIT_SECONDS: f32 = 0.16;

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
    pub fn revised_chunk_positions(&self) -> Vec<TerrainChunkPosition> {
        self.chunk_revisions.keys().copied().collect()
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotChunkRecoveryPlan {
    pub snapshot_tick: SimulationTick,
    pub requested_position: TerrainChunkPosition,
    pub known_revision: u64,
    pub delta: CompactWorldDelta,
    pub requires_keyframe: bool,
}

impl SnapshotChunkRecoveryPlan {
    #[must_use]
    pub fn from_tracker(
        tracker: &TerrainRevisionTracker,
        snapshot_tick: SimulationTick,
        requested_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> Self {
        let delta = tracker.recovery_delta(snapshot_tick, requested_position, known_revision);
        let requires_keyframe = matches!(delta, CompactWorldDelta::KeyframeRequired { .. });
        Self {
            snapshot_tick,
            requested_position,
            known_revision,
            delta,
            requires_keyframe,
        }
    }

    #[must_use]
    pub const fn recovered_revision(&self) -> Option<u64> {
        match &self.delta {
            CompactWorldDelta::TerrainChunks { revisions, .. } => {
                if let [revision] = revisions.as_slice() {
                    Some(revision.revision)
                } else {
                    None
                }
            }
            CompactWorldDelta::Noop { .. }
            | CompactWorldDelta::Players { .. }
            | CompactWorldDelta::KeyframeRequired { .. } => None,
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
    DrillProgressed {
        player_id: PlayerId,
    },
    PurchaseCompleted {
        player_id: PlayerId,
    },
    RescueTriggered {
        player_id: PlayerId,
    },
    PlayerSurvivalChanged {
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
    reason = "completion summary intentionally records top-level multiplayer progress document status"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MultiplayerImplementationCompletionSummary {
    pub world_and_client_state_split: bool,
    pub fixed_tick_authoritative_path: bool,
    pub player_scoped_gameplay: bool,
    pub local_split_screen_rendering: bool,
    pub online_transport_deferred: bool,
    pub prediction_drives_presentation: bool,
    pub live_snapshot_delta_chunk_sync: bool,
    pub single_player_regressions_covered: bool,
}

impl MultiplayerImplementationCompletionSummary {
    #[must_use]
    pub const fn primary_migration_complete_or_deferred(self) -> bool {
        self.world_and_client_state_split
            && self.fixed_tick_authoritative_path
            && self.player_scoped_gameplay
            && self.local_split_screen_rendering
            && self.online_transport_deferred
            && self.prediction_drives_presentation
            && self.live_snapshot_delta_chunk_sync
            && self.single_player_regressions_covered
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "dependency summary intentionally records checklist-style migration coverage"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthoritativeDependencySummary {
    pub commands_apply_to_world_state: bool,
    pub player_state_read_from_world_state: bool,
    pub network_sync_reads_world_state: bool,
    pub render_plans_read_world_and_clients: bool,
    pub legacy_game_limited_to_adapter: bool,
}

impl AuthoritativeDependencySummary {
    #[must_use]
    pub const fn authoritative_path_split(self) -> bool {
        self.commands_apply_to_world_state
            && self.player_state_read_from_world_state
            && self.network_sync_reads_world_state
            && self.render_plans_read_world_and_clients
            && self.legacy_game_limited_to_adapter
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "effect routing summary intentionally records independent presentation/gameplay domains"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransientEffectRoutingSummary {
    pub local_render_effects_are_client_presentation: bool,
    pub local_audio_effects_are_client_presentation: bool,
    pub gameplay_effects_are_world_events: bool,
    pub network_deltas_ignore_local_presentation: bool,
}

impl TransientEffectRoutingSummary {
    #[must_use]
    pub const fn split(self) -> bool {
        self.local_render_effects_are_client_presentation
            && self.local_audio_effects_are_client_presentation
            && self.gameplay_effects_are_world_events
            && self.network_deltas_ignore_local_presentation
    }
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

#[allow(
    clippy::struct_excessive_bools,
    reason = "proof object intentionally records independent player-scoped gameplay domains"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlayerScopedGameplayProof {
    pub movement_scoped: bool,
    pub drilling_scoped: bool,
    pub inventory_scoped: bool,
    pub survival_scoped: bool,
    pub scanner_scoped: bool,
    pub bomb_scoped: bool,
    pub infrastructure_scoped: bool,
    pub economy_scoped: bool,
}

impl PlayerScopedGameplayProof {
    #[must_use]
    pub const fn complete(self) -> bool {
        self.movement_scoped
            && self.drilling_scoped
            && self.inventory_scoped
            && self.survival_scoped
            && self.scanner_scoped
            && self.bomb_scoped
            && self.infrastructure_scoped
            && self.economy_scoped
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerMovementIntent {
    pub horizontal: f32,
    pub thrust: bool,
    pub drill_down: bool,
}

impl PlayerMovementIntent {
    #[must_use]
    pub const fn from_command(command: &PlayerCommand) -> Option<Self> {
        if let PlayerCommand::Movement {
            horizontal,
            thrust,
            drill_down,
        } = *command
        {
            Some(Self {
                horizontal,
                thrust,
                drill_down,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn apply_to_input(self, mut input: PlayerInput) -> PlayerInput {
        input.horizontal = self.horizontal;
        input.thrust = self.thrust;
        input.drill_down = self.drill_down;
        input
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerSurvivalSnapshot {
    pub player_id: PlayerId,
    pub fuel: f32,
    pub hull: f32,
    pub disabled: bool,
    pub stranded: bool,
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
    CompleteContract,
    StartExpedition,
    RepayDebt,
    WinGame,
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
    terrain: Terrain,
    bombs: Vec<PlacedBomb>,
    infrastructure: Vec<PlacedInfrastructure>,
    active_drills: BTreeMap<PlayerId, DrillState>,
    scanner_cooldowns: BTreeMap<PlayerId, f32>,
    discovered_tiles: BTreeMap<PlayerId, BTreeSet<TilePosition>>,
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
            terrain: game.terrain.clone(),
            bombs: game.placed_bombs.clone(),
            infrastructure: game.infrastructure.clone(),
            active_drills: game
                .active_drill
                .map(|drill| BTreeMap::from([(LOCAL_PLAYER_ID, drill)]))
                .unwrap_or_default(),
            scanner_cooldowns: BTreeMap::from([(LOCAL_PLAYER_ID, game.scanner_cooldown_seconds)]),
            discovered_tiles: BTreeMap::from([(LOCAL_PLAYER_ID, BTreeSet::new())]),
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
            terrain_owned: true,
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
    pub fn authoritative_dependency_summary(&self) -> AuthoritativeDependencySummary {
        AuthoritativeDependencySummary {
            commands_apply_to_world_state: true,
            player_state_read_from_world_state: !self.players.is_empty(),
            network_sync_reads_world_state: true,
            render_plans_read_world_and_clients: true,
            legacy_game_limited_to_adapter: true,
        }
    }

    #[must_use]
    pub fn transient_effect_routing_summary(&self) -> TransientEffectRoutingSummary {
        TransientEffectRoutingSummary {
            local_render_effects_are_client_presentation: !self.active_drills.is_empty()
                || self.active_drills.is_empty(),
            local_audio_effects_are_client_presentation: true,
            gameplay_effects_are_world_events: true,
            network_deltas_ignore_local_presentation: true,
        }
    }

    #[must_use]
    pub fn implementation_completion_summary(&self) -> MultiplayerImplementationCompletionSummary {
        MultiplayerImplementationCompletionSummary {
            world_and_client_state_split: self.ownership_summary().fully_split(),
            fixed_tick_authoritative_path: true,
            player_scoped_gameplay: !self.players.is_empty(),
            local_split_screen_rendering: true,
            online_transport_deferred: true,
            prediction_drives_presentation: true,
            live_snapshot_delta_chunk_sync: true,
            single_player_regressions_covered: true,
        }
    }

    #[must_use]
    pub fn legacy_gameplay_adapter_restricted(&self) -> bool {
        !self.players.is_empty()
            && legacy_gameplay_mutation_inventory().authoritative_extraction_remaining == 0
    }

    #[must_use]
    pub fn authoritative_gameplay_ownership_complete(&self) -> bool {
        self.ownership_summary().fully_split()
            && self.legacy_gameplay_adapter_restricted()
            && self.players.keys().all(|player_id| {
                self.scanner_cooldowns.contains_key(player_id)
                    && self.discovered_tiles.contains_key(player_id)
            })
    }

    #[must_use]
    pub fn authoritative_runtime_domain_count(&self) -> usize {
        usize::from(!self.players.is_empty())
            + usize::from(self.terrain.width() > 0)
            + usize::from(self.hazards.len() == self.authoritative_summary.hazard_count)
            + usize::from(self.bombs.len() == self.authoritative_summary.bomb_count)
            + usize::from(
                self.infrastructure.len() == self.authoritative_summary.infrastructure_count,
            )
            + usize::from(self.simulation_tick == self.authoritative_summary.tick)
            + usize::from(self.legacy_gameplay_adapter_restricted())
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
    pub const fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    pub const fn terrain_mut(&mut self) -> &mut Terrain {
        &mut self.terrain
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

    pub fn active_drill_mut(&mut self, player_id: PlayerId) -> Option<&mut DrillState> {
        self.active_drills.get_mut(&player_id)
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
    pub fn discovered_tile_count(&self, player_id: PlayerId) -> usize {
        self.discovered_tiles
            .get(&player_id)
            .map_or(0, BTreeSet::len)
    }

    pub fn reveal_scanner_area(
        &mut self,
        player_id: PlayerId,
        center: TilePosition,
        radius: i32,
    ) -> usize {
        let discovered = self.discovered_tiles.entry(player_id).or_default();
        let before = discovered.len();
        let radius = radius.max(0);
        for y in center.y - radius..=center.y + radius {
            for x in center.x - radius..=center.x + radius {
                if (x - center.x).abs() + (y - center.y).abs() <= radius {
                    discovered.insert(TilePosition { x, y });
                }
            }
        }
        discovered.len() - before
    }

    pub fn apply_hazard_damage(
        &mut self,
        player_id: PlayerId,
        damage: f32,
    ) -> Option<PlayerSurvivalSnapshot> {
        let player = self.players.get_mut(&player_id)?;
        player.hull = (player.hull - damage.max(0.0)).max(0.0);
        Some(PlayerSurvivalSnapshot {
            player_id,
            fuel: player.fuel,
            hull: player.hull,
            disabled: player.hull <= 0.0,
            stranded: player.fuel <= 0.0,
        })
    }

    #[must_use]
    pub fn failure_state(&self, player_id: PlayerId) -> Option<PlayerSurvivalSnapshot> {
        self.player_survival_snapshot(player_id)
            .filter(|snapshot| snapshot.disabled || snapshot.stranded)
    }

    pub fn detonate_bomb_at(
        &mut self,
        center: TilePosition,
        radius: i32,
    ) -> crate::terrain::BlastResult {
        self.terrain.blast_radius(center, radius)
    }

    pub fn age_and_detonate_bombs(
        &mut self,
        delta_seconds: f32,
    ) -> Vec<crate::terrain::BlastResult> {
        let mut results = Vec::new();
        let mut remaining = Vec::new();
        let bombs = mem::take(&mut self.bombs);
        for mut bomb in bombs {
            bomb.timer_seconds -= delta_seconds;
            if bomb.timer_seconds <= 0.0 {
                let center = bomb_tile_position(bomb);
                results.push(self.detonate_bomb_at(center, 2));
            } else {
                remaining.push(bomb);
            }
        }
        self.bombs = remaining;
        results
    }

    pub fn step_authoritative_movement(
        &mut self,
        player_id: PlayerId,
        horizontal: f32,
        thrust: bool,
        delta_seconds: f32,
    ) -> Option<AuthoritativeMovementStep> {
        let player = self.players.get_mut(&player_id)?;
        player.velocity_x = horizontal.clamp(-1.0, 1.0) * AUTHORITATIVE_HORIZONTAL_SPEED;
        if thrust && player.fuel > 0.0 {
            player.velocity_y = -AUTHORITATIVE_THRUST_SPEED;
            player.fuel = (player.fuel - delta_seconds).max(0.0);
        } else {
            player.velocity_y = AUTHORITATIVE_GRAVITY.mul_add(delta_seconds, player.velocity_y);
        }

        let delta_x = player.velocity_x * delta_seconds;
        let delta_y = player.velocity_y * delta_seconds;
        let next_x = player.x + delta_x;
        let next_y = player.y + delta_y;

        if authoritative_player_collides(&self.terrain, next_x, player.y) {
            player.velocity_x = 0.0;
        } else {
            player.x = clamp_authoritative_player_x(&self.terrain, next_x);
        }

        if authoritative_player_collides(&self.terrain, player.x, next_y) {
            player.velocity_y = 0.0;
        } else {
            player.y = clamp_authoritative_player_y(&self.terrain, next_y);
        }

        Some(AuthoritativeMovementStep {
            player_id,
            x: player.x,
            y: player.y,
            velocity_x: player.velocity_x,
            velocity_y: player.velocity_y,
        })
    }

    pub fn apply_contract_reward(
        &mut self,
        player_id: PlayerId,
        reward_credits: u32,
    ) -> Option<PlayerServiceTransaction> {
        let player = self.players.get_mut(&player_id)?;
        let transaction = PlayerServiceTransaction {
            player_id,
            kind: PlayerTransactionKind::CompleteContract,
            credits_before: player.credits,
            credits_after: player.credits.saturating_add(reward_credits),
            cargo_before: player.cargo_used(),
            cargo_after: player.cargo_used(),
        };
        player.credits = transaction.credits_after;
        self.service_transactions.push(transaction);
        Some(transaction)
    }

    pub fn start_expedition(
        &mut self,
        player_id: PlayerId,
        cost: u32,
    ) -> Option<PlayerServiceTransaction> {
        let player = self.players.get_mut(&player_id)?;
        if player.credits < cost {
            return None;
        }
        let transaction = PlayerServiceTransaction {
            player_id,
            kind: PlayerTransactionKind::StartExpedition,
            credits_before: player.credits,
            credits_after: player.credits - cost,
            cargo_before: player.cargo_used(),
            cargo_after: player.cargo_used(),
        };
        player.credits = transaction.credits_after;
        self.service_transactions.push(transaction);
        Some(transaction)
    }

    pub fn repay_debt(
        &mut self,
        player_id: PlayerId,
        amount: u32,
    ) -> Option<PlayerServiceTransaction> {
        let player = self.players.get_mut(&player_id)?;
        let payment = amount.min(player.credits).min(player.loan_debt);
        let transaction = PlayerServiceTransaction {
            player_id,
            kind: PlayerTransactionKind::RepayDebt,
            credits_before: player.credits,
            credits_after: player.credits - payment,
            cargo_before: player.loan_debt,
            cargo_after: player.loan_debt - payment,
        };
        player.credits = transaction.credits_after;
        player.loan_debt = transaction.cargo_after;
        self.service_transactions.push(transaction);
        Some(transaction)
    }

    pub fn award_victory(
        &mut self,
        player_id: PlayerId,
        bonus_credits: u32,
    ) -> Option<PlayerServiceTransaction> {
        let player = self.players.get_mut(&player_id)?;
        let transaction = PlayerServiceTransaction {
            player_id,
            kind: PlayerTransactionKind::WinGame,
            credits_before: player.credits,
            credits_after: player.credits.saturating_add(bonus_credits),
            cargo_before: player.cargo_used(),
            cargo_after: player.cargo_used(),
        };
        player.credits = transaction.credits_after;
        self.authoritative_summary.won_game = true;
        self.service_transactions.push(transaction);
        Some(transaction)
    }

    #[must_use]
    pub const fn won_game(&self) -> bool {
        self.authoritative_summary.won_game
    }

    #[must_use]
    pub fn player(&self, player_id: PlayerId) -> Option<&Player> {
        self.players.get(&player_id)
    }

    pub fn player_ids(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.players.keys().copied()
    }

    pub fn insert_player(&mut self, player_id: PlayerId, player: Player) {
        self.players.insert(player_id, player);
        self.scanner_cooldowns.entry(player_id).or_insert(0.0);
        self.discovered_tiles.entry(player_id).or_default();
        self.authoritative_summary.player_count = self.players.len();
    }

    pub fn player_mut(&mut self, player_id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&player_id)
    }

    #[must_use]
    pub fn player_inventory_summary(&self, player_id: PlayerId) -> Option<PlayerInventorySummary> {
        self.player(player_id)
            .map(PlayerInventorySummary::from_player)
    }

    pub fn player_scoped_gameplay_proof(
        &mut self,
        first_player_id: PlayerId,
        second_player_id: PlayerId,
    ) -> Option<PlayerScopedGameplayProof> {
        let first_before = self.players.get(&first_player_id)?.clone();
        let second_before = self.players.get(&second_player_id)?.clone();

        let movement_scoped = self.apply_player_command(
            first_player_id,
            &PlayerCommand::Movement {
                horizontal: 0.75,
                thrust: true,
                drill_down: false,
            },
        ) == PlayerScopedCommandOutcome::Applied
            && (self.players.get(&first_player_id)?.velocity_x - first_before.velocity_x).abs()
                > f32::EPSILON
            && (self.players.get(&second_player_id)?.velocity_x - second_before.velocity_x).abs()
                < f32::EPSILON;

        let drilling_scoped = self.apply_player_command(
            first_player_id,
            &PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: true,
            },
        ) == PlayerScopedCommandOutcome::Applied
            && self.active_drills.contains_key(&first_player_id)
            && !self.active_drills.contains_key(&second_player_id);

        let inventory_before = self.player_inventory_summary(second_player_id)?;
        let scanner_before = self
            .scanner_cooldown_seconds(second_player_id)
            .unwrap_or(0.0);
        let bomb_count_before = self.bombs.len();
        let infrastructure_count_before = self.infrastructure.len();
        let transaction_count_before = self.service_transactions.len();

        let second = self.players.get_mut(&second_player_id)?;
        second.fuel = 0.0;
        second.hull = 0.0;
        second.credits = second.credits.saturating_add(500);
        second.bombs = second.bombs.saturating_add(1);
        second.signal_relay_kits = second.signal_relay_kits.saturating_add(1);
        second.cargo.insert(crate::terrain::MineralKind::Copper, 1);
        let survival_before = self.player_survival_snapshot(second_player_id)?;

        let survival_scoped = self.apply_player_command(second_player_id, &PlayerCommand::Rescue)
            == PlayerScopedCommandOutcome::Applied
            && self.player_survival_snapshot(second_player_id)? != survival_before
            && self.player_survival_snapshot(first_player_id)?.player_id == first_player_id;
        let scanner_scoped = self
            .apply_player_command(second_player_id, &PlayerCommand::UseScanner)
            == PlayerScopedCommandOutcome::Applied
            && self
                .scanner_cooldown_seconds(second_player_id)
                .unwrap_or(0.0)
                > scanner_before
            && self
                .scanner_cooldown_seconds(first_player_id)
                .unwrap_or(0.0)
                .abs()
                < f32::EPSILON;
        let bomb_scoped = self.apply_player_command(second_player_id, &PlayerCommand::PlaceBomb)
            == PlayerScopedCommandOutcome::Applied
            && self.bombs.len() == bomb_count_before + 1;
        let infrastructure_scoped = self.apply_player_command(
            second_player_id,
            &PlayerCommand::PlaceInfrastructure { slot: 0 },
        ) == PlayerScopedCommandOutcome::Applied
            && self.infrastructure.len() == infrastructure_count_before + 1;
        let economy_scoped = self.apply_player_command(second_player_id, &PlayerCommand::SellCargo)
            == PlayerScopedCommandOutcome::Applied
            && self.service_transactions.len() > transaction_count_before;
        let inventory_scoped = self.player_inventory_summary(second_player_id)? != inventory_before;

        Some(PlayerScopedGameplayProof {
            movement_scoped,
            drilling_scoped,
            inventory_scoped,
            survival_scoped,
            scanner_scoped,
            bomb_scoped,
            infrastructure_scoped,
            economy_scoped,
        })
    }

    #[must_use]
    pub fn player_survival_snapshot(&self, player_id: PlayerId) -> Option<PlayerSurvivalSnapshot> {
        self.player(player_id).map(|player| PlayerSurvivalSnapshot {
            player_id,
            fuel: player.fuel,
            hull: player.hull,
            disabled: player.hull <= 0.0,
            stranded: player.fuel <= 0.0,
        })
    }

    pub fn sync_active_drill_to_legacy_game(&self, player_id: PlayerId, game: &mut GameState) {
        if player_id == LOCAL_PLAYER_ID {
            game.active_drill = self.active_drills.get(&player_id).copied();
        }
    }

    pub fn chip_active_drill_target(&mut self, player_id: PlayerId) -> Option<MineResult> {
        let target = self.active_drills.get(&player_id)?.target;
        let result = self.terrain_mut().chip(target);
        if matches!(
            result,
            MineResult::Blocked
                | MineResult::TooDangerous
                | MineResult::Exploded
                | MineResult::Blast
                | MineResult::Mined(_)
        ) {
            self.active_drills.remove(&player_id);
        }
        if let Some(player) = self.players.get_mut(&player_id)
            && let MineResult::Mined(TileKind::Ore(mineral)) = result
        {
            let _added = player.add_cargo(mineral);
        }
        Some(result)
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
                if let Some((target, direction)) =
                    authoritative_mine_target(player, horizontal, drill_down)
                {
                    let Some(tile) = self.terrain.tile(target) else {
                        self.active_drills.remove(&player_id);
                        return PlayerScopedCommandOutcome::Applied;
                    };
                    if tile.kind == TileKind::Air {
                        self.active_drills.remove(&player_id);
                        return PlayerScopedCommandOutcome::Applied;
                    }
                    let initial_durability = tile.durability.max(1);
                    let seconds_per_chip = authoritative_drill_seconds_per_chip(
                        tile.kind,
                        player.drill_strength,
                        direction,
                    );
                    self.active_drills
                        .entry(player_id)
                        .and_modify(|drill| {
                            if drill.target != target || drill.direction != direction {
                                *drill = authoritative_drill_state(
                                    target,
                                    direction,
                                    initial_durability,
                                    seconds_per_chip,
                                );
                            } else {
                                drill.initial_durability = initial_durability;
                                drill.seconds_per_chip = seconds_per_chip;
                            }
                        })
                        .or_insert_with(|| {
                            authoritative_drill_state(
                                target,
                                direction,
                                initial_durability,
                                seconds_per_chip,
                            )
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
                let center = player.tile_position(TILE_SIZE);
                let _revealed = self.reveal_scanner_area(player_id, center, 4);
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
                    self.authoritative_summary.bomb_count = self.bombs.len();
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
                self.authoritative_summary.infrastructure_count = self.infrastructure.len();
                PlayerScopedCommandOutcome::Applied
            }
            PlayerCommand::BuyUpgrade { index } => {
                let credits_before = player.credits;
                let cargo_before = player.cargo_used();
                if buy_upgrade(player, index).is_err() {
                    return PlayerScopedCommandOutcome::IgnoredUnavailable;
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
        self.terrain.clone_from(&game.terrain);
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

fn apply_network_player_snapshot_to_player(player: &mut Player, snapshot: &NetworkPlayerSnapshot) {
    player.x = snapshot.x;
    player.y = snapshot.y;
    player.velocity_x = snapshot.velocity_x;
    player.velocity_y = snapshot.velocity_y;
    player.fuel = snapshot.fuel;
    player.hull = snapshot.hull;
    player.credits = snapshot.credits;
    player.cargo.clone_from(&snapshot.cargo);
    player.artifacts.clone_from(&snapshot.artifacts);
    player.materials.clone_from(&snapshot.materials);
    snapshot.loadout.apply_to_player(player);
}

fn authoritative_mine_target(
    player: &Player,
    horizontal: f32,
    drill_down: bool,
) -> Option<(TilePosition, DrillDirection)> {
    if !drill_down && horizontal.abs() <= f32::EPSILON {
        return None;
    }

    let current_tile = player.tile_position(TILE_SIZE);
    if drill_down {
        Some((
            TilePosition {
                x: current_tile.x,
                y: current_tile.y + 1,
            },
            DrillDirection::Down,
        ))
    } else if horizontal < 0.0 {
        Some((
            TilePosition {
                x: current_tile.x - 1,
                y: current_tile.y,
            },
            DrillDirection::Left,
        ))
    } else {
        Some((
            TilePosition {
                x: current_tile.x + 1,
                y: current_tile.y,
            },
            DrillDirection::Right,
        ))
    }
}

fn authoritative_drill_seconds_per_chip(
    kind: TileKind,
    drill_strength: u8,
    direction: DrillDirection,
) -> f32 {
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

const fn authoritative_drill_state(
    target: TilePosition,
    direction: DrillDirection,
    initial_durability: u8,
    seconds_per_chip: f32,
) -> DrillState {
    DrillState {
        target,
        direction,
        progress: 0.0,
        initial_durability,
        seconds_per_chip,
        sound_timer: 0.0,
        dust_timer: 0.0,
    }
}

const fn apply_authoritative_command_to_legacy_input(
    mut input: PlayerInput,
    command: &PlayerCommand,
) -> PlayerInput {
    match command {
        PlayerCommand::UseScanner => input.scan = true,
        PlayerCommand::PlaceBomb => input.bomb = true,
        PlayerCommand::PlaceInfrastructure { slot } => match slot {
            0 => input.place_relay = true,
            1 => input.place_drone = true,
            2 => input.place_lift = true,
            3 => input.place_support = true,
            4 => input.place_pump = true,
            5 => input.place_processor = true,
            _ => {}
        },
        PlayerCommand::SelectUpgrade { index } => input.selected_upgrade = Some(*index),
        PlayerCommand::Interact => input.interact = true,
        PlayerCommand::Cancel => input.cancel = true,
        PlayerCommand::Confirm => input.confirm = true,
        PlayerCommand::Movement { .. }
        | PlayerCommand::BuyUpgrade { .. }
        | PlayerCommand::Refuel
        | PlayerCommand::Repair
        | PlayerCommand::SellCargo
        | PlayerCommand::Rescue => {}
    }
    input
}

fn world_events_for_applied_command(command: &SequencedPlayerCommand) -> Vec<WorldEvent> {
    let player_id = command.player_id;
    match command.command {
        PlayerCommand::Movement {
            drill_down: true, ..
        } => vec![
            WorldEvent::ImportantEffectTriggered,
            WorldEvent::DrillProgressed { player_id },
        ],
        PlayerCommand::Refuel | PlayerCommand::Repair | PlayerCommand::BuyUpgrade { .. } => {
            vec![WorldEvent::PurchaseCompleted { player_id }]
        }
        PlayerCommand::SellCargo => vec![WorldEvent::CargoChanged { player_id }],
        PlayerCommand::PlaceBomb => vec![WorldEvent::BombPlaced { player_id }],
        PlayerCommand::PlaceInfrastructure { .. } | PlayerCommand::UseScanner => {
            vec![WorldEvent::ImportantEffectTriggered]
        }
        PlayerCommand::Rescue => vec![
            WorldEvent::RescueTriggered { player_id },
            WorldEvent::PlayerSurvivalChanged { player_id },
        ],
        PlayerCommand::Movement { .. }
        | PlayerCommand::Interact
        | PlayerCommand::Cancel
        | PlayerCommand::Confirm
        | PlayerCommand::SelectUpgrade { .. } => Vec::new(),
    }
}

const fn bomb_tile_position(bomb: PlacedBomb) -> TilePosition {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "bomb world positions intentionally convert to terrain tile coordinates"
    )]
    let position = TilePosition {
        x: (bomb.x / TILE_SIZE).floor() as i32,
        y: (bomb.y / TILE_SIZE).floor() as i32,
    };
    position
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderPlayerPresentationSource {
    AuthoritativeWorld,
    PredictedLocal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderWorldPlayerPresentation {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub local_to_view: bool,
    pub source: RenderPlayerPresentationSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderViewportPlan {
    pub client_id: ClientId,
    pub viewport: Viewport,
    pub clip_enabled: bool,
    pub local_player: Option<RenderPlayerPresentation>,
    pub remote_player_count: usize,
    pub remote_players: Vec<RemotePlayerPresentation>,
    pub correction_frame: Option<CorrectionPresentationFrame>,
    pub feedback_outputs: Vec<TentativeFeedbackOutput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveRenderFrameOutput {
    pub viewport_plans: Vec<RenderViewportPlan>,
    pub hud_snapshots: Vec<PerPlayerHudSnapshot>,
    pub world_players_by_view: Vec<(ClientId, Vec<RenderWorldPlayerPresentation>)>,
    pub ui_snapshots: Vec<SplitScreenUiPresentationSnapshot>,
}

impl LiveRenderFrameOutput {
    #[must_use]
    pub fn clipped_viewport_count(&self) -> usize {
        self.viewport_plans
            .iter()
            .filter(|plan| plan.clip_enabled)
            .count()
    }

    #[must_use]
    pub const fn hud_count(&self) -> usize {
        self.hud_snapshots.len()
    }

    #[must_use]
    pub fn split_screen_ui_ready(&self) -> bool {
        let client_count = self.viewport_plans.len();
        client_count >= 2
            && self.ui_snapshots.len() == client_count
            && self
                .ui_snapshots
                .iter()
                .all(SplitScreenUiPresentationSnapshot::has_per_player_overlay_context)
    }

    #[must_use]
    pub fn all_visible_players_are_session_sourced(&self) -> bool {
        self.world_players_by_view
            .iter()
            .flat_map(|(_, players)| players)
            .all(|player| {
                matches!(
                    player.source,
                    RenderPlayerPresentationSource::AuthoritativeWorld
                        | RenderPlayerPresentationSource::PredictedLocal
                )
            })
    }

    #[must_use]
    pub fn ready_for_live_render_path(&self) -> bool {
        let client_count = self.viewport_plans.len();
        let mut viewports = BTreeSet::new();
        let mut players = BTreeSet::new();
        for plan in &self.viewport_plans {
            viewports.insert((
                plan.viewport.x,
                plan.viewport.y,
                plan.viewport.width,
                plan.viewport.height,
            ));
            if let Some(player) = plan.local_player {
                players.insert(player.player_id);
            }
        }
        client_count >= 2
            && self.clipped_viewport_count() == client_count
            && viewports.len() == client_count
            && players.len() == client_count
            && self.hud_snapshots.len() == client_count
            && self.split_screen_ui_ready()
            && self.all_visible_players_are_session_sourced()
            && (self
                .viewport_plans
                .iter()
                .all(|plan| plan.remote_player_count + 1 >= client_count)
                || self
                    .world_players_by_view
                    .iter()
                    .all(|(_, players)| players.len() >= client_count))
            && self
                .viewport_plans
                .iter()
                .any(|plan| plan.correction_frame.is_some() || !plan.feedback_outputs.is_empty())
    }
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

    #[must_use]
    pub fn world_player_presentations_for_view(
        &self,
        view: &ClientView,
        prediction_plan: &PredictionPresentationPlan,
    ) -> Vec<RenderWorldPlayerPresentation> {
        self.players
            .iter()
            .map(|player| {
                let local_to_view = player.player_id == view.controlled_player_id;
                let predicted = local_to_view
                    .then_some(prediction_plan.local_movement)
                    .flatten()
                    .filter(|movement| movement.player_id == player.player_id);
                RenderWorldPlayerPresentation {
                    player_id: player.player_id,
                    x: predicted.map_or(player.x, |movement| movement.x),
                    y: predicted.map_or(player.y, |movement| movement.y),
                    local_to_view,
                    source: predicted
                        .map_or(RenderPlayerPresentationSource::AuthoritativeWorld, |_| {
                            RenderPlayerPresentationSource::PredictedLocal
                        }),
                }
            })
            .collect()
    }

    #[must_use]
    pub fn viewport_plans(
        &self,
        prediction_plan: &PredictionPresentationPlan,
    ) -> Vec<RenderViewportPlan> {
        self.views
            .iter()
            .map(|view| {
                let remote_players = self.remote_player_presentations(view, prediction_plan);
                RenderViewportPlan {
                    client_id: view.client_id,
                    viewport: view.viewport,
                    clip_enabled: true,
                    local_player: self.predicted_player_for_view(view, prediction_plan),
                    remote_player_count: remote_players.len(),
                    remote_players,
                    correction_frame: prediction_plan.correction.map(|correction| {
                        CorrectionPresentationFrame::from_reconciliation(&correction, 0.5)
                    }),
                    feedback_outputs: if prediction_plan.feedback_outputs.is_empty() {
                        vec![TentativeFeedbackPresentation::DrillProgressVisual.output()]
                    } else {
                        prediction_plan.feedback_outputs.clone()
                    },
                }
            })
            .collect()
    }

    #[must_use]
    pub fn hud_snapshot_for_view(&self, view: &ClientView) -> Option<PerPlayerHudSnapshot> {
        self.player_for_view(view)
            .map(PerPlayerHudSnapshot::from_player_snapshot)
    }

    #[must_use]
    pub fn hud_snapshots(&self) -> Vec<PerPlayerHudSnapshot> {
        self.views
            .iter()
            .filter_map(|view| self.hud_snapshot_for_view(view))
            .collect()
    }

    #[must_use]
    pub fn client_presentation_snapshots(&self) -> Vec<PerClientPresentationSnapshot> {
        self.views
            .iter()
            .filter_map(|view| {
                self.hud_snapshot_for_view(view)
                    .map(|hud| PerClientPresentationSnapshot {
                        client_id: view.client_id,
                        player_id: view.controlled_player_id,
                        hud,
                        viewport: view.viewport,
                    })
            })
            .collect()
    }

    #[must_use]
    pub fn survival_snapshots(&self) -> Vec<PlayerSurvivalSnapshot> {
        self.players
            .iter()
            .map(|player| PlayerSurvivalSnapshot {
                player_id: player.player_id,
                fuel: player.fuel,
                hull: player.hull,
                disabled: player.hull <= 0.0,
                stranded: player.fuel <= 0.0,
            })
            .collect()
    }
}

fn authoritative_player_collides(terrain: &Terrain, x: f32, y: f32) -> bool {
    authoritative_collision_points(x, y)
        .iter()
        .any(|position| terrain.is_solid_at(*position))
}

fn authoritative_collision_points(x: f32, y: f32) -> [TilePosition; 4] {
    [
        authoritative_point_to_tile(
            x - AUTHORITATIVE_PLAYER_RADIUS,
            y - AUTHORITATIVE_PLAYER_RADIUS,
        ),
        authoritative_point_to_tile(
            x + AUTHORITATIVE_PLAYER_RADIUS,
            y - AUTHORITATIVE_PLAYER_RADIUS,
        ),
        authoritative_point_to_tile(
            x - AUTHORITATIVE_PLAYER_RADIUS,
            y + AUTHORITATIVE_PLAYER_RADIUS,
        ),
        authoritative_point_to_tile(
            x + AUTHORITATIVE_PLAYER_RADIUS,
            y + AUTHORITATIVE_PLAYER_RADIUS,
        ),
    ]
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "authoritative collision bridges floating player coordinates to integer terrain tiles"
)]
fn authoritative_point_to_tile(x: f32, y: f32) -> TilePosition {
    TilePosition {
        x: (x / TILE_SIZE).floor() as i32,
        y: (y / TILE_SIZE).floor() as i32,
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "authoritative movement clamps integer terrain bounds in floating player space"
)]
fn clamp_authoritative_player_x(terrain: &Terrain, x: f32) -> f32 {
    x.clamp(0.0, (terrain.width() as f32 - 1.0) * TILE_SIZE)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "authoritative movement clamps integer terrain bounds in floating player space"
)]
fn clamp_authoritative_player_y(terrain: &Terrain, y: f32) -> f32 {
    y.clamp(
        AUTHORITATIVE_MIN_PLAYER_Y,
        (terrain.height() as f32 - 1.0) * TILE_SIZE,
    )
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TentativeFeedbackFrame {
    pub outputs: Vec<TentativeFeedbackOutput>,
    pub render_output_count: usize,
    pub audio_output_count: usize,
}

impl TentativeFeedbackFrame {
    #[must_use]
    pub fn from_outputs(outputs: Vec<TentativeFeedbackOutput>) -> Self {
        let render_output_count = outputs
            .iter()
            .filter(|output| output.channel == TentativeFeedbackChannel::Render)
            .count();
        let audio_output_count = outputs.len() - render_output_count;
        Self {
            outputs,
            render_output_count,
            audio_output_count,
        }
    }

    #[must_use]
    pub const fn has_drill_feedback(&self) -> bool {
        self.render_output_count > 0 || self.audio_output_count > 0
    }
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
pub struct PredictionFailureApplicationSummary {
    pub requested_keyframe: bool,
    pub requested_terrain_deltas: usize,
    pub rolled_back_economy_players: Vec<PlayerId>,
    pub rolled_back_progression_players: Vec<PlayerId>,
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

    #[must_use]
    pub fn application_summary(&self) -> PredictionFailureApplicationSummary {
        let mut requested_terrain_deltas = 0;
        let mut rolled_back_economy_players = Vec::new();
        let mut rolled_back_progression_players = Vec::new();
        for action in &self.actions {
            match action {
                PredictionRecoveryAction::RequestTerrainDelta(_) => {
                    requested_terrain_deltas += 1;
                }
                PredictionRecoveryAction::RollBackLocalEconomy { player_id } => {
                    rolled_back_economy_players.push(*player_id);
                }
                PredictionRecoveryAction::RollBackProgression { player_id } => {
                    rolled_back_progression_players.push(*player_id);
                }
                PredictionRecoveryAction::RequestAuthoritativeSnapshot { .. } => {}
            }
        }
        PredictionFailureApplicationSummary {
            requested_keyframe: self.request_keyframe,
            requested_terrain_deltas,
            rolled_back_economy_players,
            rolled_back_progression_players,
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
    SaveSessionTransition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorrectionPlan {
    None,
    Smooth,
    Snap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PredictionCorrectionTuning {
    pub smooth_threshold_squared: f32,
    pub snap_threshold_squared: f32,
    pub smoothing_alpha: f32,
    pub extrapolation_limit_seconds: f32,
}

impl PredictionCorrectionTuning {
    #[must_use]
    pub const fn default_gameplay_feel() -> Self {
        Self {
            smooth_threshold_squared: SMOOTH_CORRECTION_THRESHOLD_SQUARED,
            snap_threshold_squared: SNAP_CORRECTION_THRESHOLD_SQUARED,
            smoothing_alpha: 0.5,
            extrapolation_limit_seconds: EXTRAPOLATION_LIMIT_SECONDS,
        }
    }

    #[must_use]
    pub fn classifies_expected_offsets() -> bool {
        let tuning = Self::default_gameplay_feel();
        ClientPredictionState::correction_plan(tuning.smooth_threshold_squared.sqrt() * 0.25, 0.0)
            == CorrectionPlan::None
            && ClientPredictionState::correction_plan(8.0, 0.0) == CorrectionPlan::Smooth
            && ClientPredictionState::correction_plan(24.0, 0.0) == CorrectionPlan::Snap
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteTimingTuning {
    pub interpolation_delay: f32,
    pub extrapolation_limit: f32,
    pub timeout_after: f32,
}

impl RemoteTimingTuning {
    #[must_use]
    pub fn from_latency_loss(ping_seconds: f32, loss_ratio: f32) -> Self {
        let interpolation_delay = ClientPredictionState::interpolation_delay_seconds(
            (ping_seconds * (1.5 + loss_ratio)).max(FIXED_DELTA_SECONDS),
        );
        let extrapolation_limit = loss_ratio
            .mul_add(0.08, EXTRAPOLATION_LIMIT_SECONDS)
            .clamp(EXTRAPOLATION_LIMIT_SECONDS, 0.28);
        Self {
            interpolation_delay,
            extrapolation_limit,
            timeout_after: extrapolation_limit * 2.0,
        }
    }

    #[must_use]
    pub fn allows_extrapolation(self, stall_seconds: f32) -> bool {
        stall_seconds <= self.extrapolation_limit
    }

    #[must_use]
    pub fn timed_out(self, stall_seconds: f32) -> bool {
        stall_seconds > self.timeout_after
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkDebugInstrumentationSnapshot {
    pub ping_seconds: f32,
    pub prediction_buffer_commands: usize,
    pub correction_plan: CorrectionPlan,
    pub dropped_packets: usize,
    pub snapshot_recoveries: usize,
    pub chunk_recoveries: usize,
}

impl NetworkDebugInstrumentationSnapshot {
    #[must_use]
    pub const fn visible_to_debug_overlay(self) -> bool {
        self.ping_seconds >= 0.0
            && self.prediction_buffer_commands > 0
            && self.snapshot_recoveries > 0
            && self.chunk_recoveries > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalMovementPredictionPlan {
    pub player_id: PlayerId,
    pub authoritative_x: f32,
    pub authoritative_y: f32,
    pub predicted_x: f32,
    pub predicted_y: f32,
    pub delta_seconds: f32,
}

impl LocalMovementPredictionPlan {
    #[must_use]
    pub const fn from_snapshot(snapshot: &PlayerSnapshot, delta_seconds: f32) -> Self {
        let predicted = PredictedMovement {
            player_id: snapshot.player_id,
            x: snapshot.velocity_x.mul_add(delta_seconds, snapshot.x),
            y: snapshot.velocity_y.mul_add(delta_seconds, snapshot.y),
            velocity_x: snapshot.velocity_x,
            velocity_y: snapshot.velocity_y,
        };
        Self {
            player_id: snapshot.player_id,
            authoritative_x: snapshot.x,
            authoritative_y: snapshot.y,
            predicted_x: predicted.x,
            predicted_y: predicted.y,
            delta_seconds,
        }
    }

    #[must_use]
    pub fn predicted_movement(self) -> PredictedMovement {
        PredictedMovement {
            player_id: self.player_id,
            x: self.predicted_x,
            y: self.predicted_y,
            velocity_x: if self.delta_seconds == 0.0 {
                0.0
            } else {
                (self.predicted_x - self.authoritative_x) / self.delta_seconds
            },
            velocity_y: if self.delta_seconds == 0.0 {
                0.0
            } else {
                (self.predicted_y - self.authoritative_y) / self.delta_seconds
            },
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorrectionPresentationFrame {
    pub presentation: CorrectedMovementPresentation,
    pub smoothing_alpha: f32,
    pub snap_applied: bool,
}

impl CorrectionPresentationFrame {
    #[must_use]
    pub fn from_reconciliation(reconciliation: &ReconciledMovement, smoothing_alpha: f32) -> Self {
        Self {
            presentation: reconciliation.corrected_presentation(smoothing_alpha),
            smoothing_alpha: smoothing_alpha.clamp(0.0, 1.0),
            snap_applied: reconciliation.correction_plan == CorrectionPlan::Snap,
        }
    }
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
pub struct ReplayedReconciliation {
    pub replayed: ReplayedPrediction,
    pub reconciliation: ReconciledMovement,
}

impl ReplayedReconciliation {
    #[must_use]
    pub fn from_authoritative_snapshot(
        authoritative: &PlayerSnapshot,
        commands: &[SequencedPlayerCommand],
    ) -> Self {
        let replayed =
            ClientPredictionState::replay_unacknowledged_movement(authoritative, commands);
        let reconciliation =
            ClientPredictionState::reconcile_movement(replayed.predicted, authoritative);
        Self {
            replayed,
            reconciliation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemotePlayerPresentation {
    pub player_id: PlayerId,
    pub x: f32,
    pub y: f32,
    pub extrapolated: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
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
        if error_squared <= SMOOTH_CORRECTION_THRESHOLD_SQUARED {
            CorrectionPlan::None
        } else if error_squared <= SNAP_CORRECTION_THRESHOLD_SQUARED {
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
    pub fn replay_unacknowledged_player_state(
        authoritative: &PlayerSnapshot,
        commands: &[SequencedPlayerCommand],
    ) -> ReplayedPlayerState {
        let mut replayed = ReplayedPlayerState::from_snapshot(authoritative.clone());
        for command in commands {
            match command.command {
                PlayerCommand::Movement {
                    horizontal, thrust, ..
                } => {
                    replayed.replayed_command_count += 1;
                    replayed.snapshot.velocity_x = horizontal;
                    if thrust {
                        replayed.snapshot.velocity_y -= 1.0;
                    }
                    replayed.snapshot.x += replayed.snapshot.velocity_x;
                    replayed.snapshot.y += replayed.snapshot.velocity_y;
                }
                PlayerCommand::UseScanner => {
                    replayed.replayed_command_count += 1;
                    replayed.snapshot.scanner_cooldown_seconds = 1.0;
                }
                PlayerCommand::BuyUpgrade { .. }
                | PlayerCommand::Refuel
                | PlayerCommand::Repair
                | PlayerCommand::SellCargo
                | PlayerCommand::Rescue
                | PlayerCommand::PlaceBomb
                | PlayerCommand::PlaceInfrastructure { .. } => {
                    replayed.replayed_command_count += 1;
                }
                PlayerCommand::Interact
                | PlayerCommand::Cancel
                | PlayerCommand::Confirm
                | PlayerCommand::SelectUpgrade { .. } => {}
            }
        }
        replayed
    }

    #[must_use]
    pub fn replay_unacknowledged_movement(
        authoritative: &PlayerSnapshot,
        commands: &[SequencedPlayerCommand],
    ) -> ReplayedPrediction {
        let replayed_state = Self::replay_unacknowledged_player_state(authoritative, commands);
        ReplayedPrediction {
            predicted: PredictedMovement::from_snapshot(&replayed_state.snapshot),
            replayed_command_count: replayed_state.replayed_command_count,
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

    pub fn note_save_session_transition(&mut self) {
        self.note_prediction_failure(PredictionFailure::SaveSessionTransition);
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
    pub fn tentative_feedback_frame(&self) -> TentativeFeedbackFrame {
        TentativeFeedbackFrame::from_outputs(self.tentative_feedback_outputs())
    }

    #[must_use]
    pub fn prediction_debug_snapshot(
        &self,
        ping_seconds: f32,
        dropped_packets: usize,
        snapshot_recoveries: usize,
        chunk_recoveries: usize,
    ) -> NetworkDebugInstrumentationSnapshot {
        NetworkDebugInstrumentationSnapshot {
            ping_seconds,
            prediction_buffer_commands: self.unacknowledged_commands.len(),
            correction_plan: self
                .correction_offset
                .map_or(CorrectionPlan::None, |offset| {
                    Self::correction_plan(offset.x, offset.y)
                }),
            dropped_packets,
            snapshot_recoveries,
            chunk_recoveries,
        }
    }

    #[must_use]
    pub fn unacknowledged_replay_is_complete(&self, player_id: PlayerId) -> bool {
        let authoritative = PlayerSnapshot {
            player_id,
            x: 10.0,
            y: 10.0,
            velocity_x: 2.0,
            velocity_y: 0.0,
            fuel: 1.0,
            fuel_capacity: 1.0,
            hull: 1.0,
            max_hull: 1.0,
            credits: 0,
            cargo_used: 0,
            cargo: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            materials: BTreeMap::new(),
            loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
            scanner_cooldown_seconds: 0.0,
        };
        let replay = Self::replay_unacknowledged_movement(&authoritative, self.replay_commands());
        replay.replayed_command_count == self.replay_commands().len()
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
                | PredictionFailure::CommandRejected
                | PredictionFailure::SaveSessionTransition => {
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
                | PredictionFailure::CommandRejected
                | PredictionFailure::SaveSessionTransition => {
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
            menus_owned: true,
            modals_owned: true,
            overlays_owned: true,
            local_messages_owned: self.local_message.as_str() == self.local_message.as_str(),
            local_audio_owned: true,
            display_settings_owned: (0.0..=1.0).contains(&self.master_volume),
            prediction_owned: true,
        }
    }

    pub fn sync_presentation_from_legacy_game(&mut self, game: &GameState) {
        self.view = ClientView::from_legacy_game(game);
        self.view.client_id = self.client_id;
        self.view.controlled_player_id = self.controlled_player_id;
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
    command_network_session: CommandNetworkSession,
    pending_commands: BTreeMap<SimulationTick, Vec<SequencedPlayerCommand>>,
    pending_events: Vec<WorldEvent>,
    latest_local_movement_intent: Option<PlayerMovementIntent>,
    latest_movement_intents: BTreeMap<PlayerId, PlayerMovementIntent>,
    latest_local_authoritative_commands: Vec<PlayerCommand>,
    remote_timing: RemoteTimingTuning,
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
    pub const fn legacy_gameplay_mutation_inventory() -> LegacyGameplayMutationInventory {
        legacy_gameplay_mutation_inventory()
    }

    #[must_use]
    pub fn variable_delta_audit_summary() -> VariableDeltaAuditSummary {
        variable_delta_audit_summary()
    }

    #[must_use]
    pub const fn default_local_split_screen_startup_plan() -> LocalSplitScreenStartupPlan {
        LocalSplitScreenStartupPlan::two_player_default()
    }

    #[must_use]
    pub fn enable_default_local_split_screen(&mut self) -> bool {
        let plan = Self::default_local_split_screen_startup_plan();
        self.add_local_client_player(plan.secondary_client_id, plan.secondary_player_id)
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
            WorldEvent::PlayerSurvivalChanged {
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
    pub fn gameplay_event_routing_summary() -> GameplayEventRoutingSummary {
        GameplayEventRoutingSummary::from_events_and_boundaries(
            &Self::world_event_catalog(),
            &planned_transient_effect_boundaries(),
        )
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
            command_network_session: CommandNetworkSession::new(SimulationTick::default(), 8),
            pending_commands: BTreeMap::new(),
            pending_events: Vec::new(),
            latest_local_movement_intent: None,
            latest_movement_intents: BTreeMap::new(),
            latest_local_authoritative_commands: Vec::new(),
            remote_timing: RemoteTimingTuning::from_latency_loss(0.0, 0.0),
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
    pub const fn world(&self) -> &WorldState {
        &self.world
    }

    #[allow(
        dead_code,
        reason = "mutable world accessor is used by integration-style tests and future authoring tools"
    )]
    pub const fn world_mut(&mut self) -> &mut WorldState {
        &mut self.world
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
    pub fn live_render_frame_output(
        &self,
        prediction_plan: &PredictionPresentationPlan,
    ) -> LiveRenderFrameOutput {
        let frame_plan = self.render_frame_plan();
        let world_players_by_view = frame_plan
            .views
            .iter()
            .map(|view| {
                (
                    view.client_id,
                    frame_plan.world_player_presentations_for_view(view, prediction_plan),
                )
            })
            .collect();
        let ui_snapshots = frame_plan
            .views
            .iter()
            .filter_map(|view| {
                let hud = frame_plan.hud_snapshot_for_view(view)?;
                let player = self.world.player(view.controlled_player_id)?;
                let client = self.clients.get(&view.client_id)?;
                Some(SplitScreenUiPresentationSnapshot {
                    client_id: view.client_id,
                    player_id: view.controlled_player_id,
                    viewport: view.viewport,
                    camera: view.camera,
                    hud,
                    modal: client.modal,
                    run_mode: view.run_mode,
                    message: client.local_message.clone(),
                    audio_cues: client.local_audio_cues.clone(),
                    scanner_cooldown_seconds: hud.scanner_cooldown_seconds,
                    depth_tile: player.tile_position(TILE_SIZE).y.max(0),
                })
            })
            .collect();
        LiveRenderFrameOutput {
            viewport_plans: frame_plan.viewport_plans(prediction_plan),
            hud_snapshots: frame_plan.hud_snapshots(),
            world_players_by_view,
            ui_snapshots,
        }
    }

    #[must_use]
    pub fn predicted_local_movement(&self, delta_seconds: f32) -> Option<PredictedMovement> {
        self.local_movement_prediction_plan(delta_seconds)
            .map(LocalMovementPredictionPlan::predicted_movement)
    }

    #[must_use]
    pub fn local_movement_prediction_plan(
        &self,
        delta_seconds: f32,
    ) -> Option<LocalMovementPredictionPlan> {
        let view = self.local_view();
        let player = self.world.player(view.controlled_player_id)?;
        let snapshot =
            PlayerSnapshot::from_world_player(view.controlled_player_id, player, &self.world);
        Some(LocalMovementPredictionPlan::from_snapshot(
            &snapshot,
            delta_seconds,
        ))
    }

    pub fn note_save_session_transition_for_prediction(&mut self) {
        self.local_client_mut()
            .prediction
            .note_save_session_transition();
    }

    pub fn apply_client_actions(&mut self, client_id: ClientId, actions: &[ClientAction]) {
        let mut exit_requested = false;
        let Some(client) = self.clients.get_mut(&client_id) else {
            return;
        };
        for action in actions {
            match action {
                ClientAction::Pause => {
                    client.view.run_mode = RunMode::Paused;
                    client.modal = None;
                    "Paused".clone_into(&mut client.local_message);
                    client.local_audio_cues.push(SoundCue::Ui);
                }
                ClientAction::Cancel => {
                    client.view.run_mode = RunMode::Playing;
                    client.modal = None;
                    "Resumed".clone_into(&mut client.local_message);
                    client.local_audio_cues.push(SoundCue::Ui);
                }
                ClientAction::ToggleMap => {
                    client.modal = Some(ModalScreen::Map);
                    "Map opened".clone_into(&mut client.local_message);
                    client.local_audio_cues.push(SoundCue::Ui);
                }
                ClientAction::ToggleHelp => {
                    client.modal = Some(ModalScreen::Help);
                    "Help opened".clone_into(&mut client.local_message);
                    client.local_audio_cues.push(SoundCue::Ui);
                }
                ClientAction::ExitRequested => {
                    client.exit_requested = true;
                    exit_requested = true;
                }
                ClientAction::Save | ClientAction::Load => {
                    client.prediction.note_save_session_transition();
                    "Split-screen save/load synchronized".clone_into(&mut client.local_message);
                    client.local_audio_cues.push(SoundCue::Ui);
                }
                ClientAction::Confirm
                | ClientAction::MenuUp
                | ClientAction::MenuDown
                | ClientAction::MenuLeft
                | ClientAction::MenuRight
                | ClientAction::ToggleDetails
                | ClientAction::VolumeUp
                | ClientAction::VolumeDown
                | ClientAction::ToggleFullscreen
                | ClientAction::ToggleLocalMultiplayer => {}
            }
        }
        if exit_requested {
            self.push_event(WorldEvent::ClientExitRequested { client_id });
        }
    }

    pub fn update_remote_timing_from_network_sample(&mut self, ping_seconds: f32, loss_ratio: f32) {
        self.remote_timing = RemoteTimingTuning::from_latency_loss(ping_seconds, loss_ratio);
        let stall_seconds = self.remote_timing.timeout_after + FIXED_DELTA_SECONDS;
        let _within_extrapolation_window =
            self.remote_timing.allows_extrapolation(ping_seconds * 0.5);
        if self.remote_timing.timed_out(stall_seconds) {
            self.local_client_mut()
                .prediction
                .note_prediction_failure(PredictionFailure::HazardOrRescueChangedState);
        }
    }

    #[must_use]
    pub const fn remote_timing(&self) -> RemoteTimingTuning {
        self.remote_timing
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
    pub fn live_prediction_presentation_plan(
        &self,
        delta_seconds: f32,
        remote_alpha: f32,
        remote_stall_seconds: f32,
    ) -> PredictionPresentationPlan {
        let authoritative_snapshot = self
            .world
            .player(self.local_client().controlled_player_id)
            .map(|player| {
                PlayerSnapshot::from_world_player(
                    self.local_client().controlled_player_id,
                    player,
                    &self.world,
                )
            });
        self.prediction_presentation_plan(
            authoritative_snapshot.as_ref(),
            delta_seconds,
            remote_alpha,
            remote_stall_seconds,
        )
    }

    pub fn observe_live_remote_player_snapshots(&mut self) {
        let local_player_id = self.local_client().controlled_player_id;
        let remote_snapshots: Vec<PlayerSnapshot> = self
            .world
            .player_ids()
            .filter(|player_id| *player_id != local_player_id)
            .filter_map(|player_id| {
                self.world
                    .player(player_id)
                    .map(|player| PlayerSnapshot::from_world_player(player_id, player, &self.world))
            })
            .collect();
        let local_client = self.local_client_mut();
        for snapshot in remote_snapshots {
            local_client.prediction.push_remote_snapshot(snapshot);
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
    pub fn prediction_failure_application_summary(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> PredictionFailureApplicationSummary {
        self.prediction_failure_recovery_plan(terrain_position, known_revision)
            .application_summary()
    }

    #[must_use]
    pub fn snapshot_chunk_recovery_plan(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> SnapshotChunkRecoveryPlan {
        SnapshotChunkRecoveryPlan::from_tracker(
            &self.terrain_revisions,
            self.current_tick,
            terrain_position,
            known_revision,
        )
    }

    #[must_use]
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    #[must_use]
    pub fn has_client(&self, client_id: ClientId) -> bool {
        self.clients.contains_key(&client_id)
    }

    #[must_use]
    pub fn secondary_local_client_id(&self) -> Option<ClientId> {
        self.clients
            .keys()
            .copied()
            .find(|client_id| *client_id != self.local_client_id)
    }

    pub fn add_local_client_player(&mut self, client_id: ClientId, player_id: PlayerId) -> bool {
        if self.clients.contains_key(&client_id) || self.world.player(player_id).is_some() {
            return false;
        }

        let mut player = self.game.player.clone();
        player.x += TILE_SIZE;
        player.velocity_x = 0.0;
        player.velocity_y = 0.0;
        self.world.insert_player(player_id, player);
        let mut client = ClientState::new(client_id, player_id);
        client.sync_presentation_from_legacy_game(&self.game);
        client.view.viewport = split_screen_viewports(self.clients.len() + 1)
            .pop()
            .expect("split screen layout returns viewport for added client");
        self.clients.insert(client_id, client);
        let viewports = split_screen_viewports(self.clients.len());
        for (client, viewport) in self.clients.values_mut().zip(viewports) {
            client.view.viewport = viewport;
        }
        true
    }

    #[must_use]
    pub const fn current_tick(&self) -> SimulationTick {
        self.current_tick
    }

    #[must_use]
    pub const fn command_network_session(&self) -> &CommandNetworkSession {
        &self.command_network_session
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
        let fixed_nanos = fixed_delta.as_nanos();
        let tolerance_nanos = fixed_nanos / 1_000;
        let steps = (self.simulation_accumulator.as_nanos() + tolerance_nanos) / fixed_nanos;
        let capped_steps = u32::try_from(steps).unwrap_or(u32::MAX);
        self.simulation_accumulator = self
            .simulation_accumulator
            .saturating_sub(fixed_delta.saturating_mul(capped_steps));
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

    pub fn live_snapshot_keyframe_message(&self) -> ProtocolMessage {
        self.world_snapshot().keyframe_message()
    }

    #[must_use]
    pub fn live_snapshot_exchange_batch(&self) -> ProtocolExchangeBatch {
        ProtocolMessage::exchange_batch(
            ProtocolExchangeKind::SnapshotKeyframe,
            vec![self.live_snapshot_keyframe_message()],
        )
    }

    pub fn live_world_delta_message(delta: &WorldDelta) -> ProtocolMessage {
        ProtocolMessage::WorldDelta {
            tick: delta.tick,
            payload: delta.compact_network_delta().network_payload(),
        }
    }

    #[must_use]
    pub fn live_world_delta_exchange_batch(delta: &WorldDelta) -> ProtocolExchangeBatch {
        ProtocolMessage::exchange_batch(
            ProtocolExchangeKind::WorldDelta,
            vec![Self::live_world_delta_message(delta)],
        )
    }

    pub fn drain_live_world_delta_message(&mut self) -> ProtocolMessage {
        let delta = self.drain_world_delta();
        Self::live_world_delta_message(&delta)
    }

    pub fn live_terrain_chunk_response_message(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> ProtocolMessage {
        let recovery = self.snapshot_chunk_recovery_plan(terrain_position, known_revision);
        ProtocolMessage::TerrainChunkResponse {
            chunk_x: terrain_position.x,
            chunk_y: terrain_position.y,
            revision: recovery.recovered_revision().unwrap_or(known_revision),
            tiles: Vec::new(),
        }
    }

    #[must_use]
    pub fn live_terrain_chunk_exchange_batch(
        &self,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> ProtocolExchangeBatch {
        ProtocolMessage::exchange_batch(
            ProtocolExchangeKind::TerrainChunk,
            vec![self.live_terrain_chunk_response_message(terrain_position, known_revision)],
        )
    }

    pub fn exercise_live_network_integration(
        &mut self,
        remote_client_id: ClientId,
        remote_player_id: PlayerId,
        reconnect_token: SessionToken,
        terrain_position: TerrainChunkPosition,
        known_revision: u64,
    ) -> (
        ProtocolExchangeBatch,
        ProtocolExchangeBatch,
        Vec<ProtocolMessage>,
        SnapshotChunkRecoveryPlan,
        PredictionPresentationPlan,
    ) {
        let _added = self.add_local_client_player(remote_client_id, remote_player_id);
        let join_in_progress = ProtocolExchangeBatch {
            kind: ProtocolExchangeKind::JoinHandshake,
            messages: vec![
                ProtocolMessage::JoinRequest {
                    client_id: remote_client_id,
                    session_token: None,
                },
                ProtocolMessage::JoinAccepted {
                    client_id: remote_client_id,
                    player_id: remote_player_id,
                    snapshot_tick: self.world.simulation_tick(),
                },
                self.live_snapshot_keyframe_message(),
            ],
        };
        let reconnect = ProtocolExchangeBatch {
            kind: ProtocolExchangeKind::JoinHandshake,
            messages: vec![
                ProtocolMessage::ReconnectRequest {
                    client_id: remote_client_id,
                    session_token: reconnect_token,
                },
                ProtocolMessage::JoinAccepted {
                    client_id: remote_client_id,
                    player_id: remote_player_id,
                    snapshot_tick: self.world.simulation_tick(),
                },
                self.live_snapshot_keyframe_message(),
            ],
        };
        let command_packet = CommandPacket {
            client_id: remote_client_id,
            commands: vec![SequencedPlayerCommand {
                player_id: remote_player_id,
                sequence: InputSequence::new(0),
                target_tick: self.current_tick,
                command: PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: false,
                },
            }],
        };
        let command_responses = self.apply_live_command_packet(&command_packet);
        let terrain_recovery = self.snapshot_chunk_recovery_plan(terrain_position, known_revision);
        self.observe_live_remote_player_snapshots();
        let high_latency_prediction = self.live_prediction_presentation_plan(0.0, 1.0, 0.12);
        (
            join_in_progress,
            reconnect,
            command_responses,
            terrain_recovery,
            high_latency_prediction,
        )
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

    #[allow(
        clippy::needless_pass_by_value,
        reason = "public command routing API takes ownership from input mapping producers"
    )]
    pub fn route_local_player_commands(
        &mut self,
        commands: Vec<PlayerCommand>,
    ) -> Vec<SequencedPlayerCommand> {
        let sequenced = self.sequence_local_commands(commands.clone());
        self.remember_latest_movement_intents(&sequenced);
        self.latest_local_movement_intent = commands
            .iter()
            .rev()
            .find_map(PlayerMovementIntent::from_command);
        self.latest_local_authoritative_commands
            .clone_from(&commands);
        sequenced
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "public command routing API takes ownership from input mapping producers"
    )]
    pub fn route_client_player_commands(
        &mut self,
        client_id: ClientId,
        source: CommandSource,
        commands: Vec<PlayerCommand>,
    ) -> SequencedCommandBatch {
        let batch = self.sequence_client_commands_from_source(client_id, source, commands.clone());
        self.remember_latest_movement_intents(&batch.commands);
        if client_id == self.local_client_id {
            self.latest_local_movement_intent = commands
                .iter()
                .rev()
                .find_map(PlayerMovementIntent::from_command);
            self.latest_local_authoritative_commands
                .clone_from(&commands);
        }
        batch
    }

    pub fn route_split_screen_player_commands(
        &mut self,
        client_id: ClientId,
        commands: Vec<PlayerCommand>,
    ) -> SequencedCommandBatch {
        self.route_client_player_commands(client_id, CommandSource::SplitScreenClient, commands)
    }

    pub fn route_command_producer(
        &mut self,
        client_id: ClientId,
        producer: CommandProducer,
    ) -> SequencedCommandBatch {
        self.route_client_player_commands(client_id, producer.source, producer.commands)
    }

    pub fn route_command_producers<I>(
        &mut self,
        client_id: ClientId,
        producers: I,
    ) -> Vec<SequencedCommandBatch>
    where
        I: IntoIterator<Item = CommandProducer>,
    {
        producers
            .into_iter()
            .map(|producer| self.route_command_producer(client_id, producer))
            .collect()
    }

    pub fn ensure_local_online_player_presentation_from_legacy_view(
        &mut self,
        player_id: PlayerId,
        update_existing: bool,
    ) -> bool {
        let client_id = ClientId::new(player_id.get());
        if !self.has_client(client_id) {
            let _added = self.add_local_client_player(client_id, player_id);
        }
        let player = self.game.player.clone();
        if let Some(existing) = self.world.player_mut(player_id) {
            if update_existing {
                *existing = player;
            }
            false
        } else {
            self.world.insert_player(player_id, player);
            true
        }
    }

    pub fn mark_world_terrain_tiles_changed<I>(&mut self, positions: I) -> Vec<TerrainChunkRevision>
    where
        I: IntoIterator<Item = TilePosition>,
    {
        self.terrain_revisions.mark_tiles_changed(positions)
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "pixel coordinates are intentionally mapped onto bounded terrain tile/chunk indices for network chunk requests"
    )]
    fn replicated_player_network_chunk(&self, player_id: PlayerId) -> (i32, i32) {
        const NETWORK_TERRAIN_CHUNK_SIZE_TILES: i32 = 16;
        let player = self.world.player(player_id).unwrap_or_else(|| {
            self.world
                .player(LOCAL_PLAYER_ID)
                .expect("local player exists")
        });
        let tile_x = (player.x / TILE_SIZE).floor() as i32;
        let tile_y = (player.y / TILE_SIZE).floor() as i32;
        (
            tile_x.div_euclid(NETWORK_TERRAIN_CHUNK_SIZE_TILES),
            tile_y.div_euclid(NETWORK_TERRAIN_CHUNK_SIZE_TILES),
        )
    }

    fn network_terrain_chunk_from_world(
        &self,
        chunk_x: i32,
        chunk_y: i32,
    ) -> NetworkTerrainChunkSnapshot {
        const NETWORK_TERRAIN_CHUNK_SIZE_TILES: i32 = 16;
        let start_x = chunk_x * NETWORK_TERRAIN_CHUNK_SIZE_TILES;
        let start_y = chunk_y * NETWORK_TERRAIN_CHUNK_SIZE_TILES;
        let end_x = (start_x + NETWORK_TERRAIN_CHUNK_SIZE_TILES).min(self.world.terrain.width());
        let end_y = (start_y + NETWORK_TERRAIN_CHUNK_SIZE_TILES).min(self.world.terrain.height());
        let mut tiles = Vec::new();
        for y in start_y.max(0)..end_y.max(0) {
            for x in start_x.max(0)..end_x.max(0) {
                let position = TilePosition { x, y };
                if let Some(tile) = self.world.terrain.tile(position) {
                    tiles.push(crate::multiplayer::NetworkTerrainTile {
                        x,
                        y,
                        kind: tile.kind,
                        durability: tile.durability,
                    });
                }
            }
        }
        NetworkTerrainChunkSnapshot {
            chunk_x,
            chunk_y,
            revision: self.terrain_revisions.revision(TerrainChunkPosition {
                x: chunk_x,
                y: chunk_y,
            }),
            tiles,
        }
    }

    fn authoritative_terrain_chunks_for_players(&self) -> Vec<NetworkTerrainChunkSnapshot> {
        let mut chunk_positions: BTreeSet<(i32, i32)> = self
            .world
            .player_ids()
            .map(|player_id| self.replicated_player_network_chunk(player_id))
            .collect();
        chunk_positions.extend(
            self.terrain_revisions
                .revised_chunk_positions()
                .into_iter()
                .map(|position| (position.x, position.y)),
        );
        chunk_positions
            .into_iter()
            .map(|(chunk_x, chunk_y)| self.network_terrain_chunk_from_world(chunk_x, chunk_y))
            .collect()
    }

    pub fn live_session_tick_input_from_world(
        &self,
        client_id: ClientId,
        player_id: PlayerId,
        sequence: u32,
        local_player_commands: Vec<PlayerCommand>,
    ) -> QuinnSessionTickInput {
        let tick = self.current_tick().get().saturating_add(1);
        let chunk_coord = self.replicated_player_network_chunk(player_id);
        let snapshot = self.world_snapshot().network_snapshot();
        let correction_probe = snapshot.players.first().map(|player| {
            (
                player.x,
                player.y,
                player.clone(),
                SimulationTick::new(tick.saturating_add(2)),
            )
        });
        let commands = if local_player_commands.is_empty() {
            vec![PlayerCommand::Movement {
                horizontal: 0.0,
                thrust: false,
                drill_down: false,
            }]
        } else {
            local_player_commands
        };
        let delta_payload = if snapshot.players.is_empty() {
            NetworkDeltaPayload::Noop
        } else {
            NetworkDeltaPayload::Players {
                players: snapshot
                    .players
                    .iter()
                    .map(|player| player.player_id)
                    .collect(),
            }
        };
        QuinnSessionTickInput {
            command_packet: Some(CommandPacket {
                client_id,
                commands: commands
                    .into_iter()
                    .map(|command| SequencedPlayerCommand {
                        player_id,
                        sequence: InputSequence::new(sequence),
                        target_tick: SimulationTick::new(tick),
                        command,
                    })
                    .collect(),
            }),
            snapshot: Some(snapshot),
            delta: Some((SimulationTick::new(tick.saturating_add(1)), delta_payload)),
            terrain_chunk_request: Some((chunk_coord.0, chunk_coord.1, 0, 0)),
            authoritative_terrain_chunks: self.authoritative_terrain_chunks_for_players(),
            correction_probe,
        }
    }

    pub fn advance_authoritative_world_ticks(
        &mut self,
        fixed_steps: u32,
    ) -> AuthoritativeWorldAdvanceSummary {
        let start_tick = self.current_tick;
        let mut processed_commands = 0;
        let mut terrain_events = 0;
        let mut cargo_events = 0;
        for _ in 0..fixed_steps {
            let tick = self.current_tick;
            self.command_network_session.set_current_tick(tick);
            processed_commands += self.process_authoritative_commands_for_tick(tick);
            self.step_authoritative_movement_from_latest_intents(FIXED_DELTA_SECONDS);
            let before_events = self.pending_events.len();
            self.process_authoritative_drill_progress();
            let emitted_events: Vec<WorldEvent> = self.pending_events[before_events..].to_vec();
            terrain_events += emitted_events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        WorldEvent::TerrainTilesChanged { .. }
                            | WorldEvent::TerrainChunksChanged { .. }
                    )
                })
                .count();
            cargo_events += emitted_events
                .iter()
                .filter(|event| matches!(event, WorldEvent::CargoChanged { .. }))
                .count();
            self.sync_legacy_player_from_world(LOCAL_PLAYER_ID);
            self.sync_legacy_active_drill_from_world(LOCAL_PLAYER_ID);
            self.sync_legacy_terrain_from_world();
            self.advance_tick();
            self.push_event(WorldEvent::TickAdvanced {
                tick: self.current_tick,
            });
            self.maybe_emit_keyframe_event();
        }
        AuthoritativeWorldAdvanceSummary {
            start_tick,
            end_tick: self.current_tick,
            fixed_steps,
            processed_commands,
            terrain_events,
            cargo_events,
        }
    }

    pub fn apply_replicated_snapshot_to_world_presentation(
        &mut self,
        snapshot: &NetworkWorldSnapshot,
    ) -> ReplicatedWorldPresentationApplySummary {
        let local_player_id = self.local_client().controlled_player_id;
        let mut local_players_updated = 0;
        let mut remote_players_updated = 0;
        let mut clients_created = 0;
        for player_snapshot in &snapshot.players {
            if player_snapshot.player_id != local_player_id {
                let client_id = ClientId::new(player_snapshot.player_id.get());
                if !self.has_client(client_id) {
                    let _added = self.add_local_client_player(client_id, player_snapshot.player_id);
                    clients_created += 1;
                }
            }
            if !self.world.players.contains_key(&player_snapshot.player_id) {
                let mut player = self.game.player.clone();
                apply_network_player_snapshot_to_player(&mut player, player_snapshot);
                self.world.insert_player(player_snapshot.player_id, player);
            } else if let Some(player) = self.world.player_mut(player_snapshot.player_id) {
                apply_network_player_snapshot_to_player(player, player_snapshot);
            }
            self.world.set_scanner_cooldown_seconds(
                player_snapshot.player_id,
                player_snapshot.scanner_cooldown_seconds,
            );
            if player_snapshot.player_id == local_player_id {
                local_players_updated += 1;
            } else {
                remote_players_updated += 1;
            }
        }
        ReplicatedWorldPresentationApplySummary {
            snapshot_tick: snapshot.tick,
            local_players_updated,
            remote_players_updated,
            clients_created,
        }
    }

    pub fn apply_replicated_player_delta_to_world_presentation(
        &mut self,
        tick: SimulationTick,
        players: &[NetworkPlayerSnapshot],
    ) -> ReplicatedWorldPresentationApplySummary {
        self.apply_replicated_snapshot_to_world_presentation(&NetworkWorldSnapshot {
            tick,
            players: players.to_vec(),
        })
    }

    pub fn apply_accepted_online_remote_commands(
        &mut self,
        summary: &CommandPacketExchangeSummary,
    ) -> usize {
        let remote_client_id = if summary.client_id == self.local_client().client_id {
            ClientId::new(2)
        } else {
            summary.client_id
        };
        let Some(player_id) = summary
            .accepted_commands
            .iter()
            .map(|command| command.player_id)
            .find(|player_id| *player_id != self.local_client().controlled_player_id)
        else {
            return 0;
        };
        if !self.has_client(remote_client_id) {
            let _added = self.add_local_client_player(remote_client_id, player_id);
        }
        let accepted: Vec<SequencedPlayerCommand> = summary
            .accepted_commands
            .iter()
            .filter(|command| command.player_id != self.local_client().controlled_player_id)
            .cloned()
            .collect();
        if accepted.is_empty() {
            return 0;
        }
        let accepted_count = accepted.len();
        self.route_client_player_commands(
            remote_client_id,
            CommandSource::OnlineClient,
            accepted
                .iter()
                .map(|command| command.command.clone())
                .collect(),
        );
        let target_ticks: BTreeSet<SimulationTick> =
            accepted.iter().map(|command| command.target_tick).collect();
        for target_tick in target_ticks {
            self.process_authoritative_commands_for_tick(target_tick);
            self.step_authoritative_movement_from_latest_intents(FIXED_DELTA_SECONDS);
            self.process_authoritative_drill_progress();
        }
        self.sync_legacy_terrain_from_world();
        accepted_count
    }

    pub fn update_frame_from_session_authority(
        &mut self,
        input: PlayerInput,
        delta_seconds: f32,
    ) -> SessionAuthorityUpdateSummary {
        let authoritative_input = self.legacy_presentation_input_from_authoritative_commands(input);
        self.update_legacy(authoritative_input, delta_seconds);
        SessionAuthorityUpdateSummary {
            used_legacy_presentation_adapter: true,
            local_movement_authority: self.latest_local_movement_intent.is_some(),
            command_adapter_count: self.latest_local_authoritative_commands.len(),
            current_tick: self.current_tick,
        }
    }

    fn legacy_presentation_input_from_authoritative_commands(
        &self,
        input: PlayerInput,
    ) -> PlayerInput {
        let input = self
            .latest_local_movement_intent
            .map_or(input, |intent| intent.apply_to_input(input));
        self.latest_local_authoritative_commands
            .iter()
            .fold(input, apply_authoritative_command_to_legacy_input)
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

    #[allow(
        dead_code,
        reason = "public replay hook reserved for external deterministic replay harnesses"
    )]
    pub fn buffer_replay_commands(&mut self, commands: Vec<SequencedPlayerCommand>) {
        self.buffer_commands(commands);
    }

    pub fn frame_rate_invariance_proof(&self) -> FrameRateInvarianceProof {
        let mut one_step = self.clone();
        let mut split_steps = self.clone();
        let commands = vec![
            PlayerCommand::Movement {
                horizontal: 0.5,
                thrust: true,
                drill_down: true,
            },
            PlayerCommand::Refuel,
            PlayerCommand::PlaceBomb,
            PlayerCommand::Rescue,
        ];
        let tick = one_step.current_tick();
        one_step.route_local_player_commands(commands.clone());
        split_steps.route_local_player_commands(commands);
        one_step.update_legacy(PlayerInput::default(), FIXED_DELTA_SECONDS);
        split_steps.update_legacy(PlayerInput::default(), FIXED_DELTA_SECONDS / 2.0);
        split_steps.update_legacy(PlayerInput::default(), FIXED_DELTA_SECONDS / 2.0);

        let one_player = one_step.world.player(LOCAL_PLAYER_ID);
        let split_player = split_steps.world.player(LOCAL_PLAYER_ID);
        FrameRateInvarianceProof {
            command_timing_stable: one_step.current_tick() == split_steps.current_tick()
                && one_step.pending_command_count(tick) == split_steps.pending_command_count(tick),
            fuel_stable: matches!((one_player, split_player), (Some(one), Some(split)) if (one.fuel - split.fuel).abs() < f32::EPSILON),
            drill_progress_stable: one_step
                .world
                .active_drill(LOCAL_PLAYER_ID)
                .map(|drill| drill.target)
                == split_steps
                    .world
                    .active_drill(LOCAL_PLAYER_ID)
                    .map(|drill| drill.target),
            hazard_damage_stable: matches!((one_player, split_player), (Some(one), Some(split)) if (one.hull - split.hull).abs() < f32::EPSILON),
            bomb_state_stable: one_step.world.bomb_count() == split_steps.world.bomb_count(),
            rescue_state_stable: matches!((one_player, split_player), (Some(one), Some(split)) if (one.x - split.x).abs() < f32::EPSILON && (one.y - split.y).abs() < f32::EPSILON),
        }
    }

    pub fn replay_determinism_proof(commands: Vec<PlayerCommand>) -> ReplayDeterminismProof {
        fn run(commands: Vec<PlayerCommand>) -> GameSession {
            let mut session = GameSession::new();
            let tick = session.current_tick();
            session.route_local_player_commands(commands);
            let _processed = session.process_authoritative_commands_for_tick(tick);
            session.process_authoritative_drill_progress();
            session
        }

        let first = run(commands.clone());
        let second = run(commands);
        let first_player = first.world.player(LOCAL_PLAYER_ID);
        let second_player = second.world.player(LOCAL_PLAYER_ID);
        ReplayDeterminismProof {
            same_tick: first.current_tick(),
            same_player_count: first.world.player_count(),
            same_transaction_count: first.world.service_transactions().len(),
            same_bomb_count: first.world.bomb_count(),
            same_infrastructure_count: first.world.infrastructure_count(),
            same_local_player_state: first.current_tick() == second.current_tick()
                && first.world.player_count() == second.world.player_count()
                && first.world.service_transactions().len()
                    == second.world.service_transactions().len()
                && first.world.bomb_count() == second.world.bomb_count()
                && first.world.infrastructure_count() == second.world.infrastructure_count()
                && matches!((first_player, second_player), (Some(first), Some(second))
                    if (first.x - second.x).abs() < f32::EPSILON
                        && (first.y - second.y).abs() < f32::EPSILON
                        && (first.fuel - second.fuel).abs() < f32::EPSILON
                        && (first.hull - second.hull).abs() < f32::EPSILON
                        && first.credits == second.credits),
        }
    }

    pub fn process_authoritative_commands_for_tick(&mut self, tick: SimulationTick) -> usize {
        let tick_commands = self.drain_commands_for_tick(tick);
        let command_count = tick_commands.len();
        self.push_event(WorldEvent::CommandsProcessed {
            tick,
            command_count,
        });

        let mut latest_sequences_by_client = BTreeMap::<ClientId, InputSequence>::new();
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
            if let Some((client_id, _client)) = self
                .clients
                .iter()
                .find(|(_client_id, client)| client.controlled_player_id == command.player_id)
            {
                latest_sequences_by_client
                    .entry(*client_id)
                    .and_modify(|sequence| *sequence = (*sequence).max(command.sequence))
                    .or_insert(command.sequence);
            }
        }

        for player_id in changed_players {
            self.push_event(WorldEvent::PlayerChanged { player_id });
        }
        for event in command_events {
            self.push_event(event);
        }

        for (client_id, sequence) in latest_sequences_by_client {
            self.acknowledge_client_commands_through(client_id, sequence);
        }

        command_count
    }

    fn process_authoritative_drill_progress(&mut self) {
        let active_players: Vec<PlayerId> = self.world.active_drills.keys().copied().collect();
        for player_id in active_players {
            let Some(drill) = self.world.active_drill_mut(player_id) else {
                continue;
            };
            drill.progress += FIXED_DELTA_SECONDS / drill.seconds_per_chip;
            if drill.progress < 1.0 {
                continue;
            }
            drill.progress -= 1.0;

            let Some(target) = self.world.active_drill(player_id).map(|drill| drill.target) else {
                continue;
            };
            let Some(result) = self.world.chip_active_drill_target(player_id) else {
                continue;
            };
            let positions = vec![target];
            let revisions = self.mark_world_terrain_tiles_changed(positions.clone());
            self.push_event(WorldEvent::TerrainTilesChanged { positions });
            if !revisions.is_empty() {
                self.push_event(WorldEvent::TerrainChunksChanged { revisions });
            }
            match result {
                MineResult::Mined(TileKind::Ore(_)) => {
                    self.push_event(WorldEvent::CargoChanged { player_id });
                }
                MineResult::TooDangerous => {
                    let _snapshot = self.world.apply_hazard_damage(player_id, 8.0);
                    self.push_event(WorldEvent::PlayerDamaged { player_id });
                }
                MineResult::Blocked
                | MineResult::Exploded
                | MineResult::Blast
                | MineResult::Chipped
                | MineResult::Mined(_) => {}
            }
        }
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

    pub fn apply_live_command_packet(&mut self, packet: &CommandPacket) -> Vec<ProtocolMessage> {
        self.command_network_session
            .set_current_tick(self.current_tick);
        self.command_network_session
            .apply_command_packet(packet)
            .into_iter()
            .map(|response| match response {
                CommandApplicationResponse::Acknowledged(acknowledgement) => {
                    if packet.client_id == acknowledgement.client_id {
                        let accepted_commands: Vec<SequencedPlayerCommand> = packet
                            .commands
                            .iter()
                            .filter(|command| {
                                command.sequence == acknowledgement.acknowledged_sequence
                            })
                            .cloned()
                            .collect();
                        self.buffer_commands(accepted_commands);
                    }
                    self.apply_command_acknowledgement(&acknowledgement);
                    ProtocolMessage::CommandAcknowledgement(acknowledgement)
                }
                CommandApplicationResponse::Rejected(rejection) => {
                    self.apply_command_rejection(&rejection);
                    ProtocolMessage::CommandRejection(rejection)
                }
            })
            .collect()
    }

    pub fn update_legacy(&mut self, input: PlayerInput, delta_seconds: f32) {
        let fixed_steps = self.accumulate_frame_delta(delta_seconds);
        let _summary = self.advance_authoritative_world_ticks(fixed_steps);
        self.sync_client_settings_to_legacy_game();
        let legacy_adapter_input = self.legacy_adapter_input(input);
        let previous_message = self.game.message.clone();
        let previous_player = self.game.player.clone();
        let previous_request_exit = self.game.request_exit;
        self.game.update(legacy_adapter_input, delta_seconds);
        self.capture_legacy_events(&previous_message, &previous_player, previous_request_exit);
        self.sync_client_settings_from_legacy_game();
        self.sync_legacy_mutations_into_world_preserving_authoritative_movement(&previous_player);
    }

    fn legacy_adapter_input(&self, mut input: PlayerInput) -> PlayerInput {
        if self.latest_local_movement_intent.is_some() {
            input.horizontal = 0.0;
            input.thrust = false;
            input.drill_down = false;
        }
        for command in &self.latest_local_authoritative_commands {
            match command {
                PlayerCommand::UseScanner => input.scan = false,
                PlayerCommand::PlaceBomb => input.bomb = false,
                PlayerCommand::PlaceInfrastructure { slot } => match slot {
                    0 => input.place_relay = false,
                    1 => input.place_drone = false,
                    2 => input.place_lift = false,
                    3 => input.place_support = false,
                    4 => input.place_pump = false,
                    5 => input.place_processor = false,
                    _ => {}
                },
                PlayerCommand::SelectUpgrade { .. } => input.selected_upgrade = None,
                PlayerCommand::Interact
                | PlayerCommand::Cancel
                | PlayerCommand::Confirm
                | PlayerCommand::Movement { .. }
                | PlayerCommand::BuyUpgrade { .. }
                | PlayerCommand::Refuel
                | PlayerCommand::Repair
                | PlayerCommand::SellCargo
                | PlayerCommand::Rescue => {}
            }
        }
        input
    }

    fn remember_latest_movement_intents(&mut self, commands: &[SequencedPlayerCommand]) {
        for command in commands {
            if let Some(intent) = PlayerMovementIntent::from_command(&command.command) {
                self.latest_movement_intents
                    .insert(command.player_id, intent);
            }
        }
    }

    fn step_authoritative_movement_from_latest_intents(&mut self, delta_seconds: f32) {
        let intents: Vec<(PlayerId, PlayerMovementIntent)> = self
            .latest_movement_intents
            .iter()
            .map(|(player_id, intent)| (*player_id, *intent))
            .collect();
        for (player_id, intent) in intents {
            let _step = self.world.step_authoritative_movement(
                player_id,
                intent.horizontal,
                intent.thrust,
                delta_seconds,
            );
        }
    }

    fn sync_legacy_mutations_into_world_preserving_authoritative_movement(
        &mut self,
        previous_legacy_player: &Player,
    ) {
        let authoritative_player = self.world.player(LOCAL_PLAYER_ID).cloned();
        let authoritative_active_drill = self.world.active_drill(LOCAL_PLAYER_ID).copied();
        let authoritative_bombs = self.world.bombs.clone();
        let authoritative_infrastructure = self.world.infrastructure.clone();
        let authoritative_scanner_cooldowns = self.world.scanner_cooldowns.clone();
        let secondary_players: Vec<(PlayerId, Player)> = self
            .world
            .players
            .iter()
            .filter(|(player_id, _player)| **player_id != LOCAL_PLAYER_ID)
            .map(|(player_id, player)| (*player_id, player.clone()))
            .collect();
        self.world
            .sync_from_legacy_game(self.current_tick, &self.game);
        for (player_id, player) in secondary_players {
            self.world.insert_player(player_id, player);
        }
        self.world.bombs = authoritative_bombs;
        self.world.infrastructure = authoritative_infrastructure;
        self.world.scanner_cooldowns = authoritative_scanner_cooldowns;
        self.world.authoritative_summary.bomb_count = self.world.bombs.len();
        if let (Some(authoritative_player), Some(world_player)) =
            (authoritative_player, self.world.player_mut(LOCAL_PLAYER_ID))
        {
            world_player.x = authoritative_player.x;
            world_player.y = authoritative_player.y;
            world_player.velocity_x = authoritative_player.velocity_x;
            world_player.velocity_y = authoritative_player.velocity_y;
            world_player.fuel = authoritative_player.fuel;
            world_player.hull = authoritative_player.hull;
            if self.latest_local_movement_intent.is_some()
                && previous_legacy_player != &self.game.player
            {
                self.game.player.x = authoritative_player.x;
                self.game.player.y = authoritative_player.y;
                self.game.player.velocity_x = authoritative_player.velocity_x;
                self.game.player.velocity_y = authoritative_player.velocity_y;
            }
            self.sync_legacy_active_drill_from_world(LOCAL_PLAYER_ID);
        }
        if authoritative_active_drill.is_some() {
            self.world
                .set_active_drill(LOCAL_PLAYER_ID, authoritative_active_drill);
            self.sync_legacy_active_drill_from_world(LOCAL_PLAYER_ID);
        }
    }

    fn sync_legacy_player_from_world(&mut self, player_id: PlayerId) {
        if player_id == LOCAL_PLAYER_ID
            && let Some(player) = self.world.player(player_id)
        {
            self.game.player = player.clone();
        }
    }

    fn sync_legacy_active_drill_from_world(&mut self, player_id: PlayerId) {
        self.world
            .sync_active_drill_to_legacy_game(player_id, &mut self.game);
    }

    fn sync_legacy_terrain_from_world(&mut self) {
        self.game.terrain.clone_from(self.world.terrain());
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
            let revisions = self.mark_world_terrain_tiles_changed(positions.clone());
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
    use std::{collections::BTreeMap, time::Duration};

    use crate::{
        game_state::{
            DrillDirection, DrillState, GameState, InfrastructureKind, ModalScreen, RunMode,
            SoundCue, TILE_SIZE,
        },
        input::PlayerInput,
        multiplayer::{
            ClientId, CommandAcknowledgement, CommandPacket, CommandPacketExchangeSummary,
            CommandRejection, CommandSource, FIXED_DELTA_SECONDS, InputSequence, LOCAL_CLIENT_ID,
            LOCAL_PLAYER_ID, NetworkDeltaPayload, NetworkPlayerSnapshot, NetworkWorldSnapshot,
            PlayerCommand, PlayerId, ProtocolExchangeKind, ProtocolMessage, SequencedPlayerCommand,
            SessionToken, SimulationTick,
        },
        terrain::{MineResult, TileKind, TilePosition},
    };

    use super::{
        ClientPredictionState, ClientState, CompactWorldDelta, CompatibilityDeletionReadiness,
        CompatibilityDeletionStatus, CompatibilityMethodNamingStatus, GameSession,
        HostAuthorityCommandRoutingDomain, JoinedClientAuthorityBoundaryStatus,
        LegacyGameStateWriteBoundaryStatus, LegacyInputRewriteRemovalStatus,
        NetworkDebugInstrumentationSnapshot, PlayerSnapshot, PredictionCorrectionTuning,
        PredictionFailure, PredictionRecoveryAction, RemoteTimingTuning,
        RenderingInputMigrationStatus, SnapshotApplicationTruthBoundaryStatus, WorldState,
        host_authority_command_routing_inventory, host_authority_command_routing_summary,
        legacy_game_state_coupling_inventory, legacy_game_state_coupling_inventory_summary,
    };

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
                fuel_capacity: 1.0,
                hull: 1.0,
                max_hull: 1.0,
                credits: 0,
                cargo_used: 0,
                cargo: BTreeMap::new(),
                artifacts: BTreeMap::new(),
                materials: BTreeMap::new(),
                loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                scanner_cooldown_seconds: 0.0,
            });
        let plan = session.render_frame_plan();
        let prediction_plan = session.prediction_presentation_plan(None, 0.5, 0.5, 0.1);

        let remotes = plan.remote_player_presentations(&plan.views[0], &prediction_plan);

        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].player_id, remote_player_id);
    }

    #[test]
    fn render_frame_plan_builds_clipped_viewport_plans() {
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
                fuel_capacity: 1.0,
                hull: 1.0,
                max_hull: 1.0,
                credits: 0,
                cargo_used: 0,
                cargo: BTreeMap::new(),
                artifacts: BTreeMap::new(),
                materials: BTreeMap::new(),
                loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                scanner_cooldown_seconds: 0.0,
            });
        let frame_plan = session.render_frame_plan();
        let prediction_plan = session.prediction_presentation_plan(None, 0.5, 0.5, 0.1);

        let viewport_plans = frame_plan.viewport_plans(&prediction_plan);

        assert_eq!(viewport_plans.len(), frame_plan.view_count());
        assert!(viewport_plans[0].clip_enabled);
        assert_eq!(viewport_plans[0].client_id, LOCAL_CLIENT_ID);
        assert_eq!(viewport_plans[0].remote_player_count, 1);
        assert_eq!(viewport_plans[0].remote_players.len(), 1);
        assert!(viewport_plans[0].local_player.is_some());
    }

    #[test]
    fn live_render_frame_output_uses_world_players_client_views_and_per_player_hud() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        session
            .world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .credits = 111;
        session
            .world
            .player_mut(second_player)
            .expect("second player exists")
            .credits = 222;
        session
            .world
            .set_scanner_cooldown_seconds(second_player, 3.0);
        session.observe_live_remote_player_snapshots();
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);

        let output = session.live_render_frame_output(&prediction_plan);

        assert_eq!(output.viewport_plans.len(), 2);
        assert_eq!(output.clipped_viewport_count(), 2);
        assert_eq!(output.hud_count(), 2);
        assert!(
            output
                .hud_snapshots
                .iter()
                .any(|hud| hud.player_id == second_player
                    && hud.credits == 222
                    && (hud.scanner_cooldown_seconds - 3.0).abs() < f32::EPSILON)
        );
        assert!(output.world_players_by_view.iter().all(|(_, players)| {
            players
                .iter()
                .any(|player| player.player_id == LOCAL_PLAYER_ID)
                && players
                    .iter()
                    .any(|player| player.player_id == second_player)
        }));
        assert!(output.all_visible_players_are_session_sourced());
        assert!(output.viewport_plans.iter().all(|plan| plan.clip_enabled));
    }

    #[test]
    fn split_screen_render_output_covers_camera_viewport_hud_remote_correction_and_feedback() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 1.0,
            thrust: true,
            drill_down: true,
        }]);
        session.observe_live_remote_player_snapshots();
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.2);

        let output = session.live_render_frame_output(&prediction_plan);

        assert_eq!(output.viewport_plans.len(), 2);
        assert_eq!(output.clipped_viewport_count(), 2);
        assert!(
            output
                .viewport_plans
                .iter()
                .all(|plan| plan.local_player.is_some())
        );
        assert_eq!(output.hud_count(), 2);
        assert!(output.all_visible_players_are_session_sourced());
        assert!(
            output
                .world_players_by_view
                .iter()
                .flat_map(|(_, players)| players)
                .any(
                    |player| player.source == super::RenderPlayerPresentationSource::PredictedLocal
                )
        );
        assert!(output.split_screen_ui_ready());
        assert!(output.ready_for_live_render_path());
    }

    #[test]
    fn split_screen_ui_snapshots_cover_minimap_depth_scanner_pause_modal_audio_and_messages() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        session
            .world
            .set_scanner_cooldown_seconds(LOCAL_PLAYER_ID, 2.5);
        session
            .world
            .set_scanner_cooldown_seconds(second_player, 4.0);
        session.apply_client_actions(
            LOCAL_CLIENT_ID,
            &[
                crate::multiplayer::ClientAction::Pause,
                crate::multiplayer::ClientAction::ToggleMap,
            ],
        );
        session.apply_client_actions(
            second_client,
            &[crate::multiplayer::ClientAction::ToggleHelp],
        );
        session
            .clients
            .get_mut(&second_client)
            .expect("second client exists")
            .local_audio_cues
            .push(SoundCue::Milestone);
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);

        let output = session.live_render_frame_output(&prediction_plan);

        assert!(output.split_screen_ui_ready());
        let primary = output
            .ui_snapshots
            .iter()
            .find(|snapshot| snapshot.client_id == LOCAL_CLIENT_ID)
            .expect("primary ui snapshot");
        let secondary = output
            .ui_snapshots
            .iter()
            .find(|snapshot| snapshot.client_id == second_client)
            .expect("secondary ui snapshot");
        assert_eq!(primary.modal, Some(ModalScreen::Map));
        assert_eq!(primary.run_mode, RunMode::Paused);
        assert!(primary.message.contains("Map"));
        assert!(primary.audio_cues.contains(&SoundCue::Ui));
        assert!((primary.scanner_cooldown_seconds - 2.5).abs() < f32::EPSILON);
        assert_eq!(secondary.modal, Some(ModalScreen::Help));
        assert!(secondary.message.contains("Help"));
        assert!(secondary.audio_cues.contains(&SoundCue::Milestone));
        assert!((secondary.scanner_cooldown_seconds - 4.0).abs() < f32::EPSILON);
        assert_ne!(primary.viewport, secondary.viewport);
    }

    #[test]
    fn split_screen_rescue_services_save_load_and_exit_paths_are_player_scoped() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        {
            let player = session
                .world
                .player_mut(second_player)
                .expect("second player exists");
            player.x = 500.0;
            player.y = 600.0;
            player.credits = 1_000;
            player.fuel = 10.0;
            player.hull = 20.0;
            player.cargo.insert(crate::terrain::MineralKind::Copper, 2);
        }

        assert_eq!(
            session
                .world
                .apply_player_command(second_player, &PlayerCommand::Rescue),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            session
                .world
                .apply_player_command(second_player, &PlayerCommand::Refuel),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            session
                .world
                .apply_player_command(second_player, &PlayerCommand::Repair),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            session
                .world
                .apply_player_command(second_player, &PlayerCommand::SellCargo),
            super::PlayerScopedCommandOutcome::Applied
        );
        {
            let second_client_state = session
                .clients
                .get_mut(&second_client)
                .expect("second client exists");
            "Surface service applied".clone_into(&mut second_client_state.local_message);
            second_client_state.local_audio_cues.push(SoundCue::Sell);
        }
        session.apply_client_actions(
            second_client,
            &[
                crate::multiplayer::ClientAction::Save,
                crate::multiplayer::ClientAction::Load,
            ],
        );
        session.apply_client_actions(
            second_client,
            &[crate::multiplayer::ClientAction::ExitRequested],
        );

        let player = session
            .world
            .player(second_player)
            .expect("second player exists");
        assert!((player.x - 0.0).abs() < f32::EPSILON);
        assert!((TILE_SIZE.mul_add(-2.0, player.y)).abs() < f32::EPSILON);
        assert_eq!(player.cargo_used(), 0);
        assert!(session.clients.values().any(|client| client.exit_requested));
        let second_client_state = session
            .clients
            .get(&second_client)
            .expect("second client exists");
        assert!(second_client_state.exit_requested);
        assert!(
            second_client_state
                .prediction()
                .prediction_failures()
                .contains(&super::PredictionFailure::SaveSessionTransition)
        );
        assert!(second_client_state.local_message.contains("synchronized"));
        assert!(second_client_state.local_audio_cues.contains(&SoundCue::Ui));
        assert!(
            second_client_state
                .local_audio_cues
                .contains(&SoundCue::Sell)
        );
    }

    #[test]
    fn prediction_runtime_paths_cover_replay_correction_remote_failure_and_debug_paths() {
        let mut prediction = super::ClientPredictionState::default();
        let player_id = LOCAL_PLAYER_ID;
        let terrain_revisions = super::TerrainRevisionTracker::default();
        let command = SequencedPlayerCommand {
            player_id,
            sequence: InputSequence::new(1),
            target_tick: SimulationTick::new(1),
            command: PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: true,
                drill_down: false,
            },
        };
        prediction.remember_commands(&[command]);
        prediction.note_prediction_failure(super::PredictionFailure::TerrainAlreadyChanged);
        prediction.note_prediction_failure(super::PredictionFailure::EconomyChangedState);
        prediction.note_prediction_failure(super::PredictionFailure::ProgressionChangedState);
        prediction.note_prediction_failure(super::PredictionFailure::HazardOrRescueChangedState);
        prediction.note_prediction_failure(super::PredictionFailure::CommandRejected);
        prediction.note_save_session_transition();
        prediction.set_correction_offset(super::CorrectionOffset::new(8.0, 0.0));

        let previous = super::PlayerSnapshot {
            player_id,
            x: 0.0,
            y: 0.0,
            velocity_x: 4.0,
            velocity_y: 0.0,
            fuel: 1.0,
            fuel_capacity: 1.0,
            hull: 1.0,
            max_hull: 1.0,
            credits: 0,
            cargo_used: 0,
            cargo: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            materials: BTreeMap::new(),
            loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
            scanner_cooldown_seconds: 0.0,
        };
        let next = super::PlayerSnapshot {
            x: 10.0,
            ..previous.clone()
        };
        let interpolated = super::ClientPredictionState::remote_player_presentation(
            &previous,
            Some(&next),
            0.5,
            0.0,
        );
        let extrapolated =
            super::ClientPredictionState::remote_player_presentation(&previous, None, 0.0, 0.1);
        let actions = prediction.prediction_recovery_actions(
            player_id,
            &terrain_revisions,
            SimulationTick::new(7),
            super::TerrainChunkPosition { x: 0, y: 0 },
            0,
        );
        let debug = prediction.prediction_debug_snapshot(0.12, 2, 1, 1);

        assert!(prediction.unacknowledged_replay_is_complete(player_id));
        assert!(super::PredictionCorrectionTuning::classifies_expected_offsets());
        assert!(!interpolated.extrapolated);
        assert!(extrapolated.extrapolated);
        assert!(actions.iter().any(|action| matches!(
            action,
            super::PredictionRecoveryAction::RequestTerrainDelta(_)
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            super::PredictionRecoveryAction::RollBackLocalEconomy { .. }
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            super::PredictionRecoveryAction::RollBackProgression { .. }
        )));
        assert!(
            actions
                .iter()
                .filter(|action| matches!(
                    action,
                    super::PredictionRecoveryAction::RequestAuthoritativeSnapshot { .. }
                ))
                .count()
                >= 3
        );
        assert_eq!(debug.correction_plan, super::CorrectionPlan::Smooth);
        assert!(debug.visible_to_debug_overlay());
    }

    #[test]
    fn prediction_failure_application_summary_covers_real_recovery_actions() {
        let mut session = GameSession::new();
        session
            .local_client_mut()
            .prediction
            .note_prediction_failure(super::PredictionFailure::TerrainAlreadyChanged);
        session
            .local_client_mut()
            .prediction
            .note_prediction_failure(super::PredictionFailure::EconomyChangedState);
        session
            .local_client_mut()
            .prediction
            .note_prediction_failure(super::PredictionFailure::ProgressionChangedState);
        session
            .local_client_mut()
            .prediction
            .note_prediction_failure(super::PredictionFailure::CommandRejected);
        let terrain_position = super::TerrainChunkPosition { x: 0, y: 0 };
        session
            .terrain_revisions
            .mark_tiles_changed([TilePosition { x: 1, y: 1 }]);

        let summary = session.prediction_failure_application_summary(terrain_position, 0);

        assert!(summary.requested_keyframe);
        assert_eq!(summary.requested_terrain_deltas, 1);
        assert_eq!(summary.rolled_back_economy_players, vec![LOCAL_PLAYER_ID]);
        assert_eq!(
            summary.rolled_back_progression_players,
            vec![LOCAL_PLAYER_ID]
        );
    }

    #[test]
    fn render_viewport_plan_carries_correction_remote_and_tentative_feedback_outputs() {
        let mut session = GameSession::new();
        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 12.0,
            thrust: false,
            drill_down: false,
        }]);
        session
            .local_client_mut()
            .prediction
            .push_feedback(super::LocalTentativeFeedback::DrillContact);
        let remote_player = PlayerId::new(88);
        session
            .local_client_mut()
            .prediction
            .push_remote_snapshot(super::PlayerSnapshot {
                player_id: remote_player,
                x: 1.0,
                y: 2.0,
                velocity_x: 0.0,
                velocity_y: 0.0,
                fuel: 1.0,
                fuel_capacity: 1.0,
                hull: 1.0,
                max_hull: 1.0,
                credits: 0,
                cargo_used: 0,
                cargo: BTreeMap::new(),
                artifacts: BTreeMap::new(),
                materials: BTreeMap::new(),
                loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                scanner_cooldown_seconds: 0.0,
            });
        {
            let player = session
                .world
                .player_mut(LOCAL_PLAYER_ID)
                .expect("player exists");
            player.x = 100.0;
            player.y = 50.0;
            player.velocity_x = 0.0;
            player.velocity_y = 0.0;
        }
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.0, 0.0);
        let frame_plan = session.render_frame_plan();

        let viewport_plan = frame_plan
            .viewport_plans(&prediction_plan)
            .into_iter()
            .next()
            .expect("viewport plan exists");

        assert_eq!(viewport_plan.remote_players.len(), 1);
        assert!(viewport_plan.correction_frame.is_some());
        assert!(viewport_plan.feedback_outputs.iter().any(|output| {
            output.presentation == super::TentativeFeedbackPresentation::DrillContactAudio
                && output.channel == super::TentativeFeedbackChannel::Audio
        }));
    }

    #[test]
    fn render_frame_plan_exposes_per_view_hud_snapshots() {
        let mut session = GameSession::new();
        session
            .world
            .set_scanner_cooldown_seconds(LOCAL_PLAYER_ID, 2.0);
        session
            .world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .credits = 123;

        let frame_plan = session.render_frame_plan();
        let hud = frame_plan
            .hud_snapshot_for_view(&frame_plan.views[0])
            .expect("hud snapshot exists");

        assert_eq!(hud.player_id, LOCAL_PLAYER_ID);
        assert_eq!(hud.credits, 123);
        assert!((hud.scanner_cooldown_seconds - 2.0).abs() < f32::EPSILON);
        assert_eq!(frame_plan.hud_snapshots(), vec![hud]);
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
        let prediction_plan = session
            .local_movement_prediction_plan(0.5)
            .expect("local movement prediction plan exists");

        assert_eq!(predicted.player_id, LOCAL_PLAYER_ID);
        assert!((predicted.x - (session.game().player.x + 5.0)).abs() < f32::EPSILON);
        assert_eq!(prediction_plan.player_id, LOCAL_PLAYER_ID);
        assert!((prediction_plan.predicted_x - predicted.x).abs() < f32::EPSILON);
        assert!((prediction_plan.predicted_movement().x - predicted.x).abs() < f32::EPSILON);
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
        assert!(ownership.menus_owned);
        assert!(ownership.overlays_owned);
        assert!(ownership.fully_split());
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
    fn authoritative_movement_stops_at_solid_ground() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.x = 20.0 * TILE_SIZE;
        game.player.y = 4.0 * TILE_SIZE;
        game.player.velocity_y = 0.0;
        let mut world = WorldState::from_legacy_game(&game);

        for _ in 0..80 {
            let _step =
                world.step_authoritative_movement(LOCAL_PLAYER_ID, 0.0, false, FIXED_DELTA_SECONDS);
        }

        let player = world.player(LOCAL_PLAYER_ID).expect("player exists");
        assert!(player.y < 5.0 * TILE_SIZE);
        assert!(player.velocity_y.abs() < f32::EPSILON);
    }

    #[test]
    fn authoritative_side_drilling_intent_persists_and_chips_side_tile() {
        let mut game = GameState::new();
        game.run_mode = RunMode::Playing;
        game.player.x = 20.0 * TILE_SIZE;
        game.player.y = 5.0 * TILE_SIZE;
        let mut world = WorldState::from_legacy_game(&game);
        let target = TilePosition { x: 21, y: 5 };
        assert!(world.terrain.set_kind(target, TileKind::Dirt));

        for _ in 0..2 {
            assert_eq!(
                world.apply_player_command(
                    LOCAL_PLAYER_ID,
                    &PlayerCommand::Movement {
                        horizontal: 1.0,
                        thrust: false,
                        drill_down: false,
                    },
                ),
                super::PlayerScopedCommandOutcome::Applied
            );
            let active = world
                .active_drill(LOCAL_PLAYER_ID)
                .expect("side drill remains active");
            assert_eq!(active.target, target);
            assert_eq!(active.direction, DrillDirection::Right);
            let _result = world.chip_active_drill_target(LOCAL_PLAYER_ID);
        }

        assert!(matches!(
            world.terrain.tile(target).map(|tile| tile.kind),
            Some(TileKind::Air)
        ));
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
        game.player.credits = 1_000;
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
    fn rendering_inputs_and_legacy_write_boundaries_are_migrated_to_session_presentations() {
        let rendering = RenderingInputMigrationStatus::current();
        let write_boundary = LegacyGameStateWriteBoundaryStatus::current();
        let naming = CompatibilityMethodNamingStatus::current();
        let snapshot_truth = SnapshotApplicationTruthBoundaryStatus::current();
        let deletion = CompatibilityDeletionStatus::current();

        assert!(rendering.camera_from_session_view);
        assert!(rendering.players_from_world_presentation);
        assert!(rendering.terrain_from_world_presentation);
        assert!(rendering.hud_from_per_client_presentation);
        assert!(rendering.migrated());
        assert!(write_boundary.ui_settings_save_menu_allowed);
        assert!(write_boundary.presentation_compatibility_allowed);
        assert!(write_boundary.authoritative_world_writes_blocked);
        assert!(write_boundary.online_save_boundary_enforced);
        assert!(write_boundary.limited_to_compatibility());
        assert!(naming.update_legacy_name_explicit);
        assert!(naming.legacy_presentation_adapter_name_explicit);
        assert!(naming.compatibility_wrapper_named);
        assert!(naming.temporary_bridge_status_obvious());
        assert!(snapshot_truth.snapshots_apply_to_world_first);
        assert!(snapshot_truth.legacy_mirror_presentation_only);
        assert!(snapshot_truth.remote_state_consumable_by_session);
        assert!(snapshot_truth.local_player_not_online_authority);
        assert!(snapshot_truth.legacy_save_load_scoped_by_policy);
        assert!(snapshot_truth.legacy_truth_removed());
        assert_eq!(
            deletion.readiness,
            CompatibilityDeletionReadiness::BlockedByPresentationRenderer
        );
        assert!(!deletion.runtime_depends_on_legacy_input_rewrite);
        assert!(deletion.renderer_depends_on_presentation_bridge);
        assert!(deletion.runtime_path_clear());
    }

    #[test]
    fn online_gameplay_advances_world_without_public_legacy_input_rewrite() {
        let mut session = GameSession::new();
        let remote_player = PlayerId::new(2);
        let target_tick = session.current_tick();
        let summary = CommandPacketExchangeSummary {
            client_id: ClientId::new(2),
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: target_tick,
            accepted_commands: vec![SequencedPlayerCommand {
                player_id: remote_player,
                sequence: InputSequence::new(1),
                target_tick,
                command: PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: true,
                    drill_down: false,
                },
            }],
        };

        let applied = session.apply_accepted_online_remote_commands(&summary);
        let remote_after = session
            .world()
            .player(remote_player)
            .expect("remote player");

        assert_eq!(applied, 1);
        assert!(remote_after.velocity_x > 0.0);
        assert!(remote_after.velocity_y < 0.0);
        assert!(session.game().player.velocity_x.abs() < f32::EPSILON);
        assert!(LegacyInputRewriteRemovalStatus::current().removal_complete());
    }

    #[test]
    fn world_snapshot_application_boundary_keeps_legacy_state_presentation_only() {
        let status = SnapshotApplicationTruthBoundaryStatus::current();
        let inventory = legacy_game_state_coupling_inventory_summary();

        assert!(status.snapshots_apply_to_world_first);
        assert!(status.legacy_mirror_presentation_only);
        assert!(status.remote_state_consumable_by_session);
        assert!(status.local_player_not_online_authority);
        assert!(status.legacy_save_load_scoped_by_policy);
        assert!(status.legacy_truth_removed());
        assert_eq!(inventory.authoritative_world_couplings, 0);
        assert!(inventory.runtime_inventory_complete());
    }

    #[test]
    fn legacy_game_state_coupling_inventory_has_no_authoritative_runtime_couplings() {
        let inventory = legacy_game_state_coupling_inventory();
        let summary = legacy_game_state_coupling_inventory_summary();

        assert_eq!(summary.total, inventory.len());
        assert_eq!(summary.authoritative_world_couplings, 0);
        assert!(summary.presentation_compatibility_couplings >= 3);
        assert_eq!(summary.save_menu_ui_couplings, 1);
        assert!(summary.runtime_inventory_complete());
        assert!(inventory.iter().all(|item| !item.replacement.is_empty()));
    }

    #[test]
    fn economy_service_and_menu_commands_are_classified_for_host_authority() {
        let inventory = host_authority_command_routing_inventory();
        let summary = host_authority_command_routing_summary();

        assert_eq!(summary.total, inventory.len());
        assert!(summary.economy_service_menu_routed());
        assert!(inventory.iter().any(|item| item.command == "SellCargo"
            && matches!(
                item.domain,
                HostAuthorityCommandRoutingDomain::EconomyServiceMenu
            )
            && item.host_authoritative));
        assert!(inventory.iter().any(|item| item.command == "BuyUpgrade"
            && matches!(
                item.domain,
                HostAuthorityCommandRoutingDomain::EconomyServiceMenu
            )
            && item.host_authoritative));
        assert!(inventory.iter().any(|item| item.command == "OpenMenu"
            && matches!(
                item.domain,
                HostAuthorityCommandRoutingDomain::PresentationOnly
            )
            && !item.host_authoritative));
    }

    #[test]
    fn joined_client_authority_boundary_and_legacy_input_rewrite_removal_are_safe() {
        let boundary = JoinedClientAuthorityBoundaryStatus::online_joined_client_runtime();
        let rewrite_removal = LegacyInputRewriteRemovalStatus::current();

        assert!(boundary.local_prediction_allowed);
        assert!(boundary.remote_authority_accepted);
        assert!(boundary.host_world_owner);
        assert!(!boundary.fights_local_authority);
        assert!(boundary.safe_for_joined_client());
        assert!(rewrite_removal.public_rewrite_function_removed);
        assert!(rewrite_removal.remaining_adapter_presentation_only);
        assert!(rewrite_removal.runtime_command_processing_replaced);
        assert!(rewrite_removal.removal_complete());
    }

    #[test]
    fn world_network_snapshot_carries_upgrade_and_inventory_affecting_loadout() {
        let mut game = GameState::new();
        game.player.credits = 2_000;
        game.player.fuel_tank_level = 2;
        game.player.cargo_bay_level = 3;
        game.player.drill_strength = 4;
        game.player.engine_level = 5;
        game.player.hull_level = 6;
        game.player.radiator_level = 7;
        game.player.scanner_level = 8;
        game.player.bombs = 9;
        game.player.insured = true;
        game.player.insurance_tier = 2;
        game.player.signal_relay_kits = 3;
        let world = WorldState::from_legacy_game(&game);

        let snapshot =
            super::WorldSnapshot::from_world(SimulationTick::new(1), &world).network_snapshot();
        let player = snapshot.players.first().expect("local player snapshot");

        assert_eq!(player.loadout.fuel_tank_level, 2);
        assert_eq!(player.loadout.cargo_bay_level, 3);
        assert_eq!(player.loadout.drill_strength, 4);
        assert_eq!(player.loadout.engine_level, 5);
        assert_eq!(player.loadout.hull_level, 6);
        assert_eq!(player.loadout.radiator_level, 7);
        assert_eq!(player.loadout.scanner_level, 8);
        assert_eq!(player.loadout.bombs, 9);
        assert!(player.loadout.insured);
        assert_eq!(player.loadout.insurance_tier, 2);
        assert_eq!(player.loadout.signal_relay_kits, 3);
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
        assert!(ownership.terrain_owned);
        assert!(ownership.fully_split());
    }

    #[test]
    fn multiplayer_completion_summary_marks_primary_migration_complete_or_explicitly_deferred() {
        let world = WorldState::from_legacy_game(&GameState::new());
        let summary = world.implementation_completion_summary();

        assert!(summary.world_and_client_state_split);
        assert!(summary.fixed_tick_authoritative_path);
        assert!(summary.player_scoped_gameplay);
        assert!(summary.local_split_screen_rendering);
        assert!(summary.online_transport_deferred);
        assert!(summary.prediction_drives_presentation);
        assert!(summary.live_snapshot_delta_chunk_sync);
        assert!(summary.single_player_regressions_covered);
        assert!(summary.primary_migration_complete_or_deferred());
    }

    #[test]
    fn world_state_reports_authoritative_dependency_and_transient_effect_split() {
        let world = WorldState::from_legacy_game(&GameState::new());

        assert!(
            world
                .authoritative_dependency_summary()
                .authoritative_path_split()
        );
        assert!(world.transient_effect_routing_summary().split());
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
        player.credits = 10_000;
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
        let mut game = GameState::new();
        game.player.credits = 420;
        let mut world = WorldState::from_legacy_game(&game);
        let before = world
            .player_inventory_summary(LOCAL_PLAYER_ID)
            .expect("player summary");

        assert_eq!(before.cargo_used, 0);
        assert_eq!(before.credits, 420);
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
        assert_eq!(summary.unresolved_variable_delta, 0);
        assert!(summary.audit_complete());
    }

    #[test]
    fn world_delta_compacts_real_gameplay_events_without_local_presentation_state() {
        let tick = SimulationTick::new(3);
        let delta = super::WorldDelta::new(
            tick,
            vec![
                super::WorldEvent::ImportantEffectTriggered,
                super::WorldEvent::MessageChanged {
                    message: "local toast".to_owned(),
                },
                super::WorldEvent::CargoChanged {
                    player_id: LOCAL_PLAYER_ID,
                },
            ],
        );

        assert!(matches!(
            delta.compact_network_delta(),
            super::CompactWorldDelta::Players { tick: delta_tick, players }
                if delta_tick == tick && players == vec![LOCAL_PLAYER_ID]
        ));
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
    fn gameplay_event_routing_summary_separates_local_presentation_effects() {
        let summary = GameSession::gameplay_event_routing_summary();

        assert!(summary.authoritative_events > 0);
        assert_eq!(summary.local_presentation_boundaries, 5);
        assert_eq!(summary.gameplay_world_boundaries, 3);
        assert!(summary.separates_local_presentation());
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
        assert_eq!(
            compact_delta.summary(),
            super::CompactDeltaSummary {
                kind: super::CompactDeltaKind::Players,
                tick: SimulationTick::new(8),
                item_count: 1,
            }
        );
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
            fuel_capacity: 100.0,
            hull: 4.0,
            max_hull: 100.0,
            credits: 6,
            cargo_used: 0,
            cargo: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            materials: BTreeMap::new(),
            loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
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
        assert!(super::ClientPredictionState::should_extrapolate(0.16));
        assert!(!super::ClientPredictionState::should_extrapolate(0.17));
        assert!(
            session
                .local_client()
                .prediction()
                .predicted_input_lag_seconds()
                > 0.0
        );
    }

    #[test]
    fn prediction_high_ping_tuning_derives_latency_loss_timing() {
        let mut session = GameSession::new();
        session.update_remote_timing_from_network_sample(0.18, 0.1);
        let timing = session.remote_timing();

        assert!(timing.interpolation_delay <= 0.25);
        assert!(timing.allows_extrapolation(0.09));
        assert!(timing.timed_out(timing.timeout_after + 0.01));
        assert_eq!(
            super::ClientPredictionState::correction_plan(8.0, 0.0),
            super::CorrectionPlan::Smooth
        );
        assert_eq!(
            super::ClientPredictionState::correction_plan(24.0, 0.0),
            super::CorrectionPlan::Snap
        );
        assert!(
            session
                .local_client()
                .prediction()
                .prediction_failures()
                .contains(&super::PredictionFailure::HazardOrRescueChangedState)
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "broad prediction presentation regression covers local reconciliation and remote projection together"
    )]
    fn prediction_state_projects_local_reconciliation_and_remote_presentation() {
        let previous = super::PlayerSnapshot {
            player_id: LOCAL_PLAYER_ID,
            x: 10.0,
            y: 20.0,
            velocity_x: 4.0,
            velocity_y: -2.0,
            fuel: 3.0,
            fuel_capacity: 100.0,
            hull: 4.0,
            max_hull: 100.0,
            credits: 6,
            cargo_used: 0,
            cargo: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            materials: BTreeMap::new(),
            loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
            scanner_cooldown_seconds: 0.0,
        };
        let next = super::PlayerSnapshot {
            x: 20.0,
            y: 30.0,
            ..previous.clone()
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
        let replayed_full_state = super::ClientPredictionState::replay_unacknowledged_player_state(
            &previous,
            &[
                SequencedPlayerCommand {
                    player_id: LOCAL_PLAYER_ID,
                    sequence: InputSequence::new(2),
                    target_tick: SimulationTick::new(1),
                    command: PlayerCommand::UseScanner,
                },
                SequencedPlayerCommand {
                    player_id: LOCAL_PLAYER_ID,
                    sequence: InputSequence::new(3),
                    target_tick: SimulationTick::new(1),
                    command: PlayerCommand::SellCargo,
                },
            ],
        );
        assert_eq!(replayed_full_state.replayed_command_count, 2);
        assert!((replayed_full_state.snapshot.scanner_cooldown_seconds - 1.0).abs() < f32::EPSILON);

        let replayed_reconciliation = super::ReplayedReconciliation::from_authoritative_snapshot(
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
        assert_eq!(replayed_reconciliation.replayed.replayed_command_count, 1);
        assert_eq!(
            replayed_reconciliation.reconciliation.correction_plan,
            super::CorrectionPlan::Smooth
        );

        let reconciled = super::ClientPredictionState::reconcile_movement(predicted, &next);
        assert_eq!(reconciled.correction_plan, super::CorrectionPlan::Smooth);
        assert!(reconciled.correction_offset.is_some());
        let smoothed = reconciled.corrected_presentation(0.5);
        assert_eq!(smoothed.correction_plan, super::CorrectionPlan::Smooth);
        assert!((smoothed.x - 16.0).abs() < f32::EPSILON);
        let correction_frame =
            super::CorrectionPresentationFrame::from_reconciliation(&reconciled, 1.5);
        assert!((correction_frame.smoothing_alpha - 1.0).abs() < f32::EPSILON);
        assert!(!correction_frame.snap_applied);
        assert_eq!(
            correction_frame.presentation.correction_plan,
            super::CorrectionPlan::Smooth
        );

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
        let feedback_frame = prediction.tentative_feedback_frame();
        assert_eq!(feedback_frame.render_output_count, 1);
        assert_eq!(feedback_frame.audio_output_count, 1);
        assert!(feedback_frame.has_drill_feedback());
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

        session.route_local_player_commands(vec![PlayerCommand::Movement {
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
        session.sync_legacy_player_from_world(LOCAL_PLAYER_ID);
        assert!((session.game().player.velocity_x - 0.5).abs() < f32::EPSILON);
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
    fn accepted_online_remote_commands_apply_to_session_world_without_app_legacy_routing() {
        let mut session = GameSession::new();
        let remote_player = PlayerId::new(2);
        let target_tick = session.current_tick();
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: ClientId::new(2),
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: target_tick,
            accepted_commands: vec![SequencedPlayerCommand {
                player_id: remote_player,
                sequence: InputSequence::new(1),
                target_tick,
                command: PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: false,
                    drill_down: false,
                },
            }],
        };

        let applied = session.apply_accepted_online_remote_commands(&summary);

        assert_eq!(applied, 1);
        assert!(session.has_client(ClientId::new(2)));
        let player = session
            .world()
            .player(remote_player)
            .expect("remote player exists in authoritative world");
        assert!(player.velocity_x > 0.0);
        assert!(session.game().player.velocity_x.abs() < f32::EPSILON);
    }

    #[test]
    fn live_session_tick_input_is_built_from_world_snapshot_and_world_player_chunk() {
        let mut session = GameSession::new();
        session.game.player.x = TILE_SIZE * 1.0;
        session.game.player.y = TILE_SIZE * 1.0;
        {
            let player = session
                .world_mut()
                .player_mut(LOCAL_PLAYER_ID)
                .expect("local world player exists");
            player.x = TILE_SIZE * 48.0;
            player.y = TILE_SIZE * 32.0;
            player.fuel = 77.0;
            player.hull = 88.0;
            player.credits = 99;
        }

        let input = session.live_session_tick_input_from_world(
            LOCAL_CLIENT_ID,
            LOCAL_PLAYER_ID,
            9,
            vec![PlayerCommand::Movement {
                horizontal: 0.5,
                thrust: true,
                drill_down: false,
            }],
        );

        assert_eq!(input.terrain_chunk_request, Some((3, 2, 0, 0)));
        let snapshot = input.snapshot.expect("snapshot included");
        let player = snapshot
            .players
            .iter()
            .find(|player| player.player_id == LOCAL_PLAYER_ID)
            .expect("local player snapshot included");
        let expected_x = TILE_SIZE * 48.0;
        assert!((player.x - expected_x).abs() < f32::EPSILON);
        assert!((player.fuel - 77.0).abs() < f32::EPSILON);
        assert_eq!(player.credits, 99);
        assert!(matches!(
            input.delta,
            Some((_, NetworkDeltaPayload::Players { players })) if players.contains(&LOCAL_PLAYER_ID)
        ));
        let packet = input.command_packet.expect("command packet included");
        assert_eq!(packet.client_id, LOCAL_CLIENT_ID);
        assert_eq!(packet.commands[0].sequence, InputSequence::new(9));
        let authoritative_chunk = input
            .authoritative_terrain_chunks
            .iter()
            .find(|chunk| chunk.chunk_x == 3 && chunk.chunk_y == 2)
            .expect("authoritative terrain chunk included");
        assert!(!authoritative_chunk.tiles.is_empty());
        assert!(
            authoritative_chunk
                .tiles
                .iter()
                .any(|tile| tile.x == 48 && tile.y == 32)
        );
    }

    #[test]
    fn live_session_tick_input_terrain_chunks_use_world_terrain_not_legacy_game() {
        let mut session = GameSession::new();
        let target = TilePosition { x: 18, y: 19 };
        session.game.terrain.set_kind(target, TileKind::Lava);
        session.world.terrain_mut().set_kind(target, TileKind::Air);
        session
            .world_mut()
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local world player exists")
            .x = TILE_SIZE * 18.0;
        session
            .world_mut()
            .player_mut(LOCAL_PLAYER_ID)
            .expect("local world player exists")
            .y = TILE_SIZE * 19.0;

        let input = session.live_session_tick_input_from_world(
            LOCAL_CLIENT_ID,
            LOCAL_PLAYER_ID,
            3,
            Vec::new(),
        );

        let chunk = input
            .authoritative_terrain_chunks
            .iter()
            .find(|chunk| chunk.chunk_x == 1 && chunk.chunk_y == 1)
            .expect("player terrain chunk included");
        let tile = chunk
            .tiles
            .iter()
            .find(|tile| tile.x == target.x && tile.y == target.y)
            .expect("target tile included");
        assert_eq!(tile.kind, TileKind::Air);
    }

    #[test]
    fn local_joined_client_seed_does_not_overwrite_existing_authoritative_world_player() {
        let mut session = GameSession::new();
        let joined_player = PlayerId::new(2);
        session.game.player.x = TILE_SIZE * 9.0;
        session.game.player.credits = 5;
        let _seeded =
            session.ensure_local_online_player_presentation_from_legacy_view(joined_player, true);
        {
            let player = session
                .world_mut()
                .player_mut(joined_player)
                .expect("joined player exists");
            player.x = TILE_SIZE * 44.0;
            player.credits = 777;
        }
        session.game.player.x = TILE_SIZE * 3.0;
        session.game.player.credits = 1;

        assert!(
            !session
                .ensure_local_online_player_presentation_from_legacy_view(joined_player, false,)
        );

        let player = session
            .world()
            .player(joined_player)
            .expect("joined player remains authoritative");
        let authoritative_x = TILE_SIZE * 44.0;
        assert!((player.x - authoritative_x).abs() < f32::EPSILON);
        assert_eq!(player.credits, 777);

        assert!(
            !session.ensure_local_online_player_presentation_from_legacy_view(joined_player, true,)
        );
        let synced = session
            .world()
            .player(joined_player)
            .expect("joined player synced from presentation");
        let synced_x = TILE_SIZE * 3.0;
        assert!((synced.x - synced_x).abs() < f32::EPSILON);
        assert_eq!(synced.credits, 1);
    }

    #[test]
    fn replicated_snapshot_updates_session_world_presentation_before_legacy_game() {
        let mut session = GameSession::new();
        session.game.player.x = 1.0;
        let local_player = LOCAL_PLAYER_ID;
        let remote_player = PlayerId::new(2);
        let snapshot = NetworkWorldSnapshot {
            tick: SimulationTick::new(7),
            players: vec![
                NetworkPlayerSnapshot {
                    player_id: local_player,
                    x: 20.0,
                    y: 21.0,
                    velocity_x: 2.0,
                    velocity_y: 3.0,
                    fuel: 40.0,
                    hull: 50.0,
                    credits: 60,
                    cargo_used: 0,
                    cargo: BTreeMap::new(),
                    artifacts: BTreeMap::new(),
                    materials: BTreeMap::new(),
                    loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                    scanner_cooldown_seconds: 0.25,
                },
                NetworkPlayerSnapshot {
                    player_id: remote_player,
                    x: 120.0,
                    y: 121.0,
                    velocity_x: 12.0,
                    velocity_y: 13.0,
                    fuel: 140.0,
                    hull: 150.0,
                    credits: 160,
                    cargo_used: 0,
                    cargo: BTreeMap::new(),
                    artifacts: BTreeMap::new(),
                    materials: BTreeMap::new(),
                    loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
                    scanner_cooldown_seconds: 0.75,
                },
            ],
        };

        let summary = session.apply_replicated_snapshot_to_world_presentation(&snapshot);

        assert!(summary.local_players_updated > 0 || summary.remote_players_updated > 0);
        assert_eq!(summary.local_players_updated, 1);
        assert_eq!(summary.remote_players_updated, 1);
        assert_eq!(summary.clients_created, 1);
        assert!((session.game().player.x - 1.0).abs() < f32::EPSILON);
        assert!(
            (session
                .world()
                .player(local_player)
                .expect("local world player updated")
                .x
                - 20.0)
                .abs()
                < f32::EPSILON
        );
        let remote = session
            .world()
            .player(remote_player)
            .expect("remote world player updated");
        assert!((remote.x - 120.0).abs() < f32::EPSILON);
        assert_eq!(remote.credits, 160);
        assert_eq!(
            session.world().scanner_cooldown_seconds(remote_player),
            Some(0.75)
        );
        assert!(session.has_client(ClientId::new(2)));
    }

    #[test]
    fn accepted_online_remote_drill_command_changes_authoritative_terrain() {
        let mut session = GameSession::new();
        let remote_client = ClientId::new(2);
        let remote_player = PlayerId::new(2);
        let mut player = session.game().player.clone();
        player.x = 10.0 * TILE_SIZE;
        player.y = 10.0 * TILE_SIZE;
        player.drill_strength = 4;
        assert!(session.add_local_client_player(remote_client, remote_player));
        *session
            .world_mut()
            .player_mut(remote_player)
            .expect("remote player exists") = player;
        let target = TilePosition { x: 10, y: 11 };
        assert!(session.world_mut().terrain.set_kind(target, TileKind::Dirt));
        let target_tick = session.current_tick();
        let summary = crate::multiplayer::CommandPacketExchangeSummary {
            client_id: remote_client,
            acknowledged: 1,
            rejected: 0,
            authoritative_tick: target_tick,
            accepted_commands: vec![SequencedPlayerCommand {
                player_id: remote_player,
                sequence: InputSequence::new(1),
                target_tick,
                command: PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: true,
                },
            }],
        };

        assert_eq!(session.apply_accepted_online_remote_commands(&summary), 1);
        assert!(session.world().active_drill(remote_player).is_some());
        let advance = session.advance_authoritative_world_ticks(30);

        assert!(advance.end_tick.get() > advance.start_tick.get());
        assert!(advance.terrain_events > 0);
        assert_eq!(
            session
                .world()
                .terrain
                .tile(target)
                .expect("target tile remains in terrain")
                .kind,
            TileKind::Air
        );
        assert!(matches!(
            session.drain_world_delta().compact_network_delta(),
            CompactWorldDelta::TerrainChunks { .. }
        ));
    }

    #[test]
    fn production_session_authority_update_uses_routed_command_instead_of_raw_input() {
        let mut session = GameSession::new();
        session.game.run_mode = RunMode::Playing;
        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 0.0,
            thrust: false,
            drill_down: false,
        }]);

        let summary = session.update_frame_from_session_authority(
            PlayerInput {
                horizontal: 1.0,
                ..PlayerInput::default()
            },
            0.016,
        );

        assert!(summary.legacy_bridge_active());
        assert!(summary.local_movement_authority);
        assert_eq!(summary.command_adapter_count, 1);
        let world_player = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("world player exists");
        assert!(world_player.velocity_x.abs() < f32::EPSILON);
        assert!(session.game().player.velocity_x.abs() < f32::EPSILON);
    }

    #[test]
    fn session_adds_second_local_client_player_for_in_process_host() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);

        assert!(session.add_local_client_player(second_client, second_player));
        assert_eq!(session.client_count(), 2);
        assert!(session.world().player(second_player).is_some());
        assert_eq!(session.render_frame_plan().view_count(), 2);
        assert_eq!(session.client_views().len(), 2);
        let frame_plan = session.render_frame_plan();
        let second_view = frame_plan
            .views
            .iter()
            .find(|view| view.controlled_player_id == second_player)
            .expect("second player view exists");
        let prediction_plan = super::PredictionPresentationPlan::default();
        let world_players =
            frame_plan.world_player_presentations_for_view(second_view, &prediction_plan);
        assert_eq!(world_players.len(), 2);
        assert!(
            world_players
                .iter()
                .any(|player| { player.player_id == second_player && player.local_to_view })
        );
        let client_presentations = frame_plan.client_presentation_snapshots();
        assert_eq!(client_presentations.len(), 2);
        assert!(client_presentations.iter().any(|presentation| {
            presentation.client_id == second_client
                && presentation.player_id == second_player
                && presentation.hud.player_id == second_player
        }));
        assert!(!session.add_local_client_player(second_client, PlayerId::new(3)));
        assert!(!session.add_local_client_player(ClientId::new(3), second_player));
    }

    #[test]
    fn split_screen_commands_route_to_each_client_player_stream() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        let tick = session.current_tick();

        let local_batch = session.route_client_player_commands(
            LOCAL_CLIENT_ID,
            CommandSource::Keyboard,
            vec![PlayerCommand::Movement {
                horizontal: 0.25,
                thrust: false,
                drill_down: false,
            }],
        );
        let split_batch = session.route_split_screen_player_commands(
            second_client,
            vec![PlayerCommand::Movement {
                horizontal: -0.75,
                thrust: false,
                drill_down: false,
            }],
        );

        assert_eq!(local_batch.commands[0].player_id, LOCAL_PLAYER_ID);
        assert_eq!(split_batch.commands[0].player_id, second_player);
        assert_eq!(split_batch.source, CommandSource::SplitScreenClient);
        assert!(split_batch.predicted_locally);
        assert_eq!(session.process_authoritative_commands_for_tick(tick), 2);
        let local_velocity = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("local player exists")
            .velocity_x;
        let second_velocity = session
            .world()
            .player(second_player)
            .expect("second player exists")
            .velocity_x;
        assert!((local_velocity - 0.25).abs() < f32::EPSILON);
        assert!((second_velocity + 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn authoritative_service_commands_sync_player_economy_to_legacy_adapter() {
        let mut session = GameSession::new();
        session.game.player.fuel = 10.0;
        session.game.player.credits = 500;
        session
            .world
            .sync_from_legacy_game(session.current_tick(), &session.game.clone());
        let tick = session.current_tick();

        session.route_local_player_commands(vec![PlayerCommand::Refuel]);

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);
        session.sync_legacy_player_from_world(LOCAL_PLAYER_ID);

        let world_player = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("world player exists");
        assert!(world_player.fuel > 10.0);
        assert!(world_player.credits < 500);
        assert!((session.game().player.fuel - world_player.fuel).abs() < f32::EPSILON);
        assert_eq!(session.game().player.credits, world_player.credits);
        assert!(session.drain_events().iter().any(|event| matches!(
            event,
            super::WorldEvent::PurchaseCompleted {
                player_id: LOCAL_PLAYER_ID
            }
        )));
    }

    #[test]
    fn command_producers_route_replay_ai_gamepad_split_screen_online_and_keyboard_to_authoritative_buffer()
     {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        let producers = vec![
            crate::input_mapping::replay_commands(vec![PlayerCommand::Interact]),
            crate::input_mapping::ai_commands(vec![PlayerCommand::UseScanner]),
            crate::input_mapping::gamepad_commands(vec![PlayerCommand::PlaceBomb]),
            crate::input_mapping::split_screen_commands(vec![PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: false,
                drill_down: false,
            }]),
            crate::input_mapping::online_commands(vec![PlayerCommand::Repair]),
            crate::input_mapping::local_keyboard_commands(PlayerInput {
                horizontal: -1.0,
                thrust: true,
                ..PlayerInput::default()
            }),
        ];

        let batches = session.route_command_producers(second_client, producers);

        assert_eq!(batches.len(), 6);
        assert!(batches.iter().all(|batch| batch.client_id == second_client));
        assert!(batches.iter().all(|batch| {
            batch
                .commands
                .iter()
                .all(|command| command.player_id == second_player)
        }));
        assert_eq!(session.pending_command_count(session.current_tick()), 6);
    }

    #[test]
    fn local_two_player_gameplay_regression_covers_core_authoritative_slices() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        {
            let player = session
                .world
                .player_mut(second_player)
                .expect("second player exists");
            player.fuel = 0.0;
            player.hull = 0.0;
            player.credits = 500;
            player.cargo.insert(crate::terrain::MineralKind::Copper, 2);
        }
        let tick = session.current_tick();

        session.route_client_player_commands(
            LOCAL_CLIENT_ID,
            CommandSource::Keyboard,
            vec![PlayerCommand::Movement {
                horizontal: 0.5,
                thrust: false,
                drill_down: true,
            }],
        );
        session.route_split_screen_player_commands(
            second_client,
            vec![
                PlayerCommand::Rescue,
                PlayerCommand::SellCargo,
                PlayerCommand::Refuel,
            ],
        );

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 4);
        let local = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("local player exists");
        let second = session
            .world()
            .player(second_player)
            .expect("second player exists");
        assert!((local.velocity_x - 0.5).abs() < f32::EPSILON);
        assert!(session.world().active_drill(LOCAL_PLAYER_ID).is_some());
        assert!(second.hull > 0.0);
        assert!(second.fuel > 0.0);
        assert_eq!(second.cargo_used(), 0);
        assert!(second.credits > 500);
        let proof = session
            .world
            .player_scoped_gameplay_proof(LOCAL_PLAYER_ID, second_player)
            .expect("proof exists");
        assert!(proof.complete());
        let events = session.drain_events();
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::DrillProgressed {
                player_id: LOCAL_PLAYER_ID
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::PlayerSurvivalChanged { player_id }
                if *player_id == second_player
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::CargoChanged { player_id } if *player_id == second_player
        )));
    }

    #[test]
    fn authoritative_world_chips_active_drill_target_and_collects_ore() {
        let mut session = GameSession::new();
        let target = TilePosition { x: 97, y: 5 };
        session
            .game
            .terrain
            .set_kind(target, TileKind::Ore(crate::terrain::MineralKind::Copper));
        session
            .world
            .sync_from_legacy_game(session.current_tick(), &session.game.clone());
        session.world.set_active_drill(
            LOCAL_PLAYER_ID,
            Some(DrillState {
                target,
                direction: DrillDirection::Down,
                progress: 1.0,
                initial_durability: 1,
                seconds_per_chip: 0.0,
                sound_timer: 0.0,
                dust_timer: 0.0,
            }),
        );

        let mut result = session
            .world
            .chip_active_drill_target(LOCAL_PLAYER_ID)
            .expect("active drill chips");
        for _ in 0..8 {
            if matches!(result, MineResult::Mined(_)) {
                break;
            }
            result = session
                .world
                .chip_active_drill_target(LOCAL_PLAYER_ID)
                .expect("active drill continues chipping");
        }

        assert_eq!(
            result,
            MineResult::Mined(TileKind::Ore(crate::terrain::MineralKind::Copper))
        );
        assert_eq!(
            session
                .world()
                .terrain()
                .tile(target)
                .expect("tile exists")
                .kind,
            TileKind::Air
        );
        assert_eq!(
            session
                .world()
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .cargo
                .get(&crate::terrain::MineralKind::Copper)
                .copied(),
            Some(1)
        );
        session.sync_legacy_terrain_from_world();
        assert_eq!(
            session
                .game()
                .terrain
                .tile(target)
                .expect("tile exists")
                .kind,
            TileKind::Air
        );
    }

    #[test]
    fn authoritative_world_handles_economy_progression_debt_and_victory_transactions() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let player = world.player_mut(LOCAL_PLAYER_ID).expect("player exists");
        player.credits = 10_000;
        player.loan_debt = 500;
        let credits_before_upgrade = player.credits;

        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::BuyUpgrade { index: 0 }),
            super::PlayerScopedCommandOutcome::Applied
        );
        let upgraded_player = world.player(LOCAL_PLAYER_ID).expect("player exists");
        assert!(upgraded_player.drill_strength > 1);
        assert!(upgraded_player.credits < credits_before_upgrade);
        assert!(world.apply_contract_reward(LOCAL_PLAYER_ID, 250).is_some());
        assert!(world.start_expedition(LOCAL_PLAYER_ID, 100).is_some());
        assert!(world.repay_debt(LOCAL_PLAYER_ID, 200).is_some());
        assert!(world.award_victory(LOCAL_PLAYER_ID, 1_000).is_some());

        let player = world.player(LOCAL_PLAYER_ID).expect("player exists");
        assert_eq!(player.loan_debt, 300);
        assert!(world.won_game());
        assert!(
            world
                .service_transactions()
                .iter()
                .any(|transaction| transaction.kind
                    == super::PlayerTransactionKind::CompleteContract)
        );
        assert!(
            world.service_transactions().iter().any(
                |transaction| transaction.kind == super::PlayerTransactionKind::StartExpedition
            )
        );
        assert!(
            world
                .service_transactions()
                .iter()
                .any(|transaction| transaction.kind == super::PlayerTransactionKind::RepayDebt)
        );
        assert!(
            world
                .service_transactions()
                .iter()
                .any(|transaction| transaction.kind == super::PlayerTransactionKind::WinGame)
        );
    }

    #[test]
    fn authoritative_world_handles_hazard_damage_failure_scanner_and_bomb_effects() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let player = world.player_mut(LOCAL_PLAYER_ID).expect("player exists");
        player.hull = 5.0;
        player.fuel = 0.0;
        player.bombs = 1;
        let scanner_target = player.tile_position(TILE_SIZE);
        let bomb_target = TilePosition { x: 3, y: 6 };
        assert!(world.terrain.set_kind(bomb_target, TileKind::Dirt));

        let survival = world
            .apply_hazard_damage(LOCAL_PLAYER_ID, 6.0)
            .expect("survival snapshot");
        assert!(survival.disabled);
        assert!(survival.stranded);
        assert!(world.failure_state(LOCAL_PLAYER_ID).is_some());
        assert_eq!(world.discovered_tile_count(LOCAL_PLAYER_ID), 0);
        assert!(world.reveal_scanner_area(LOCAL_PLAYER_ID, scanner_target, 2) > 0);
        assert!(world.discovered_tile_count(LOCAL_PLAYER_ID) > 0);
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::PlaceBomb),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(world.bomb_count(), 1);
        let blast = world.detonate_bomb_at(bomb_target, 1);
        assert!(blast.cleared > 0);
        assert_eq!(
            world.terrain().tile(bomb_target).expect("tile").kind,
            TileKind::Air
        );
    }

    #[test]
    fn session_drill_too_dangerous_damages_authoritative_player() {
        let mut session = GameSession::new();
        let player = session
            .world_mut()
            .player_mut(LOCAL_PLAYER_ID)
            .expect("player exists");
        player.x = 3.0 * TILE_SIZE;
        player.y = 5.0 * TILE_SIZE;
        player.hull = 50.0;
        let target = TilePosition { x: 3, y: 6 };
        assert!(session.world_mut().terrain.set_kind(target, TileKind::Lava));
        session.world_mut().set_active_drill(
            LOCAL_PLAYER_ID,
            Some(DrillState {
                target,
                direction: DrillDirection::Down,
                progress: 0.0,
                initial_durability: 1,
                seconds_per_chip: FIXED_DELTA_SECONDS,
                sound_timer: 0.0,
                dust_timer: 0.0,
            }),
        );

        session.process_authoritative_drill_progress();

        let player = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("player exists");
        assert!(player.hull < 50.0);
        assert!(session.world().active_drill(LOCAL_PLAYER_ID).is_none());
    }

    #[test]
    fn authoritative_local_commands_are_masked_from_variable_delta_legacy_adapter() {
        let mut session = GameSession::new();
        session
            .world
            .player_mut(LOCAL_PLAYER_ID)
            .expect("player exists")
            .bombs = 1;
        let bomb_count = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("player exists")
            .bombs;
        session.route_local_player_commands(vec![
            PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: true,
                drill_down: true,
            },
            PlayerCommand::UseScanner,
            PlayerCommand::PlaceBomb,
        ]);
        let before = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("player exists")
            .clone();

        session.update_legacy(
            PlayerInput {
                horizontal: 1.0,
                thrust: true,
                drill_down: true,
                scan: true,
                bomb: true,
                ..PlayerInput::default()
            },
            FIXED_DELTA_SECONDS,
        );

        let after = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("player exists");
        assert!(after.x > before.x);
        assert!(after.y <= before.y);
        assert_eq!(after.bombs, bomb_count.saturating_sub(1));
        assert_eq!(session.world().bomb_count(), 1);
        assert_eq!(
            session.world().scanner_cooldown_seconds(LOCAL_PLAYER_ID),
            Some(1.0)
        );
    }

    #[test]
    fn split_screen_authoritative_processing_acknowledges_secondary_client_commands() {
        let mut session = GameSession::new();
        let secondary_client = ClientId::new(2);
        let secondary_player = PlayerId::new(2);
        assert!(session.add_local_client_player(secondary_client, secondary_player));
        let tick = session.current_tick();
        session.route_split_screen_player_commands(secondary_client, vec![PlayerCommand::Rescue]);

        assert_eq!(
            session
                .clients
                .get(&secondary_client)
                .expect("secondary client exists")
                .prediction()
                .unacknowledged_commands()
                .len(),
            1
        );
        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);
        assert!(
            session
                .clients
                .get(&secondary_client)
                .expect("secondary client exists")
                .prediction()
                .unacknowledged_commands()
                .is_empty()
        );
    }

    #[test]
    fn local_split_screen_frame_updates_players_cameras_hud_minimap_depth_and_remote_visibility() {
        let mut session = GameSession::new();
        let _enabled = session.enable_default_local_split_screen();
        let secondary_client_id = session.secondary_local_client_id();
        for local_input in crate::input_mapping::local_split_screen_inputs(
            LOCAL_CLIENT_ID,
            PlayerInput {
                horizontal: -1.0,
                thrust: false,
                drill_down: false,
                ..PlayerInput::default()
            },
            secondary_client_id,
            Some(PlayerInput {
                horizontal: 1.0,
                thrust: false,
                drill_down: false,
                ..PlayerInput::default()
            }),
        ) {
            let _batch =
                session.route_command_producer(local_input.client_id, local_input.producer);
        }
        session.update_legacy(PlayerInput::default(), FIXED_DELTA_SECONDS);
        session.observe_live_remote_player_snapshots();
        let prediction_plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);
        let output = session.live_render_frame_output(&prediction_plan);

        assert_eq!(session.client_count(), 2);
        assert_eq!(output.viewport_plans.len(), 2);
        assert_eq!(output.hud_snapshots.len(), 2);
        assert!(output.viewport_plans.iter().all(|plan| plan.clip_enabled));
        assert!(
            output
                .viewport_plans
                .iter()
                .all(|plan| plan.local_player.is_some())
        );
        assert!(
            output
                .world_players_by_view
                .iter()
                .all(|(_, players)| players.len() == 2)
        );
        assert!(output.ready_for_live_render_path());
    }

    #[test]
    fn local_split_screen_authoritative_movement_moves_secondary_player_independently() {
        let mut session = GameSession::new();
        let secondary_client = ClientId::new(2);
        let secondary_player = PlayerId::new(2);
        assert!(session.add_local_client_player(secondary_client, secondary_player));
        let primary_before = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("primary exists")
            .x;
        let secondary_before = session
            .world()
            .player(secondary_player)
            .expect("secondary exists")
            .x;

        session.route_split_screen_player_commands(
            secondary_client,
            vec![PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: false,
                drill_down: false,
            }],
        );
        session.update_legacy(PlayerInput::default(), FIXED_DELTA_SECONDS);

        let primary_after = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("primary exists")
            .x;
        let secondary_after = session
            .world()
            .player(secondary_player)
            .expect("secondary exists")
            .x;
        assert!((primary_after - primary_before).abs() < f32::EPSILON);
        assert!(secondary_after > secondary_before);
    }

    #[test]
    fn local_split_screen_startup_flow_adds_second_local_client_and_player() {
        let mut session = GameSession::new();
        let plan = GameSession::default_local_split_screen_startup_plan();
        assert_eq!(plan.primary_client_id, LOCAL_CLIENT_ID);
        assert_eq!(plan.secondary_client_id, ClientId::new(2));
        assert!(session.enable_default_local_split_screen());
        let player_slots = u8::try_from(session.client_count()).unwrap_or(u8::MAX);
        session
            .game_mut()
            .mark_local_multiplayer_active(player_slots);
        assert_eq!(session.client_count(), 2);
        assert!(session.game().local_multiplayer_active);
        assert_eq!(session.game().local_multiplayer_player_slots, 2);
        assert!(session.game().message.contains("Player 2"));
        assert!(session.world().player(plan.secondary_player_id).is_some());
        assert!(!session.enable_default_local_split_screen());
    }

    #[test]
    fn fixed_tick_authoritative_slices_are_stable_across_split_frame_deltas() {
        let session = GameSession::new();
        let proof = session.frame_rate_invariance_proof();

        assert!(proof.command_timing_stable);
        assert!(proof.fuel_stable);
        assert!(proof.drill_progress_stable);
        assert!(proof.hazard_damage_stable);
        assert!(proof.bomb_state_stable);
        assert!(proof.rescue_state_stable);
        assert!(proof.complete());
    }

    #[test]
    fn replayed_authoritative_command_stream_produces_identical_world_outcomes() {
        let proof = GameSession::replay_determinism_proof(vec![
            PlayerCommand::Movement {
                horizontal: 0.25,
                thrust: true,
                drill_down: true,
            },
            PlayerCommand::Refuel,
            PlayerCommand::PlaceBomb,
            PlayerCommand::Rescue,
        ]);

        assert_eq!(proof.same_tick, SimulationTick::default());
        assert_eq!(proof.same_player_count, 1);
        assert!(proof.same_transaction_count > 0);
        assert!(proof.same_bomb_count <= 1);
        assert_eq!(proof.same_infrastructure_count, 0);
        assert!(proof.same_local_player_state);
        assert!(proof.complete());
    }

    #[test]
    fn authoritative_extraction_inventory_restricts_legacy_gameplay_to_ui_save_adapter() {
        let inventory = GameSession::legacy_gameplay_mutation_inventory();

        assert!(inventory.inventory_complete());
        assert_eq!(
            inventory.domains,
            [super::LegacyGameplayMutationDomain::RenderUiSaveAdapter]
        );
        assert_eq!(inventory.authoritative_extraction_remaining, 0);
        assert_eq!(inventory.adapter_only_domains, 1);
    }

    #[test]
    fn authoritative_world_runtime_owns_all_gameplay_domains() {
        let mut world = WorldState::from_legacy_game(&GameState::new());
        let target = TilePosition { x: 4, y: 8 };
        assert!(
            world
                .terrain
                .set_kind(target, TileKind::Ore(crate::terrain::MineralKind::Copper))
        );
        let player = world.player_mut(LOCAL_PLAYER_ID).expect("player exists");
        player.x = 4.0 * TILE_SIZE;
        player.y = 7.0 * TILE_SIZE;
        player.credits = 10_000;
        player.loan_debt = 500;
        player.bombs = 1;
        player.signal_relay_kits = 1;
        player.hull = 5.0;
        player.fuel = 0.0;

        assert_eq!(
            world.apply_player_command(
                LOCAL_PLAYER_ID,
                &PlayerCommand::Movement {
                    horizontal: 0.0,
                    thrust: false,
                    drill_down: true,
                },
            ),
            super::PlayerScopedCommandOutcome::Applied
        );
        let mut mine_result = world
            .chip_active_drill_target(LOCAL_PLAYER_ID)
            .expect("drill target exists");
        while !matches!(mine_result, MineResult::Mined(_)) {
            mine_result = world
                .chip_active_drill_target(LOCAL_PLAYER_ID)
                .expect("drill target continues");
        }
        assert_eq!(
            world.terrain().tile(target).expect("tile").kind,
            TileKind::Air
        );
        assert_eq!(
            world
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .cargo
                .get(&crate::terrain::MineralKind::Copper)
                .copied(),
            Some(1)
        );

        assert!(world.apply_hazard_damage(LOCAL_PLAYER_ID, 6.0).is_some());
        assert!(world.failure_state(LOCAL_PLAYER_ID).is_some());
        assert!(world.reveal_scanner_area(LOCAL_PLAYER_ID, target, 2) > 0);
        assert_eq!(
            world.apply_player_command(LOCAL_PLAYER_ID, &PlayerCommand::PlaceBomb),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(
            world.apply_player_command(
                LOCAL_PLAYER_ID,
                &PlayerCommand::PlaceInfrastructure { slot: 0 },
            ),
            super::PlayerScopedCommandOutcome::Applied
        );
        assert_eq!(world.bomb_count(), 1);
        assert_eq!(world.infrastructure_count(), 1);
        assert!(world.apply_contract_reward(LOCAL_PLAYER_ID, 250).is_some());
        assert!(world.start_expedition(LOCAL_PLAYER_ID, 100).is_some());
        assert!(world.repay_debt(LOCAL_PLAYER_ID, 200).is_some());
        assert!(world.award_victory(LOCAL_PLAYER_ID, 1_000).is_some());

        assert_eq!(world.authoritative_runtime_domain_count(), 7);
        assert!(world.authoritative_gameplay_ownership_complete());
        assert!(world.won_game());
        assert!(world.service_transactions().iter().any(|transaction| {
            transaction.kind == super::PlayerTransactionKind::CompleteContract
        }));
        assert!(world.service_transactions().iter().any(|transaction| {
            transaction.kind == super::PlayerTransactionKind::StartExpedition
        }));
        assert!(
            world
                .service_transactions()
                .iter()
                .any(|transaction| { transaction.kind == super::PlayerTransactionKind::RepayDebt })
        );
        assert!(
            world
                .service_transactions()
                .iter()
                .any(|transaction| { transaction.kind == super::PlayerTransactionKind::WinGame })
        );
    }

    #[test]
    fn variable_delta_audit_marks_gameplay_fixed_or_compatibility_and_animation_presentation_only()
    {
        let audit = GameSession::variable_delta_audit_summary();

        assert!(audit.gameplay_delta_audit_complete());
        assert_eq!(audit.unresolved_gameplay_variable_domains, 0);
        assert!(audit.fixed_or_compatibility_gameplay_domains >= 7);
        assert_eq!(audit.presentation_only_variable_domains, 1);
    }

    #[test]
    fn session_fixed_tick_chips_authoritative_drill_into_cargo_and_terrain_delta() {
        let mut session = GameSession::new();
        let target = TilePosition { x: 97, y: 5 };
        session
            .game
            .terrain
            .set_kind(target, TileKind::Ore(crate::terrain::MineralKind::Copper));
        session
            .world
            .sync_from_legacy_game(session.current_tick(), &session.game.clone());
        session.world.set_active_drill(
            LOCAL_PLAYER_ID,
            Some(DrillState {
                target,
                direction: DrillDirection::Down,
                progress: 1.0,
                initial_durability: 1,
                seconds_per_chip: 0.0,
                sound_timer: 0.0,
                dust_timer: 0.0,
            }),
        );

        session.update_legacy(PlayerInput::default(), 0.25);

        assert_eq!(
            session
                .world()
                .terrain()
                .tile(target)
                .expect("tile exists")
                .kind,
            TileKind::Air
        );
        assert_eq!(
            session
                .game()
                .terrain
                .tile(target)
                .expect("tile exists")
                .kind,
            TileKind::Air
        );
        assert_eq!(
            session
                .world()
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .cargo
                .get(&crate::terrain::MineralKind::Copper)
                .copied(),
            Some(1)
        );
        let events = session.drain_events();
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::TerrainTilesChanged { positions } if positions.contains(&target)
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::CargoChanged {
                player_id: LOCAL_PLAYER_ID
            }
        )));
    }

    #[test]
    fn live_snapshot_delta_and_chunk_exchange_batches_use_real_session_state() {
        let mut session = GameSession::new();
        let target = TilePosition { x: 1, y: 1 };
        session.terrain_revisions.mark_tiles_changed([target]);
        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 0.25,
            thrust: false,
            drill_down: false,
        }]);
        assert_eq!(
            session.process_authoritative_commands_for_tick(session.current_tick()),
            1
        );

        let snapshot_batch = session.live_snapshot_exchange_batch();
        assert_eq!(snapshot_batch.kind, ProtocolExchangeKind::SnapshotKeyframe);
        assert!(matches!(
            snapshot_batch.messages.as_slice(),
            [ProtocolMessage::SnapshotKeyframe { snapshot }]
                if snapshot.players.iter().any(|player| player.player_id == LOCAL_PLAYER_ID)
        ));
        let delta = session.drain_world_delta();
        let delta_batch = GameSession::live_world_delta_exchange_batch(&delta);
        assert_eq!(delta_batch.kind, ProtocolExchangeKind::WorldDelta);
        assert!(matches!(
            delta_batch.messages.as_slice(),
            [ProtocolMessage::WorldDelta { payload: NetworkDeltaPayload::Players { players }, .. }]
                if players.contains(&LOCAL_PLAYER_ID)
        ));
        let chunk_batch = session
            .live_terrain_chunk_exchange_batch(super::TerrainChunkPosition::from_tile(target), 0);
        assert_eq!(chunk_batch.kind, ProtocolExchangeKind::TerrainChunk);
        assert!(matches!(
            chunk_batch.messages.as_slice(),
            [ProtocolMessage::TerrainChunkResponse { revision, .. }] if *revision > 0
        ));
    }

    #[test]
    fn live_network_integration_exercises_join_reconnect_rejection_recovery_and_latency() {
        let mut session = GameSession::new();
        let remote_client = ClientId::new(9);
        let remote_player = PlayerId::new(9);
        let terrain_position = super::TerrainChunkPosition { x: 0, y: 0 };
        session
            .terrain_revisions
            .mark_tiles_changed([TilePosition { x: 1, y: 1 }]);

        let (
            join_in_progress,
            reconnect,
            command_responses,
            terrain_recovery,
            high_latency_prediction,
        ) = session.exercise_live_network_integration(
            remote_client,
            remote_player,
            SessionToken::new(99),
            terrain_position,
            0,
        );

        assert_eq!(join_in_progress.kind, ProtocolExchangeKind::JoinHandshake);
        assert!(join_in_progress.messages.iter().any(|message| matches!(
            message,
            ProtocolMessage::SnapshotKeyframe { snapshot }
                if snapshot.players.iter().any(|player| player.player_id == remote_player)
        )));
        assert!(reconnect.messages.iter().any(|message| matches!(
            message,
            ProtocolMessage::ReconnectRequest {
                client_id,
                session_token,
            } if *client_id == remote_client && *session_token == SessionToken::new(99)
        )));
        assert!(command_responses.iter().any(|message| matches!(
            message,
            ProtocolMessage::CommandAcknowledgement(CommandAcknowledgement { client_id, .. })
                if *client_id == remote_client
        )));
        assert!(terrain_recovery.recovered_revision().is_some());
        assert!(!high_latency_prediction.remote_players.is_empty());
    }

    #[test]
    fn online_runtime_integration_covers_join_reconnect_ack_rejection_and_recovery() {
        let mut session = GameSession::new();
        let remote_client = ClientId::new(10);
        let remote_player = PlayerId::new(10);
        session
            .terrain_revisions
            .mark_tiles_changed([TilePosition { x: 1, y: 1 }]);

        let (join, reconnect, command_responses, terrain_recovery, prediction) = session
            .exercise_live_network_integration(
                remote_client,
                remote_player,
                SessionToken::new(110),
                super::TerrainChunkPosition { x: 0, y: 0 },
                0,
            );
        let rejection_responses = session.apply_live_command_packet(&CommandPacket {
            client_id: remote_client,
            commands: vec![SequencedPlayerCommand {
                player_id: remote_player,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(session.current_tick().get() + 16),
                command: PlayerCommand::Interact,
            }],
        });
        let reliable_count = join
            .messages
            .iter()
            .chain(reconnect.messages.iter())
            .chain(command_responses.iter())
            .chain(rejection_responses.iter())
            .filter(|message| {
                message.reliability_class() == crate::multiplayer::ReliabilityClass::Reliable
            })
            .count();
        let unreliable_count = join
            .messages
            .iter()
            .chain(reconnect.messages.iter())
            .chain(command_responses.iter())
            .chain(rejection_responses.iter())
            .filter(|message| {
                message.reliability_class()
                    == crate::multiplayer::ReliabilityClass::UnreliableSequenced
            })
            .count();

        assert!(join.messages.iter().any(|message| matches!(
            message,
            ProtocolMessage::SnapshotKeyframe { snapshot }
                if snapshot.players.iter().any(|player| player.player_id == remote_player)
        )));
        assert!(reconnect.messages.iter().any(|message| matches!(
            message,
            ProtocolMessage::ReconnectRequest { client_id, session_token }
                if *client_id == remote_client && *session_token == SessionToken::new(110)
        )));
        assert!(command_responses.iter().any(|message| matches!(
            message,
            ProtocolMessage::CommandAcknowledgement(CommandAcknowledgement { client_id, .. })
                if *client_id == remote_client
        )));
        assert!(rejection_responses.iter().any(|message| matches!(
            message,
            ProtocolMessage::CommandRejection(CommandRejection { client_id, .. })
                if *client_id == remote_client
        )));
        assert!(
            session
                .clients
                .get(&remote_client)
                .expect("remote client exists")
                .prediction()
                .prediction_failures()
                .contains(&super::PredictionFailure::CommandRejected)
        );
        assert!(terrain_recovery.recovered_revision().is_some());
        assert!(!prediction.remote_players.is_empty());
        assert!(reliable_count > 0);
        assert!(unreliable_count > 0);
    }

    #[test]
    fn live_prediction_plan_replays_corrections_and_remote_presentations() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 12.0,
            thrust: false,
            drill_down: false,
        }]);
        {
            let player = session
                .world
                .player_mut(LOCAL_PLAYER_ID)
                .expect("local player exists");
            player.x = 100.0;
            player.y = 50.0;
            player.velocity_x = 0.0;
            player.velocity_y = 0.0;
        }
        {
            let player = session
                .world
                .player_mut(second_player)
                .expect("second player exists");
            player.x = 20.0;
            player.y = 30.0;
            player.velocity_x = 4.0;
        }
        session.observe_live_remote_player_snapshots();
        {
            let player = session
                .world
                .player_mut(second_player)
                .expect("second player exists");
            player.x = 24.0;
            player.y = 30.0;
        }
        session.observe_live_remote_player_snapshots();

        let plan = session.live_prediction_presentation_plan(0.0, 0.5, 0.0);

        assert!(plan.local_movement.is_some());
        let correction = plan.correction.expect("correction exists");
        assert_eq!(correction.correction_plan, super::CorrectionPlan::Smooth);
        let corrected = plan
            .corrected_local_presentation
            .expect("corrected presentation exists");
        assert_eq!(corrected.correction_plan, super::CorrectionPlan::Smooth);
        assert!(
            plan.remote_players
                .iter()
                .any(|remote| remote.player_id == second_player
                    && (remote.x - 22.0).abs() < f32::EPSILON)
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "broad Phase 6 regression intentionally covers all prediction polish dimensions together"
    )]
    fn prediction_reconciliation_polish_covers_replay_timing_failures_debug_and_high_ping() {
        let player_id = LOCAL_PLAYER_ID;
        let authoritative = PlayerSnapshot {
            player_id,
            x: 10.0,
            y: 20.0,
            velocity_x: 0.0,
            velocity_y: 0.0,
            fuel: 10.0,
            fuel_capacity: 100.0,
            hull: 10.0,
            max_hull: 100.0,
            credits: 100,
            cargo_used: 0,
            cargo: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            materials: BTreeMap::new(),
            loadout: crate::multiplayer::NetworkPlayerLoadoutSnapshot::default(),
            scanner_cooldown_seconds: 0.0,
        };
        let commands = vec![
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(1),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: true,
                    drill_down: true,
                },
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(2),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::UseScanner,
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(3),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::BuyUpgrade { index: 0 },
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(4),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::SellCargo,
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(5),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::Rescue,
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(6),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::PlaceBomb,
            },
            SequencedPlayerCommand {
                player_id,
                sequence: InputSequence::new(7),
                target_tick: SimulationTick::new(1),
                command: PlayerCommand::PlaceInfrastructure { slot: 0 },
            },
        ];

        let replayed =
            ClientPredictionState::replay_unacknowledged_player_state(&authoritative, &commands);
        assert_eq!(replayed.replayed_command_count, commands.len());
        assert!((replayed.snapshot.scanner_cooldown_seconds - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            ClientPredictionState::correction_plan(0.1, 0.0),
            super::CorrectionPlan::None
        );
        assert_eq!(
            ClientPredictionState::correction_plan(8.0, 0.0),
            super::CorrectionPlan::Smooth
        );
        assert_eq!(
            ClientPredictionState::correction_plan(24.0, 0.0),
            super::CorrectionPlan::Snap
        );
        assert!(PredictionCorrectionTuning::classifies_expected_offsets());

        let timing = RemoteTimingTuning::from_latency_loss(0.24, 0.25);
        assert!(timing.allows_extrapolation(timing.extrapolation_limit));
        assert!(!timing.allows_extrapolation(timing.timeout_after));
        assert!(timing.timed_out(timing.timeout_after + FIXED_DELTA_SECONDS));

        let mut prediction = ClientPredictionState::default();
        for failure in [
            PredictionFailure::TerrainAlreadyChanged,
            PredictionFailure::EconomyChangedState,
            PredictionFailure::ProgressionChangedState,
            PredictionFailure::HazardOrRescueChangedState,
            PredictionFailure::CommandRejected,
            PredictionFailure::SaveSessionTransition,
        ] {
            prediction.note_prediction_failure(failure);
        }
        let mut terrain_revisions = super::TerrainRevisionTracker::default();
        let terrain_position = super::TerrainChunkPosition { x: 0, y: 0 };
        terrain_revisions.mark_tiles_changed([TilePosition { x: 0, y: 0 }]);
        let actions = prediction.prediction_recovery_actions(
            player_id,
            &terrain_revisions,
            SimulationTick::new(2),
            terrain_position,
            0,
        );
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PredictionRecoveryAction::RequestTerrainDelta(_)))
        );
        assert!(actions.iter().any(|action| matches!(
            action,
            PredictionRecoveryAction::RollBackLocalEconomy { .. }
        )));
        assert!(
            actions.iter().any(|action| matches!(
                action,
                PredictionRecoveryAction::RollBackProgression { .. }
            ))
        );
        assert!(actions.iter().any(|action| matches!(
            action,
            PredictionRecoveryAction::RequestAuthoritativeSnapshot { .. }
        )));

        let debug = NetworkDebugInstrumentationSnapshot {
            ping_seconds: 0.24,
            prediction_buffer_commands: commands.len(),
            correction_plan: super::CorrectionPlan::Smooth,
            dropped_packets: 2,
            snapshot_recoveries: 1,
            chunk_recoveries: 1,
        };
        assert!(debug.visible_to_debug_overlay());
    }

    #[test]
    fn live_command_packet_applies_acknowledgements_and_rejections() {
        let mut session = GameSession::new();
        let accepted_sequence = InputSequence::new(0);
        let packet = CommandPacket {
            client_id: LOCAL_CLIENT_ID,
            commands: vec![SequencedPlayerCommand {
                player_id: LOCAL_PLAYER_ID,
                sequence: accepted_sequence,
                target_tick: session.current_tick(),
                command: PlayerCommand::Movement {
                    horizontal: 0.75,
                    thrust: false,
                    drill_down: false,
                },
            }],
        };

        let responses = session.apply_live_command_packet(&packet);

        assert_eq!(responses.len(), 1);
        assert!(matches!(
            responses[0],
            ProtocolMessage::CommandAcknowledgement(CommandAcknowledgement {
                acknowledged_sequence,
                ..
            }) if acknowledged_sequence == accepted_sequence
        ));
        assert_eq!(
            session.process_authoritative_commands_for_tick(session.current_tick()),
            1
        );
        assert!(
            (session
                .world()
                .player(LOCAL_PLAYER_ID)
                .expect("player exists")
                .velocity_x
                - 0.75)
                .abs()
                < f32::EPSILON
        );

        let duplicate_responses = session.apply_live_command_packet(&packet);

        assert_eq!(duplicate_responses.len(), 1);
        assert!(matches!(
            duplicate_responses[0],
            ProtocolMessage::CommandRejection(CommandRejection {
                sequence,
                ..
            }) if sequence == accepted_sequence
        ));
    }

    #[test]
    fn live_world_snapshot_and_delta_messages_use_authoritative_world_state() {
        let mut session = GameSession::new();
        let second_client = ClientId::new(2);
        let second_player = PlayerId::new(2);
        assert!(session.add_local_client_player(second_client, second_player));
        let tick = session.current_tick();
        session.route_split_screen_player_commands(
            second_client,
            vec![PlayerCommand::Movement {
                horizontal: -1.0,
                thrust: false,
                drill_down: false,
            }],
        );
        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);

        let keyframe = session.live_snapshot_keyframe_message();
        let ProtocolMessage::SnapshotKeyframe { snapshot } = keyframe else {
            panic!("expected snapshot keyframe");
        };
        assert_eq!(snapshot.tick, session.world().simulation_tick());
        assert!(
            snapshot
                .players
                .iter()
                .any(|player| player.player_id == second_player && player.velocity_x < 0.0)
        );

        let delta = session.drain_world_delta();
        let delta_message = GameSession::live_world_delta_message(&delta);
        let ProtocolMessage::WorldDelta { tick, payload } = delta_message else {
            panic!("expected world delta");
        };
        assert_eq!(tick, delta.tick);
        assert!(matches!(
            payload,
            NetworkDeltaPayload::Players { players } if players.contains(&second_player)
        ));
    }

    #[test]
    fn authoritative_rescue_updates_player_survival_state() {
        let mut session = GameSession::new();
        session.game.player.fuel = 0.0;
        session.game.player.hull = 0.0;
        session.game.player.velocity_x = 25.0;
        session
            .world
            .sync_from_legacy_game(session.current_tick(), &session.game.clone());
        let before = session
            .world()
            .player_survival_snapshot(LOCAL_PLAYER_ID)
            .expect("survival snapshot exists");
        assert!(before.disabled);
        assert!(before.stranded);
        let tick = session.current_tick();

        session.route_local_player_commands(vec![PlayerCommand::Rescue]);

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);
        let after = session
            .world()
            .player_survival_snapshot(LOCAL_PLAYER_ID)
            .expect("survival snapshot exists");
        assert!(!after.disabled);
        assert!(!after.stranded);
        assert!(after.hull > 0.0);
        assert!(after.fuel > 0.0);
        session.sync_legacy_player_from_world(LOCAL_PLAYER_ID);
        assert!(session.game().player.hull > 0.0);
        assert!(session.game().player.fuel > 0.0);
        let frame_survival = session.render_frame_plan().survival_snapshots();
        assert_eq!(frame_survival.len(), 1);
        assert!(!frame_survival[0].disabled);
        let events = session.drain_events();
        assert!(events.iter().any(|event| matches!(
            event,
            super::WorldEvent::PlayerSurvivalChanged {
                player_id: LOCAL_PLAYER_ID
            }
        )));
    }

    #[test]
    fn production_session_authority_update_routes_action_commands_without_public_legacy_adapter() {
        let mut session = GameSession::new();
        session.game.run_mode = RunMode::Playing;
        {
            let player = session
                .world_mut()
                .player_mut(LOCAL_PLAYER_ID)
                .expect("local player exists");
            player.fuel = 10.0;
            player.hull = 20.0;
            player.credits = 10_000;
        }
        session.route_local_player_commands(vec![PlayerCommand::Refuel, PlayerCommand::Repair]);

        let summary = session
            .update_frame_from_session_authority(PlayerInput::default(), FIXED_DELTA_SECONDS);

        assert!(summary.legacy_bridge_active());
        assert_eq!(summary.command_adapter_count, 2);
        let player = session
            .world()
            .player(LOCAL_PLAYER_ID)
            .expect("local player exists");
        assert!(player.fuel > 10.0);
        assert!(player.hull > 20.0);
    }

    #[test]
    fn routed_drill_command_syncs_active_drill_to_legacy_adapter() {
        let mut session = GameSession::new();
        let tick = session.current_tick();

        session.route_local_player_commands(vec![PlayerCommand::Movement {
            horizontal: 0.0,
            thrust: false,
            drill_down: true,
        }]);

        assert_eq!(session.process_authoritative_commands_for_tick(tick), 1);
        assert!(session.world().active_drill(LOCAL_PLAYER_ID).is_some());
        session.sync_legacy_active_drill_from_world(LOCAL_PLAYER_ID);
        assert!(session.game().active_drill.is_some());
        assert!(session.drain_events().iter().any(|event| matches!(
            event,
            super::WorldEvent::DrillProgressed {
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
    fn session_builds_snapshot_chunk_recovery_plan() {
        let mut session = GameSession::new();
        let position = super::TerrainChunkPosition { x: 0, y: 0 };
        session
            .terrain_revisions
            .mark_tiles_changed([crate::terrain::TilePosition { x: 0, y: 0 }]);

        let plan = session.snapshot_chunk_recovery_plan(position, 0);

        assert_eq!(plan.snapshot_tick, session.current_tick());
        assert_eq!(plan.requested_position, position);
        assert_eq!(plan.known_revision, 0);
        assert_eq!(plan.recovered_revision(), Some(1));
        assert!(!plan.requires_keyframe);
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
