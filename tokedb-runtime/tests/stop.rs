#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::io::{BufRead, BufReader};

use tokedb_runtime::runtime::process::{spawn_isolated, CommandSpec};

#[test]
fn isolated_process_responds_to_stop_signal() {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let spec = CommandSpec::new("/bin/sh")
        .arg("-c")
        .arg("trap 'exit 42' TERM; echo READY; sleep 5")
        .kill_on_parent_exit(false);
    let mut spawned = spawn_isolated(&spec).unwrap();
    assert!(spawned.is_pid_one());

    let mut line = String::new();
    BufReader::new(spawned.take_stdout().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "READY");

    spawned.stop().unwrap();
    assert!(!spawned.wait().unwrap().success());
}
