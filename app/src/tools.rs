use rig_core::completion::ToolDefinition;
use serde::Deserialize;

use crate::agents::{DirectoryPolicy, GUEST_PROJECT, ToolId};
use crate::sandbox::{GuestExec, GuestSandbox};
use crate::sessions::Job;

pub(crate) const MAXIMUM_TOOL_BYTES: usize = 64 * 1024;
pub(crate) const MAXIMUM_WRITE_BYTES: usize = 256 * 1024;
pub(crate) const MAXIMUM_COMMAND_BYTES: usize = 32_768;

impl ToolId {
    fn description(self) -> &'static str {
        match self {
            Self::List => "List files in a granted directory.",
            Self::Read => "Read a file inside a granted directory.",
            Self::Write => {
                "Write a file inside a writable granted directory. Creates parent directories."
            }
            Self::Run => "Run a shell command. Starts in the primary directory.",
        }
    }

    fn parameters(self) -> serde_json::Value {
        match self {
            Self::List | Self::Read => serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path inside a granted guest directory. Defaults to /project."
                    }
                },
                "additionalProperties": false
            }),
            Self::Write => serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path inside a writable granted guest directory."
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
                        "description": "Shell command to run. Starts in /project."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        }
    }
}

pub(crate) fn definitions(selected: &[ToolId]) -> Vec<ToolDefinition> {
    selected
        .iter()
        .copied()
        .map(|kind| ToolDefinition {
            name: kind.as_str().to_owned(),
            description: kind.description().to_owned(),
            parameters: kind.parameters(),
        })
        .collect()
}

pub(crate) struct AgentToolContext<'a> {
    pub(crate) sandbox: &'a GuestSandbox,
    pub(crate) policy: &'a DirectoryPolicy,
    pub(crate) job: &'a Job,
    pub(crate) tools: &'a [ToolId],
}

pub(crate) struct ToolTrace {
    pub(crate) label: String,
    pub(crate) output: String,
}

pub(crate) async fn invoke(
    context: &AgentToolContext<'_>,
    name: &str,
    arguments: &serde_json::Value,
) -> ToolTrace {
    let Some(kind) = ToolId::parse(name).filter(|kind| context.tools.contains(kind)) else {
        return ToolTrace {
            label: name.to_owned(),
            output: "That tool is not available.".to_owned(),
        };
    };
    match dispatch(context, kind, arguments).await {
        Ok((label, output)) => ToolTrace { label, output },
        Err(message) => ToolTrace {
            label: kind.as_str().to_owned(),
            output: message.to_owned(),
        },
    }
}

