use std::{io::Write, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::multiplayer::{
    QuinnClientConnector, QuinnEndpointConfig, QuinnHostConnectionDescriptor, QuinnHostListener,
    local_online_smoke_summary, production_online_acceptance_summary,
    scripted_latency_loss_online_playtest_summary,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OnlineCliAction {
    LocalSmoke,
    LatencyLossPlaytest,
    ProductionAcceptance,
    ProductionAcceptanceJson,
    HostDescriptorJson,
    HostDescriptorFile { path: PathBuf },
    JoinDescriptorFile { path: PathBuf },
    HostGameplayDescriptorFile { path: PathBuf, ticks: u32 },
    JoinGameplayDescriptorFile { path: PathBuf, ticks: u32 },
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
            "--online-latency-loss-playtest" => {
                return Some(OnlineCliAction::LatencyLossPlaytest);
            }
            "--online-production-acceptance" => {
                return Some(OnlineCliAction::ProductionAcceptance);
            }
            "--online-production-acceptance-json" => {
                return Some(OnlineCliAction::ProductionAcceptanceJson);
            }
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
            "--online-host-gameplay-descriptor-file" => {
                let path = args.next()?;
                let ticks = args.next()?.as_ref().parse().ok()?;
                if ticks == 0 {
                    return None;
                }
                return Some(OnlineCliAction::HostGameplayDescriptorFile {
                    path: PathBuf::from(path.as_ref()),
                    ticks,
                });
            }
            "--online-join-gameplay-descriptor-file" => {
                let path = args.next()?;
                let ticks = args.next()?.as_ref().parse().ok()?;
                if ticks == 0 {
                    return None;
                }
                return Some(OnlineCliAction::JoinGameplayDescriptorFile {
                    path: PathBuf::from(path.as_ref()),
                    ticks,
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
        OnlineCliAction::LocalSmoke => run_local_smoke_cli_action(),
        OnlineCliAction::LatencyLossPlaytest => run_latency_loss_playtest_cli_action(),
        OnlineCliAction::ProductionAcceptance => run_production_acceptance_cli_action(),
        OnlineCliAction::ProductionAcceptanceJson => run_production_acceptance_json_cli_action(),
        OnlineCliAction::HostDescriptorJson => run_host_descriptor_json_cli_action(),
        OnlineCliAction::HostDescriptorFile { path } => run_host_descriptor_file_cli_action(path),
        OnlineCliAction::JoinDescriptorFile { path } => run_join_descriptor_file_cli_action(path),
        OnlineCliAction::HostGameplayDescriptorFile { path, ticks } => {
            run_host_gameplay_descriptor_file_cli_action(path, ticks)
        }
        OnlineCliAction::JoinGameplayDescriptorFile { path, ticks } => {
            run_join_gameplay_descriptor_file_cli_action(path, ticks)
        }
    }
}

fn build_current_thread_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
}

fn run_local_smoke_cli_action() -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    let summary = runtime
        .block_on(local_online_smoke_summary())
        .map_err(format_debug_error)?;
    if summary.passed() {
        Ok("local online smoke passed".to_owned())
    } else {
        Err("local online smoke did not satisfy all readiness checks".to_owned())
    }
}

fn run_latency_loss_playtest_cli_action() -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    let summary = runtime
        .block_on(scripted_latency_loss_online_playtest_summary())
        .map_err(format_debug_error)?;
    if summary.passed() {
        Ok("scripted latency/loss online playtest passed".to_owned())
    } else {
        Err("scripted latency/loss online playtest did not satisfy all readiness checks".to_owned())
    }
}

fn run_production_acceptance_cli_action() -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    let summary = runtime
        .block_on(production_online_acceptance_summary())
        .map_err(format_debug_error)?;
    if summary.direct_connect_mvp_passed() {
        Ok("production online direct-connect acceptance passed".to_owned())
    } else {
        Err(
            "production online direct-connect acceptance did not satisfy all readiness checks"
                .to_owned(),
        )
    }
}

fn run_production_acceptance_json_cli_action() -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    let summary = runtime
        .block_on(production_online_acceptance_summary())
        .map_err(format_debug_error)?;
    serde_json::to_string_pretty(&summary).map_err(format_debug_error)
}

fn run_host_descriptor_json_cli_action() -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    let _guard = runtime.enter();
    let listener = QuinnHostListener::bind_localhost(QuinnEndpointConfig::localhost_ephemeral())
        .map_err(format_debug_error)?;
    let descriptor = listener
        .connection_descriptor()
        .map_err(format_debug_error)?;
    serde_json::to_string(&descriptor).map_err(|error| error.to_string())
}

