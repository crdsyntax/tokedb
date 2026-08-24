use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::cli::{arch_name, state_name};
use crate::config::RuntimeConfig;
use crate::error::{Result, RuntimeError};
use crate::runtime::{Container, ContainerState, ResourceLimits};
use crate::service::{CreateRequest, RuntimeService};

static INTERRUPT: AtomicBool = AtomicBool::new(false);

/// Labels of background tasks still running (detached start); drained on exit.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub fn run(config: &RuntimeConfig, command: Option<&str>) -> Result<()> {
    let service = RuntimeService::new(config.clone());
    match command {
        Some(script) => run_batch(&service, script),
        None if !std::io::stdin().is_terminal() => run_stdin_batch(&service),
        None => run_repl(&service),
    }
}

fn run_repl(service: &RuntimeService) -> Result<()> {
    let mut editor =
        DefaultEditor::new().map_err(|err| RuntimeError::Process(format!("console: {err}")))?;
    ctrlc::set_handler(|| INTERRUPT.store(true, Ordering::Relaxed))
        .map_err(|err| RuntimeError::Process(format!("console: {err}")))?;

    println!("tokedb console — manage database engine images, containers and volumes");
    println!("type `help` for commands, `exit` to quit");

    loop {
        match editor.readline("tokedb> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let _ = editor.add_history_entry(line);
                match execute(service, line) {
                    Ok(Outcome::Exit) => break,
                    Ok(Outcome::Continue) => {}
                    Err(err) => eprintln!("error: {err}"),
                }
            }
            Err(ReadlineError::Interrupted) => {
                INTERRUPT.store(false, Ordering::Relaxed);
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("console: {err}");
                break;
            }
        }
    }

    let pending = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !pending.is_empty() {
        eprintln!(
            "warning: {} background task(s) still running; exiting stops them",
            pending.len()
        );
    }
    Ok(())
}

/// Runs a `;`-separated script through the same parser as the interactive
/// console; stops at the first error and propagates it (typed exit code).
fn run_batch(service: &RuntimeService, script: &str) -> Result<()> {
    for raw in split_commands(script) {
        let command = raw.trim();
        if command.is_empty() {
            continue;
        }
        match execute(service, command)? {
            Outcome::Exit => break,
            Outcome::Continue => {}
        }
    }
    Ok(())
}

/// Reads command lines from stdin when the console is piped (non-TTY),
/// reusing the same parser and errors as the interactive console.
fn run_stdin_batch(service: &RuntimeService) -> Result<()> {
    use std::io::BufRead;
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|err| RuntimeError::Io {
            path: "<stdin>".to_string(),
            message: err.to_string(),
        })?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match execute(service, line)? {
            Outcome::Exit => break,
            Outcome::Continue => {}
        }
    }
    Ok(())
}

/// Splits a script on `;` outside double quotes, returning trimmed non-empty
/// commands.
fn split_commands(input: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ';' if !quoted => commands.push(std::mem::take(&mut current)),
            ch => current.push(ch),
        }
    }
    commands.push(current);
    commands
        .into_iter()
        .map(|command| command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect()
}

#[derive(Debug)]
pub enum Outcome {
    Continue,
    Exit,
}

pub fn execute(service: &RuntimeService, line: &str) -> Result<Outcome> {
    let tokens = tokenize(line)?;
    let Some(command) = tokens.first() else {
        return Ok(Outcome::Continue);
    };

    match command.as_str() {
        "help" => {
            print_help();
            Ok(Outcome::Continue)
        }
        "exit" | "quit" => Ok(Outcome::Exit),
        "images" => run_images(service),
        "list" => run_list(service),
        "pull" => run_pull(service, &tokens),
        "import" => run_import(service, &tokens),
        "export" => run_export(service, &tokens),
        "rmi" => run_rmi(service, &tokens),
        "create" => run_create(service, &tokens),
        "start" => run_start(service, &tokens),
        "stop" => run_stop(service, &tokens),
        "logs" => run_logs(service, &tokens),
        "watch" => run_watch(service, &tokens),
        "inspect" => run_inspect(service, &tokens),
        "volume" => run_volume(service, &tokens),
        "registry" => run_registry(service, &tokens),
        "config" => run_config(service, &tokens),
        "destroy" => run_destroy(service, &tokens),
        other => Err(RuntimeError::InvalidConfig(format!(
            "unknown command `{other}`; type `help`"
        ))),
    }
}

