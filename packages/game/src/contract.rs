use serde::{Deserialize, Serialize};

use crate::{
    player::Player,
    terrain::{ArtifactKind, MineralKind},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContractLog {
    pub active: Contract,
    pub completed: u32,
}

impl ContractLog {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: Contract::new(ContractTarget::Mineral(MineralKind::Copper), 4, 90),
            completed: 0,
        }
    }

    pub fn try_complete(&mut self, player: &mut Player) -> Option<u32> {
        if !self.active.is_satisfied(player) {
            return None;
        }

        self.active.consume(player);
        let reward = self.active.reward;
        player.credits += reward;
        self.completed += 1;
        self.active = contract_for_index(self.completed);
        Some(reward)
    }
}

impl Default for ContractLog {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Contract {
    pub target: ContractTarget,
    pub required: u32,
    pub reward: u32,
}

impl Contract {
    #[must_use]
    pub const fn new(target: ContractTarget, required: u32, reward: u32) -> Self {
        Self {
            target,
            required,
            reward,
        }
    }

    #[must_use]
    pub fn progress(&self, player: &Player) -> u32 {
        match self.target {
            ContractTarget::Mineral(mineral) => player.cargo.get(&mineral).copied().unwrap_or(0),
            ContractTarget::Artifact(artifact) => {
                player.artifacts.get(&artifact).copied().unwrap_or(0)
            }
        }
    }

    #[must_use]
    pub fn is_satisfied(&self, player: &Player) -> bool {
        self.progress(player) >= self.required
    }

    fn consume(&self, player: &mut Player) {
        match self.target {
            ContractTarget::Mineral(mineral) => {
                consume_count(&mut player.cargo, &mineral, self.required);
            }
            ContractTarget::Artifact(artifact) => {
                consume_count(&mut player.artifacts, &artifact, self.required);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum ContractTarget {
    Mineral(MineralKind),
    Artifact(ArtifactKind),
}

impl ContractTarget {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mineral(mineral) => mineral.name(),
            Self::Artifact(artifact) => artifact.name(),
        }
    }
}

const fn contract_for_index(index: u32) -> Contract {
    match index % 6 {
        0 => Contract::new(ContractTarget::Mineral(MineralKind::Copper), 4, 90),
        1 => Contract::new(ContractTarget::Mineral(MineralKind::Silver), 3, 180),
        2 => Contract::new(ContractTarget::Artifact(ArtifactKind::Fossil), 1, 280),
        3 => Contract::new(ContractTarget::Mineral(MineralKind::Gold), 4, 360),
        4 => Contract::new(ContractTarget::Mineral(MineralKind::Ruby), 2, 520),
        _ => Contract::new(ContractTarget::Artifact(ArtifactKind::BuriedIdol), 1, 700),
    }
}

fn consume_count<K: Ord>(items: &mut std::collections::BTreeMap<K, u32>, key: &K, count: u32) {
    let Some(available) = items.get_mut(key) else {
        return;
    };

    *available = available.saturating_sub(count);
    if *available == 0 {
        items.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        contract::{ContractLog, ContractTarget},
        player::Player,
        terrain::MineralKind,
    };

    #[test]
    fn contract_completion_consumes_items_and_pays_reward() {
        let mut player = Player::new(0.0, 0.0);
        for _ in 0..4 {
            assert!(player.add_cargo(MineralKind::Copper));
        }

        let mut contracts = ContractLog::new();
        let reward = contracts.try_complete(&mut player);

        assert_eq!(reward, Some(90));
        assert_eq!(player.credits, 90);
        assert_eq!(player.cargo_used(), 0);
        assert!(matches!(
            contracts.active.target,
            ContractTarget::Mineral(_)
        ));
    }
}
