//! Windows-to-WSL2 delegation: the Windows CLI is a thin client that
//! forwards every command to a tokedb binary inside a WSL2 distro, so
//! `start`/`stop`/`logs` behave exactly like they do on Linux. The Linux
//! binary owns all state; the Windows side only translates the working
//! directory and `TOKEDB_DATA_ROOT` into `/mnt/...` paths.

use std::path::Path;

use crate::error::{Result, RuntimeError};

const ENV_DISTRO: &str = "TOKEDB_WSL_DISTRO";
const ENV_BINARY: &str = "TOKEDB_WSL_BINARY";
const ENV_DATA_ROOT: &str = "TOKEDB_DATA_ROOT";

pub fn run_via_wsl(argv: &[String]) -> Result<()> {
    let distro = std::env::var(ENV_DISTRO).unwrap_or_else(|_| "Ubuntu-24.04".to_string());
    let binary = std::env::var(ENV_BINARY).unwrap_or_else(|_| "/usr/local/bin/tokedb".to_string());

    let cwd = std::env::current_dir().map_err(|err| {
        RuntimeError::Process(format!("could not read the working directory: {err}"))
    })?;
    let cwd_unix = to_unix_path(&cwd)?;

    let mut script = String::new();
    if let Ok(root) = std::env::var(ENV_DATA_ROOT) {
        if !root.trim().is_empty() {
            script.push_str(&format!(
                "export {ENV_DATA_ROOT}={}; ",
                shell_quote(&to_unix_path(Path::new(&root))?)
            ));
        }
    }
    script.push_str(&format!(
        "cd {} && exec {} ",
        shell_quote(&cwd_unix),
        shell_quote(&binary)
    ));
    for arg in argv {
        script.push_str(&shell_quote(&translate_arg(arg, &cwd)));
        script.push(' ');
    }

    let status = std::process::Command::new("wsl.exe")
        .arg("-d")
        .arg(&distro)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .map_err(|err| {
            RuntimeError::Process(format!(
                "could not start wsl.exe: {err} \
                 (install WSL2 and the `{distro}` distro, or set {ENV_DISTRO})"
            ))
        })?;

    match status.code() {
        Some(0) => Ok(()),
        Some(code) => Err(RuntimeError::Process(format!(
            "wsl backend exited with status {code} \
             (is tokedb installed in the distro? set {ENV_BINARY})"
        ))),
        None => Err(RuntimeError::Process(
            "wsl backend terminated without an exit status".to_string(),
        )),
    }
}

/// Converts an absolute Windows path (`C:\foo\bar`) into its WSL form
/// (`/mnt/c/foo/bar`). UNC paths are rejected.
pub fn to_unix_path(path: &Path) -> Result<String> {
    let raw = path.to_string_lossy();
    if raw.starts_with("\\\\") {
        return Err(RuntimeError::InvalidConfig(format!(
            "UNC path `{raw}` cannot be bridged into WSL"
        )));
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = raw[2..].replace('\\', "/");
        return Ok(format!("/mnt/{drive}{rest}"));
    }
    Err(RuntimeError::InvalidConfig(format!(
        "path `{raw}` is not an absolute Windows path"
    )))
}

fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Translates Windows paths in a command-line argument into their WSL form.
/// Absolute drive paths (`C:\...`) map straight to `/mnt/c/...`; relative
/// paths containing backslashes are joined with the Windows working
/// directory first. References (`mariadb:11.4`), URLs and plain names pass
/// through untouched.
fn translate_arg(arg: &str, cwd: &Path) -> String {
    let bytes = arg.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        to_unix_path(Path::new(arg)).unwrap_or_else(|_| arg.to_string())
    } else if arg.contains('\\') {
        to_unix_path(&cwd.join(arg)).unwrap_or_else(|_| arg.to_string())
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unix_path_translates_drive_letters() {
        assert_eq!(
            to_unix_path(Path::new("C:\\Users\\xpunt\\data")).unwrap(),
            "/mnt/c/Users/xpunt/data"
        );
        assert_eq!(
            to_unix_path(Path::new("D:/GitHub/tokedb")).unwrap(),
            "/mnt/d/GitHub/tokedb"
        );
    }

    #[test]
    fn to_unix_path_rejects_relative_and_unc_paths() {
        assert!(to_unix_path(Path::new("relative/path")).is_err());
        assert!(to_unix_path(Path::new("\\\\server\\share")).is_err());
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn translate_arg_maps_windows_paths_and_passes_through_others() {
        let cwd = Path::new("C:\\Users\\xpunt");
        assert_eq!(
            translate_arg("C:\\data\\bundle.tar.gz", cwd),
            "/mnt/c/data/bundle.tar.gz"
        );
        assert_eq!(
            translate_arg("..\\rel\\out.tar.gz", cwd),
            "/mnt/c/Users/xpunt/../rel/out.tar.gz"
        );
        assert_eq!(translate_arg("mariadb:11.4", cwd), "mariadb:11.4");
        assert_eq!(translate_arg("testdb", cwd), "testdb");
        assert_eq!(
            translate_arg("https://registry.example.com", cwd),
            "https://registry.example.com"
        );
    }
}
