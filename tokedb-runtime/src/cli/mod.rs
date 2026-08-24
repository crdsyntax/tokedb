use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[cfg(any(not(windows), test))]
use crate::config::RuntimeConfig;
use crate::error::Result;
#[cfg(any(not(windows), test))]
use crate::error::RuntimeError;
#[cfg(any(not(windows), test))]
use crate::image::Architecture;
#[cfg(any(not(windows), test))]
use crate::runtime::{ContainerState, ResourceLimits};
#[cfg(any(not(windows), test))]
use crate::service::{CreateRequest, RuntimeService};

#[cfg(windows)]
pub mod wsl;

#[derive(Debug, Parser)]
#[command(
    name = "tokedb",
    version,
    about = "Container runtime for databases",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// Emit typed JSON DTOs instead of human-readable tables.
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Console(ConsoleArgs),
    Pull(PullArgs),
    Import(ImportArgs),
    Export(ExportArgs),
    Images,
    Rmi(RmiArgs),
    Create(CreateArgs),
    Start(NameArgs),
    Stop(NameArgs),
    Logs(LogsArgs),
    Inspect(NameArgs),
    Destroy(NameArgs),
    List,
}

#[derive(Debug, Args)]
pub struct ConsoleArgs {
    #[arg(
        short,
        long,
        value_name = "SCRIPT",
        help = "run semicolon-separated commands and exit"
    )]
    pub command: Option<String>,
}

#[derive(Debug, Args)]
pub struct PullArgs {
    pub reference: String,
    #[arg(long, value_name = "URL_OR_PATH")]
    pub registry: Option<String>,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    pub reference: String,
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct RmiArgs {
    pub reference: String,
}

#[derive(Debug, Args)]
pub struct NameArgs {
    pub name: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    pub name: String,
    pub image: String,
    #[arg(long)]
    pub memory_mb: Option<u64>,
    #[arg(long)]
    pub cpu_quota: Option<f64>,
    #[arg(long)]
    pub pids_max: Option<u64>,
    #[arg(
        long,
        value_name = "HOST:CONTAINER",
        help = "publish a port (HOST:CONTAINER or CONTAINER)"
    )]
    pub port: Vec<String>,
    #[arg(
        long,
        value_name = "KEY=VALUE",
        help = "set an environment variable on the engine (repeatable)"
    )]
    pub env: Vec<String>,
    #[arg(
        long,
        value_name = "ARG",
        help = "append an argument to the engine startup command (repeatable)"
    )]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    pub name: String,
    #[arg(short, long)]
    pub follow: bool,
}

pub fn run(cli: Cli) -> Result<()> {
    #[cfg(windows)]
    {
        // The Windows binary is a thin client: parse/validate locally (help,
        // version and argument errors work natively), then forward the raw
        // invocation to the Linux backend inside WSL2.
        let _ = cli;
        wsl::run_via_wsl(&std::env::args().skip(1).collect::<Vec<_>>())
    }
    #[cfg(not(windows))]
    {
        run_with_config(RuntimeConfig::from_env()?, cli)
    }
}

