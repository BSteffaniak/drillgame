# Drillgame

A Rust-native 2D mining game mechanically inspired by classic drill/mining games, with original names, procedural programmer art, and a desktop-first initial target.

## Development

```bash
cargo fmt
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo run --bin drillgame
```

With Nix:

```bash
nix develop -c cargo run --bin drillgame
```

## Current Gameplay

Drill below the surface, gather minerals and artifacts, manage fuel/hull/cargo/debt, complete HQ contracts, buy upgrades, use scanners and bombs, and push deeper toward the Star Core. Deeper layers add harder rock, richer ore, artifacts, gas, lava, magma vents, pressure pockets, explosive pockets, falling boulders, cave-ins, and stronger heat pressure.

Successful return trips award streak bonuses. Emergency rescue returns the rig to the surface for a fee, drops part of the cargo at the rescue site, and records a depot invoice; lost cargo can be recovered later if there is cargo space.

## Controls

- `A` / `D` or left/right arrows: move horizontally and drill sideways while grounded
- `W` / up arrow / space: thrust upward
- `S` / down arrow: drill downward
- `C`: trigger an active scanner pulse when a scanner is installed
- `B`: place a purchased timed bomb underground
- `E` / `Enter`: interact with buildings/interior counters, confirm menus, or start game
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

Press `E` at a surface building to walk inside its room. Move with `A`/`D`, use the service counter with `E`, and leave through the door or `Backspace`/`Esc`.

- Fuel Station: refills fuel in partial or full service increments
- Repair Garage: repairs hull in partial or full service increments
- Ore Depot: sells mineral/artifact cargo and shows receipt history
- HQ: completes contracts, provides named story/radio briefings, and offers/collects debt advances
- Upgrade Shop: sells drill, fuel tank, cargo bay, engine, hull, radiator, scanner, and bomb-pack upgrades

## Multiplayer Status

Multiplayer support is under active rearchitecture. Current code has an in-memory faithful transport adapter, host/client runtime scaffolding, player-scoped authoritative command slices, split-screen render planning, prediction/reconciliation coverage, deterministic replay coverage, and save/session metadata tests.

This is not yet a production online release. Known limitations:

- local split-screen still needs manual live-window QA and screenshots/video notes
- second-player input binding metadata exists, but full multi-controller/keyboard polling needs productization
- production online transport is planned as a QUIC-style backend, but no socket dependency is integrated yet
- NAT traversal, matchmaking/server browser, platform invites, and host migration are explicitly deferred
- legacy `GameState` still participates in live gameplay as compatibility glue while authoritative systems are extracted
- prediction/reconciliation polish is covered by tests and plans, but still needs live high-ping playtesting

## Saves and Settings

- Quick save: `drillgame-save.json`
- Save slots: `drillgame-save-slot-1.json` through `drillgame-save-slot-3.json`
- Slot UI displays depth, credits, cargo, completed contracts, playtime, timestamp, and victory state.
- Settings are stored in `drillgame-settings.json` and persist volume/fullscreen preferences.
