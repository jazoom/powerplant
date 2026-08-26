use rig_core::completion::ToolDefinition;
use serde::Deserialize;

use crate::sandbox::{GUEST_PROJECT, GuestExec, GuestSandbox};
use crate::sessions::Job;

pub(super) const MAXIMUM_TOOL_BYTES: usize = 64 * 1024;
pub(super) const MAXIMUM_WRITE_BYTES: usize = 256 * 1024;
pub(super) const MAXIMUM_COMMAND_BYTES: usize = 32_768;

#[derive(Clone, Copy)]
pub(super) enum ToolKind {
    List,
    Read,
    Write,
    Run,
    GitStatus,
    GitDiff,
    GitCommit,
}

impl ToolKind {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "list" => Some(Self::List),
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "run" => Some(Self::Run),
            "git_status" => Some(Self::GitStatus),
            "git_diff" => Some(Self::GitDiff),
            "git_commit" => Some(Self::GitCommit),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Read => "read",
            Self::Write => "write",
            Self::Run => "run",
            Self::GitStatus => "git_status",
            Self::GitDiff => "git_diff",
            Self::GitCommit => "git_commit",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::List => "List files in a project directory.",
            Self::Read => "Read a project file.",
            Self::Write => "Write a project file. Creates parent directories.",
            Self::Run => "Run a shell command in the project directory.",
            Self::GitStatus => "Show git status for the project.",
            Self::GitDiff => "Show the git diff for the project or one path.",
            Self::GitCommit => "Stage every change in the project and create a commit.",
        }
    }

    fn parameters(self) -> serde_json::Value {
        match self {
            Self::List | Self::Read => serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path inside /project. Defaults to /project."
                    }
                },
                "additionalProperties": false
            }),
            Self::Write => serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path inside /project."
                    },
                    "contents": {
                        "type": "string",
                        "description": "File contents."
                    }
                },
                "required": ["path", "contents"],
                "additionalProperties": false
            }),
            Self::Run => serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run in /project."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            Self::GitStatus => serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            Self::GitDiff => serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional path inside /project."
                    }
                },
                "additionalProperties": false
            }),
            Self::GitCommit => serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Commit message."
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        }
    }
}

pub(super) fn definitions() -> Vec<ToolDefinition> {
    [
        ToolKind::List,
        ToolKind::Read,
        ToolKind::Write,
        ToolKind::Run,
        ToolKind::GitStatus,
        ToolKind::GitDiff,
        ToolKind::GitCommit,
    ]
    .into_iter()
    .map(|kind| ToolDefinition {
        name: kind.name().to_owned(),
        description: kind.description().to_owned(),
        parameters: kind.parameters(),
    })
    .collect()
}

pub(super) struct ToolTrace {
    pub(super) label: String,
    pub(super) output: String,
}

pub(super) async fn invoke(
    sandbox: &GuestSandbox,
    name: &str,
    arguments: &serde_json::Value,
    job: &Job,
) -> ToolTrace {
    let Some(kind) = ToolKind::parse(name) else {
        return ToolTrace {
            label: name.to_owned(),
            output: "Unknown tool.".to_owned(),
        };
    };
    match dispatch(sandbox, kind, arguments, job).await {
        Ok((label, output)) => ToolTrace { label, output },
        Err(message) => ToolTrace {
            label: kind.name().to_owned(),
            output: message.to_owned(),
        },
    }
}