fn run_host_descriptor_file_cli_action(path: PathBuf) -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    runtime.block_on(async move {
        let listener =
            QuinnHostListener::bind_localhost(QuinnEndpointConfig::localhost_ephemeral())
                .map_err(format_debug_error)?;
        let descriptor = listener
            .connection_descriptor()
            .map_err(format_debug_error)?;
        let json = serde_json::to_string(&descriptor).map_err(|error| error.to_string())?;
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
        println!("online host descriptor ready");
        std::io::stdout()
            .flush()
            .map_err(|error| error.to_string())?;
        let packet_io = tokio::time::timeout(Duration::from_secs(5), listener.accept_packet_io())
            .await
            .map_err(|_| "timed out waiting for descriptor-file client".to_owned())?
            .map_err(format_debug_error)?;
        run_host_descriptor_file_packet_exchange(&packet_io).await?;
        let reconnect_io =
            tokio::time::timeout(Duration::from_secs(5), listener.accept_packet_io())
                .await
                .map_err(|_| "timed out waiting for descriptor-file reconnect".to_owned())?
                .map_err(format_debug_error)?;
        run_host_descriptor_file_reconnect_exchange(&reconnect_io).await?;
        Ok("host descriptor file exchanged command/snapshot/correction/reconnect".to_owned())
    })
}

fn run_join_descriptor_file_cli_action(path: PathBuf) -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    runtime.block_on(async move {
        let json = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let descriptor: QuinnHostConnectionDescriptor =
            serde_json::from_str(&json).map_err(|error| error.to_string())?;
        let connector = QuinnClientConnector::bind_from_host_descriptor(
            QuinnEndpointConfig::localhost_ephemeral(),
            &descriptor,
        )
        .map_err(format_debug_error)?;
        let packet_io = connector
            .connect_packet_io(descriptor.host_addr, &descriptor.server_name)
            .await
            .map_err(format_debug_error)?;
        run_join_descriptor_file_packet_exchange(&packet_io).await?;
        let reconnect_io = connector
            .connect_packet_io(descriptor.host_addr, &descriptor.server_name)
            .await
            .map_err(format_debug_error)?;
        run_join_descriptor_file_reconnect_exchange(&reconnect_io).await?;
        Ok("joined descriptor host and exchanged command/snapshot/correction/reconnect".to_owned())
    })
}

fn run_host_gameplay_descriptor_file_cli_action(
    path: PathBuf,
    ticks: u32,
) -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    runtime.block_on(async move {
        let listener =
            QuinnHostListener::bind_localhost(QuinnEndpointConfig::localhost_ephemeral())
                .map_err(format_debug_error)?;
        let descriptor = listener
            .connection_descriptor()
            .map_err(format_debug_error)?;
        let json = serde_json::to_string(&descriptor).map_err(|error| error.to_string())?;
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
        println!("online gameplay host descriptor ready");
        std::io::stdout()
            .flush()
            .map_err(|error| error.to_string())?;
        let packet_io = tokio::time::timeout(Duration::from_secs(5), listener.accept_packet_io())
            .await
            .map_err(|_| "timed out waiting for gameplay descriptor-file client".to_owned())?
            .map_err(format_debug_error)?;
        run_host_gameplay_descriptor_ticks(&packet_io, ticks).await?;
        Ok(format!("host gameplay descriptor file ran {ticks} ticks"))
    })
}

fn run_join_gameplay_descriptor_file_cli_action(
    path: PathBuf,
    ticks: u32,
) -> Result<String, String> {
    let runtime = build_current_thread_runtime()?;
    runtime.block_on(async move {
        let json = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let descriptor: QuinnHostConnectionDescriptor =
            serde_json::from_str(&json).map_err(|error| error.to_string())?;
        let connector = QuinnClientConnector::bind_from_host_descriptor(
            QuinnEndpointConfig::localhost_ephemeral(),
            &descriptor,
        )
        .map_err(format_debug_error)?;
        let packet_io = connector
            .connect_packet_io(descriptor.host_addr, &descriptor.server_name)
            .await
            .map_err(format_debug_error)?;
        run_join_gameplay_descriptor_ticks(&packet_io, ticks).await?;
        Ok(format!("joined gameplay descriptor host for {ticks} ticks"))
    })
}

