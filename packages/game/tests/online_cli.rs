use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use serde::Deserialize;

static ONLINE_CLI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn online_cli_test_lock() -> std::sync::MutexGuard<'static, ()> {
    ONLINE_CLI_TEST_LOCK
        .lock()
        .expect("online CLI integration test lock is not poisoned")
}

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

fn unused_loopback_udp_addr() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("ephemeral UDP socket binds");
    socket.local_addr().expect("ephemeral UDP addr is visible")
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
    let _lock = online_cli_test_lock();
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
fn spawned_online_cli_host_and_join_exchange_descriptor_file_on_advertised_addr() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let descriptor_path = temporary_descriptor_path("host-join-advertised");
    let bind_addr = unused_loopback_udp_addr();
    let mut host = Command::new(binary)
        .arg("--online-host-descriptor-file-on-addr")
        .arg(&descriptor_path)
        .arg(bind_addr.to_string())
        .arg(bind_addr.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("advertised host descriptor process starts");

    let stdout = host.stdout.take().expect("host stdout is piped");
    let stdout_lines = spawn_stdout_line_reader(stdout);
    let readiness_line = stdout_lines
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| {
            if let Some(status) = host.try_wait().expect("host status can be polled") {
                panic!("advertised host exited before readiness marker: {status}");
            }
            panic!("advertised host readiness marker was not emitted: {error}");
        });
    assert_eq!(readiness_line, "online host descriptor ready");
    assert!(descriptor_path.exists(), "descriptor file was not written");
    let descriptor_json =
        std::fs::read_to_string(&descriptor_path).expect("advertised descriptor file can be read");
    let descriptor: HostDescriptorProbe =
        serde_json::from_str(&descriptor_json).expect("advertised descriptor JSON parses");
    assert_eq!(descriptor.host_addr, bind_addr.to_string());

    let join_bind_addr = unused_loopback_udp_addr();
    let join_output = Command::new(binary)
        .arg("--online-join-descriptor-file-on-addr")
        .arg(&descriptor_path)
        .arg(join_bind_addr.to_string())
        .output()
        .expect("join advertised descriptor process runs");
    assert!(
        join_output.status.success(),
        "join stderr: {}",
        String::from_utf8_lossy(&join_output.stderr)
    );

    let host_output = host
        .wait_with_output()
        .expect("advertised host descriptor process exits after join");
    assert!(
        host_output.status.success(),
        "host stderr: {}",
        String::from_utf8_lossy(&host_output.stderr)
    );

    let _ignored = std::fs::remove_file(descriptor_path);
}

#[test]
fn spawned_online_cli_host_and_join_gameplay_descriptor_ticks() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let descriptor_path = temporary_descriptor_path("gameplay-host-join");
    let mut host = Command::new(binary)
        .arg("--online-host-gameplay-descriptor-file")
        .arg(&descriptor_path)
        .arg("3")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("gameplay host descriptor process starts");

    let stdout = host.stdout.take().expect("host stdout is piped");
    let stdout_lines = spawn_stdout_line_reader(stdout);
    let readiness_line = stdout_lines
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| {
            if let Some(status) = host.try_wait().expect("host status can be polled") {
                panic!("gameplay host exited before readiness marker: {status}");
            }
            panic!("gameplay host readiness marker was not emitted: {error}");
        });
    assert_eq!(readiness_line, "online gameplay host descriptor ready");
    assert!(descriptor_path.exists(), "descriptor file was not written");

    let join_output = Command::new(binary)
        .arg("--online-join-gameplay-descriptor-file")
        .arg(&descriptor_path)
        .arg("3")
        .output()
        .expect("gameplay join descriptor process runs");
    assert!(
        join_output.status.success(),
        "gameplay join stderr: {}",
        String::from_utf8_lossy(&join_output.stderr)
    );
    assert!(String::from_utf8_lossy(&join_output.stdout).contains("for 3 ticks"));

    let host_output = host
        .wait_with_output()
        .expect("gameplay host descriptor process exits after join");
    assert!(
        host_output.status.success(),
        "gameplay host stderr: {}",
        String::from_utf8_lossy(&host_output.stderr)
    );
    let accepted_line = stdout_lines
        .recv_timeout(Duration::from_secs(1))
        .expect("gameplay host completion marker is emitted");
    assert!(accepted_line.contains("ran 3 ticks"));

    let _ignored = std::fs::remove_file(descriptor_path);
}

