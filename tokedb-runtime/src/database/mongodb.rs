use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "mongodb",
    default_port: 27017,
    data_directory: "/data/db",
    healthcheck_port: 27017,
    healthcheck_timeout_secs: 5,
    startup_command: &["mongod"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mongodb_spec_is_static() {
        assert_eq!(SPEC.engine, "mongodb");
        assert_eq!(SPEC.startup_command, &["mongod"] as &[&str]);
    }
}
