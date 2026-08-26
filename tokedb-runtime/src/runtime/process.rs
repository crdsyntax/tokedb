use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::process::{ChildStderr, ChildStdout, ExitStatus};

#[cfg(target_os = "linux")]
use std::os::unix::process::ExitStatusExt;

use serde::{Deserialize, Serialize};

use crate::error::{Result, RuntimeError};
use crate::filesystem::RootfsPrep;
use crate::isolation::SecurityProfile;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub kill_on_parent_exit: bool,
    pub security: Option<SecurityProfile>,
    pub netns: bool,
    pub cwd: Option<PathBuf>,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        CommandSpec {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            kill_on_parent_exit: true,
            security: None,
            netns: false,
            cwd: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn kill_on_parent_exit(mut self, enabled: bool) -> Self {
        self.kill_on_parent_exit = enabled;
        self
    }

    pub fn security(mut self, profile: SecurityProfile) -> Self {
        self.security = Some(profile);
        self
    }

    pub fn netns(mut self, enabled: bool) -> Self {
        self.netns = enabled;
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSignal {
    Term,
    Kill,
}






#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct SpawnedProcess {
    container_pid: u32,
    helper_pid: i32,
    helper_reaped: bool,
    isolated: bool,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    status_pipe: Option<i32>,
    last_status: u32,
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
#[allow(dead_code)]
pub struct SpawnedProcess {
    isolated: bool,
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
impl SpawnedProcess {
    #[cfg(target_os = "linux")]
    pub fn host_pid(&self) -> u32 {
        self.container_pid
    }

    #[cfg(not(target_os = "linux"))]
    pub fn host_pid(&self) -> u32 {
        0
    }

    pub fn is_pid_one(&self) -> bool {
        self.isolated
    }

    pub fn wait(&mut self) -> Result<ExitStatus> {
        #[cfg(target_os = "linux")]
        {
            match self.reap_helper(true)? {
                Some(status) => Ok(status),
                None => Err(RuntimeError::Process(
                    "container never reported an exit status".to_string(),
                )),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(RuntimeError::UnsupportedPlatform("process wait"))
        }
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        #[cfg(target_os = "linux")]
        {
            self.reap_helper(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(RuntimeError::UnsupportedPlatform("process wait"))
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        self.signal(ProcessSignal::Term)
    }

    pub fn kill(&mut self) -> Result<()> {
        self.signal(ProcessSignal::Kill)
    }

    #[cfg(target_os = "linux")]
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.stdout.take()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        None
    }

    #[cfg(target_os = "linux")]
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }

    #[cfg(not(target_os = "linux"))]
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        None
    }

    #[cfg(target_os = "linux")]
    fn signal(&self, signal: ProcessSignal) -> Result<()> {
        use nix::sys::signal::{kill, Signal};

        let native = match signal {
            ProcessSignal::Term => Signal::SIGTERM,
            ProcessSignal::Kill => Signal::SIGKILL,
        };
        kill(
            nix::unistd::Pid::from_raw(self.container_pid as i32),
            native,
        )
        .map_err(RuntimeError::from)
    }

    #[cfg(not(target_os = "linux"))]
    fn signal(&self, _signal: ProcessSignal) -> Result<()> {
        Err(RuntimeError::UnsupportedPlatform("process signals"))
    }

    
    
    
    #[cfg(target_os = "linux")]
    fn reap_helper(&mut self, blocking: bool) -> Result<Option<ExitStatus>> {
        if self.helper_reaped {
            return Ok(Some(ExitStatus::from_raw(self.last_status as i32)));
        }
        let flags = if blocking { 0 } else { libc::WNOHANG };
        let mut status = 0i32;
        loop {
            let waited = unsafe { libc::waitpid(self.helper_pid, &mut status, flags) };
            if waited < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(RuntimeError::Process(format!("waitpid(helper): {err}")));
            }
            if waited == 0 {
                return Ok(None);
            }
            break;
        }
        self.helper_reaped = true;
        let status = self.read_status_pipe()?;
        self.last_status = status as u32;
        Ok(Some(ExitStatus::from_raw(status)))
    }

    #[cfg(target_os = "linux")]
    fn read_status_pipe(&mut self) -> Result<i32> {
        if let Some(fd) = self.status_pipe.take() {
            let mut raw = [0u8; 4];
            let mut total = 0usize;
            loop {
                let n = unsafe { libc::read(fd, raw[total..].as_mut_ptr().cast(), 4 - total) };
                if n <= 0 {
                    break;
                }
                total += n as usize;
                if total == 4 {
                    break;
                }
                if n == -1 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                }
            }
            unsafe { libc::close(fd) };
            if total == 4 {
                return Ok(i32::from_ne_bytes(raw));
            }
            return Err(RuntimeError::Process(
                "helper died without forwarding a container status".to_string(),
            ));
        }
        Err(RuntimeError::Process(
            "container status pipe already consumed".to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
impl Drop for SpawnedProcess {
    fn drop(&mut self) {
        if self.helper_reaped {
            return;
        }
        
        
        let _ = self.reap_helper(false);
        if !self.helper_reaped {
            let _ = self.signal(ProcessSignal::Kill);
            let _ = self.reap_helper(true);
        }
    }
}

pub fn spawn_isolated(spec: &CommandSpec) -> Result<SpawnedProcess> {
    spawn_with_prep(spec, None)
}

pub fn spawn_with_prep(spec: &CommandSpec, prep: Option<RootfsPrep>) -> Result<SpawnedProcess> {
    #[cfg(target_os = "linux")]
    {
        linux_sys::spawn_isolated_impl(spec, prep)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (spec, prep);
        Err(RuntimeError::UnsupportedPlatform(
            "process and filesystem isolation",
        ))
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux_sys {
    use std::io;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    use nix::sched::{unshare, CloneFlags};
    use nix::sys::prctl;
    use nix::sys::signal::Signal;

    use super::{CommandSpec, SpawnedProcess};
    use crate::error::{Result, RuntimeError};
    use crate::filesystem::{prepare_container_root, RootfsPrep};

    fn pipe2(cloexec: bool) -> Result<(i32, i32)> {
        let mut fds = [0i32; 2];
        let flags = if cloexec { libc::O_CLOEXEC } else { 0 };
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), flags) };
        if rc < 0 {
            return Err(RuntimeError::Process(format!(
                "pipe2: {}",
                io::Error::last_os_error()
            )));
        }
        Ok((fds[0], fds[1]))
    }

    pub(super) fn spawn_isolated_impl(
        spec: &CommandSpec,
        prep: Option<RootfsPrep>,
    ) -> crate::error::Result<SpawnedProcess> {
        let mut command = std::process::Command::new(&spec.program);
        command.args(&spec.args);
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        
        let (stdout_r, stdout_w) = pipe2(true)?;
        let (stderr_r, stderr_w) = pipe2(true)?;
        command.stdout(unsafe { Stdio::from_raw_fd(stdout_w) });
        command.stderr(unsafe { Stdio::from_raw_fd(stderr_w) });

        let kill_on_parent_exit = spec.kill_on_parent_exit;
        let security = spec.security.clone();
        let netns = spec.netns;
        unsafe {
            command.pre_exec(move || {
                let result = (|| -> crate::error::Result<()> {
                    if netns {
                        unshare(CloneFlags::CLONE_NEWNET).map_err(|err| {
                            RuntimeError::Process(format!("unshare(CLONE_NEWNET): {err}"))
                        })?;
                    }
                    unshare(CloneFlags::CLONE_NEWNS).map_err(|err| {
                        RuntimeError::Process(format!("unshare(CLONE_NEWNS): {err}"))
                    })?;
                    if let Some(prep) = &prep {
                        prepare_container_root(prep)?;
                    }
                    if kill_on_parent_exit {
                        prctl::set_pdeathsig(Some(Signal::SIGTERM)).map_err(RuntimeError::from)?;
                    }
                    if let Some(profile) = &security {
                        crate::isolation::apply_security(profile)?;
                    }
                    Ok(())
                })();
                result.map_err(io::Error::other)
            });
        }

        let (status_r, status_w) = pipe2(true)?;

        let helper = unsafe { libc::fork() };
        if helper < 0 {
            return Err(RuntimeError::Process(format!(
                "fork: {}",
                io::Error::last_os_error()
            )));
        }
        if helper == 0 {
            unsafe {
                helper_main(command, status_w, stdout_w, stderr_w);
            }
        }

        
        
        
        unsafe {
            let _ = libc::close(status_w);
        }

        
        let mut pid_bytes = [0u8; 4];
        let mut total = 0usize;
        loop {
            let n =
                unsafe { libc::read(status_r, pid_bytes[total..].as_mut_ptr().cast(), 4 - total) };
            if n > 0 {
                total += n as usize;
                if total == 4 {
                    break;
                }
                continue;
            }
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                unsafe {
                    let _ = libc::close(status_r);
                }
                return Err(RuntimeError::Process(format!("read(status pipe): {err}")));
            }
            
            let code = reap_helper_exit_code();
            return Err(match code {
                125 => RuntimeError::Process("unshare(CLONE_NEWPID) failed in helper".to_string()),
                126 => RuntimeError::Process("could not spawn container program".to_string()),
                other => RuntimeError::Process(format!(
                    "container helper exited prematurely with code {other}"
                )),
            });
        }
        let container_pid = u32::from_ne_bytes(pid_bytes);
        if container_pid == 0 {
            unsafe {
                let _ = libc::close(status_r);
            }
            return Err(RuntimeError::Process(
                "helper reported an invalid container pid".to_string(),
            ));
        }

        let stdout = unsafe { OwnedFd::from_raw_fd(stdout_r) }.into();
        let stderr = unsafe { OwnedFd::from_raw_fd(stderr_r) }.into();

        Ok(SpawnedProcess {
            container_pid,
            helper_pid: helper,
            helper_reaped: false,
            isolated: true,
            stdout: Some(stdout),
            stderr: Some(stderr),
            status_pipe: Some(status_r),
            last_status: 0,
        })
    }

    fn reap_helper_exit_code() -> i32 {
        let mut status = 0i32;
        loop {
            let waited = unsafe { libc::waitpid(-1, &mut status, 0) };
            if waited > 0 {
                break;
            }
            if waited < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
        }
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            127
        }
    }

    
    
    
    
    
    unsafe fn helper_main(
        mut command: std::process::Command,
        status_w: i32,
        stdout_w: i32,
        stderr_w: i32,
    ) -> ! {
        if let Err(_err) = unshare(CloneFlags::CLONE_NEWPID) {
            unsafe { libc::_exit(125) };
        }
        
        let child = match command.spawn() {
            Ok(child) => child,
            Err(_) => unsafe { libc::_exit(126) },
        };
        let pid = child.id() as i32;
        if pid <= 0 {
            unsafe { libc::_exit(126) };
        }
        
        let pid_bytes = (pid as u32).to_ne_bytes();
        unsafe { libc::write(status_w, pid_bytes.as_ptr().cast(), 4) };
        
        unsafe { libc::close(stdout_w) };
        unsafe { libc::close(stderr_w) };
        loop {
            let mut stat = 0i32;
            let waited = unsafe { libc::waitpid(-1, &mut stat, 0) };
            if waited == pid {
                let stat_bytes = stat.to_ne_bytes();
                unsafe { libc::write(status_w, stat_bytes.as_ptr().cast(), 4) };
                unsafe { libc::close(status_w) };
                unsafe { libc::_exit(0) };
            }
            if waited < 0 {
                
                
                let stat_bytes = stat.to_ne_bytes();
                unsafe { libc::write(status_w, stat_bytes.as_ptr().cast(), 4) };
                unsafe { libc::close(status_w) };
                unsafe { libc::_exit(0) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_spec_builds_typed_fields() {
        let spec = CommandSpec::new("/bin/sh")
            .arg("-c")
            .arg("echo hi")
            .env("A", "1")
            .env("B", "2")
            .kill_on_parent_exit(false);
        assert_eq!(spec.program, PathBuf::from("/bin/sh"));
        assert_eq!(spec.args, vec!["-c", "echo hi"]);
        assert_eq!(
            spec.env,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string())
            ]
        );
        assert!(!spec.kill_on_parent_exit);
        assert!(spec.security.is_none());
        assert!(!spec.netns);
        assert!(spec.cwd.is_none());
    }

    #[test]
    fn command_spec_accepts_security_profile() {
        let profile = crate::isolation::SecurityProfile::default();
        let spec = CommandSpec::new("/bin/sh").security(profile.clone());
        assert_eq!(spec.security, Some(profile));
    }

    #[test]
    fn command_spec_accepts_netns_flag() {
        let spec = CommandSpec::new("/bin/sh").netns(true);
        assert!(spec.netns);
        let spec = CommandSpec::new("/bin/sh").netns(false);
        assert!(!spec.netns);
    }

    #[test]
    fn command_spec_defaults_kill_on_parent_exit() {
        let spec = CommandSpec::new("/bin/true");
        assert!(spec.kill_on_parent_exit);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn spawn_isolated_rejects_other_platforms() {
        let err = spawn_isolated(&CommandSpec::new("/bin/true")).unwrap_err();
        assert!(matches!(err, RuntimeError::UnsupportedPlatform(_)));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn spawn_with_prep_rejects_other_platforms() {
        let err = spawn_with_prep(&CommandSpec::new("/bin/true"), None).unwrap_err();
        assert!(matches!(err, RuntimeError::UnsupportedPlatform(_)));
    }
}
