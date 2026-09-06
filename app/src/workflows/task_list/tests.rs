use super::*;

#[test]
fn parser_keeps_preamble_details_and_completion_state() {
    let list = parse(
        "# Release tasks\n\nShared context.\n\n- [x] Prepare release\n  Keep this detail.\n- [ ] Publish release\n  - Nested detail\n",
    )
    .expect("task list");
    assert_eq!(list.heading, "Release tasks");
    assert_eq!(list.preamble, "# Release tasks\n\nShared context.\n\n");
    assert!(list.tasks[0].checked);
    assert_eq!(
        list.tasks[1].markdown,
        "- [ ] Publish release\n  - Nested detail\n"
    );
    assert_eq!(list.eligible_tasks().count(), 1);
}

#[test]
fn parser_ignores_checkbox_examples_inside_fences_and_indented_details() {
    let list = parse(
        "# Tasks\n\n```markdown\n- [ ] Literal example\n```\n\n- [ ] Actual task\n  - [ ] Detail only\n",
    )
    .expect("task list");
    assert_eq!(list.tasks.len(), 1);
    assert!(list.tasks[0].markdown.contains("Detail only"));
}

#[test]
fn parser_rejects_missing_structure_and_bounds() {
    assert_eq!(parse("- [ ] Task").err(), Some(TaskListError::Heading));
    assert_eq!(parse("# Tasks\n").err(), Some(TaskListError::Tasks));
    assert_eq!(
        parse(&format!("# Tasks\n{}", "x".repeat(MAXIMUM_TASK_LIST_BYTES))).err(),
        Some(TaskListError::TooLarge)
    );
    assert_eq!(
        parse(&format!(
            "# Tasks\n{}",
            "- [ ] Task\n".repeat(MAXIMUM_TASKS + 1)
        ))
        .err(),
        Some(TaskListError::TaskLimit)
    );
}

#[test]
fn fences_cannot_supply_headings_or_close_with_another_delimiter() {
    assert_eq!(
        parse("```\n# Example\n```\n- [ ] Task\n").err(),
        Some(TaskListError::Heading)
    );
    let source = "# Tasks\r\n\r\n````markdown\r\n```\r\n- [ ] Example\r\n~~~\r\n- [ ] Another example\r\n````\r\n- [X] Done\r\n  Detail\r\n+ [ ] Pending\r\n";
    let list = parse(source).expect("task list");
    assert_eq!(list.tasks.len(), 2);
    assert_eq!(list.tasks[0].markdown, "- [X] Done\r\n  Detail\r\n");
    assert_eq!(
        list.preamble.clone()
            + &list
                .tasks
                .iter()
                .map(|task| task.markdown.as_str())
                .collect::<String>(),
        source
    );
    assert_eq!(
        list.eligible_tasks()
            .map(|task| task.index)
            .collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(
        parse("# Tasks\n- [ ]not a checkbox\n- [ ] \n").err(),
        Some(TaskListError::Tasks)
    );
}

#[test]
fn imported_paths_cannot_select_host_paths_or_parent_components() {
    for path in [
        "",
        "/etc/passwd",
        "../tasks.md",
        "tasks/../secret",
        "tasks//file",
        "./tasks",
        "tasks/",
        "tasks\nfile",
    ] {
        assert!(!valid_project_path(path), "{path:?}");
    }
    assert!(valid_project_path("docs/tasks for release.md"));
    assert!(!valid_project_path(&"x".repeat(4097)));
}