async fn run_host_gameplay_descriptor_ticks(
    packet_io: &crate::multiplayer::QuinnPacketIo,
    ticks: u32,
) -> Result<(), String> {
    for tick_index in 0..ticks {
        let command_packet =
            tokio::time::timeout(Duration::from_secs(5), packet_io.receive_datagram_packet())
                .await
                .map_err(|_| "timed out waiting for gameplay command packet".to_owned())?
                .map_err(format_debug_error)?;
        let crate::multiplayer::ProtocolMessage::CommandPacket(packet) = command_packet.message
        else {
            return Err("gameplay host expected command packet".to_owned());
        };
        if packet.commands.len() != 1 {
            return Err("gameplay host expected one command per tick".to_owned());
        }
        packet_io
            .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
                crate::multiplayer::ProtocolMessage::SnapshotKeyframe {
                    snapshot: gameplay_descriptor_snapshot(tick_index),
                },
            ))
            .await
            .map_err(format_debug_error)?;
    }
    tokio::task::yield_now().await;
    packet_io.close(b"gameplay descriptor exchange complete");
    Ok(())
}

async fn run_join_gameplay_descriptor_ticks(
    packet_io: &crate::multiplayer::QuinnPacketIo,
    ticks: u32,
) -> Result<(), String> {
    for tick_index in 0..ticks {
        packet_io
            .send_packet(gameplay_descriptor_command_packet(tick_index))
            .await
            .map_err(format_debug_error)?;
        let snapshot =
            tokio::time::timeout(Duration::from_secs(5), packet_io.receive_datagram_packet())
                .await
                .map_err(|_| "timed out waiting for gameplay snapshot".to_owned())?
                .map_err(format_debug_error)?;
        let crate::multiplayer::ProtocolMessage::SnapshotKeyframe { snapshot } = snapshot.message
        else {
            return Err("gameplay join expected snapshot".to_owned());
        };
        if snapshot.tick != crate::multiplayer::SimulationTick::new(u64::from(tick_index) + 10) {
            return Err("gameplay join received unexpected snapshot tick".to_owned());
        }
    }
    tokio::task::yield_now().await;
    Ok(())
}

async fn run_host_descriptor_file_packet_exchange(
    packet_io: &crate::multiplayer::QuinnPacketIo,
) -> Result<(), String> {
    let terrain_request =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_reliable_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file terrain request".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
        chunk_x,
        chunk_y,
        known_revision: _,
    } = terrain_request.message
    else {
        return Err("descriptor-file host expected terrain request".to_owned());
    };
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::TerrainChunkResponse {
                chunk_x,
                chunk_y,
                revision: 1,
            },
        ))
        .await
        .map_err(format_debug_error)?;
    let command_packet =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_datagram_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file command packet".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::CommandPacket(_) = command_packet.message else {
        return Err("descriptor-file host expected command packet".to_owned());
    };
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::SnapshotKeyframe {
                snapshot: descriptor_file_snapshot(),
            },
        ))
        .await
        .map_err(format_debug_error)?;
    let snapshot_ack =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_reliable_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file snapshot ack".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
        chunk_x: 99,
        chunk_y: 99,
        known_revision: 1,
    } = snapshot_ack.message
    else {
        return Err("descriptor-file host expected snapshot ack".to_owned());
    };
    packet_io.close(b"descriptor-file exchange complete");
    Ok(())
}

async fn run_host_descriptor_file_reconnect_exchange(
    packet_io: &crate::multiplayer::QuinnPacketIo,
) -> Result<(), String> {
    let reconnect =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_reliable_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file reconnect request".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::ReconnectRequest {
        client_id,
        session_token,
    } = reconnect.message
    else {
        return Err("descriptor-file host expected reconnect request".to_owned());
    };
    if session_token != descriptor_file_session_token() {
        return Err("descriptor-file host received wrong reconnect token".to_owned());
    }
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::JoinAccepted {
                client_id,
                player_id: crate::multiplayer::PlayerId::new(1),
                snapshot_tick: crate::multiplayer::SimulationTick::new(3),
            },
        ))
        .await
        .map_err(format_debug_error)?;
    tokio::task::yield_now().await;
    packet_io.close(b"descriptor-file reconnect complete");
    Ok(())
}

