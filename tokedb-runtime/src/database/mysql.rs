use crate::database::DatabaseSpec;
use crate::isolation::ContainerUser;

pub const SPEC: DatabaseSpec = DatabaseSpec {
    engine: "mysql",
    default_port: 3306,
    data_directory: "/var/lib/mysql",
    healthcheck_port: 3306,
    healthcheck_timeout_secs: 5,
    startup_command: &["mysqld"],
    container_user: ContainerUser { uid: 999, gid: 999 },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_spec_is_static() {
        assert_eq!(SPEC.engine, "mysql");
        assert_eq!(SPEC.startup_command, &["mysqld"] as &[&str]);
    }
}
