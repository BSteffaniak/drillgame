#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "service pricing intentionally converts small fuel/hull unit counts"
)]

use serde::{Deserialize, Serialize};

use crate::player::Player;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum SurfaceZone {
    Fuel,
    Repair,
    Depot,
    Headquarters,
    Shop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeKind {
    Drill,
    FuelTank,
    CargoBay,
    Engine,
    Hull,
    Radiator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpgradeOffer {
    pub kind: UpgradeKind,
    pub name: &'static str,
    pub level: u8,
    pub cost: u32,
    pub description: &'static str,
}

pub const MAX_UPGRADE_LEVEL: u8 = 5;
pub const REFUEL_UNIT_COST: u32 = 1;
pub const REPAIR_UNIT_COST: u32 = 2;

#[must_use]
pub fn upgrade_offers(player: &Player) -> [UpgradeOffer; 6] {
    [
        offer(UpgradeKind::Drill, player.drill_strength),
        offer(UpgradeKind::FuelTank, player.fuel_tank_level),
        offer(UpgradeKind::CargoBay, player.cargo_bay_level),
        offer(UpgradeKind::Engine, player.engine_level),
        offer(UpgradeKind::Hull, player.hull_level),
        offer(UpgradeKind::Radiator, player.radiator_level),
    ]
}

pub fn refuel_amount(player: &mut Player, fraction: f32) -> u32 {
    let missing = (player.fuel_capacity - player.fuel).ceil().max(0.0);
    let target = (missing * fraction).ceil().max(1.0).min(missing);
    let cost = affordable_service_cost(target, REFUEL_UNIT_COST, player.credits);
    let fuel_added = cost / REFUEL_UNIT_COST;
    player.credits -= cost;
    player.fuel = (player.fuel + fuel_added as f32).min(player.fuel_capacity);
    cost
}

#[cfg(test)]
pub fn refuel(player: &mut Player) -> u32 {
    refuel_amount(player, 1.0)
}

pub fn repair_amount(player: &mut Player, fraction: f32) -> u32 {
    let missing = (player.max_hull() - player.hull).ceil().max(0.0);
    let target = (missing * fraction).ceil().max(1.0).min(missing);
    let cost = affordable_service_cost(target, REPAIR_UNIT_COST, player.credits);
    let hull_added = cost / REPAIR_UNIT_COST;
    player.credits -= cost;
    player.hull = (player.hull + hull_added as f32).min(player.max_hull());
    cost
}

#[cfg(test)]
#[allow(
    clippy::missing_const_for_fn,
    reason = "uses player max hull calculation"
)]
pub fn repair(player: &mut Player) -> u32 {
    repair_amount(player, 1.0)
}

pub fn sell_cargo(player: &mut Player) -> u32 {
    let mineral_payout: u32 = player
        .cargo
        .iter()
        .map(|(mineral, count)| mineral.value() * count)
        .sum();
    let artifact_payout: u32 = player
        .artifacts
        .iter()
        .map(|(artifact, count)| artifact.value() * count)
        .sum();
    let payout = mineral_payout + artifact_payout;
    player.credits += payout;
    player.cargo.clear();
    player.artifacts.clear();
    payout
}

pub fn buy_upgrade(player: &mut Player, index: usize) -> Result<UpgradeOffer, PurchaseError> {
    let offers = upgrade_offers(player);
    let Some(offer) = offers.get(index).copied() else {
        return Err(PurchaseError::InvalidSelection);
    };

    if offer.level >= MAX_UPGRADE_LEVEL {
        return Err(PurchaseError::MaxLevel);
    }

    if player.credits < offer.cost {
        return Err(PurchaseError::NotEnoughCredits);
    }

    player.credits -= offer.cost;
    apply_upgrade(player, offer.kind);
    Ok(offer)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PurchaseError {
    InvalidSelection,
    MaxLevel,
    NotEnoughCredits,
}

fn apply_upgrade(player: &mut Player, kind: UpgradeKind) {
    match kind {
        UpgradeKind::Drill => player.drill_strength += 1,
        UpgradeKind::FuelTank => {
            player.fuel_tank_level += 1;
            player.fuel_capacity += 50.0;
            player.fuel = player.fuel_capacity;
        }
        UpgradeKind::CargoBay => {
            player.cargo_bay_level += 1;
            player.cargo_capacity += 8;
        }
        UpgradeKind::Engine => player.engine_level += 1,
        UpgradeKind::Hull => {
            player.hull_level += 1;
            player.hull = player.max_hull();
        }
        UpgradeKind::Radiator => player.radiator_level += 1,
    }
}

fn offer(kind: UpgradeKind, current_level: u8) -> UpgradeOffer {
    let next_level = current_level.saturating_add(1).min(MAX_UPGRADE_LEVEL);
    UpgradeOffer {
        kind,
        name: upgrade_name(kind),
        level: current_level,
        cost: upgrade_cost(kind, next_level),
        description: upgrade_description(kind),
    }
}

const fn upgrade_name(kind: UpgradeKind) -> &'static str {
    match kind {
        UpgradeKind::Drill => "Drill Bit",
        UpgradeKind::FuelTank => "Fuel Tank",
        UpgradeKind::CargoBay => "Cargo Bay",
        UpgradeKind::Engine => "Engine",
        UpgradeKind::Hull => "Hull Plating",
        UpgradeKind::Radiator => "Radiator",
    }
}