async fn run_join_descriptor_file_packet_exchange(
    packet_io: &crate::multiplayer::QuinnPacketIo,
) -> Result<(), String> {
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
                chunk_x: 1,
                chunk_y: 2,
                known_revision: 0,
            },
        ))
        .await
        .map_err(format_debug_error)?;
    let terrain_response =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_reliable_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file terrain response".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::TerrainChunkResponse { revision: 1, .. } =
        terrain_response.message
    else {
        return Err("descriptor-file join expected terrain response".to_owned());
    };
    packet_io
        .send_packet(descriptor_file_command_packet())
        .await
        .map_err(format_debug_error)?;
    let snapshot =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_datagram_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file snapshot".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::SnapshotKeyframe { snapshot } = snapshot.message
    else {
        return Err("descriptor-file join expected snapshot".to_owned());
    };
    validate_descriptor_file_correction(&snapshot)?;
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::TerrainChunkRequest {
                chunk_x: 99,
                chunk_y: 99,
                known_revision: 1,
            },
        ))
        .await
        .map_err(format_debug_error)?;
    tokio::task::yield_now().await;
    Ok(())
}

fn validate_descriptor_file_correction(
    snapshot: &crate::multiplayer::NetworkWorldSnapshot,
) -> Result<(), String> {
    let Some(authoritative) = snapshot.players.first() else {
        return Err("descriptor-file snapshot was empty".to_owned());
    };
    let authoritative_snapshot = crate::session::PlayerSnapshot {
        player_id: authoritative.player_id,
        x: authoritative.x,
        y: authoritative.y,
        velocity_x: authoritative.velocity_x,
        velocity_y: authoritative.velocity_y,
        fuel: authoritative.fuel,
        hull: authoritative.hull,
        credits: authoritative.credits,
        cargo_used: authoritative.cargo_used,
        scanner_cooldown_seconds: authoritative.scanner_cooldown_seconds,
    };
    let predicted = crate::session::PredictedMovement {
        player_id: authoritative.player_id,
        x: authoritative.x + 24.0,
        y: authoritative.y,
        velocity_x: authoritative.velocity_x,
        velocity_y: authoritative.velocity_y,
    };
    let reconciled = crate::session::ClientPredictionState::reconcile_movement(
        predicted,
        &authoritative_snapshot,
    );
    if reconciled.correction_plan != crate::session::CorrectionPlan::Snap {
        return Err("descriptor-file correction did not request snap".to_owned());
    }
    let presentation =
        crate::session::CorrectionPresentationFrame::from_reconciliation(&reconciled, 0.5);
    if !presentation.snap_applied {
        return Err("descriptor-file correction snap was not applied".to_owned());
    }
    Ok(())
}

async fn run_join_descriptor_file_reconnect_exchange(
    packet_io: &crate::multiplayer::QuinnPacketIo,
) -> Result<(), String> {
    packet_io
        .send_packet(crate::multiplayer::VersionedProtocolPacket::new(
            crate::multiplayer::ProtocolMessage::ReconnectRequest {
                client_id: crate::multiplayer::ClientId::new(1),
                session_token: descriptor_file_session_token(),
            },
        ))
        .await
        .map_err(format_debug_error)?;
    let accepted =
        tokio::time::timeout(Duration::from_secs(5), packet_io.receive_reliable_packet())
            .await
            .map_err(|_| "timed out waiting for descriptor-file reconnect accepted".to_owned())?
            .map_err(format_debug_error)?;
    let crate::multiplayer::ProtocolMessage::JoinAccepted {
        client_id,
        player_id,
        snapshot_tick,
    } = accepted.message
    else {
        return Err("descriptor-file join expected reconnect acceptance".to_owned());
    };
    if client_id != crate::multiplayer::ClientId::new(1)
        || player_id != crate::multiplayer::PlayerId::new(1)
        || snapshot_tick != crate::multiplayer::SimulationTick::new(3)
    {
        return Err("descriptor-file reconnect acceptance carried unexpected identity".to_owned());
    }
    Ok(())
}

fn format_debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

const fn descriptor_file_session_token() -> crate::multiplayer::SessionToken {
    crate::multiplayer::SessionToken::new(0xD411_600D_0000_0001)
}

fn gameplay_descriptor_command_packet(
    tick_index: u32,
) -> crate::multiplayer::VersionedProtocolPacket {
    crate::multiplayer::VersionedProtocolPacket::new(
        crate::multiplayer::ProtocolMessage::CommandPacket(crate::multiplayer::CommandPacket {
            client_id: crate::multiplayer::ClientId::new(1),
            commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: crate::multiplayer::PlayerId::new(1),
                sequence: crate::multiplayer::InputSequence::new(tick_index + 1),
                target_tick: crate::multiplayer::SimulationTick::new(u64::from(tick_index) + 10),
                command: crate::multiplayer::PlayerCommand::Movement {
                    horizontal: if tick_index.is_multiple_of(2) {
                        1.0
                    } else {
                        -1.0
                    },
                    thrust: tick_index.is_multiple_of(2),
                    drill_down: !tick_index.is_multiple_of(2),
                },
            }],
        }),
    )
}

