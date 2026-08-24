#![cfg(all(target_os = "linux", feature = "integration-linux"))]

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};

use tokedb_runtime::isolation::{CgroupManager, ResourceLimits};

const BASE: &str = "/sys/fs/cgroup/tokedb-test";

const ALLOCATOR_C: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(void) {
    sleep(2);
    size_t sz = 64UL * 1024 * 1024;
    char *p = malloc(sz);
    if (!p) { puts("MALLOC_FAILED"); return 2; }
    puts("STARTED");
    fflush(stdout);
    volatile char *vp = p;
    for (size_t i = 0; i < sz; i += 4096) vp[i] = 1;
    pause();
    return 0;
}
"#;

const FORKER_C: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
int main(void) {
    sleep(2);
    int pids[512];
    int count = 0;
    for (; count < 512; count++) {
        pid_t pid = fork();
        if (pid < 0) break;
        if (pid == 0) { sleep(60); _exit(0); }
        pids[count] = pid;
    }
    printf("COUNT %d\n", count);
    fflush(stdout);
    for (int i = 0; i < count; i++) kill(pids[i], SIGKILL);
    while (count > 0) { wait(NULL); count--; }
    return 0;
}
"#;

fn cgroup_supported() -> bool {
    if !nix::unistd::Uid::effective().is_root() {
        eprintln!("skipping: requires root");
        return false;
    }
    let controllers = match fs::read_to_string("/sys/fs/cgroup/cgroup.controllers") {
        Ok(contents) => contents,
        Err(_) => {
            eprintln!("skipping: cgroup v2 not available");
            return false;
        }
    };
    let has = |name: &str| {
        controllers
            .lines()
            .any(|line| line.split_whitespace().any(|c| c == name))
    };
    if !(has("memory") && has("pids")) {
        eprintln!("skipping: memory or pids controller not available");
        return false;
    }
    true
}

fn compile_c(source: &str, out: &Path) -> bool {
    let status = Command::new("gcc")
        .arg("-O1")
        .arg("-x")
        .arg("c")
        .arg("-")
        .arg("-o")
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(source.as_bytes())?;
            child.wait()
        });
    matches!(status, Ok(status) if status.success())
}

fn unique_name(test: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{test}-{}-{nanos}", std::process::id())
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    seconds: u64,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("child did not exit within {seconds}s"),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn memory_max_kills_allocator() {
    if !cgroup_supported() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let allocator = work.path().join("alloc");
    if !compile_c(ALLOCATOR_C, &allocator) {
        eprintln!("skipping: gcc not available");
        return;
    }

    let name = unique_name("mem");
    let manager = CgroupManager::new(BASE);
    manager.create(&name).unwrap();
    manager
        .apply(
            &name,
            &ResourceLimits {
                memory_bytes: Some(16 * 1024 * 1024),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let mut child = Command::new(&allocator)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    manager.attach(&name, child.id()).unwrap();

    let mut line = String::new();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    reader.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "STARTED");

    let status = wait_with_timeout(&mut child, 30).expect("allocator should be killed by OOM");
    drop(reader);
    let _ = manager.remove(&name);
    let _ = fs::remove_dir(BASE);

    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(9),
        "expected SIGKILL from OOM, got {status:?}"
    );
}

#[test]
fn pids_max_limits_forks() {
    if !cgroup_supported() {
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let forker = work.path().join("forker");
    if !compile_c(FORKER_C, &forker) {
        eprintln!("skipping: gcc not available");
        return;
    }

    let name = unique_name("pids");
    let manager = CgroupManager::new(BASE);
    manager.create(&name).unwrap();
    manager
        .apply(
            &name,
            &ResourceLimits {
                pids_max: Some(32),
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let mut child = Command::new(&forker)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    manager.attach(&name, child.id()).unwrap();

    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let status = wait_with_timeout(&mut child, 30).expect("forker should exit");
    let _ = manager.remove(&name);
    let _ = fs::remove_dir(BASE);

    assert!(status.success());
    let count = output
        .lines()
        .find_map(|line| line.strip_prefix("COUNT "))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(usize::MAX as u32);
    assert!(count < 512, "forks were not limited: {output}");
}