fn run_images(service: &RuntimeService) -> Result<Outcome> {
    for summary in service.images()? {
        println!(
            "{}  {}:{}  {}  {}  {} layer(s)",
            summary.reference,
            summary.database,
            summary.version,
            arch_name(summary.architecture),
            summary.digest,
            summary.layer_count
        );
    }
    Ok(Outcome::Continue)
}

fn run_list(service: &RuntimeService) -> Result<Outcome> {
    render_containers(&service.list()?);
    Ok(Outcome::Continue)
}

fn render_containers(containers: &[Container]) {
    for container in containers {
        let pid = container
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<8}  {:<20}  {:<12}  {:<9}  {}",
            container.id,
            container.name,
            container.image,
            colorize(container.state),
            pid
        );
    }
}

fn colorize(state: ContainerState) -> String {
    if !use_color() {
        return state_name(state).to_string();
    }
    let color = match state {
        ContainerState::Running => "32",
        ContainerState::Starting | ContainerState::Stopping => "33",
        ContainerState::Stopped | ContainerState::Destroyed => "31",
        ContainerState::Created => "36",
    };
    format!("\x1b[{color}m{}\x1b[0m", state_name(state))
}

fn use_color() -> bool {
    std::io::stdout().is_terminal()
}

/// Runs a bounded blocking operation on a background thread, animating a
/// spinner while it completes. Non-TTY output degrades to `title ...`.
fn spinner_wait<T: Send + 'static>(
    title: &str,
    op: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(op());
    });

    if !use_color() {
        println!("{title} ...");
        return rx
            .recv()
            .map_err(|_| RuntimeError::Process(format!("background task `{title}` aborted")))?;
    }

    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut frame = 0usize;
    loop {
        match rx.try_recv() {
            Ok(result) => {
                print!("\r\x1b[K");
                return result;
            }
            Err(TryRecvError::Empty) => {
                print!("\r{} {title}", frames[frame % frames.len()]);
                frame += 1;
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(TryRecvError::Disconnected) => {
                print!("\r\x1b[K");
                return Err(RuntimeError::Process(format!(
                    "background task `{title}` aborted"
                )));
            }
        }
    }
}

/// Runs an unbounded operation (e.g. a foreground `start`) on a detached
/// thread; the prompt returns immediately and the outcome is reported when the
/// task finishes. Exit-time process teardown kills in-flight container
/// processes via `kill_on_parent_exit`, so nothing is orphaned.
fn run_detached(label: String, op: impl FnOnce() -> Result<()> + Send + 'static) {
    PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(label.clone());
    std::thread::spawn(move || {
        let outcome = op();
        PENDING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|l| l != &label);
        match outcome {
            Ok(()) => println!("\n[done] {label}"),
            Err(err) => eprintln!("\n[error] {label}: {err}"),
        }
    });
}

fn run_pull(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "pull <reference> [--registry <source>]")?;
    let reference = tokens[1].clone();
    let registry = single_option(tokens, 2, "registry")?;
    let service = service.clone();
    let image = spinner_wait(&format!("pulling `{reference}`"), move || {
        service.pull(&reference, registry.as_deref())
    })?;
    println!(
        "pulled {} ({} layer(s), digest {})",
        image.reference,
        image.manifest.layers.len(),
        image.manifest.digest
    );
    Ok(Outcome::Continue)
}

fn run_import(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "import <path>")?;
    let image = service.import(Path::new(&tokens[1]))?;
    println!(
        "imported {} ({} layer(s), digest {})",
        image.reference,
        image.manifest.layers.len(),
        image.manifest.digest
    );
    Ok(Outcome::Continue)
}

fn run_export(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 3, "export <reference> <output>")?;
    service.export(&tokens[1], Path::new(&tokens[2]))?;
    println!("exported {} -> {}", tokens[1], tokens[2]);
    Ok(Outcome::Continue)
}