#[test]
fn spawned_online_cli_host_and_join_gameplay_descriptor_ticks_on_advertised_addr() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let descriptor_path = temporary_descriptor_path("gameplay-host-join-advertised");
    let bind_addr = unused_loopback_udp_addr();
    let mut host = Command::new(binary)
        .arg("--online-host-gameplay-descriptor-file-on-addr")
        .arg(&descriptor_path)
        .arg(bind_addr.to_string())
        .arg(bind_addr.to_string())
        .arg("3")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("advertised gameplay host descriptor process starts");

    let stdout = host.stdout.take().expect("host stdout is piped");
    let stdout_lines = spawn_stdout_line_reader(stdout);
    let readiness_line = stdout_lines
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|error| {
            if let Some(status) = host.try_wait().expect("host status can be polled") {
                panic!("advertised gameplay host exited before readiness marker: {status}");
            }
            panic!("advertised gameplay host readiness marker was not emitted: {error}");
        });
    assert_eq!(readiness_line, "online gameplay host descriptor ready");
    let descriptor_json = std::fs::read_to_string(&descriptor_path)
        .expect("advertised gameplay descriptor file can be read");
    let descriptor: HostDescriptorProbe =
        serde_json::from_str(&descriptor_json).expect("advertised gameplay descriptor JSON parses");
    assert_eq!(descriptor.host_addr, bind_addr.to_string());

    let join_bind_addr = unused_loopback_udp_addr();
    let join_output = Command::new(binary)
        .arg("--online-join-gameplay-descriptor-file-on-addr")
        .arg(&descriptor_path)
        .arg(join_bind_addr.to_string())
        .arg("3")
        .output()
        .expect("advertised gameplay join descriptor process runs");
    assert!(
        join_output.status.success(),
        "gameplay join stderr: {}",
        String::from_utf8_lossy(&join_output.stderr)
    );

    let host_output = host
        .wait_with_output()
        .expect("advertised gameplay host descriptor process exits after join");
    assert!(
        host_output.status.success(),
        "gameplay host stderr: {}",
        String::from_utf8_lossy(&host_output.stderr)
    );
    let accepted_line = stdout_lines
        .recv_timeout(Duration::from_secs(1))
        .expect("advertised gameplay host completion marker is emitted");
    assert!(accepted_line.contains("ran 3 ticks"));

    let _ignored = std::fs::remove_file(descriptor_path);
}

