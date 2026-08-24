use crate::error::{Result, RuntimeError};

const DEFAULT_TAG: &str = "latest";

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(is_name_char)
        && !name.starts_with('.')
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.ends_with('-')
}

pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.chars().all(is_tag_char)
        && !tag.starts_with('.')
        && !tag.starts_with('-')
}

pub fn parse_reference(reference: &str) -> Result<(String, String)> {
    if reference.is_empty() {
        return Err(invalid(reference, "must not be empty"));
    }
    if reference.contains('/') || reference.contains('\\') || reference.contains('\0') {
        return Err(invalid(reference, "separators are not allowed"));
    }
    let (name, tag) = match reference.split_once(':') {
        Some((name, tag)) => (name, tag),
        None => (reference, DEFAULT_TAG),
    };
    if !valid_name(name) {
        return Err(invalid(reference, "invalid image name"));
    }
    if !valid_tag(tag) {
        return Err(invalid(reference, "invalid image tag"));
    }
    Ok((name.to_string(), tag.to_string()))
}

pub fn join_reference(name: &str, tag: &str) -> String {
    format!("{name}:{tag}")
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
}

fn is_tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn invalid(reference: &str, reason: &'static str) -> RuntimeError {
    RuntimeError::InvalidReference {
        reference: reference.to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_tag_to_latest() {
        assert_eq!(
            parse_reference("mariadb").unwrap(),
            ("mariadb".into(), "latest".into())
        );
        assert_eq!(
            parse_reference("mariadb:11").unwrap(),
            ("mariadb".into(), "11".into())
        );
    }

    #[test]
    fn parse_accepts_valid_references() {
        for reference in ["mariadb:11.4.2", "my-db_1:rc-2", "postgres:17beta1"] {
            assert!(parse_reference(reference).is_ok(), "{reference}");
        }
    }

    #[test]
    fn parse_rejects_invalid_references() {
        for reference in [
            "",
            "a/b",
            "a\\b",
            "a\0b",
            ":tag",
            "name:",
            "name:ta g",
            "NAME:11",
            "name:master:tag",
            "name:../x",
            ".name:1",
            "-name:1",
            "name.:1",
            "name:-",
            "name:.tag",
            "name:-tag",
        ] {
            let err = parse_reference(reference).unwrap_err();
            assert!(
                matches!(err, RuntimeError::InvalidReference { .. }),
                "{reference}"
            );
        }
    }

    #[test]
    fn join_reference_combines_name_and_tag() {
        assert_eq!(join_reference("mariadb", "11"), "mariadb:11");
    }

    #[test]
    fn valid_name_and_tag_rules() {
        assert!(valid_name("mariadb"));
        assert!(valid_name("my_db-1.2"));
        assert!(!valid_name(""));
        assert!(!valid_name("Mariadb"));
        assert!(!valid_name(".mariadb"));
        assert!(!valid_name("mariadb-"));
        assert!(valid_tag("11.4.2"));
        assert!(!valid_tag(""));
        assert!(!valid_tag(".x"));
        assert!(!valid_tag("-x"));
        assert!(!valid_tag("a!b"));
    }
}
