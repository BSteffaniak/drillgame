# Drillgame

A Rust-native 2D mining game mechanically inspired by classic drill/mining games, with original names, procedural programmer art, and a desktop-first initial target.

## Development

```bash
cargo fmt
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --bin drillgame
```

With Nix:

```bash
nix develop -c cargo run --bin drillgame
```

## Current Gameplay

Drill below the surface, gather minerals and artifacts, manage fuel/hull/cargo, complete HQ contracts, buy upgrades, and push deeper toward the Star Core. Deeper layers add harder rock, richer ore, artifacts, gas, lava, magma vents, pressure pockets, explosive pockets, falling boulders, cave-ins, and stronger heat pressure.

Successful return trips award streak bonuses. Emergency rescue returns the rig to the surface for a fee, drops part of the cargo at the rescue site, and records a depot invoice; lost cargo can be recovered later if there is cargo space.

## Controls

- `A` / `D` or left/right arrows: move horizontally and drill sideways while grounded
- `W` / up arrow / space: thrust upward
- `S` / down arrow: drill downward
- `E` / `Enter`: interact with buildings, confirm menus, or start game
- `Backspace` / `Esc`: close menus; `P` pauses/resumes
- `M`: large mine map with ore, hazard, rescue, and lost-cargo markers
- `H`: help screen
- hold `Tab`: detailed cargo/contract/status overlay
- `1`-`6`: quick-select upgrades while the shop is open
- `F5` / `F9`: quick save/load using `drillgame-save.json`
- Pause menu: save/load slots, options, exit confirmation
- `F11`: fullscreen toggle
- `+` / `-`: volume hotkeys

## Surface Buildings

- Fuel Station: refills fuel in partial or full service increments
- Repair Garage: repairs hull in partial or full service increments
- Ore Depot: sells mineral/artifact cargo and shows receipt history
- HQ: completes contracts and provides named story/radio briefings
- Upgrade Shop: sells drill, fuel tank, cargo bay, engine, hull, and radiator upgrades

## Saves and Settings

- Quick save: `drillgame-save.json`
- Save slots: `drillgame-save-slot-1.json` through `drillgame-save-slot-3.json`
- Slot UI displays depth, credits, cargo, completed contracts, playtime, timestamp, and victory state.
- Settings are stored in `drillgame-settings.json` and persist volume/fullscreen preferences.