const fn upgrade_description(kind: UpgradeKind) -> &'static str {
    match kind {
        UpgradeKind::Drill => "Mine harder and faster",
        UpgradeKind::FuelTank => "Carry more fuel",
        UpgradeKind::CargoBay => "Carry more ore",
        UpgradeKind::Engine => "Stronger lift and handling",
        UpgradeKind::Hull => "Survive harder impacts",
        UpgradeKind::Radiator => "Resist deep heat pressure",
    }
}

pub fn upgrade_tier_name(kind: UpgradeKind, level: u8) -> &'static str {
    let next_level = level.saturating_add(1).min(MAX_UPGRADE_LEVEL);
    match (kind, next_level) {
        (UpgradeKind::Drill, 1) => "Steel Drill",
        (UpgradeKind::Drill, 2) => "Carbide Drill",
        (UpgradeKind::Drill, 3) => "Diamond Drill",
        (UpgradeKind::Drill, 4) => "Plasma Drill",
        (UpgradeKind::Drill, _) => "Star Drill",
        (UpgradeKind::FuelTank, 1) => "Aux Fuel Tank",
        (UpgradeKind::FuelTank, 2) => "Twin Fuel Tank",
        (UpgradeKind::FuelTank, 3) => "Pressure Fuel Tank",
        (UpgradeKind::FuelTank, 4) => "Deep Fuel Tank",
        (UpgradeKind::FuelTank, _) => "Endurance Fuel Tank",
        (UpgradeKind::CargoBay, 1) => "Ore Basket",
        (UpgradeKind::CargoBay, 2) => "Cargo Bay",
        (UpgradeKind::CargoBay, 3) => "Expanded Cargo Bay",
        (UpgradeKind::CargoBay, 4) => "Industrial Cargo Bay",
        (UpgradeKind::CargoBay, _) => "Vault Cargo Bay",
        (UpgradeKind::Engine, 1) => "Lift Engine",
        (UpgradeKind::Engine, 2) => "Tuned Engine",
        (UpgradeKind::Engine, 3) => "Turbo Engine",
        (UpgradeKind::Engine, 4) => "Vector Engine",
        (UpgradeKind::Engine, _) => "Fusion Engine",
        (UpgradeKind::Hull, 1) => "Riveted Hull",
        (UpgradeKind::Hull, 2) => "Reinforced Hull",
        (UpgradeKind::Hull, 3) => "Titanium Hull",
        (UpgradeKind::Hull, 4) => "Ablative Hull",
        (UpgradeKind::Hull, _) => "Star Hull",
        (UpgradeKind::Radiator, 1) => "Basic Radiator",
        (UpgradeKind::Radiator, 2) => "Copper Radiator",
        (UpgradeKind::Radiator, 3) => "Liquid Radiator",
        (UpgradeKind::Radiator, 4) => "Cryo Radiator",
        (UpgradeKind::Radiator, _) => "Magma Radiator",
    }
}

pub const fn upgrade_effect(kind: UpgradeKind) -> &'static str {
    match kind {
        UpgradeKind::Drill => "+1 drill tier; unlocks harder strata",
        UpgradeKind::FuelTank => "+50 fuel and full refill",
        UpgradeKind::CargoBay => "+8 cargo slots",
        UpgradeKind::Engine => "+18% thrust/handling",
        UpgradeKind::Hull => "+40 max hull and full repair",
        UpgradeKind::Radiator => "reduces deep heat damage",
    }
}

fn upgrade_cost(kind: UpgradeKind, next_level: u8) -> u32 {
    let base = match kind {
        UpgradeKind::Drill => 120,
        UpgradeKind::FuelTank => 95,
        UpgradeKind::CargoBay => 110,
        UpgradeKind::Engine => 115,
        UpgradeKind::Hull => 105,
        UpgradeKind::Radiator => 130,
    };
    base * u32::from(next_level) * u32::from(next_level)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "service amount is bounded by small fuel/hull capacities"
)]
fn affordable_service_cost(missing: f32, unit_cost: u32, credits: u32) -> u32 {
    let requested = missing as u32 * unit_cost;
    requested.min(credits)
}

#[cfg(test)]
mod tests {
    use crate::{
        economy::{refuel, repair, sell_cargo},
        player::Player,
        terrain::{ArtifactKind, MineralKind},
    };

    #[test]
    fn depot_sells_minerals_and_artifacts() {
        let mut player = Player::new(0.0, 0.0);
        assert!(player.add_cargo(MineralKind::Gold));
        assert!(player.add_artifact(ArtifactKind::Fossil));

        let payout = sell_cargo(&mut player);

        assert_eq!(
            payout,
            MineralKind::Gold.value() + ArtifactKind::Fossil.value()
        );
        assert_eq!(player.credits, payout);
        assert_eq!(player.cargo_used(), 0);
    }

    #[test]
    fn service_costs_are_limited_by_available_credits() {
        let mut player = Player::new(0.0, 0.0);
        player.credits = 10;
        player.fuel = 0.0;
        player.hull = 0.0;

        assert_eq!(refuel(&mut player), 10);
        assert_eq!(player.credits, 0);
        assert_eq!(repair(&mut player), 0);
    }
}
