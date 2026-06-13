# Drillgame

A Rust-native 2D mining game mechanically inspired by classic drill/mining games, with original names, placeholder programmer art, and a desktop-first initial target.

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

## Controls

- `A` / `D` or left/right arrows: move horizontally and drill sideways
- `W` / up arrow / space: thrust upward
- `S` / down arrow: drill downward
- side movement drills sideways only while grounded
- `E` / `Enter`: interact with surface buildings, confirm menus, or start game
- `Backspace` / `Esc`: close menus
- `P`: pause/resume
- hold `Tab`: show detailed cargo/contract panel
- `1`-`6`: quick-select upgrades while the shop menu is open
- `F5`: save to `drillgame-save.json`
- `F9`: load from `drillgame-save.json`
- `Esc`: quit

## Surface Buildings

- Fuel Station: refills fuel
- Repair Garage: repairs hull
- Ore Depot: sells cargo by mineral value
- Upgrade Shop: sells drill, fuel tank, cargo bay, engine, hull, and radiator upgrades