async fn dispatch(
    sandbox: &GuestSandbox,
    kind: ToolKind,
    arguments: &serde_json::Value,
    job: &Job,
) -> Result<(String, String), &'static str> {
    match kind {
        ToolKind::List => {
            let args: PathArgs = parse_args(arguments)?;
            let path = guest_path(&args.path)?;
            let output = capture(
                sandbox,
                confined_existing_command(&path, "ls", &["-la"]),
                job,
            )
            .await?;
            Ok((format!("list `{path}`"), output))
        }
        ToolKind::Read => {
            let args: PathArgs = parse_args(arguments)?;
            let path = guest_path(&args.path)?;
            if path == GUEST_PROJECT {
                return Err("Choose a file to read.");
            }
            let maximum = MAXIMUM_TOOL_BYTES.to_string();
            let output = capture(
                sandbox,
                confined_existing_command(&path, "head", &["-c", &maximum]),
                job,
            )
            .await?;
            Ok((format!("read `{path}`"), output))
        }
        ToolKind::Write => {
            let args: WriteArgs = parse_args(arguments)?;
            let path = guest_path(&args.path)?;
            if path == GUEST_PROJECT {
                return Err("Choose a file to write.");
            }
            if args.contents.len() > MAXIMUM_WRITE_BYTES {
                return Err("That file is too large to write.");
            }
            let output = capture(
                sandbox,
                confined_write_command(&path).with_stdin(args.contents.into_bytes()),
                job,
            )
            .await?;
            let body = if output.trim().is_empty() {
                "Wrote the file.".to_owned()
            } else {
                output
            };
            Ok((format!("write `{path}`"), body))
        }
        ToolKind::Run => {
            let args: RunArgs = parse_args(arguments)?;
            let command = args.command.trim();
            if command.is_empty() {
                return Err("Enter a command.");
            }
            if command.len() > MAXIMUM_COMMAND_BYTES {
                return Err("That command is too long.");
            }
            let output = capture(sandbox, GuestExec::shell(command), job).await?;
            Ok((format!("run `{command}`"), output))
        }
        ToolKind::GitStatus => {
            let output = capture(
                sandbox,
                GuestExec::command(
                    "git",
                    vec![
                        "status".to_owned(),
                        "--short".to_owned(),
                        "--branch".to_owned(),
                    ],
                ),
                job,
            )
            .await?;
            Ok(("git status".to_owned(), output))
        }
        ToolKind::GitDiff => {
            let args: PathArgs = parse_args(arguments)?;
            let mut git_args = vec!["diff".to_owned()];
            let label = if args.path.trim().is_empty() {
                "git diff".to_owned()
            } else {
                let path = guest_path(&args.path)?;
                git_args.push("--".to_owned());
                git_args.push(path.clone());
                format!("git diff `{path}`")
            };
            let output = capture(sandbox, GuestExec::command("git", git_args), job).await?;
            Ok((label, output))
        }
        ToolKind::GitCommit => {
            let args: CommitArgs = parse_args(arguments)?;
            let message = args.message.trim();
            if message.is_empty() {
                return Err("Enter a commit message.");
            }
            if message.len() > MAXIMUM_COMMAND_BYTES {
                return Err("That commit message is too long.");
            }
            let add = capture(
                sandbox,
                GuestExec::command("git", vec!["add".to_owned(), "-A".to_owned()]),
                job,
            )
            .await?;
            let commit = capture(
                sandbox,
                GuestExec::command(
                    "git",
                    vec![
                        "-c".to_owned(),
                        "user.name=Power Plant".to_owned(),
                        "-c".to_owned(),
                        "user.email=agent@localhost".to_owned(),
                        "commit".to_owned(),
                        "-m".to_owned(),
                        message.to_owned(),
                    ],
                ),
                job,
            )
            .await?;
            let output = join_outputs([&add, &commit]);
            Ok((format!("git commit `{message}`"), output))
        }
    }
}

#[derive(Deserialize)]
struct PathArgs {
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    contents: String,
}

#[derive(Deserialize)]
struct RunArgs {
    command: String,
}

#[derive(Deserialize)]
struct CommitArgs {
    message: String,
}

fn parse_args<T: for<'de> Deserialize<'de>>(
    arguments: &serde_json::Value,
) -> Result<T, &'static str> {
    serde_json::from_value(arguments.clone()).map_err(|_| "Those tool arguments are not valid.")
}

pub(super) fn guest_path(raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(GUEST_PROJECT.to_owned());
    }
    if raw.chars().any(char::is_control) {
        return Err("That path is not valid.");
    }
    let joined = if raw.starts_with('/') {
        raw.to_owned()
    } else {
        format!("{GUEST_PROJECT}/{raw}")
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if parts.is_empty() {
                return Err("Stay inside the project directory.");
            }
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    let normalised = format!("/{}", parts.join("/"));
    if normalised == GUEST_PROJECT || normalised.starts_with(&format!("{GUEST_PROJECT}/")) {
        Ok(normalised)
    } else {
        Err("Stay inside the project directory.")
    }
}

