use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use serde::Deserialize;

#[derive(Deserialize)]
struct HostDescriptorProbe {
    host_addr: String,
    server_name: String,
    certificate_der: Vec<u8>,
}

fn temporary_descriptor_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "drillgame-{name}-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos()
    ))
}

fn spawn_stdout_line_reader(stdout: std::process::ChildStdout) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

#[test]
fn spawned_online_cli_host_and_join_exchange_descriptor_file() {
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let descriptor_path = temporary_descriptor_path("host-join");
    let mut host = Command::new(binary)
        .arg("--online-host-descriptor-file")
        .arg(&descriptor_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("host descriptor process starts");

    let stdout = host.stdout.take().expect("host stdout is piped");
    let stdout_lines = spawn_stdout_line_reader(stdout);
    let readiness_line = stdout_lines
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| {
            if let Some(status) = host.try_wait().expect("host status can be polled") {
                panic!("host exited before readiness marker: {status}");
            }
            panic!("host readiness marker was not emitted: {error}");
        });
    assert_eq!(readiness_line, "online host descriptor ready");
    assert!(descriptor_path.exists(), "descriptor file was not written");

    let join_output = Command::new(binary)
        .arg("--online-join-descriptor-file")
        .arg(&descriptor_path)
        .output()
        .expect("join descriptor process runs");
    assert!(
        join_output.status.success(),
        "join stderr: {}",
        String::from_utf8_lossy(&join_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&join_output.stdout)
            .contains("command/snapshot/correction/reconnect")
    );

    let host_output = host
        .wait_with_output()
        .expect("host descriptor process exits after join");
    assert!(
        host_output.status.success(),
        "host stderr: {}",
        String::from_utf8_lossy(&host_output.stderr)
    );
    let accepted_line = stdout_lines
        .recv_timeout(Duration::from_secs(1))
        .expect("host accepted marker is emitted");
    assert!(accepted_line.contains("command/snapshot/correction/reconnect"));

    let _ignored = std::fs::remove_file(descriptor_path);
}

#[test]
fn spawned_online_cli_emits_serialized_host_descriptor() {
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-host-descriptor-json")
        .output()
        .expect("online descriptor CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("descriptor stdout is utf8");
    let descriptor: HostDescriptorProbe =
        serde_json::from_str(stdout.trim()).expect("descriptor stdout decodes");

    assert!(descriptor.host_addr.contains("127.0.0.1"));
    assert!(!descriptor.server_name.is_empty());
    assert!(!descriptor.certificate_der.is_empty());
}

#[test]
fn spawned_online_cli_runs_local_smoke() {
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-local-smoke")
        .output()
        .expect("online smoke CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("smoke stdout is utf8");
    assert!(stdout.contains("local online smoke passed"));
}
