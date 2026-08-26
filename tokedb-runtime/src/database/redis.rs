use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "redis",
    default_port: 6379,
    data_directory: "/data",
    healthcheck_port: 6379,
    healthcheck_timeout_secs: 5,
    startup_command: &["redis-server"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};