fn gameplay_descriptor_snapshot(tick_index: u32) -> crate::multiplayer::NetworkWorldSnapshot {
    let tick_offset = f32::from(u16::try_from(tick_index).unwrap_or(u16::MAX));
    crate::multiplayer::NetworkWorldSnapshot {
        tick: crate::multiplayer::SimulationTick::new(u64::from(tick_index) + 10),
        players: vec![crate::multiplayer::NetworkPlayerSnapshot {
            player_id: crate::multiplayer::PlayerId::new(1),
            x: 10.0 + tick_offset,
            y: 20.0 + tick_offset,
            velocity_x: 1.0,
            velocity_y: 0.0,
            fuel: 99.0 - tick_offset,
            hull: 100.0,
            credits: 5 + tick_index,
            cargo_used: tick_index,
            scanner_cooldown_seconds: 0.0,
        }],
    }
}

fn descriptor_file_command_packet() -> crate::multiplayer::VersionedProtocolPacket {
    crate::multiplayer::VersionedProtocolPacket::new(
        crate::multiplayer::ProtocolMessage::CommandPacket(crate::multiplayer::CommandPacket {
            client_id: crate::multiplayer::ClientId::new(1),
            commands: vec![crate::multiplayer::SequencedPlayerCommand {
                player_id: crate::multiplayer::PlayerId::new(1),
                sequence: crate::multiplayer::InputSequence::new(1),
                target_tick: crate::multiplayer::SimulationTick::new(1),
                command: crate::multiplayer::PlayerCommand::Movement {
                    horizontal: 1.0,
                    thrust: true,
                    drill_down: false,
                },
            }],
        }),
    )
}

fn descriptor_file_snapshot() -> crate::multiplayer::NetworkWorldSnapshot {
    crate::multiplayer::NetworkWorldSnapshot {
        tick: crate::multiplayer::SimulationTick::new(2),
        players: vec![crate::multiplayer::NetworkPlayerSnapshot {
            player_id: crate::multiplayer::PlayerId::new(1),
            x: 10.0,
            y: 20.0,
            velocity_x: 1.0,
            velocity_y: 0.0,
            fuel: 99.0,
            hull: 100.0,
            credits: 5,
            cargo_used: 0,
            scanner_cooldown_seconds: 0.0,
        }],
    }
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
            parse_online_cli_action(["--online-latency-loss-playtest"]),
            Some(OnlineCliAction::LatencyLossPlaytest)
        );
        assert_eq!(
            parse_online_cli_action(["--online-production-acceptance"]),
            Some(OnlineCliAction::ProductionAcceptance)
        );
        assert_eq!(
            parse_online_cli_action(["--online-production-acceptance-json"]),
            Some(OnlineCliAction::ProductionAcceptanceJson)
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
        assert_eq!(
            parse_online_cli_action([
                "--online-host-gameplay-descriptor-file",
                "/tmp/gameplay-host.json",
                "3"
            ]),
            Some(OnlineCliAction::HostGameplayDescriptorFile {
                path: PathBuf::from("/tmp/gameplay-host.json"),
                ticks: 3,
            })
        );
        assert_eq!(
            parse_online_cli_action([
                "--online-join-gameplay-descriptor-file",
                "/tmp/gameplay-host.json",
                "3"
            ]),
            Some(OnlineCliAction::JoinGameplayDescriptorFile {
                path: PathBuf::from("/tmp/gameplay-host.json"),
                ticks: 3,
            })
        );
        assert_eq!(
            parse_online_cli_action([
                "--online-join-gameplay-descriptor-file",
                "/tmp/gameplay-host.json",
                "0"
            ]),
            None
        );
        assert_eq!(
            parse_online_cli_action(vec![
                "--online-host-gameplay-descriptor-file".to_owned(),
                "/tmp/owned-gameplay-host.json".to_owned(),
                "2".to_owned(),
            ]),
            Some(OnlineCliAction::HostGameplayDescriptorFile {
                path: PathBuf::from("/tmp/owned-gameplay-host.json"),
                ticks: 2,
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
