use serde::{Deserialize, Serialize};

use crate::multiplayer::{QuinnEndpointConfig, QuinnHostListener, local_online_smoke_summary};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OnlineCliAction {
    LocalSmoke,
    HostDescriptorJson,
}

impl OnlineCliAction {
    #[must_use]
    pub const fn success_message(self) -> &'static str {
        match self {
            Self::LocalSmoke => "local online smoke passed",
            Self::HostDescriptorJson => "host descriptor emitted",
        }
    }
}

#[must_use]
pub fn parse_online_cli_action<I, S>(args: I) -> Option<OnlineCliAction>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().find_map(|arg| match arg.as_ref() {
        "--online-local-smoke" => Some(OnlineCliAction::LocalSmoke),
        "--online-host-descriptor-json" => Some(OnlineCliAction::HostDescriptorJson),
        _ => None,
    })
}

/// Execute a one-shot online CLI action.
///
/// # Errors
///
/// Returns an error when the Tokio runtime cannot be created, Quinn setup fails, or smoke checks fail.
pub fn run_online_cli_action(action: OnlineCliAction) -> Result<String, String> {
    match action {
        OnlineCliAction::LocalSmoke => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            let summary = runtime
                .block_on(local_online_smoke_summary())
                .map_err(format_debug_error)?;
            if summary.passed() {
                Ok(action.success_message().to_owned())
            } else {
                Err("local online smoke did not satisfy all readiness checks".to_owned())
            }
        }
        OnlineCliAction::HostDescriptorJson => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            let _guard = runtime.enter();
            let listener =
                QuinnHostListener::bind_localhost(QuinnEndpointConfig::localhost_ephemeral())
                    .map_err(format_debug_error)?;
            let descriptor = listener
                .connection_descriptor()
                .map_err(format_debug_error)?;
            serde_json::to_string(&descriptor).map_err(|error| error.to_string())
        }
    }
}

fn format_debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplayer::QuinnHostConnectionDescriptor;

    #[test]
    fn online_cli_parser_recognizes_local_smoke_and_descriptor_actions() {
        assert_eq!(
            parse_online_cli_action(["--online-local-smoke"]),
            Some(OnlineCliAction::LocalSmoke)
        );
        assert_eq!(
            parse_online_cli_action(["--online-host-descriptor-json"]),
            Some(OnlineCliAction::HostDescriptorJson)
        );
        assert_eq!(parse_online_cli_action(["--fullscreen"]), None);
    }

    #[test]
    fn online_cli_descriptor_action_emits_serialized_descriptor() {
        let json = run_online_cli_action(OnlineCliAction::HostDescriptorJson)
            .expect("descriptor json is emitted");
        let descriptor: QuinnHostConnectionDescriptor =
            serde_json::from_str(&json).expect("descriptor decodes");

        assert!(!descriptor.certificate_der.is_empty());
        assert!(!descriptor.server_name.is_empty());
    }
}
