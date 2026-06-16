use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::multiplayer::{
    QuinnClientConnector, QuinnEndpointConfig, QuinnHostConnectionDescriptor, QuinnHostListener,
    local_online_smoke_summary,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OnlineCliAction {
    LocalSmoke,
    HostDescriptorJson,
    HostDescriptorFile { path: PathBuf },
    JoinDescriptorFile { path: PathBuf },
}

#[must_use]
pub fn parse_online_cli_action<I, S>(args: I) -> Option<OnlineCliAction>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_ref() {
            "--online-local-smoke" => return Some(OnlineCliAction::LocalSmoke),
            "--online-host-descriptor-json" => return Some(OnlineCliAction::HostDescriptorJson),
            "--online-host-descriptor-file" => {
                let path = args.next()?;
                return Some(OnlineCliAction::HostDescriptorFile {
                    path: PathBuf::from(path.as_ref()),
                });
            }
            "--online-join-descriptor-file" => {
                let path = args.next()?;
                return Some(OnlineCliAction::JoinDescriptorFile {
                    path: PathBuf::from(path.as_ref()),
                });
            }
            _ => {}
        }
    }
    None
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
                Ok("local online smoke passed".to_owned())
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
        OnlineCliAction::HostDescriptorFile { path } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(async move {
                let listener =
                    QuinnHostListener::bind_localhost(QuinnEndpointConfig::localhost_ephemeral())
                        .map_err(format_debug_error)?;
                let descriptor = listener
                    .connection_descriptor()
                    .map_err(format_debug_error)?;
                let json = serde_json::to_string(&descriptor).map_err(|error| error.to_string())?;
                std::fs::write(&path, json).map_err(|error| error.to_string())?;
                tokio::time::timeout(Duration::from_secs(5), listener.accept_packet_io())
                    .await
                    .map_err(|_| "timed out waiting for descriptor-file client".to_owned())?
                    .map_err(format_debug_error)?;
                Ok("host descriptor file emitted and connection accepted".to_owned())
            })
        }
        OnlineCliAction::JoinDescriptorFile { path } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(async move {
                let json = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
                let descriptor: QuinnHostConnectionDescriptor =
                    serde_json::from_str(&json).map_err(|error| error.to_string())?;
                let connector = QuinnClientConnector::bind_from_host_descriptor(
                    QuinnEndpointConfig::localhost_ephemeral(),
                    &descriptor,
                )
                .map_err(format_debug_error)?;
                let _packet_io = connector
                    .connect_packet_io(descriptor.host_addr, &descriptor.server_name)
                    .await
                    .map_err(format_debug_error)?;
                Ok("joined descriptor host".to_owned())
            })
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
        assert_eq!(
            parse_online_cli_action(["--online-host-descriptor-file", "/tmp/host.json"]),
            Some(OnlineCliAction::HostDescriptorFile {
                path: PathBuf::from("/tmp/host.json")
            })
        );
        assert_eq!(
            parse_online_cli_action(["--online-join-descriptor-file", "/tmp/host.json"]),
            Some(OnlineCliAction::JoinDescriptorFile {
                path: PathBuf::from("/tmp/host.json")
            })
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
