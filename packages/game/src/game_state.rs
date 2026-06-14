#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "world coordinates intentionally cross integer tile and floating render spaces"
)]

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{
    contract::ContractLog,
    economy::{
        DeepClaimStatus, PurchaseError, SurfaceZone, TownBuilding, TownDevelopment, buy_upgrade,
        refuel_amount, repair_amount, sell_cargo, upgrade_offers, upgrade_tier_name,
    },
    input::PlayerInput,
    player::Player,
    save::{load_game, load_game_slot, save_exists, save_game, save_game_slot, save_slot_count},
    surface::surface_building_at_tile,
    terrain::{
        ArtifactKind, MineResult, MineralKind, StrategicResourceKind, Terrain, TileKind,
        TilePosition,
    },
};

pub const TILE_SIZE: f32 = 32.0;
const WORLD_WIDTH: i32 = 240;
const WORLD_HEIGHT: i32 = 90;
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RunMode {
    Title,
    Playing,
    Interior,
    Paused,
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
    Map,
    Help,
    TownDevelopment,
    ExpeditionBoard,
    ResearchLog,
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
}

impl RecipeKind {
    pub const ALL: [Self; 8] = [
        Self::ReinforcedBulkhead,
        Self::AuxiliaryTank,
        Self::ExpandedSorter,
        Self::SignalRelayKit,
        Self::SurveyDroneKit,
        Self::CargoLiftKit,
        Self::TunnelSupportKit,
        Self::PumpStationKit,
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
        }
    }
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExpeditionObjectiveKind {
    #[default]
    ReachDepth,
    DeliverCargo,
    ScanHazards,
    BuildPumpStations,
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
        }
    }
    #[must_use]
    pub const fn risk_label(self) -> &'static str {
        match self.kind {
            ExpeditionObjectiveKind::ReachDepth if self.required >= 120 => "extreme",
            ExpeditionObjectiveKind::ReachDepth if self.required >= 90 => "high",
            ExpeditionObjectiveKind::DeliverCargo if self.required >= 3 => "medium",
            ExpeditionObjectiveKind::ScanHazards | ExpeditionObjectiveKind::BuildPumpStations => {
                "medium"
            }
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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
    pub rescue_count: u32,
    #[serde(default)]
    pub artifacts_found: u32,
    #[serde(default)]
    pub trip_best_depth: i32,
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
    #[serde(skip)]
    visual_changes: VisualChanges,
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
    pub fn new() -> Self {
        Self {
            terrain: Terrain::new_seeded(WORLD_WIDTH, WORLD_HEIGHT, WORLD_SEED),
            player: Player::new(PLAYER_SPAWN_X, PLAYER_SPAWN_Y),
            save_version: current_save_version(),
            message: "Mine ore, sell cargo, and buy upgrades. Press E at surface buildings."
                .to_owned(),
            current_zone: None,
            interior_zone: None,
            interior_x: 88.0,
            interior_facing: 1.0,
            contracts: ContractLog::new(),
            run_mode: RunMode::Title,
            modal: None,
            selected_menu_item: 0,
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
            rescue_count: 0,
            artifacts_found: 0,
            trip_best_depth: 0,
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
            visual_changes: VisualChanges {
                full_terrain_refresh: true,
                changed_tiles: Vec::new(),
            },
        }
    }

    pub fn take_visual_changes(&mut self) -> VisualChanges {
        std::mem::take(&mut self.visual_changes)
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
        let expected_tiles = (self.terrain.width() * self.terrain.height()) as usize;
        if self.explored_tiles.len() != expected_tiles {
            self.explored_tiles = vec![false; expected_tiles];
        }
        self.request_exit = false;
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
        self.camera_shake_seconds = (self.camera_shake_seconds - delta_seconds).max(0.0);
        self.screen_flash_seconds = (self.screen_flash_seconds - delta_seconds).max(0.0);
        self.update_hazards(delta_seconds);
        self.recover_lost_cargo_if_near();
        self.reveal_near_player();
        self.reveal_scanner_area();
        self.update_survey_drones();
        self.drill_flash_seconds = (self.drill_flash_seconds - delta_seconds).max(0.0);
        if matches!(self.run_mode, RunMode::Playing | RunMode::Interior)
            && !self.game_over
            && !self.won_game
        {
            self.play_seconds += delta_seconds;
        }

        match self.run_mode {
            RunMode::Title => {
                if input.confirm {
                    self.run_mode = RunMode::Playing;
                    self.sound_cues.push(SoundCue::Milestone);
                    "Welcome to the dig site. Visit the depot for contracts."
                        .clone_into(&mut self.message);
                }
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
        self.update_escape_sequence(delta_seconds);
        self.update_layer_band();
        self.award_return_bonus();
        self.update_warning_messages();
        self.update_status_messages();
        self.check_failure();
        self.update_camera(delta_seconds);
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
                        if scanner_can_mark(tile.kind, self.player.scanner_level)
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
    pub fn mineral_market_value(&self, mineral: MineralKind) -> u32 {
        let factor =
            self.mineral_market_factor(mineral) + u32::from(self.town_development.depot_level) * 3;
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
        if self.modal == Some(ModalScreen::ExitConfirm) {
            if input.cancel {
                self.modal = None;
                return;
            }
            if input.confirm {
                self.request_exit = true;
            }
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
            PauseOption::ExitToDesktop => self.modal = Some(ModalScreen::ExitConfirm),
        }
    }

    fn handle_save_load(&mut self, input: PlayerInput) {
        if input.save {
            match save_game(self) {
                Ok(()) => "Game saved to drillgame-save.json.".clone_into(&mut self.message),
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

        if self.try_use_cargo_lift() {
            return;
        }

        if let Some(zone) = self.current_zone {
            self.enter_interior(zone);
        } else {
            "No surface service here.".clone_into(&mut self.message);
        }
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
            self.modal = None;
            return true;
        }

        if input.menu_up {
            self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
        }
        if input.menu_down {
            let max_item = match modal {
                ModalScreen::Depot
                | ModalScreen::Fuel
                | ModalScreen::Repair
                | ModalScreen::Options
                | ModalScreen::Bank
                | ModalScreen::Explosives
                | ModalScreen::Salvage => 2,
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
                | ModalScreen::Map
                | ModalScreen::Help
                | ModalScreen::ResearchLog => {}
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
        match save_game_slot(self, slot) {
            Ok(()) => self.message = format!("Saved to slot {}.", slot + 1),
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
            if self.town_event_day.is_multiple_of(5) {
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
            if self.town_event_day % 5 == 2 {
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
            consume_expedition_delivery(expedition, &mut self.player);
        }
        if reward > 0 {
            self.player.credits = self.player.credits.saturating_add(reward);
            self.total_earnings = self.total_earnings.saturating_add(reward);
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
            _ => self.salvage_sell_scrap_tip(),
        }
    }

    fn buy_insurance(&mut self) {
        if self.player.insured {
            "Already insured for the next rescue.".clone_into(&mut self.message);
            return;
        }
        let next_tier = self.player.insurance_tier.saturating_add(1).min(3);
        let cost = 70 + u32::from(next_tier) * 55;
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
            self.player.credits += 250;
            self.player.loan_debt = 300;
            "HQ finance issued a 250 credit advance. Repay 300 credits before the board gets loud."
                .clone_into(&mut self.message);
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
            _ => self.modal = Some(ModalScreen::DepotReceiptHistory),
        }
    }

    fn confirm_complete_contract(&mut self) {
        if let Some(completion) = self.contracts.try_complete(&mut self.player) {
            self.sound_cues.push(SoundCue::Sell);
            self.total_earnings += completion.reward;
            if completion.finished_story {
                self.won_game = true;
                self.deep_claim_status = DeepClaimStatus::Unlocked;
                self.message = format!(
                    "{} complete! Star Core secured. Deep Claim charter unlocked. Bonus: {} credits.",
                    completion.completed_title, completion.reward
                );
            } else {
                let story = ContractLog::story_for_completed(self.contracts.completed);
                self.message = format!(
                    "{} complete! Bonus paid: {} credits. {story}",
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
        let adjusted = self
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
                "MARKET mineral pricing applied"
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
            self.message = format!("Sold cargo for {adjusted} credits at current mineral markets.");
        }
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

    fn handle_scanner(&mut self, input: PlayerInput) {
        if !input.scan {
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
        self.placed_bombs.push(PlacedBomb {
            x: self.player.x,
            y: self.player.y + TILE_SIZE * 0.4,
            timer_seconds: 2.4,
        });
        self.sound_cues.push(SoundCue::Ui);
        self.message = format!(
            "Bomb armed: 2.4 seconds. {} bombs left. Clear out!",
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
    }

    fn place_infrastructure_kit(&mut self, kind: InfrastructureKind, empty_message: &str) {
        let kits = match kind {
            InfrastructureKind::SignalRelay => self.player.signal_relay_kits,
            InfrastructureKind::SurveyDrone => self.player.survey_drone_kits,
            InfrastructureKind::CargoLift => self.player.cargo_lift_kits,
            InfrastructureKind::TunnelSupport => self.player.tunnel_support_kits,
            InfrastructureKind::PumpStation => self.player.pump_station_kits,
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
        }
        self.infrastructure.push(PlacedInfrastructure {
            kind,
            position,
            durability: default_infrastructure_durability(),
        });
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
        };
        self.sound_cues.push(SoundCue::Upgrade);
    }

    fn apply_movement(&mut self, input: PlayerInput, delta_seconds: f32) {
        let can_burn_fuel = self.player.fuel > 0.0;
        let grounded = self.is_grounded();
        let cargo_ratio =
            self.player.cargo_used() as f32 / self.player.cargo_capacity.max(1) as f32;
        let cargo_penalty = 1.0 - cargo_ratio.min(1.0) * 0.18;
        let engine_multiplier =
            (1.0 + f32::from(self.player.engine_level.saturating_sub(1)) * 0.28) * cargo_penalty;
        let horizontal_acceleration = if grounded {
            HORIZONTAL_ACCELERATION * 1.35
        } else {
            HORIZONTAL_ACCELERATION * 0.65
        };

        self.player.velocity_x +=
            input.horizontal * horizontal_acceleration * engine_multiplier * delta_seconds;

        if input.thrust && can_burn_fuel {
            self.player.velocity_y -= THRUST_ACCELERATION * engine_multiplier * delta_seconds;
            let efficiency = 1.0 - f32::from(self.player.fuel_tank_level.saturating_sub(1)) * 0.06;
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

        let damage = (self.player.velocity_y - SAFE_LANDING_SPEED) * CRASH_DAMAGE_SCALE;
        self.player.hull = (self.player.hull - damage).max(0.0);
        self.sound_cues.push(SoundCue::Damage);
        self.shake_camera(0.28, 7.0);
        self.spawn_sparks();
        self.message = format!("Hard landing! Hull took {damage:.0} damage.");
    }

    fn apply_bump_damage(&mut self, speed: f32) {
        let damage = (speed - SAFE_LANDING_SPEED * 0.75) * CRASH_DAMAGE_SCALE * 0.5;
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
        if self
            .terrain
            .hardness_at(target)
            .is_some_and(|hardness| hardness > self.player.drill_strength)
        {
            self.active_drill = None;
            self.sound_cues.push(SoundCue::Damage);
            self.shake_camera(0.16, 4.0);
            self.spawn_sparks();
            "That layer is too hard. Upgrade your drill.".clone_into(&mut self.message);
            return;
        }

        let seconds_per_chip =
            drill_seconds_per_chip(tile.kind, self.player.drill_strength, direction);
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
            if self.player.add_cargo(mineral) {
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
            if self.player.add_artifact(artifact) {
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
            self.detonate_bomb(center, 2);
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
        let damage = HEAT_DAMAGE_PER_SECOND * depth_factor * delta_seconds;
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
        let damage = if self.is_pump_protected(player_tile) {
            2.5 * delta_seconds
        } else {
            9.0 * delta_seconds
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

    fn award_return_bonus(&mut self) {
        if self.current_zone.is_none() || self.trip_best_depth < 15 {
            return;
        }
        self.return_streak += 1;
        if self.player.loan_debt > 0 {
            self.player.loan_debt = self.player.loan_debt.saturating_add(12);
        }
        let depth = u32::try_from(self.trip_best_depth).unwrap_or(0);
        let reward = (depth / 4).saturating_mul(self.return_streak.min(5));
        self.player.credits += reward;
        self.total_earnings += reward;
        self.message = format!(
            "Successful return from {}m. Trip streak x{} bonus: {reward} credits.",
            self.trip_best_depth, self.return_streak
        );
        self.trip_best_depth = 0;
        self.advance_town_event();
    }

    fn advance_town_event(&mut self) {
        self.town_event_day = self.town_event_day.saturating_add(1);
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
        let mut lost_items = if self.player.insured && self.player.insurance_tier >= 2 {
            0
        } else if self.player.insured {
            drop_quarter_cargo(&mut self.player)
        } else {
            drop_half_cargo(&mut self.player)
        };
        if self.player.y >= 70.0 * TILE_SIZE && relay_count == 0 {
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
        return "Director Vale: The Star Core is secure. The mine is yours to master.".to_owned();
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
#[cfg(test)]
mod tests {
    use super::*;

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
}
