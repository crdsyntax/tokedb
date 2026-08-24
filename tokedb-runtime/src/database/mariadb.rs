use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "mariadb",
    default_port: 3306,
    data_directory: "/var/lib/mysql",
    healthcheck_port: 3306,
    healthcheck_timeout_secs: 5,
    startup_command: &["mariadbd"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mariadb_spec_is_static() {
        assert_eq!(SPEC.engine, "mariadb");
        assert_eq!(SPEC.startup_command, &["mariadbd"] as &[&str]);
    }
}
