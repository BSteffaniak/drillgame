use crate::player::Player;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceZone {
    Fuel,
    Repair,
    Depot,
    Shop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeKind {
    Drill,
    FuelTank,
    CargoBay,
    Engine,
    Hull,
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

#[must_use]
pub fn upgrade_offers(player: &Player) -> [UpgradeOffer; 5] {
    [
        offer(UpgradeKind::Drill, player.drill_strength),
        offer(UpgradeKind::FuelTank, player.fuel_tank_level),
        offer(UpgradeKind::CargoBay, player.cargo_bay_level),
        offer(UpgradeKind::Engine, player.engine_level),
        offer(UpgradeKind::Hull, player.hull_level),
    ]
}

pub const fn refuel(player: &mut Player) {
    player.fuel = player.fuel_capacity;
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "uses player max hull calculation"
)]
pub fn repair(player: &mut Player) {
    player.hull = player.max_hull();
}

pub fn sell_cargo(player: &mut Player) -> u32 {
    let payout = player
        .cargo
        .iter()
        .map(|(mineral, count)| mineral.value() * count)
        .sum();
    player.credits += payout;
    player.cargo.clear();
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
    }
}

const fn upgrade_description(kind: UpgradeKind) -> &'static str {
    match kind {
        UpgradeKind::Drill => "Mine harder and faster",
        UpgradeKind::FuelTank => "Carry more fuel",
        UpgradeKind::CargoBay => "Carry more ore",
        UpgradeKind::Engine => "Stronger lift and handling",
        UpgradeKind::Hull => "Survive harder impacts",
    }
}

fn upgrade_cost(kind: UpgradeKind, next_level: u8) -> u32 {
    let base = match kind {
        UpgradeKind::Drill => 90,
        UpgradeKind::FuelTank => 70,
        UpgradeKind::CargoBay => 80,
        UpgradeKind::Engine => 85,
        UpgradeKind::Hull => 75,
    };
    base * u32::from(next_level) * u32::from(next_level)
}