const CONFINED_EXISTING_SCRIPT: &str = r#"
resolved=$(realpath "$1") || { printf '%s\n' 'That path does not exist.'; exit 1; }
case "$resolved" in
    /project|/project/*) ;;
    *) printf '%s\n' 'Stay inside the project directory.'; exit 1 ;;
esac
shift
exec "$@" -- "$resolved"
"#;

fn confined_existing_command(path: &str, program: &str, args: &[&str]) -> GuestExec {
    let mut command_args = vec![
        "-c".to_owned(),
        CONFINED_EXISTING_SCRIPT.to_owned(),
        "project-path".to_owned(),
        path.to_owned(),
        program.to_owned(),
    ];
    command_args.extend(args.iter().map(|arg| (*arg).to_owned()));
    GuestExec::command("sh", command_args)
}

// Resolve the nearest existing ancestor before mkdir. This prevents a project symlink from redirecting the write.
const CONFINED_WRITE_SCRIPT: &str = r#"
target=$1
parent=${target%/*}
ancestor=$parent
suffix=
while [ ! -e "$ancestor" ] && [ ! -L "$ancestor" ]; do
    name=${ancestor##*/}
    suffix=/$name$suffix
    ancestor=${ancestor%/*}
done
resolved=$(realpath "$ancestor") || { printf '%s\n' 'That path is not valid.'; exit 1; }
case "$resolved" in
    /project|/project/*) ;;
    *) printf '%s\n' 'Stay inside the project directory.'; exit 1 ;;
esac
parent=$resolved$suffix
mkdir -p -- "$parent" || exit 1
resolved=$(realpath "$parent") || { printf '%s\n' 'That path is not valid.'; exit 1; }
case "$resolved" in
    /project|/project/*) ;;
    *) printf '%s\n' 'Stay inside the project directory.'; exit 1 ;;
esac
target=$resolved/${target##*/}
if [ -L "$target" ]; then
    target=$(realpath "$target") || { printf '%s\n' 'That path is not valid.'; exit 1; }
    case "$target" in
        /project/*) ;;
        *) printf '%s\n' 'Stay inside the project directory.'; exit 1 ;;
    esac
fi
cat > "$target"
"#;

fn confined_write_command(path: &str) -> GuestExec {
    GuestExec::command(
        "sh",
        vec![
            "-c".to_owned(),
            CONFINED_WRITE_SCRIPT.to_owned(),
            "project-write".to_owned(),
            path.to_owned(),
        ],
    )
}

async fn capture(
    sandbox: &GuestSandbox,
    request: GuestExec,
    job: &Job,
) -> Result<String, &'static str> {
    let mut session = sandbox
        .exec_cmd(request)
        .await
        .map_err(|error| error.message())?;
    let mut output = String::new();
    let mut exit = None;
    loop {
        let event = tokio::select! {
            biased;
            _ = job.cancelled() => {
                session.kill().await;
                session.close().await;
                return Err("Stopped.");
            }
            event = session.recv() => event,
        };
        let Some(event) = event else {
            break;
        };
        match event {
            crate::sandbox::CommandEvent::Output(text) => {
                if append_bounded(&mut output, &text) {
                    session.kill().await;
                    session.close().await;
                    mark_truncated(&mut output);
                    return Ok(output);
                }
            }
            crate::sandbox::CommandEvent::Exited(code) => {
                exit = Some(code);
                break;
            }
            crate::sandbox::CommandEvent::Failed => {
                session.close().await;
                return Err("Power Plant could not run the command. Try again.");
            }
        }
    }
    session.close().await;
    match exit {
        Some(0) => Ok(empty_output(output)),
        None => Err("Power Plant lost the command result. Try again."),
        Some(code) => {
            let mut failed = output;
            if !failed.is_empty() && !failed.ends_with('\n') {
                failed.push('\n');
            }
            failed.push_str(&format!("The command exited with code {code}."));
            Ok(failed)
        }
    }
}

const TRUNCATED_OUTPUT: &str = "\n[output truncated]";

fn mark_truncated(output: &mut String) {
    let maximum = MAXIMUM_TOOL_BYTES.saturating_sub(TRUNCATED_OUTPUT.len());
    let mut end = output.len().min(maximum);
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    output.push_str(TRUNCATED_OUTPUT);
}

fn empty_output(output: String) -> String {
    if output.is_empty() {
        "(no output)".to_owned()
    } else {
        output
    }
}

fn join_outputs(parts: [&str; 2]) -> String {
    let mut out = String::new();
    for part in parts {
        if part.is_empty() || part == "(no output)" {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(part);
    }
    empty_output(out)
}

fn append_bounded(buffer: &mut String, piece: &str) -> bool {
    let remaining = MAXIMUM_TOOL_BYTES.saturating_sub(buffer.len());
    if piece.len() <= remaining {
        buffer.push_str(piece);
        return false;
    }
    let mut end = remaining;
    while end > 0 && !piece.is_char_boundary(end) {
        end -= 1;
    }
    buffer.push_str(&piece[..end]);
    true
}

pub(super) fn redact(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|value| !value.is_empty()) else {
        return text.to_owned();
    };
    text.replace(secret, "[redacted]")
}

pub(super) fn render_trace(label: &str, output: &str) -> String {
    let label = escape_markdown_text(label);
    let fence = "~".repeat(longest_tilde_run(output).saturating_add(1).max(3));
    format!("**{label}**\n\n{fence}\n{output}\n{fence}\n")
}

fn escape_markdown_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            escaped.push(' ');
        } else {
            if character.is_ascii_punctuation() {
                escaped.push('\\');
            }
            escaped.push(character);
        }
    }
    escaped
}

fn longest_tilde_run(text: &str) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for character in text.chars() {
        if character == '~' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

#[cfg(test)]
mod tests;
