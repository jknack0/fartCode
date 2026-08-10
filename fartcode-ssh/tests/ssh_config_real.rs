//! Exercises `ssh -G` against the real OpenSSH binary (E12-03 AC-4/AC-7).
//! Skipped when `ssh` is not installed.

use fartcode_ssh::config::{parse_ssh_g, resolve_ssh_config};

fn ssh_available() -> bool {
    std::process::Command::new("ssh")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn resolves_an_arbitrary_host_through_real_ssh() {
    if !ssh_available() {
        eprintln!("skipping: no ssh binary");
        return;
    }

    // Unknown alias still resolves: ssh falls back to defaults, so this asserts
    // our parser agrees with real output shape rather than any user's config.
    let cfg = resolve_ssh_config("fartcode-e12-03-probe.invalid")
        .await
        .expect("ssh -G should succeed for an unmatched host");

    assert_eq!(cfg.hostname, "fartcode-e12-03-probe.invalid");
    assert_eq!(cfg.port, 22);
    assert!(!cfg.user.is_empty(), "ssh -G always emits a user");
    assert!(
        !cfg.identity_files.is_empty(),
        "ssh -G always emits default identityfile candidates"
    );
}

#[tokio::test]
async fn parser_matches_real_output_for_port_override() {
    if !ssh_available() {
        eprintln!("skipping: no ssh binary");
        return;
    }

    let raw = std::process::Command::new("ssh")
        .args(["-G", "-p", "2222", "fartcode-e12-03-probe.invalid"])
        .output()
        .expect("spawn ssh");
    assert!(raw.status.success());

    let cfg = parse_ssh_g(&String::from_utf8_lossy(&raw.stdout));
    assert_eq!(cfg.port, 2222);
}
