use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "sql",
    default_port: 1433,
    data_directory: "/var/opt/mssql/data",
    healthcheck_port: 1433,
    healthcheck_timeout_secs: 5,
    startup_command: &["sqlservr"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};