async fn dispatch(
    context: &AgentToolContext<'_>,
    kind: ToolId,
    arguments: &serde_json::Value,
) -> Result<(String, String), &'static str> {
    match kind {
        ToolId::List => {
            let args: PathArgs = parse_args(arguments)?;
            let (path, _) = context.policy.resolve(&args.path)?;
            let output = capture(
                context,
                confined_existing_command(&path, &context.policy.guest_roots(), "ls", &["-la"]),
            )
            .await?;
            Ok((format!("list `{path}`"), output))
        }
        ToolId::Read => {
            let args: PathArgs = parse_args(arguments)?;
            let (path, _) = context.policy.resolve(&args.path)?;
            if path == GUEST_PROJECT
                || context
                    .policy
                    .grants()
                    .iter()
                    .any(|grant| grant.guest_path == path)
            {
                return Err("Choose a file to read.");
            }
            let maximum = MAXIMUM_TOOL_BYTES.to_string();
            let output = capture(
                context,
                confined_existing_command(
                    &path,
                    &context.policy.guest_roots(),
                    "head",
                    &["-c", &maximum],
                ),
            )
            .await?;
            Ok((format!("read `{path}`"), output))
        }
        ToolId::Write => {
            let args: WriteArgs = parse_args(arguments)?;
            let (path, access) = context.policy.resolve(&args.path)?;
            if !access.is_writable() {
                return Err("That path is read-only.");
            }
            if path == GUEST_PROJECT
                || context
                    .policy
                    .grants()
                    .iter()
                    .any(|grant| grant.guest_path == path)
            {
                return Err("Choose a file to write.");
            }
            if args.contents.len() > MAXIMUM_WRITE_BYTES {
                return Err("That file is too large to write.");
            }
            let output = capture(
                context,
                confined_write_command(&path, &context.policy.writable_roots())
                    .with_stdin(args.contents.into_bytes()),
            )
            .await?;
            let body = if output.trim().is_empty() {
                "Wrote the file.".to_owned()
            } else {
                output
            };
            Ok((format!("write `{path}`"), body))
        }
        ToolId::Run => {
            let args: RunArgs = parse_args(arguments)?;
            let command = args.command.trim();
            if command.is_empty() {
                return Err("Enter a command.");
            }
            if command.len() > MAXIMUM_COMMAND_BYTES {
                return Err("That command is too long.");
            }
            let output = capture(
                context,
                GuestExec::shell(command).in_dir(context.policy.primary_guest()),
            )
            .await?;
            Ok((format!("run `{command}`"), output))
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

fn parse_args<T: for<'de> Deserialize<'de>>(
    arguments: &serde_json::Value,
) -> Result<T, &'static str> {
    serde_json::from_value(arguments.clone()).map_err(|_| "Those tool arguments are not valid.")
}

const CONFINED_EXISTING_SCRIPT: &str = r#"
roots=$1
resolved=$(realpath "$2") || { printf '%s\n' 'That path does not exist.'; exit 1; }
ok=0
oldifs=$IFS
IFS=:
for root in $roots; do
    case "$resolved" in
        "$root"|"$root"/*) ok=1 ;;
    esac
done
IFS=$oldifs
if [ "$ok" -ne 1 ]; then
    printf '%s\n' 'Stay inside a granted directory.'
    exit 1
fi
shift 2
exec "$@" -- "$resolved"
"#;

fn confined_existing_command(
    path: &str,
    roots: &[String],
    program: &str,
    args: &[&str],
) -> GuestExec {
    let mut command_args = vec![
        "-c".to_owned(),
        CONFINED_EXISTING_SCRIPT.to_owned(),
        "project-path".to_owned(),
        encode_roots(roots),
        path.to_owned(),
        program.to_owned(),
    ];
    command_args.extend(args.iter().map(|arg| (*arg).to_owned()));
    GuestExec::command("sh", command_args)
}

const CONFINED_WRITE_SCRIPT: &str = r#"
roots=$1
target=$2
parent=${target%/*}
ancestor=$parent
suffix=
while [ ! -e "$ancestor" ] && [ ! -L "$ancestor" ]; do
    name=${ancestor##*/}
    suffix=/$name$suffix
    ancestor=${ancestor%/*}
done
resolved=$(realpath "$ancestor") || { printf '%s\n' 'That path is not valid.'; exit 1; }
ok=0
oldifs=$IFS
IFS=:
for root in $roots; do
    case "$resolved" in
        "$root"|"$root"/*) ok=1 ;;
    esac
done
IFS=$oldifs
if [ "$ok" -ne 1 ]; then
    printf '%s\n' 'Stay inside a granted directory.'
    exit 1
fi
parent=$resolved$suffix
mkdir -p -- "$parent" || exit 1
resolved=$(realpath "$parent") || { printf '%s\n' 'That path is not valid.'; exit 1; }
ok=0
oldifs=$IFS
IFS=:
for root in $roots; do
    case "$resolved" in
        "$root"|"$root"/*) ok=1 ;;
    esac
done
IFS=$oldifs
if [ "$ok" -ne 1 ]; then
    printf '%s\n' 'Stay inside a granted directory.'
    exit 1
fi
target=$resolved/${target##*/}
if [ -L "$target" ]; then
    target=$(realpath "$target") || { printf '%s\n' 'That path is not valid.'; exit 1; }
    ok=0
    oldifs=$IFS
    IFS=:
    for root in $roots; do
        case "$target" in
            "$root"/*) ok=1 ;;
        esac
    done
    IFS=$oldifs
    if [ "$ok" -ne 1 ]; then
        printf '%s\n' 'Stay inside a granted directory.'
        exit 1
    fi
fi
cat > "$target"
"#;

fn confined_write_command(path: &str, roots: &[String]) -> GuestExec {
    GuestExec::command(
        "sh",
        vec![
            "-c".to_owned(),
            CONFINED_WRITE_SCRIPT.to_owned(),
            "project-write".to_owned(),
            encode_roots(roots),
            path.to_owned(),
        ],
    )
}

fn encode_roots(roots: &[String]) -> String {
    roots.join(":")
}

async fn capture(
    context: &AgentToolContext<'_>,
    request: GuestExec,
) -> Result<String, &'static str> {
    let mut session = context
        .sandbox
        .exec_cmd(request)
        .await
        .map_err(|error| error.message())?;
    let mut output = String::new();
    let mut exit = None;
    loop {
        let event = tokio::select! {
            biased;
            _ = context.job.cancelled() => {
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

pub(super) fn mark_truncated(output: &mut String) {
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

pub(crate) fn redact(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|value| !value.is_empty()) else {
        return text.to_owned();
    };
    text.replace(secret, "[redacted]")
}

pub(crate) fn render_trace(label: &str, output: &str) -> String {
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