#[test]
fn spawned_online_cli_emits_serialized_host_descriptor() {
    let _lock = online_cli_test_lock();
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
fn spawned_online_cli_inspects_descriptor_file() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let descriptor_path = temporary_descriptor_path("inspect-descriptor");
    std::fs::write(
        &descriptor_path,
        r#"{"host_addr":"127.0.0.1:4242","server_name":"localhost","certificate_der":[1,2,3,4]}"#,
    )
    .expect("descriptor fixture writes");

    let output = Command::new(binary)
        .arg("--online-inspect-descriptor-file")
        .arg(&descriptor_path)
        .output()
        .expect("descriptor inspection CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("descriptor inspection stdout is utf8");
    assert!(stdout.contains("127.0.0.1:4242"));
    assert!(stdout.contains("certificate_der_bytes"));
    let _ignored = std::fs::remove_file(descriptor_path);
}

#[test]
fn spawned_online_cli_prints_lan_qa_checklist_markdown() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-lan-qa-checklist-md")
        .arg("/tmp/drillgame-lan-host.json")
        .arg("0.0.0.0:4242")
        .arg("192.0.2.15:4242")
        .arg("0.0.0.0:0")
        .arg("60")
        .output()
        .expect("LAN QA checklist CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("LAN QA checklist stdout is utf8");
    assert!(stdout.contains("Drillgame LAN Multiplayer QA Checklist"));
    assert!(stdout.contains("Host firewall allows the advertised UDP port"));
}

#[test]
fn spawned_online_cli_prints_lan_qa_plan_json() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-lan-qa-plan-json")
        .arg("/tmp/drillgame-lan-host.json")
        .arg("0.0.0.0:4242")
        .arg("192.0.2.15:4242")
        .arg("0.0.0.0:0")
        .arg("60")
        .output()
        .expect("LAN QA plan CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("LAN QA plan stdout is utf8");
    assert!(stdout.contains("online-host-gameplay-descriptor-file-on-addr"));
    assert!(stdout.contains("online-join-gameplay-descriptor-file-on-addr"));
}

#[test]
fn spawned_online_cli_prints_help() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-help")
        .output()
        .expect("online help CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("online help stdout is utf8");
    assert!(stdout.contains("Online multiplayer CLI actions"));
    assert!(stdout.contains("--online-production-acceptance-json"));
    assert!(stdout.contains("--online-local-soak <ticks>"));
    assert!(stdout.contains("--online-local-soak-json <ticks>"));
    assert!(stdout.contains("--online-local-degraded-soak <ticks>"));
    assert!(stdout.contains("--online-local-degraded-soak-json <ticks>"));
}

#[test]
fn spawned_online_cli_runs_local_smoke() {
    let _lock = online_cli_test_lock();
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

#[test]
fn spawned_online_cli_runs_local_soak() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .args(["--online-local-soak", "6"])
        .output()
        .expect("online soak CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("soak stdout is utf8");
    assert!(stdout.contains("local online soak passed"));
    assert!(stdout.contains("ticks=6"));
    assert!(stdout.contains("corrections=6"));
}

#[test]
fn spawned_online_cli_emits_local_soak_json() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .args(["--online-local-soak-json", "4"])
        .output()
        .expect("online soak JSON CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("soak JSON stdout is utf8");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("soak output is JSON");
    assert_eq!(json["ticks_requested"], 4);
    assert_eq!(json["ticks_completed"], 4);
    assert_eq!(json["commands_exchanged"], 4);
    assert_eq!(json["corrections_replicated"], 4);
}

#[test]
fn spawned_online_cli_runs_local_degraded_soak() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .args(["--online-local-degraded-soak", "4"])
        .output()
        .expect("online degraded soak CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("degraded soak stdout is utf8");
    assert!(stdout.contains("local online degraded soak passed"));
    assert!(stdout.contains("ticks=4"));
}

#[test]
fn spawned_online_cli_emits_local_degraded_soak_json() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .args(["--online-local-degraded-soak-json", "4"])
        .output()
        .expect("online degraded soak JSON CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("degraded soak JSON stdout is utf8");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("degraded soak output is JSON");
    assert_eq!(json["real_quinn_soak"]["ticks_requested"], 4);
    assert_eq!(json["real_quinn_soak"]["ticks_completed"], 4);
    assert!(json["degraded_network"]["covered"].is_array());
}

#[test]
fn spawned_online_cli_runs_latency_loss_playtest() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-latency-loss-playtest")
        .output()
        .expect("online latency/loss playtest CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("playtest stdout is utf8");
    assert!(stdout.contains("scripted latency/loss online playtest passed"));
}

#[test]
fn spawned_online_cli_runs_production_acceptance() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-production-acceptance")
        .output()
        .expect("online production acceptance CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("acceptance stdout is utf8");
    assert!(stdout.contains("production online direct-connect acceptance passed"));
}

#[test]
fn spawned_online_cli_emits_production_acceptance_json() {
    let _lock = online_cli_test_lock();
    let binary = env!("CARGO_BIN_EXE_drillgame");
    let output = Command::new(binary)
        .arg("--online-production-acceptance-json")
        .output()
        .expect("online production acceptance JSON CLI process runs");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("acceptance JSON stdout is utf8");
    assert!(stdout.contains("DirectConnectTransport"));
    assert!(stdout.contains("ScriptedLatencyLoss"));
}
