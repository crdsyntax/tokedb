#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use tokedb_runtime::isolation::{
    apply_security, capabilities, ContainerUser, SeccompProfile, SeccompSyscall, SecurityProfile,
};
use tokedb_runtime::runtime::process::{spawn_isolated, CommandSpec};

fn capability_mask(output: &str) -> u64 {
    output
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .map(|value| u64::from_str_radix(value.trim(), 16).unwrap())
        .unwrap()
}

fn spawn_with_security(profile: SecurityProfile) -> (std::process::ExitStatus, String) {
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            r#"grep CapEff /proc/self/status; rm -f /tmp/seccomp-probe; touch /tmp/seccomp-probe; chmod 600 /tmp/seccomp-probe 2>/dev/null; echo CHMOD_RC=$?; echo UID=$(id -u); echo GID=$(id -g)"#,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || apply_security(&profile).map_err(std::io::Error::other));
    }
    let mut child = command.spawn().unwrap();
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let status = child.wait().unwrap();
    (status, output)
}

fn field<'a>(output: &'a str, key: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap_or_else(|| panic!("missing {key} in output: {output}"))
}

#[test]
fn spawn_isolated_applies_restricted_capabilities() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let profile = SecurityProfile {
        capabilities: capabilities::default_allowlist(),
        seccomp: None,
        user: None,
    };
    let spec = CommandSpec::new("/bin/sh")
        .arg("-c")
        .arg("grep CapEff /proc/self/status")
        .security(profile);
    let mut spawned = spawn_isolated(&spec).unwrap();
    let mut output = String::new();
    spawned
        .take_stdout()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let status = spawned.wait().unwrap();
    assert!(status.success());

    let expected = capabilities::default_allowlist().mask();
    assert_eq!(
        capability_mask(&output),
        expected,
        "CapEff should be exactly the allowlist: {output}"
    );
}

#[test]
fn seccomp_denylist_returns_eperm() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let profile = SecurityProfile {
        capabilities: capabilities::default_allowlist(),
        seccomp: Some(SeccompProfile {
            blocked: vec![SeccompSyscall::Ptrace],
        }),
        user: None,
    };
    let (status, output) = spawn_with_security(profile);
    assert!(status.success());
    assert_eq!(
        capability_mask(&output),
        capabilities::default_allowlist().mask(),
        "caps must be unaffected when seccomp blocks only ptrace: {output}"
    );

    let profile = SecurityProfile {
        capabilities: capabilities::default_allowlist(),
        seccomp: Some(SeccompProfile {
            blocked: vec![SeccompSyscall::Chmod, SeccompSyscall::Fchmodat],
        }),
        user: None,
    };
    let (status, output) = spawn_with_security(profile);
    assert!(status.success());
    let chmod_rc: u32 = field(&output, "CHMOD_RC=").trim().parse().unwrap();
    assert_eq!(chmod_rc, 1, "chmod should be blocked with EPERM: {output}");
}

#[test]
fn privdrop_runs_as_container_user() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let profile = SecurityProfile {
        capabilities: capabilities::default_allowlist(),
        seccomp: None,
        user: Some(ContainerUser {
            uid: 1000,
            gid: 1000,
        }),
    };
    let (status, output) = spawn_with_security(profile);
    assert!(status.success());
    assert_eq!(field(&output, "UID="), "1000");
    assert_eq!(field(&output, "GID="), "1000");
}
