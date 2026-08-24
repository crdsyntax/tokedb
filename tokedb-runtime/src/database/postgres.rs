use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "postgres",
    default_port: 5432,
    data_directory: "/var/lib/postgresql/data",
    healthcheck_port: 5432,
    healthcheck_timeout_secs: 5,
    startup_command: &["postgres"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_spec_is_static() {
        assert_eq!(SPEC.engine, "postgres");
        assert_eq!(SPEC.startup_command, &["postgres"] as &[&str]);
    }
}
