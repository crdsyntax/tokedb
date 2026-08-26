use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "sqlite",
    default_port: 54321,
    data_directory: "/var/lib/sqlite",
    healthcheck_port: 54321,
    healthcheck_timeout_secs: 5,
    startup_command: &["sqlite3"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};