fn run_rmi(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "rmi <reference>")?;
    service.remove_image(&tokens[1])?;
    println!("removed {}", tokens[1]);
    Ok(Outcome::Continue)
}

fn run_create(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(
        tokens,
        3,
        "create <name> <image> [--memory-mb N] [--cpu-quota F] [--pids-max N] [--port HOST:CONTAINER]",
    )?;
    let mut memory_mb = None;
    let mut cpu_quota = None;
    let mut pids_max = None;
    let mut ports = Vec::new();
    let mut index = 3;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--memory-mb" => {
                index += 1;
                memory_mb = Some(parse_u64(tokens, index, "--memory-mb")?);
            }
            "--cpu-quota" => {
                index += 1;
                cpu_quota = Some(parse_f64(tokens, index, "--cpu-quota")?);
            }
            "--pids-max" => {
                index += 1;
                pids_max = Some(parse_u64(tokens, index, "--pids-max")?);
            }
            "--port" => {
                index += 1;
                ports.push(take_value(tokens, index, "--port")?.to_string());
            }
            other => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "unknown option `{other}`"
                )))
            }
        }
        index += 1;
    }

    let container = service.create(&CreateRequest {
        name: tokens[1].clone(),
        image: tokens[2].clone(),
        resources: ResourceLimits {
            memory_bytes: memory_mb.map(|mb| mb.saturating_mul(1024 * 1024)),
            cpu_quota,
            pids_max,
        },
        ports,
        env: Vec::new(),
        args: Vec::new(),
    })?;
    println!(
        "created container `{}` (id {}) with data volume `{}-data`",
        container.name, container.id, container.name
    );
    Ok(Outcome::Continue)
}

fn run_start(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "start <name>")?;
    service.inspect(&tokens[1])?;
    let name = tokens[1].clone();
    let service = service.clone();
    let label = format!("container `{name}` stopped");
    let task_name = name.clone();
    run_detached(label, move || service.start(&task_name));
    println!("starting `{name}` in the background; the prompt returns now");
    Ok(Outcome::Continue)
}

fn run_stop(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "stop <name>")?;
    let name = tokens[1].clone();
    let service = service.clone();
    let title = format!("stopping `{name}`");
    let task_name = name.clone();
    spinner_wait(&title, move || service.stop(&task_name))?;
    println!("stopped `{name}`");
    Ok(Outcome::Continue)
}

fn run_logs(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "logs <name> [-f|--follow]")?;
    let follow = tokens.iter().skip(2).any(|t| t == "-f" || t == "--follow");
    if follow {
        INTERRUPT.store(false, Ordering::Relaxed);
        follow_logs(service, &tokens[1], POLL_INTERVAL, || {
            INTERRUPT.load(Ordering::Relaxed)
        })?;
    } else {
        service.logs(&tokens[1])?;
    }
    Ok(Outcome::Continue)
}

fn follow_logs(
    service: &RuntimeService,
    name: &str,
    interval: Duration,
    stop: impl Fn() -> bool,
) -> Result<()> {
    let mut previous = String::new();
    loop {
        let logs = service.read_logs(name)?;
        let combined = format!("{}{}", logs.stdout, logs.stderr);
        for line in delta_lines(&previous, &combined) {
            println!("{line}");
        }
        previous = combined;
        std::thread::sleep(interval);
        if stop() {
            break;
        }
    }
    Ok(())
}

fn delta_lines<'a>(previous: &'a str, current: &'a str) -> Vec<&'a str> {
    let previous_lines: Vec<&str> = previous.lines().collect();
    let current_lines: Vec<&str> = current.lines().collect();
    let common = previous_lines
        .iter()
        .zip(&current_lines)
        .take_while(|(old, new)| old == new)
        .count();
    current_lines[common..].to_vec()
}

fn run_watch(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "watch list [--interval <secs>]")?;
    if tokens[1] != "list" {
        return Err(RuntimeError::InvalidConfig(format!(
            "unknown watch target `{}`; expected `list`",
            tokens[1]
        )));
    }
    let mut interval = Duration::from_secs(2);
    let mut index = 2;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "--interval" => {
                index += 1;
                interval = Duration::from_secs(parse_u64(tokens, index, "--interval")?.max(1));
            }
            other => {
                return Err(RuntimeError::InvalidConfig(format!(
                    "unknown option `{other}`"
                )))
            }
        }
        index += 1;
    }
    INTERRUPT.store(false, Ordering::Relaxed);
    watch_loop(service, interval, || INTERRUPT.load(Ordering::Relaxed));
    Ok(Outcome::Continue)
}

