#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::io::Read;

use tokedb_runtime::runtime::process::{spawn_isolated, CommandSpec};

#[test]
fn isolated_process_sees_itself_as_pid_one() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let spec = CommandSpec::new("/bin/sh")
        .arg("-c")
        .arg("echo $$")
        .kill_on_parent_exit(false);
    let mut spawned = spawn_isolated(&spec).unwrap();
    assert!(spawned.is_pid_one());
    let mut stdout = String::new();
    spawned
        .take_stdout()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout.trim(), "1");
    assert!(spawned.wait().unwrap().success());
}
