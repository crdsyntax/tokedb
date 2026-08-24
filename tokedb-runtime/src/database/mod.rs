use crate::isolation::ContainerUser;

pub mod mariadb;
pub mod mongodb;
pub mod mysql;
pub mod postgres;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseSpec {
    pub engine: &'static str,
    pub default_port: u16,
    pub data_directory: &'static str,
    pub healthcheck_port: u16,
    pub healthcheck_timeout_secs: u16,
    pub startup_command: &'static [&'static str],
    pub container_user: ContainerUser,
}

pub fn all() -> &'static [DatabaseSpec] {
    &[
        crate::database::mariadb::SPEC,
        crate::database::mysql::SPEC,
        crate::database::postgres::SPEC,
        crate::database::mongodb::SPEC,
    ]
}

pub fn for_engine(engine: &str) -> Option<DatabaseSpec> {
    all().iter().copied().find(|spec| spec.engine == engine)
}

pub fn engines() -> &'static [&'static str] {
    &["mariadb", "mysql", "postgres", "mongodb"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_engine_resolves_all_known_engines() {
        for engine in engines() {
            let spec = for_engine(engine).expect("known engine");
            assert_eq!(spec.engine, *engine);
            assert_ne!(spec.default_port, 0);
            assert_ne!(spec.healthcheck_port, 0);
            assert!(!spec.data_directory.is_empty());
            assert!(!spec.startup_command.is_empty());
        }
    }

    #[test]
    fn for_engine_returns_none_for_unknown() {
        assert!(for_engine("oracle").is_none());
        assert!(for_engine("").is_none());
    }

    #[test]
    fn specs_have_per_engine_ports_and_users() {
        let mariadb = for_engine("mariadb").unwrap();
        let postgres = for_engine("postgres").unwrap();
        let mongodb = for_engine("mongodb").unwrap();
        assert_eq!(mariadb.default_port, 3306);
        assert_eq!(postgres.default_port, 5432);
        assert_eq!(mongodb.default_port, 27017);
        assert_ne!(mariadb.data_directory, postgres.data_directory);
        assert_eq!(mariadb.container_user, ContainerUser { uid: 999, gid: 999 });
    }

    #[test]
    fn all_specs_never_run_as_root() {
        for spec in all() {
            assert_ne!(spec.container_user.uid, 0);
            assert_ne!(spec.container_user.gid, 0);
        }
    }
}