fn watch_loop(service: &RuntimeService, interval: Duration, stop: impl Fn() -> bool) {
    loop {
        match service.list() {
            Ok(containers) => {
                if use_color() {
                    print!("\x1b[2J\x1b[H");
                }
                render_containers(&containers);
            }
            Err(err) => {
                eprintln!("error: {err}");
                break;
            }
        }
        std::thread::sleep(interval);
        if stop() {
            break;
        }
    }
}

fn run_inspect(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "inspect <name>")?;
    let container = service.inspect(&tokens[1])?;
    println!(
        "{}",
        serde_json::to_string_pretty(&container).map_err(RuntimeError::from)?
    );
    Ok(Outcome::Continue)
}

fn run_destroy(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "destroy <name>")?;
    service.destroy(&tokens[1])?;
    println!(
        "destroyed container `{}` (data volume `{}-data` kept)",
        tokens[1], tokens[1]
    );
    Ok(Outcome::Continue)
}

fn run_volume(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "volume <list|create|remove|backup> ...")?;
    match tokens[1].as_str() {
        "list" => {
            for volume in service.volume_list()? {
                println!("{:<20}  {}", volume.name, volume.path.display());
            }
            Ok(Outcome::Continue)
        }
        "create" => {
            require(tokens, 3, "volume create <name>")?;
            let volume = service.volume_create(&tokens[2])?;
            println!(
                "created volume `{}` at {}",
                volume.name,
                volume.path.display()
            );
            Ok(Outcome::Continue)
        }
        "remove" => {
            require(tokens, 3, "volume remove <name>")?;
            service.volume_remove(&tokens[2])?;
            println!("removed volume `{}`", tokens[2]);
            Ok(Outcome::Continue)
        }
        "backup" => {
            require(tokens, 4, "volume backup <name> <dest-dir>")?;
            let archive = service.volume_backup(&tokens[2], Path::new(&tokens[3]))?;
            println!("backed up volume `{}` -> {}", tokens[2], archive.display());
            Ok(Outcome::Continue)
        }
        other => Err(RuntimeError::InvalidConfig(format!(
            "unknown volume command `{other}`"
        ))),
    }
}

fn run_registry(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "registry <list|publish> ...")?;
    match tokens[1].as_str() {
        "list" => {
            for entry in service.registry_list()? {
                println!(
                    "{:<20}  {}  {} layer(s)",
                    entry.reference,
                    entry.manifest_digest,
                    entry.layers.len()
                );
            }
            Ok(Outcome::Continue)
        }
        "publish" => {
            require(
                tokens,
                3,
                "registry publish <reference> [--registry <path>]",
            )?;
            let reference = tokens[2].clone();
            let registry = single_option(tokens, 3, "registry")?;
            service.registry_publish(&reference, registry.as_deref())?;
            println!("published {} to the local registry", reference);
            Ok(Outcome::Continue)
        }
        other => Err(RuntimeError::InvalidConfig(format!(
            "unknown registry command `{other}`"
        ))),
    }
}

fn run_config(service: &RuntimeService, tokens: &[String]) -> Result<Outcome> {
    require(tokens, 2, "config show")?;
    if tokens[1] != "show" {
        return Err(RuntimeError::InvalidConfig(format!(
            "unknown config command `{}`; expected `show`",
            tokens[1]
        )));
    }
    let config = service.config();
    println!("data root:     {}", config.data_root.display());
    println!("images dir:    {}", config.images_dir.display());
    println!("containers dir:{}", config.containers_dir.display());
    println!("volumes dir:   {}", config.volumes_dir.display());
    println!("bridge:        {}", config.bridge_name);
    Ok(Outcome::Continue)
}

fn require(tokens: &[String], minimum: usize, usage: &str) -> Result<()> {
    if tokens.len() < minimum {
        return Err(RuntimeError::InvalidConfig(format!("usage: {usage}")));
    }
    Ok(())
}