#[cfg(any(not(windows), test))]
fn run_with_config(config: RuntimeConfig, cli: Cli) -> Result<()> {
    let service = RuntimeService::new(config.clone());
    match &cli.command {
        Command::Console(args) => crate::console::run(&config, args.command.as_deref()),
        Command::Pull(args) => {
            let image = service.pull(&args.reference, args.registry.as_deref())?;
            println!(
                "pulled {} ({} layer(s), digest {})",
                image.reference,
                image.manifest.layers.len(),
                image.manifest.digest
            );
            Ok(())
        }
        Command::Import(args) => {
            let image = service.import(&args.path)?;
            println!(
                "imported {} ({} layer(s), digest {})",
                image.reference,
                image.manifest.layers.len(),
                image.manifest.digest
            );
            Ok(())
        }
        Command::Export(args) => {
            service.export(&args.reference, &args.output)?;
            println!("exported {} -> {}", args.reference, args.output.display());
            Ok(())
        }
        Command::Images => {
            let images = service.images()?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&images).map_err(RuntimeError::from)?
                );
            } else {
                for summary in &images {
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
            }
            Ok(())
        }
        Command::Rmi(args) => {
            service.remove_image(&args.reference)?;
            println!("removed {}", args.reference);
            Ok(())
        }
        Command::Create(args) => {
            let env = args
                .env
                .iter()
                .filter_map(|entry| {
                    let (key, value) = entry.split_once('=')?;
                    Some((key.to_string(), value.to_string()))
                })
                .collect::<Vec<_>>();
            let container = service.create(&CreateRequest {
                name: args.name.clone(),
                image: args.image.clone(),
                resources: ResourceLimits {
                    memory_bytes: args.memory_mb.map(|mb| mb.saturating_mul(1024 * 1024)),
                    cpu_quota: args.cpu_quota,
                    pids_max: args.pids_max,
                },
                ports: args.port.clone(),
                env,
                args: args.args.clone(),
            })?;
            println!(
                "created container `{}` (id {}) with data volume `{}-data`",
                container.name, container.id, container.name
            );
            Ok(())
        }
        Command::Start(args) => service.start(&args.name),
        Command::Stop(args) => service.stop(&args.name),
        Command::Logs(args) => {
            if args.follow {
                follow_logs(&service, &args.name)?;
            } else {
                service.logs(&args.name)?;
            }
            Ok(())
        }
        Command::Inspect(args) => {
            let container = service.inspect(&args.name)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&container).map_err(RuntimeError::from)?
            );
            Ok(())
        }
        Command::Destroy(args) => {
            service.destroy(&args.name)?;
            println!(
                "destroyed container `{}` (data volume `{}-data` kept)",
                args.name, args.name
            );
            Ok(())
        }
        Command::List => {
            let containers = service.list()?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&containers).map_err(RuntimeError::from)?
                );
            } else {
                for container in &containers {
                    let pid = container
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    println!(
                        "{:<8}  {:<20}  {:<12}  {:<9}  {}",
                        container.id,
                        container.name,
                        container.image,
                        state_name(container.state),
                        pid
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(any(not(windows), test))]
pub(crate) fn state_name(state: ContainerState) -> &'static str {
    match state {
        ContainerState::Created => "created",
        ContainerState::Starting => "starting",
        ContainerState::Running => "running",
        ContainerState::Stopping => "stopping",
        ContainerState::Stopped => "stopped",
        ContainerState::Destroyed => "destroyed",
    }
}

#[cfg(any(not(windows), test))]
pub(crate) fn arch_name(architecture: Architecture) -> &'static str {
    match architecture {
        Architecture::Amd64 => "amd64",
        Architecture::Arm64 => "arm64",
    }
}

