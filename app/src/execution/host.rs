use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

use crate::sessions::Job;

pub(crate) const COMMAND_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(120)
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostIdentity {
    pub(crate) username: String,
    pub(crate) uid: u32,
    pub(crate) euid: u32,
}

impl HostIdentity {
    pub(crate) fn current() -> Self {
        current_identity()
    }

    pub(crate) fn elevated(&self) -> bool {
        self.euid == 0 || self.euid != self.uid
    }

    pub(crate) fn authority_summary(&self) -> String {
        if self.elevated() {
            format!(
                "Commands run as {} (uid {}, effective uid {}). This process is already elevated. Power Plant adds no privileges of its own.",
                self.username, self.uid, self.euid
            )
        } else {
            format!(
                "Commands run as {} (uid {}). They have that user's existing host authority. Power Plant adds no privileges of its own.",
                self.username, self.uid
            )
        }
    }
}

pub(crate) fn command_directory(directories: &[super::DirectoryGrant]) -> PathBuf {
    directories
        .first()
        .map(|grant| grant.host_path.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

pub(crate) async fn run_shell(
    command: &str,
    directory: &Path,
    job: &Job,
    timeout: Duration,
) -> Result<String, &'static str> {
    if job.cancel_requested() {
        return Err("Stopped.");
    }
    if command.is_empty() || command.contains('\0') {
        return Err("Enter a command.");
    }
    if !directory.is_absolute() {
        return Err("The command directory is not valid.");
    }
    let metadata =
        std::fs::metadata(directory).map_err(|_| "The command directory is not available.")?;
    if !metadata.is_dir() {
        return Err("The command directory is not available.");
    }
    let mut child = Command::new("/bin/sh");
    child
        .arg("-c")
        .arg(command)
        .current_dir(directory)
        .env_clear()
        .envs(sanitised_environment())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        child.process_group(0);
    }
    let mut child = child
        .spawn()
        .map_err(|_| "Power Plant could not start the command. Try again.")?;
    // The group outlives the shell when a descendant retains an output pipe.
    let _group = ProcessGroup(child.id());
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut stdout_text = String::new();
    let mut stderr_text = String::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut status = None;
    loop {
        if status.is_some() && stdout.is_none() && stderr.is_none() {
            break;
        }
        tokio::select! {
            biased;
            _ = job.cancelled() => {
                terminate(&mut child);
                let _ = child.wait().await;
                return Err("Stopped.");
            }
            _ = tokio::time::sleep_until(deadline) => {
                terminate(&mut child);
                let _ = child.wait().await;
                return Err("The command exceeded the time limit.");
            }
            result = read_pipe(stdout.as_mut(), &mut stdout_text) => {
                let count = result?;
                if count.is_none() {
                    stdout = None;
                }
                if count == Some(true) {
                    terminate(&mut child);
                    let _ = child.wait().await;
                    let mut output = merge_output(stdout_text, stderr_text);
                    crate::tools::mark_truncated(&mut output);
                    return Ok(output);
                }
            }
            result = read_pipe(stderr.as_mut(), &mut stderr_text) => {
                let count = result?;
                if count.is_none() {
                    stderr = None;
                }
                if count == Some(true) {
                    terminate(&mut child);
                    let _ = child.wait().await;
                    let mut output = merge_output(stdout_text, stderr_text);
                    crate::tools::mark_truncated(&mut output);
                    return Ok(output);
                }
            }
            result = child.wait(), if status.is_none() => {
                status = Some(result.map_err(|_| "Power Plant lost the command result. Try again.")?);
            }
        }
    }
    let status = status.ok_or("Power Plant lost the command result. Try again.")?;
    let mut output = merge_output(stdout_text, stderr_text);
    if output.len() > tools_limit() {
        crate::tools::mark_truncated(&mut output);
        return Ok(output);
    }
    match status.code() {
        Some(0) => Ok(empty_output(output)),
        None => Err("Power Plant lost the command result. Try again."),
        Some(code) => {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&format!("The command exited with code {code}."));
            if output.len() > tools_limit() {
                crate::tools::mark_truncated(&mut output);
            }
            Ok(output)
        }
    }
}

fn tools_limit() -> usize {
    crate::tools::MAXIMUM_TOOL_BYTES
}

fn empty_output(output: String) -> String {
    if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output
    }
}

pub(super) fn sanitised_environment() -> Vec<(String, String)> {
    let mut env = Vec::new();
    push_inherited(&mut env, "PATH", "/usr/local/bin:/usr/bin:/bin");
    push_inherited(&mut env, "HOME", "");
    push_inherited(&mut env, "USER", "");
    push_inherited(&mut env, "LOGNAME", "");
    push_inherited(&mut env, "LANG", "C.UTF-8");
    push_inherited(&mut env, "LC_ALL", "");
    env.push(("TERM".to_owned(), "dumb".to_owned()));
    env.push(("TMPDIR".to_owned(), "/tmp".to_owned()));
    env
}

fn push_inherited(env: &mut Vec<(String, String)>, key: &str, fallback: &str) {
    match std::env::var(key) {
        Ok(value) if !value.is_empty() => env.push((key.to_owned(), value)),
        _ if !fallback.is_empty() => env.push((key.to_owned(), fallback.to_owned())),
        _ => {}
    }
}

struct ProcessGroup(Option<u32>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(-(pid as i32)),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

fn terminate(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-(pid as i32)),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.start_kill();
}

fn merge_output(stdout: String, stderr: String) -> String {
    let mut output = stdout;
    output.push_str(&stderr);
    output
}

async fn read_pipe<R: AsyncRead + Unpin>(
    pipe: Option<&mut R>,
    output: &mut String,
) -> Result<Option<bool>, &'static str> {
    let Some(pipe) = pipe else {
        return std::future::pending().await;
    };
    let mut buffer = [0_u8; 4096];
    let count = pipe
        .read(&mut buffer)
        .await
        .map_err(|_| "Power Plant lost the command result. Try again.")?;
    if count == 0 {
        return Ok(None);
    }
    Ok(Some(append_output(output, &buffer[..count])))
}

fn append_output(output: &mut String, bytes: &[u8]) -> bool {
    let piece = String::from_utf8_lossy(bytes);
    let remaining = crate::tools::MAXIMUM_TOOL_BYTES.saturating_sub(output.len());
    if piece.len() <= remaining {
        output.push_str(&piece);
        return false;
    }
    let mut end = remaining;
    while end > 0 && !piece.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&piece[..end]);
    true
}

#[cfg(unix)]
fn current_identity() -> HostIdentity {
    let uid = nix::unistd::getuid();
    let euid = nix::unistd::geteuid();
    let username = nix::unistd::User::from_uid(euid)
        .ok()
        .flatten()
        .map(|user| user.name)
        .unwrap_or_else(|| format!("uid {}", uid.as_raw()));
    HostIdentity {
        username,
        uid: uid.as_raw(),
        euid: euid.as_raw(),
    }
}

#[cfg(not(unix))]
fn current_identity() -> HostIdentity {
    HostIdentity {
        username: "unknown".to_owned(),
        uid: 0,
        euid: 0,
    }
}

#[cfg(test)]
mod tests;