fn take_value<'a>(tokens: &'a [String], index: usize, option: &str) -> Result<&'a str> {
    tokens
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| RuntimeError::InvalidConfig(format!("option `--{option}` requires a value")))
}

fn parse_u64(tokens: &[String], index: usize, option: &str) -> Result<u64> {
    take_value(tokens, index, option)?
        .parse()
        .map_err(|_| RuntimeError::InvalidConfig(format!("invalid value for `--{option}`")))
}

fn parse_f64(tokens: &[String], index: usize, option: &str) -> Result<f64> {
    take_value(tokens, index, option)?
        .parse()
        .map_err(|_| RuntimeError::InvalidConfig(format!("invalid value for `--{option}`")))
}

fn single_option(tokens: &[String], start: usize, key: &str) -> Result<Option<String>> {
    let mut value = None;
    let mut index = start;
    while index < tokens.len() {
        if tokens[index].strip_prefix("--") == Some(key) {
            index += 1;
            value = Some(take_value(tokens, index, key)?.to_string());
        } else {
            return Err(RuntimeError::InvalidConfig(format!(
                "unexpected argument `{}`",
                tokens[index]
            )));
        }
        index += 1;
    }
    Ok(value)
}

fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => quoted = !quoted,
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch => current.push(ch),
        }
    }
    if quoted {
        return Err(RuntimeError::InvalidConfig(
            "unterminated quote".to_string(),
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn print_help() {
    println!("commands:");
    println!("  help                                    show this help");
    println!("  exit | quit                             leave the console");
    println!("  images                                  list database engine images");
    println!("  pull <reference> [--registry <source>]  fetch and verify an image (async spinner)");
    println!("  import <path>                           import an image bundle");
    println!("  export <reference> <output>             export an image bundle");
    println!("  rmi <reference>                         remove an image");
    println!("  create <name> <image> [--memory-mb N] [--cpu-quota F] [--pids-max N] [--port HOST:CONTAINER]");
    println!("  start <name>                            start a container in the background (Linux, root)");
    println!("  stop <name>                             stop a running container (async spinner)");
    println!("  logs <name> [-f|--follow]               print logs or follow live output (Ctrl+C to stop)");
    println!("  watch list [--interval <secs>]          refresh container states periodically (Ctrl+C to stop)");
    println!("  inspect <name>                          show container metadata as JSON");
    println!(
        "  destroy <name>                          remove a container (keeps its data volume)"
    );
    println!("  list                                    list containers");
    println!("  volume list | create <name> | remove <name> | backup <name> <dest-dir>");
    println!("  registry list | publish <reference> [--registry <path>]");
    println!("  config show                             show the runtime configuration");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RuntimeError;

    #[test]
    fn tokenize_splits_words_and_quotes() {
        assert_eq!(
            tokenize("create mi-db mariadb:11 --port \"18080:3306\"").unwrap(),
            vec!["create", "mi-db", "mariadb:11", "--port", "18080:3306"]
        );
        assert_eq!(tokenize("  images  ").unwrap(), vec!["images"]);
        assert_eq!(tokenize("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn tokenize_rejects_unterminated_quote() {
        assert!(matches!(
            tokenize("import \"foo"),
            Err(RuntimeError::InvalidConfig(_))
        ));
    }

    #[test]
    fn execute_help_and_exit() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        assert!(matches!(
            execute(&service, "help").unwrap(),
            Outcome::Continue
        ));
        assert!(matches!(execute(&service, "exit").unwrap(), Outcome::Exit));
        assert!(matches!(execute(&service, "quit").unwrap(), Outcome::Exit));
        assert!(matches!(
            execute(&service, "bogus"),
            Err(RuntimeError::InvalidConfig(_))
        ));
    }

    #[test]
    fn execute_create_requires_image_in_store() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let err = execute(&service, "create mi-db mariadb:11").unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn execute_usage_errors() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        for line in [
            "pull", "create", "start", "stop", "logs", "watch", "inspect", "destroy", "export",
            "import", "rmi", "volume", "registry", "config",
        ] {
            assert!(
                matches!(execute(&service, line), Err(RuntimeError::InvalidConfig(_))),
                "expected usage error for `{line}`"
            );
        }
    }

    #[test]
    fn execute_import_images_and_rmi_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let (manifest_path, bundle_path) = test_bundle(temp.path());
        execute(&service, &format!("import {}", bundle_path.display())).unwrap();
        assert!(manifest_path.exists());

        execute(&service, "images").unwrap();
        assert_eq!(service.images().unwrap().len(), 1);

        execute(&service, "rmi mariadb:11.4").unwrap();
        assert!(service.images().unwrap().is_empty());
    }

    #[test]
    fn execute_create_list_inspect_destroy_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        execute(&service, &format!("import {}", bundle_path.display())).unwrap();

        execute(
            &service,
            "create testdb mariadb:11.4 --memory-mb 256 --port 18080:3306",
        )
        .unwrap();

        let container = service.inspect("testdb").unwrap();
        assert_eq!(container.image, "mariadb:11.4");
        assert_eq!(container.resources.memory_bytes, Some(256 * 1024 * 1024));
        assert_eq!(container.ports[0].host_port, 18080);
        assert_eq!(container.ports[0].container_port, 3306);
        assert_eq!(container.volumes[0].name, "testdb-data");
        assert_eq!(container.volumes[0].mount_path, "/var/lib/mysql");

        execute(&service, "list").unwrap();
        execute(&service, "destroy testdb").unwrap();
        assert!(matches!(
            service.inspect("testdb"),
            Err(RuntimeError::ContainerNotFound { .. })
        ));
    }

    #[test]
    fn delta_lines_reports_only_new_content() {
        assert_eq!(delta_lines("", "a\nb\n"), vec!["a", "b"]);
        assert_eq!(delta_lines("a\nb\n", "a\nb\nc\n"), vec!["c"]);
        assert_eq!(delta_lines("a\nb\nc\n", "a\n"), Vec::<&str>::new());
        assert_eq!(delta_lines("a\nb\n", "a\nx\n"), vec!["x"]);
    }

    #[test]
    fn follow_logs_errors_for_unknown_container() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let stop = AtomicBool::new(true);
        assert!(matches!(
            follow_logs(&service, "missing", Duration::from_millis(1), || {
                stop.load(Ordering::Relaxed)
            }),
            Err(RuntimeError::ContainerNotFound { .. })
        ));
    }

    #[test]
    fn follow_logs_streams_existing_container() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        execute(&service, &format!("import {}", bundle_path.display())).unwrap();
        execute(&service, "create testdb mariadb:11.4").unwrap();
        let stop = AtomicBool::new(true);
        follow_logs(&service, "testdb", Duration::from_millis(1), || {
            stop.load(Ordering::Relaxed)
        })
        .unwrap();
    }

    #[test]
    fn execute_watch_usage_errors() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        for line in ["watch", "watch bogus", "watch list --interval"] {
            assert!(
                matches!(execute(&service, line), Err(RuntimeError::InvalidConfig(_))),
                "expected usage error for `{line}`"
            );
        }
    }

    #[test]
    fn spinner_wait_returns_result() {
        let value = spinner_wait("probe", || Ok::<u32, RuntimeError>(42)).unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn spinner_wait_reports_error() {
        let err = spinner_wait("probe", || {
            Err::<u32, _>(RuntimeError::InvalidConfig("boom".to_string()))
        })
        .unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
    }

    #[test]
    fn detached_task_reports_completion() {
        let (tx, rx) = std::sync::mpsc::channel();
        run_detached("probe".to_string(), move || {
            tx.send(()).unwrap();
            Ok(())
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn detached_task_is_removed_from_registry() {
        let label = format!("probe-{}", std::process::id());
        run_detached(label.clone(), || Ok(()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !PENDING
                .lock()
                .unwrap()
                .iter()
                .any(|pending| pending == &label)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("detached task was not removed from the registry");
    }

    #[test]
    fn execute_start_returns_immediately_for_unknown_container() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        assert!(matches!(
            execute(&service, "start missing"),
            Err(RuntimeError::ContainerNotFound { .. })
        ));
    }

    #[test]
    fn execute_volume_roundtrip_and_busy_backup() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));

        execute(&service, "volume create data").unwrap();
        let volumes = service.volume_list().unwrap();
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].name, "data");

        let backups = temp.path().join("backups");
        execute(
            &service,
            &format!("volume backup data {}", backups.display()),
        )
        .unwrap();
        assert!(backups.join("data.tar").is_file());

        let lock =
            crate::storage::VolumeLock::acquire(&temp.path().join("volumes"), "data").unwrap();
        assert!(matches!(
            service.volume_backup("data", &backups),
            Err(RuntimeError::VolumeBusy { ref name }) if name == "data"
        ));
        drop(lock);

        execute(&service, "volume remove data").unwrap();
        assert!(service.volume_list().unwrap().is_empty());
        assert!(matches!(
            execute(&service, "volume remove data"),
            Err(RuntimeError::VolumeNotFound { .. })
        ));
    }

    #[test]
    fn execute_registry_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        execute(&service, &format!("import {}", bundle_path.display())).unwrap();

        execute(&service, "registry publish mariadb:11.4").unwrap();
        let entries = service.registry_list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].reference, "mariadb:11.4");
    }

    #[test]
    fn execute_config_show_and_rejects_unknown_subcommands() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        assert!(matches!(
            execute(&service, "config show").unwrap(),
            Outcome::Continue
        ));
        assert!(matches!(
            execute(&service, "config bogus"),
            Err(RuntimeError::InvalidConfig(_))
        ));
        assert!(matches!(
            execute(&service, "volume bogus"),
            Err(RuntimeError::InvalidConfig(_))
        ));
        assert!(matches!(
            execute(&service, "registry bogus"),
            Err(RuntimeError::InvalidConfig(_))
        ));
    }

    #[test]
    fn split_commands_splits_outside_quotes() {
        assert_eq!(split_commands("images; list"), vec!["images", "list"]);
        assert_eq!(
            split_commands("create a b --port \"1;2\""),
            vec!["create a b --port \"1;2\""]
        );
        assert_eq!(split_commands(" ; images "), vec!["images"]);
        assert_eq!(split_commands(""), Vec::<String>::new());
    }

    #[test]
    fn batch_runs_sequentially_and_stops_on_error() {
        let temp = tempfile::tempdir().unwrap();
        let service =
            RuntimeService::new(crate::config::RuntimeConfig::new(temp.path().to_path_buf()));
        let err = run_batch(&service, "images; bogus; images").unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
        run_batch(&service, "images; exit; images").unwrap();
    }

    fn test_bundle(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use sha2::{Digest, Sha256};
        use std::io::Write;

        let layer_bytes = {
            let mut tar_bytes = Vec::new();
            {
                let mut builder = tar::Builder::new(&mut tar_bytes);
                let mut header = tar::Header::new_gnu();
                header.set_size(5);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, "db.txt", "hello".as_bytes())
                    .unwrap();
                builder.finish().unwrap();
            }
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&tar_bytes).unwrap();
            encoder.finish().unwrap()
        };
        let hex = format!("{:x}", Sha256::digest(&layer_bytes));

        let mut manifest = crate::image::manifest::ImageManifest {
            database: "mariadb".into(),
            version: "11.4".into(),
            architecture: crate::image::Architecture::Amd64,
            digest: String::new(),
            default_port: 3306,
            data_directory: "/var/lib/mysql".into(),
            healthcheck: crate::image::manifest::Healthcheck {
                port: 3306,
                timeout_secs: 5,
            },
            startup_command: vec!["mariadbd".into()],
            layers: vec![crate::image::manifest::LayerRef {
                digest: format!("sha256:{hex}"),
                size: layer_bytes.len() as u64,
            }],
        };
        manifest.digest = manifest.compute_digest().unwrap();

        let manifest_path = root.join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let layer_path = root.join(format!("{hex}.tar.gz"));
        std::fs::write(&layer_path, &layer_bytes).unwrap();

        let bundle_path = root.join("bundle.tar.gz");
        let file = std::fs::File::create(&bundle_path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_path_with_name(&manifest_path, "manifest.json")
            .unwrap();
        builder
            .append_path_with_name(&layer_path, format!("layers/{hex}.tar.gz"))
            .unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
        (manifest_path, bundle_path)
    }
}