/// Streams a container's logs to the terminal, printing only the deltas between
/// successive reads, until the process is interrupted (Ctrl+C). Reuses the same
/// `read_logs` service used by the console's live tail.
#[cfg(any(not(windows), test))]
fn follow_logs(service: &RuntimeService, name: &str) -> Result<()> {
    use std::io::Write;
    use std::time::Duration;

    let mut prev_out = String::new();
    let mut prev_err = String::new();
    loop {
        let logs = service.read_logs(name)?;
        if logs.stdout.len() > prev_out.len() {
            print!("{}", &logs.stdout[prev_out.len()..]);
            prev_out = logs.stdout;
        }
        if logs.stderr.len() > prev_err.len() {
            eprint!("{}", &logs.stderr[prev_err.len()..]);
            prev_err = logs.stderr;
        }
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::path::Path;

    use crate::error::RuntimeError;
    use crate::image::registry::LocalRegistry;
    use crate::runtime::ContainerStore;
    use crate::state::StateLayout;

    fn run_cli(root: &Path, args: &[&str]) -> Result<()> {
        let config = RuntimeConfig::new(root.to_path_buf());
        run_with_config(config, Cli::try_parse_from(args).unwrap())
    }

    fn test_bundle(root: &Path) -> (PathBuf, PathBuf) {
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

    #[test]
    fn cli_parses_all_subcommands() {
        for args in [
            vec!["tokedb", "console"],
            vec!["tokedb", "pull", "mariadb:11"],
            vec![
                "tokedb",
                "pull",
                "mariadb:11",
                "--registry",
                "https://registry.example.com",
            ],
            vec!["tokedb", "pull", "mariadb:11", "--registry", "./reg"],
            vec!["tokedb", "import", "imagen.tar.gz"],
            vec!["tokedb", "export", "mariadb:11", "out.tar.gz"],
            vec!["tokedb", "images"],
            vec!["tokedb", "rmi", "mariadb:11"],
            vec!["tokedb", "create", "mariadb-prod", "mariadb:11"],
            vec![
                "tokedb",
                "create",
                "mariadb-prod",
                "mariadb:11",
                "--memory-mb",
                "4096",
                "--cpu-quota",
                "2.0",
                "--pids-max",
                "100",
                "--port",
                "3306",
                "--port",
                "33060",
            ],
            vec!["tokedb", "start", "mariadb-prod"],
            vec!["tokedb", "stop", "mariadb-prod"],
            vec!["tokedb", "logs", "mariadb-prod"],
            vec!["tokedb", "logs", "mariadb-prod", "--follow"],
            vec!["tokedb", "inspect", "mariadb-prod"],
            vec!["tokedb", "destroy", "mariadb-prod"],
            vec!["tokedb", "list"],
        ] {
            assert!(
                Cli::try_parse_from(args.clone()).is_ok(),
                "failed: {:?}",
                args
            );
        }
    }

    #[test]
    fn cli_console_accepts_script_flag() {
        let cli = Cli::try_parse_from([
            "tokedb",
            "console",
            "-c",
            "create mi-db mariadb:11.4; start mi-db",
        ])
        .unwrap();
        match cli.command {
            Command::Console(args) => {
                assert_eq!(
                    args.command.as_deref(),
                    Some("create mi-db mariadb:11.4; start mi-db")
                );
            }
            _ => panic!("expected Console"),
        }
    }

    #[test]
    fn cli_json_flag_parses_before_and_after_subcommand() {
        assert!(
            Cli::try_parse_from(["tokedb", "images", "--json"])
                .unwrap()
                .json
        );
        assert!(
            Cli::try_parse_from(["tokedb", "--json", "list"])
                .unwrap()
                .json
        );
        let cli = Cli::try_parse_from(["tokedb", "list"]).unwrap();
        assert!(!cli.json);
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        let err = Cli::try_parse_from(["tokedb", "frobnicate"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn cli_rejects_missing_arguments() {
        let err = Cli::try_parse_from(["tokedb", "pull"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn run_create_rejects_missing_image() {
        let cli = Cli::try_parse_from(["tokedb", "create", "x", "mariadb:11"]).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let err = run_with_config(RuntimeConfig::new(temp.path().to_path_buf()), cli).unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn run_rejects_unsafe_names() {
        let cli = Cli::try_parse_from(["tokedb", "create", "a/b", "mariadb:11"]).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let err = run_with_config(RuntimeConfig::new(temp.path().to_path_buf()), cli).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidName { .. }));
    }

    #[test]
    fn run_create_list_inspect_logs_destroy_roundtrip() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();

        run_cli(
            temp.path(),
            &[
                "tokedb",
                "create",
                "testdb",
                "mariadb:11.4",
                "--memory-mb",
                "256",
                "--port",
                "18080:3306",
            ],
        )
        .unwrap();

        let layout = StateLayout::new(RuntimeConfig::new(temp.path().to_path_buf()));
        let containers = ContainerStore::new(layout);
        let container = containers.find("testdb").unwrap();
        assert_eq!(container.image, "mariadb:11.4");
        assert_eq!(container.resources.memory_bytes, Some(256 * 1024 * 1024));
        assert_eq!(container.ports.len(), 1);
        assert_eq!(container.ports[0].host_port, 18080);
        assert_eq!(container.ports[0].container_port, 3306);
        assert_eq!(container.volumes[0].name, "testdb-data");
        assert_eq!(container.volumes[0].mount_path, "/var/lib/mysql");

        assert!(temp.path().join("volumes").join("testdb-data").is_dir());

        run_cli(temp.path(), &["tokedb", "list"]).unwrap();
        run_cli(temp.path(), &["tokedb", "inspect", "testdb"]).unwrap();
        run_cli(temp.path(), &["tokedb", "logs", "testdb"]).unwrap();
        run_cli(temp.path(), &["tokedb", "destroy", "testdb"]).unwrap();

        let err = containers.find("testdb").unwrap_err();
        assert!(matches!(err, RuntimeError::ContainerNotFound { .. }));
    }

    #[test]
    fn run_create_rejects_bad_port_binding() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();

        let err = run_cli(
            temp.path(),
            &[
                "tokedb",
                "create",
                "testdb",
                "mariadb:11.4",
                "--port",
                "banana",
            ],
        )
        .unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn run_start_rejects_unsupported_platform() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();
        run_cli(temp.path(), &["tokedb", "create", "testdb", "mariadb:11.4"]).unwrap();

        let err = run_cli(temp.path(), &["tokedb", "start", "testdb"]).unwrap_err();
        assert!(matches!(err, RuntimeError::UnsupportedPlatform(_)));

        run_cli(temp.path(), &["tokedb", "destroy", "testdb"]).unwrap();
    }

    #[test]
    fn run_import_and_rmi_roundtrip() {
        let temp = tempfile::tempdir().unwrap();

        let (manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();
        assert!(manifest_path.exists());

        run_cli(temp.path(), &["tokedb", "rmi", "mariadb:11.4"]).unwrap();
        let err = run_cli(temp.path(), &["tokedb", "rmi", "mariadb:11.4"]).unwrap_err();
        assert!(matches!(err, RuntimeError::ImageNotFound { .. }));
    }

    #[test]
    fn run_export_creates_bundle() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();

        let out = temp.path().join("exported.tar.gz");
        run_cli(
            temp.path(),
            &["tokedb", "export", "mariadb:11.4", out.to_str().unwrap()],
        )
        .unwrap();
        assert!(out.is_file());
    }

    #[test]
    fn run_pull_from_local_registry() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();

        let store = crate::image::ImageStore::new(temp.path().join("images"));
        let image = store.get("mariadb:11.4").unwrap();
        let registry = LocalRegistry::new(temp.path().join("registry"));
        registry.publish(&image).unwrap();

        run_cli(temp.path(), &["tokedb", "rmi", "mariadb:11.4"]).unwrap();

        run_cli(
            temp.path(),
            &[
                "tokedb",
                "pull",
                "mariadb:11.4",
                "--registry",
                temp.path().join("registry").to_str().unwrap(),
            ],
        )
        .unwrap();

        let pulled = store.get("mariadb:11.4").unwrap();
        store.verify(&pulled.reference).unwrap();
        assert_eq!(pulled.manifest.database, "mariadb");
    }

    #[test]
    fn run_pull_uses_default_local_registry() {
        let temp = tempfile::tempdir().unwrap();

        let (_manifest_path, bundle_path) = test_bundle(temp.path());
        run_cli(
            temp.path(),
            &["tokedb", "import", bundle_path.to_str().unwrap()],
        )
        .unwrap();

        let store = crate::image::ImageStore::new(temp.path().join("images"));
        let image = store.get("mariadb:11.4").unwrap();
        let registry = LocalRegistry::new(temp.path().join("registry"));
        registry.publish(&image).unwrap();

        run_cli(temp.path(), &["tokedb", "rmi", "mariadb:11.4"]).unwrap();
        run_cli(temp.path(), &["tokedb", "pull", "mariadb:11.4"]).unwrap();

        let pulled = store.get("mariadb:11.4").unwrap();
        store.verify(&pulled.reference).unwrap();
    }

    #[test]
    fn run_pull_rejects_bad_registry_url() {
        let temp = tempfile::tempdir().unwrap();
        let err = run_cli(
            temp.path(),
            &["tokedb", "pull", "mariadb:11", "--registry", "ftp://nope"],
        )
        .unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));
    }
}
