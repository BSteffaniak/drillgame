#![allow(
    dead_code,
    reason = "some command-source helpers are kept for multiplayer cleanup without frame-time scaffolding observer"
)]

use crate::{
    game_state::session_gameplay_commands_from_input,
    input::PlayerInput,
    multiplayer::{ClientAction, ClientId, CommandSource, PlayerCommand},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedInput {
    pub client_actions: Vec<ClientAction>,
    pub player_commands: Vec<PlayerCommand>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandProducer {
    pub source: CommandSource,
    pub commands: Vec<PlayerCommand>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalInputProducer {
    pub client_id: ClientId,
    pub producer: CommandProducer,
}

impl LocalInputProducer {
    #[must_use]
    pub const fn new(client_id: ClientId, producer: CommandProducer) -> Self {
        Self {
            client_id,
            producer,
        }
    }
}

impl CommandProducer {
    #[allow(
        dead_code,
        reason = "kept for command-source cleanup without frame-time scaffolding observer"
    )]
    #[must_use]
    pub const fn new(source: CommandSource, commands: Vec<PlayerCommand>) -> Self {
        Self { source, commands }
    }

    #[must_use]
    pub const fn uses_authoritative_path(&self) -> bool {
        self.source.uses_authoritative_command_path()
    }
}

#[must_use]
pub const fn replay_commands(commands: Vec<PlayerCommand>) -> CommandProducer {
    CommandProducer::new(CommandSource::Replay, commands)
}

#[must_use]
pub const fn ai_commands(commands: Vec<PlayerCommand>) -> CommandProducer {
    CommandProducer::new(CommandSource::Ai, commands)
}

#[must_use]
pub const fn gamepad_commands(commands: Vec<PlayerCommand>) -> CommandProducer {
    CommandProducer::new(CommandSource::Gamepad, commands)
}

#[must_use]
pub const fn split_screen_commands(commands: Vec<PlayerCommand>) -> CommandProducer {
    CommandProducer::new(CommandSource::SplitScreenClient, commands)
}

#[must_use]
pub const fn online_commands(commands: Vec<PlayerCommand>) -> CommandProducer {
    CommandProducer::new(CommandSource::OnlineClient, commands)
}

#[must_use]
pub fn local_keyboard_commands(input: PlayerInput) -> CommandProducer {
    CommandProducer::new(
        CommandSource::Keyboard,
        session_gameplay_commands_from_input(input),
    )
}

#[must_use]
pub fn split_screen_keyboard_commands(input: PlayerInput) -> CommandProducer {
    CommandProducer::new(
        CommandSource::SplitScreenClient,
        session_gameplay_commands_from_input(input),
    )
}

#[must_use]
pub fn local_split_screen_inputs(
    primary_client_id: ClientId,
    primary_input: PlayerInput,
    secondary_client_id: Option<ClientId>,
    secondary_input: Option<PlayerInput>,
) -> Vec<LocalInputProducer> {
    let mut producers = vec![LocalInputProducer::new(
        primary_client_id,
        local_keyboard_commands(primary_input),
    )];
    if let (Some(client_id), Some(input)) = (secondary_client_id, secondary_input) {
        producers.push(LocalInputProducer::new(
            client_id,
            split_screen_keyboard_commands(input),
        ));
    }
    producers
}

#[must_use]
pub fn map_local_input(input: PlayerInput) -> MappedInput {
    MappedInput {
        client_actions: map_client_actions(input),
        player_commands: session_gameplay_commands_from_input(input),
    }
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
        input.local_multiplayer_toggle,
        ClientAction::ToggleLocalMultiplayer,
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

#[cfg(test)]
mod tests {
    use crate::{
        input::PlayerInput,
        multiplayer::{ClientAction, ClientId, CommandSource},
    };

    use super::{
        PlayerCommand, ai_commands, gamepad_commands, local_keyboard_commands,
        local_split_screen_inputs, map_local_input, online_commands, replay_commands,
        split_screen_commands,
    };

    #[test]
    fn maps_movement_when_pressed() {
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
    fn does_not_emit_idle_movement_commands() {
        let mapped = map_local_input(PlayerInput::default());

        assert!(mapped.player_commands.is_empty());
    }

    #[test]
    fn separates_client_actions_from_player_commands() {
        let input = PlayerInput {
            pause: true,
            fullscreen: true,
            local_multiplayer_toggle: true,
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
        assert!(
            mapped
                .client_actions
                .contains(&ClientAction::ToggleLocalMultiplayer)
        );
        assert!(mapped.player_commands.contains(&PlayerCommand::PlaceBomb));
    }

    #[test]
    fn local_split_screen_inputs_route_primary_and_secondary_clients_separately() {
        let primary = PlayerInput {
            horizontal: -1.0,
            thrust: true,
            ..PlayerInput::default()
        };
        let secondary = PlayerInput {
            horizontal: 1.0,
            drill_down: true,
            ..PlayerInput::default()
        };

        let producers = local_split_screen_inputs(
            ClientId::new(1),
            primary,
            Some(ClientId::new(2)),
            Some(secondary),
        );

        assert_eq!(producers.len(), 2);
        assert_eq!(producers[0].client_id, ClientId::new(1));
        assert_eq!(producers[0].producer.source, CommandSource::Keyboard);
        assert_eq!(producers[1].client_id, ClientId::new(2));
        assert_eq!(
            producers[1].producer.source,
            CommandSource::SplitScreenClient
        );
        assert_eq!(
            producers[1].producer.commands[0],
            PlayerCommand::Movement {
                horizontal: 1.0,
                thrust: false,
                drill_down: true,
            }
        );
    }

    #[test]
    fn command_producers_cover_future_input_sources() {
        let commands = vec![PlayerCommand::Confirm];
        let producers = [
            local_keyboard_commands(PlayerInput::default()),
            gamepad_commands(commands.clone()),
            split_screen_commands(commands.clone()),
            online_commands(commands.clone()),
            replay_commands(commands.clone()),
            ai_commands(commands),
        ];

        assert!(
            producers
                .iter()
                .all(super::CommandProducer::uses_authoritative_path)
        );
        assert_eq!(producers[0].source, CommandSource::Keyboard);
        assert_eq!(producers[1].source, CommandSource::Gamepad);
        assert_eq!(producers[2].source, CommandSource::SplitScreenClient);
        assert_eq!(producers[3].source, CommandSource::OnlineClient);
        assert_eq!(producers[4].source, CommandSource::Replay);
        assert_eq!(producers[5].source, CommandSource::Ai);
    }
}
