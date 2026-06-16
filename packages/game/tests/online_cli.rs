use std::process::Command;

use serde::Deserialize;

#[derive(Deserialize)]
struct HostDescriptorProbe {
    host_addr: String,
    server_name: String,
    certificate_der: Vec<u8>,
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
