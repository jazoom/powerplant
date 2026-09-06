#[cfg(test)]
mod tests;

pub(crate) const MAXIMUM_TASK_LIST_BYTES: usize = 64 * 1024;
pub(crate) const MAXIMUM_TASKS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskList {
    pub(crate) heading: String,
    pub(crate) preamble: String,
    pub(crate) tasks: Vec<TaskItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskItem {
    pub(crate) index: u32,
    pub(crate) checked: bool,
    /// Byte-exact source, including details and line endings.
    pub(crate) markdown: String,
}

impl TaskList {
    pub(crate) fn eligible_tasks(&self) -> impl Iterator<Item = &TaskItem> {
        self.tasks.iter().filter(|task| !task.checked)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskListError {
    TooLarge,
    Heading,
    Tasks,
    TaskLimit,
}

pub(crate) fn parse(markdown: &str) -> Result<TaskList, TaskListError> {
    if markdown.len() > MAXIMUM_TASK_LIST_BYTES {
        return Err(TaskListError::TooLarge);
    }
    let mut heading = None;
    let mut fence: Option<(u8, usize)> = None;
    let mut starts = Vec::new();
    let mut offset = 0;
    for source in markdown.split_inclusive('\n') {
        let line = source.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied();
        let width = trimmed
            .bytes()
            .take_while(|byte| Some(*byte) == marker)
            .count();
        if let Some((delimiter, minimum)) = fence {
            if indent <= 3
                && marker == Some(delimiter)
                && width >= minimum
                && trimmed[width..].trim().is_empty()
            {
                fence = None;
            }
        } else if indent <= 3
            && matches!(marker, Some(b'`' | b'~'))
            && width >= 3
            && (marker != Some(b'`') || !trimmed[width..].contains('`'))
        {
            fence = Some((marker.expect("a fence has a delimiter"), width));
        } else if indent == 0 {
            if starts.is_empty() && heading.is_none() {
                heading = line
                    .strip_prefix("# ")
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| text.trim().to_owned());
            }
            if let Some(checked) = checkbox(line) {
                if starts.len() == MAXIMUM_TASKS {
                    return Err(TaskListError::TaskLimit);
                }
                starts.push((offset, checked));
            }
        }
        offset += source.len();
    }
    let heading = heading.ok_or(TaskListError::Heading)?;
    let preamble_end = starts.first().ok_or(TaskListError::Tasks)?.0;
    let tasks = starts
        .iter()
        .enumerate()
        .map(|(position, (start, checked))| {
            let end = starts
                .get(position + 1)
                .map_or(markdown.len(), |item| item.0);
            TaskItem {
                index: u32::try_from(position).expect("bounded task count"),
                checked: *checked,
                markdown: markdown[*start..end].to_owned(),
            }
        })
        .collect();
    Ok(TaskList {
        heading,
        preamble: markdown[..preamble_end].to_owned(),
        tasks,
    })
}

pub(crate) fn valid_project_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !path.starts_with('/')
}

fn checkbox(line: &str) -> Option<bool> {
    let remainder = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))?;
    let (state, text) = remainder.split_once(']')?;
    if !text.starts_with([' ', '\t']) || text.trim().is_empty() {
        return None;
    }
    match state {
        "[ " => Some(false),
        "[x" | "[X" => Some(true),
        _ => None,
    }
}
