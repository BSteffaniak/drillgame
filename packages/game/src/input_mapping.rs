use crate::{
    input::PlayerInput,
    multiplayer::{ClientAction, PlayerCommand},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedInput {
    pub client_actions: Vec<ClientAction>,
    pub player_commands: Vec<PlayerCommand>,
}

#[must_use]
pub fn map_local_input(input: PlayerInput) -> MappedInput {
    MappedInput {
        client_actions: map_client_actions(input),
        player_commands: map_player_commands(input),
    }
}

fn map_player_commands(input: PlayerInput) -> Vec<PlayerCommand> {
    let mut commands = vec![PlayerCommand::Movement {
        horizontal: input.horizontal,
        thrust: input.thrust,
        drill_down: input.drill_down,
    }];

    push_if(&mut commands, input.interact, PlayerCommand::Interact);
    push_if(&mut commands, input.scan, PlayerCommand::UseScanner);
    push_if(&mut commands, input.bomb, PlayerCommand::PlaceBomb);

    for (pressed, slot) in infrastructure_slots(input) {
        if pressed {
            commands.push(PlayerCommand::PlaceInfrastructure { slot });
        }
    }

    if let Some(index) = input.selected_upgrade {
        commands.push(PlayerCommand::SelectUpgrade { index });
    }

    commands
}

fn map_client_actions(input: PlayerInput) -> Vec<ClientAction> {
    let mut actions = Vec::new();

    push_if(&mut actions, input.confirm, ClientAction::Confirm);
    push_if(&mut actions, input.cancel, ClientAction::Cancel);
    push_if(&mut actions, input.pause, ClientAction::Pause);
    push_if(&mut actions, input.menu_up, ClientAction::MenuUp);
    push_if(&mut actions, input.menu_down, ClientAction::MenuDown);
    push_if(&mut actions, input.menu_left, ClientAction::MenuLeft);
    push_if(&mut actions, input.menu_right, ClientAction::MenuRight);
    push_if(&mut actions, input.details, ClientAction::ToggleDetails);
    push_if(&mut actions, input.save, ClientAction::Save);
    push_if(&mut actions, input.load, ClientAction::Load);
    push_if(&mut actions, input.map, ClientAction::ToggleMap);
    push_if(&mut actions, input.help, ClientAction::ToggleHelp);
    push_if(&mut actions, input.volume_up, ClientAction::VolumeUp);
    push_if(&mut actions, input.volume_down, ClientAction::VolumeDown);
    push_if(
        &mut actions,
        input.fullscreen,
        ClientAction::ToggleFullscreen,
    );
    push_if(
        &mut actions,
        input.exit_requested,
        ClientAction::ExitRequested,
    );

    actions
}

fn push_if<T>(items: &mut Vec<T>, condition: bool, item: T) {
    if condition {
        items.push(item);
    }
}

const fn infrastructure_slots(input: PlayerInput) -> [(bool, u8); 6] {
    [
        (input.place_relay, 0),
        (input.place_drone, 1),
        (input.place_lift, 2),
        (input.place_support, 3),
        (input.place_pump, 4),
        (input.place_processor, 5),
    ]
}

#[cfg(test)]
mod tests {
    use crate::{input::PlayerInput, multiplayer::ClientAction};

    use super::{PlayerCommand, map_local_input};

    #[test]
    fn maps_movement_every_frame() {
        let input = PlayerInput {
            horizontal: 1.0,
            thrust: true,
            drill_down: true,
            ..PlayerInput::default()
        };

        let mapped = map_local_input(input);

        assert_eq!(
            mapped.player_commands[0],
            PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: true,
                drill_down: true,
            }
        );
    }

    #[test]
    fn separates_client_actions_from_player_commands() {
        let input = PlayerInput {
            pause: true,
            fullscreen: true,
            bomb: true,
            ..PlayerInput::default()
        };

        let mapped = map_local_input(input);

        assert!(mapped.client_actions.contains(&ClientAction::Pause));
        assert!(
            mapped
                .client_actions
                .contains(&ClientAction::ToggleFullscreen)
        );
        assert!(mapped.player_commands.contains(&PlayerCommand::PlaceBomb));
    }
}
