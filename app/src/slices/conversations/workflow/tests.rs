use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

fn directory_settings() -> crate::execution::ExecutionSettings {
    crate::execution::ExecutionSettings::new(
        crate::providers::ModelSelection::new(
            crate::providers::ProviderKind::Xai,
            "grok-4.6".to_owned(),
            None,
        )
        .unwrap(),
        "Use the pinned instructions.".to_owned(),
        crate::agents::ToolId::ALL.to_vec(),
        crate::tests::test_environment_id(),
    )
    .unwrap()
}

#[test]
fn directory_launch_pins_non_git_roots_and_read_only_review_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut grants = Vec::new();
    for name in ["first", "second"] {
        let path = root.path().join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("same.txt"), name).unwrap();
        let mut grant = crate::execution::DirectoryGrant::from_selected(&path, &grants).unwrap();
        grant.access = crate::execution::DirectoryAccess::ReviewBeforeApply;
        grants.push(grant);
    }
    let mut settings = directory_settings().with_directories(grants).unwrap();
    let definition = workflows::seeds::implement_and_review_definition(settings.environment)
        .with_conversation_settings(&settings)
        .unwrap();
    let authority = crate::execution::ProjectFreeAuthority::from_settings(1, &settings).unwrap();
    let reviewer = &definition.steps()[1];
    let capabilities =
        workflows::capabilities::AttemptCapabilities::derive_project_free(reviewer, &authority)
            .unwrap();
    assert!(
        capabilities
            .directories
            .iter()
            .all(|directory| directory.access == AccessMode::ReadOnly)
    );
    assert!(matches!(&definition.steps().last().unwrap().action,
        workflows::definition::StepAction::SystemCommand(action)
            if action.command == workflows::commands::SystemCommandId::ApplyChanges));
    let state = connected_state();
    let source = workflows::artefacts::CandidateCapture::capture_set(
        &settings.directories,
        &root.path().join("data"),
        &state.workflow_artefacts,
    )
    .unwrap();
    assert_eq!(source.roots.len(), 2);
    let phases = phase_steps(&definition)
        .into_iter()
        .map(|step| PhaseModelSelection {
            step: step.key.clone(),
            selection: settings.model.clone(),
            instructions: settings.instructions.clone(),
            preset: None,
            settings: Some(settings.clone()),
        })
        .collect();
    let pinned = workflows::definition::PinnedWorkflowDefinition::pin(None, definition);
    let environments = crate::tests::test_environment_set(&pinned.definition);
    let mut run = WorkflowRun::create_source_free_for_conversation(
        workflows::RunId::generate().unwrap(),
        1,
        crate::conversations::ConversationId::generate().unwrap(),
        pinned,
        environments,
        phases,
    );
    run.kind = workflows::run::RunKind::Configured;
    run.launch_brief = "Update both files".to_owned();
    let bytes = source.manifest_bytes().unwrap();
    let kind = workflows::definition::ArtefactKind::CandidateRevision;
    let artefact = workflows::artefacts::ArtefactRecord {
        id: workflows::ArtefactId::generate().unwrap(),
        kind,
        artefact_hash: workflows::artefacts::artefact_hash_for(
            kind,
            workflows::artefacts::CANDIDATE_SCHEMA,
            &bytes,
        ),
        object_hash: state.workflow_artefacts.publish(&bytes).unwrap(),
        payload_bytes: bytes.len() as u64,
        created_at_ms: 1,
        provenance: workflows::artefacts::ArtefactProvenance {
            run_id: run.id,
            producer: workflows::artefacts::ArtefactProducer::RunSourceCapture,
            inputs: Vec::new(),
        },
        summary: workflows::artefacts::ArtefactSummary::Candidate {
            candidate: source.candidate_hash,
            entries: 2,
            bytes: 11,
            disposition: workflows::artefacts::ProductionDisposition::RequiredOutput,
        },
    };
    let reference = workflows::artefacts::ArtefactReference {
        id: artefact.id,
        kind,
        artefact_hash: artefact.artefact_hash,
    };
    run.record_initial_candidate(artefact).unwrap();
    let step = run.pinned.definition.steps()[0].clone();
    let capabilities =
        workflows::capabilities::AttemptCapabilities::derive_project_free(&step, &authority)
            .unwrap();
    run.start_attempt(
        workflows::AttemptId::generate().unwrap(),
        vec![workflows::run::AttemptArtefactInput {
            key: workflows::definition::InputKey::parse("candidate").unwrap(),
            artefact: reference,
        }],
        capabilities,
        workflows::run::AttemptSandboxRecord {
            kind: workflows::run::AttemptSandboxKind::IsolatedAttempt,
            snapshot_digest: run.environments.steps[0].snapshot_digest.clone(),
        },
        2,
    )
    .unwrap();
    settings.directories.clear();
    settings.instructions.clear();
    state.workflow_runs.create(run.clone()).unwrap();
    let loaded = state.workflow_runs.get(&run.id).unwrap();
    assert_eq!(loaded.directory_settings().unwrap().directories.len(), 2);
    assert_eq!(
        loaded.directory_settings().unwrap().instructions,
        "Use the pinned instructions."
    );
    assert!(loaded.project_id.is_none());
    assert!(loaded.agent_id.is_none());
    std::fs::rename(root.path().join("second"), root.path().join("old-second")).unwrap();
    std::fs::create_dir(root.path().join("second")).unwrap();
    assert!(
        crate::execution::ProjectFreeAuthority::from_settings(
            1,
            loaded.directory_settings().unwrap()
        )
        .is_err()
    );
}

#[test]
fn a_directory_launch_form_does_not_require_a_project_target() {
    let (form, _) = parse_fields::<WorkflowLaunchForm>(vec![
        ("revision".to_owned(), "1".to_owned()),
        ("workflow".to_owned(), "selected".to_owned()),
        ("brief".to_owned(), "Prepare a plan".to_owned()),
    ])
    .unwrap();
    assert!(form.target.is_empty());
    assert!(form.preview_target.is_empty());
}

#[test]
fn source_free_plans_have_no_candidate_and_missing_review_sources_fail_before_execution() {
    let settings = directory_settings();
    let definition = workflows::seeds::plan_a_change_definition(settings.environment)
        .with_conversation_settings(&settings)
        .unwrap();
    assert!(definition.steps().iter().all(|step| step.inputs.is_empty()));
    assert!(
        workflows::seeds::review_current_code_definition(settings.environment)
            .with_conversation_settings(&settings)
            .is_err()
    );
    assert!(
        workflows::seeds::implement_with_approval_definition(settings.environment)
            .with_conversation_settings(&settings)
            .is_err()
    );
    let authority = crate::execution::ProjectFreeAuthority::from_settings(1, &settings).unwrap();
    let capabilities = workflows::capabilities::AttemptCapabilities::derive_project_free(
        &definition.steps()[0],
        &authority,
    )
    .unwrap();
    assert!(authority.policy.grants().is_empty());
    assert_eq!(capabilities.primary().unwrap().guest_path, "/workspace");
    let phases = vec![PhaseModelSelection {
        step: definition.first_step().clone(),
        selection: settings.model.clone(),
        instructions: settings.instructions.clone(),
        preset: None,
        settings: Some(settings),
    }];
    let pinned = workflows::definition::PinnedWorkflowDefinition::pin(None, definition);
    let environments = crate::tests::test_environment_set(&pinned.definition);
    let mut run = WorkflowRun::create_source_free_for_conversation(
        workflows::RunId::generate().unwrap(),
        1,
        crate::conversations::ConversationId::generate().unwrap(),
        pinned,
        environments,
        phases,
    );
    run.kind = workflows::run::RunKind::Configured;
    run.launch_brief = "Prepare a plan".to_owned();
    let state = connected_state();
    state.workflow_runs.create(run.clone()).unwrap();
    let loaded = state.workflow_runs.get(&run.id).unwrap();
    assert!(matches!(loaded.source, workflows::RunSource::None));
    assert!(loaded.artefacts.is_empty());
}

#[test]
fn sensitive_workflow_launch_needs_live_destination_consent() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data = home.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let mut state = connected_state();
    state.local_data = crate::local_data::LocalDataReset::for_test(data);
    let grant = crate::execution::DirectoryGrant::from_selected(&home, &[]).unwrap();
    let settings = directory_settings()
        .with_directories(vec![grant.clone()])
        .unwrap();
    let record = state
        .conversations
        .create_saved(
            crate::conversations::ConversationId::generate().unwrap(),
            None,
            None,
            Some(crate::conversations::ConversationModelConfiguration {
                settings: settings.clone(),
                preset: None,
            }),
            None,
        )
        .unwrap();
    let session = crate::sessions::generate_session_token().unwrap().id();
    state.sessions.insert(session);
    assert!(conversation_directory_authority(&state, session, &record, &settings).is_err());
    let request = state
        .access_consent
        .request_conversation(session, record.id, &settings, &grant)
        .unwrap();
    state
        .access_consent
        .approve_conversation(&request, session, record.id, &settings, &grant)
        .unwrap();
    assert!(conversation_directory_authority(&state, session, &record, &settings).is_ok());
    let mut copy = record.clone();
    copy.id = crate::conversations::ConversationId::generate().unwrap();
    assert!(conversation_directory_authority(&state, session, &copy, &settings).is_err());
    state.access_consent.retain_sessions(|_| false);
    assert!(conversation_directory_authority(&state, session, &record, &settings).is_err());
}

fn connected_state() -> AppState {
    let state = crate::tests::test_state(crate::config::RuntimeConfig::development());
    state
        .vault
        .put(crate::providers::ProviderConnection::with_key(
            crate::providers::ProviderKind::Xai,
            "test-key",
            "grok-4.6",
        ))
        .expect("provider");
    state
}

#[tokio::test]
async fn task_loop_launch_accepts_a_whole_list_without_a_task_index() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Loop".to_owned())
        .expect("conversation");
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            "# Tasks\n\n- [x] Done\n- [ ] Remaining\n".to_owned(),
            None,
        )
        .expect("tasks");
    let document_token = format!(
        "{}/1/{}",
        document.id.as_hex(),
        document.current().content_hash.as_str()
    );
    let workflow = state
        .workflows
        .create(workflows::seeds::ralph_task_loop_definition(
            crate::tests::test_environment_id(),
        ))
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let view = launch_view(
        &state,
        &conversation,
        Some(&selection),
        None,
        "Implement",
        "",
        "",
        &document_token,
        "",
        "",
        "",
        &[],
        "",
    )
    .await;
    assert!(view.error.is_empty(), "{}", view.error);
    assert!(view.task_preview.contains("[x] Done"));
    assert!(
        resolve_launch_task(
            ExecutionMode::TaskList,
            &state,
            &conversation,
            &document_token,
            "",
            "",
            "1"
        )
        .is_err()
    );
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let response = app.oneshot(Request::builder().method("POST")
        .uri(format!("/conversations/{}/workflow", conversation.id.as_hex()))
        .header(header::COOKIE, format!("powerplant_session={}", token.raw().as_str()))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(hypergraft::GRAFT_REQUEST, "patch").header(header::ACCEPT, hypergraft::MEDIA_TYPE)
        .body(Body::from(format!("revision={}&workflow={selection}&preview_workflow={selection}&brief=Implement&target=&task_document={document_token}&preview_task_document={document_token}", conversation.revision))).expect("request"))
        .await.expect("response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(
        body.contains("Choose an available target project."),
        "{body}"
    );
    assert!(
        state
            .task_loops
            .for_conversation(&conversation.id)
            .is_empty()
    );
}

#[test]
fn task_selection_rejects_foreign_checked_and_removed_items() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Tasks".to_owned())
        .expect("conversation");
    let other = state
        .conversations
        .create("Other".to_owned())
        .expect("conversation");
    let markdown =
        "# Tasks\n\nShared context.\n\n- [x] Done.\n- [ ] First pending.\n- [ ] Selected.\n";
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            markdown.to_owned(),
            None,
        )
        .expect("tasks");
    let id = document.id.as_hex();
    let hash = document.current().content_hash.as_str();
    let resolve = |record: &ConversationRecord, index: &str, hash: &str| {
        super::resolve_selected_task(&state, record, &id, "1", hash, index)
    };
    assert!(resolve(&other, "2", &hash).is_err());
    for index in ["0", "3", "-1", "4294967296"] {
        assert!(resolve(&conversation, index, &hash).is_err());
    }
    assert!(
        resolve(
            &conversation,
            "2",
            &workflows::artefacts::ObjectHash::of(b"other").as_str()
        )
        .is_err()
    );
    let revised = state
        .documents
        .revise(
            &document.id,
            1,
            "Tasks".to_owned(),
            "# Tasks\n\n- [ ] Different task.\n".to_owned(),
            None,
        )
        .expect("revision");
    let selected = resolve(&conversation, "2", &hash)
        .expect("selection")
        .expect("task");
    assert_eq!(selected.task_list, markdown);
    assert_eq!(selected.markdown, "- [ ] Selected.\n");
    state
        .documents
        .disassociate(&document.id, revised.current().revision, conversation.id)
        .expect("remove");
    assert!(resolve(&conversation, "2", &hash).is_err());
}

#[test]
fn saved_plan_selection_rejects_substitution_and_removal_but_pins_old_revisions() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let plan = state
        .documents
        .create_from_text(
            conversation.id,
            "Implementation plan".to_owned(),
            "# Selected plan\n\nKeep this revision.\n".to_owned(),
            None,
        )
        .expect("plan");
    let base = workflows::seeds::sequential_team_definition(crate::tests::test_environment_id());
    let mut steps = base.steps().to_vec();
    let implementer = steps
        .iter_mut()
        .find(|step| step.key.as_str() == "implementer")
        .expect("implementer");
    let plan_input = implementer
        .inputs
        .iter_mut()
        .find(|input| input.kind == workflows::definition::ArtefactKind::Plan)
        .expect("plan input");
    plan_input.source = workflows::definition::ArtefactSource::LaunchInput {
        source: workflows::definition::LaunchInputSource::SavedPlan,
    };
    let definition = workflows::definition::WorkflowDefinition::from_parts(
        base.name().to_owned(),
        base.default_environment(),
        base.roles().to_vec(),
        steps,
    )
    .expect("definition");
    let token = super::plan_choice_token(&plan.id, plan.current());
    let resolve = |raw: &str| super::resolve_selected_plan(&state, &conversation, raw, &definition);
    assert!(resolve("").is_err());
    let tasks = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Tasks".to_owned(),
            "# Tasks\n- [ ] Implement\n".to_owned(),
            None,
        )
        .expect("tasks");
    assert!(resolve(&super::plan_choice_token(&tasks.id, tasks.current())).is_err());
    let selected = resolve(&token).expect("selection").expect("plan");
    let mut changed: super::PlanChoiceToken = serde_json::from_str(&token).expect("token");
    changed.content_hash = workflows::artefacts::ObjectHash::of(b"substitute").as_str();
    assert!(resolve(&serde_json::to_string(&changed).expect("changed token")).is_err());
    assert!(super::resolve_selected_plan(&state, &conversation, &token, &base).is_err());
    let other = state
        .conversations
        .create("Other".to_owned())
        .expect("conversation");
    assert!(super::resolve_selected_plan(&state, &other, &token, &definition).is_err());

    let corrected = state
        .documents
        .revise(
            &plan.id,
            1,
            "Corrected plan".to_owned(),
            "A different plan.".to_owned(),
            None,
        )
        .expect("correction");
    assert_eq!(
        resolve(&token)
            .expect("old selection")
            .expect("plan")
            .content,
        selected.content
    );
    let imported = workflows::artefacts::import_saved_plan(
        workflows::RunId::generate().expect("run"),
        workflows::now_ms(),
        conversation.id,
        plan.id,
        &selected.reference,
        &selected.content,
        &state.workflow_artefacts,
    )
    .expect("import");
    state
        .documents
        .disassociate(&plan.id, corrected.current().revision, conversation.id)
        .expect("remove association");
    assert!(resolve(&token).is_err());
    let bytes = state
        .workflow_artefacts
        .get(&imported.object_hash)
        .expect("run-owned plan");
    assert_eq!(
        workflows::artefacts::parse_typed_payload(imported.kind, &bytes).expect("payload"),
        workflows::artefacts::TypedPayload::Plan(workflows::artefacts::payload::PlanArtefact {
            format_version: 1,
            markdown: selected.content,
        }),
    );
}

#[tokio::test]
async fn launch_rejects_stale_definitions_without_reserving_the_conversation() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let mut seeds =
        workflows::seeds::production_seeds(crate::tests::test_environment_id()).into_iter();
    let workflow = state
        .workflows
        .create(seeds.next().expect("seed").definition)
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    state
        .workflows
        .update(
            &workflow.id,
            workflow.revision,
            seeds.next().expect("seed").definition,
        )
        .expect("update");
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/conversations/{}/workflow",
                    conversation.id.as_hex()
                ))
                .header(
                    header::COOKIE,
                    format!("powerplant_session={}", token.raw().as_str()),
                )
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(hypergraft::GRAFT_REQUEST, "patch")
                .header(header::ACCEPT, hypergraft::MEDIA_TYPE)
                .body(Body::from(format!(
                    "revision={}&workflow={selection}&brief=Inspect&target=&phase=first&phase=second",
                    conversation.revision
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(body.contains(ResolveWorkflowError::Changed.message()));
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("conversation"),
        conversation
    );
    assert!(!state.sessions.busy(&token.id()));
    assert!(
        state
            .workflow_runs
            .for_conversation(&conversation.id)
            .is_empty()
    );
}

#[tokio::test]
async fn launch_sheet_supports_document_navigation_and_selection_preview() {
    let state = connected_state();
    let conversation = state
        .conversations
        .create("Workflow".to_owned())
        .expect("conversation");
    let token = crate::sessions::generate_session_token().expect("session");
    state.sessions.insert(token.id());
    let app = crate::slices::router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::sessions::resolve_session,
        ))
        .layer(axum::middleware::from_fn(hypergraft::middleware::classify))
        .with_state(state.clone());
    let workflow = state
        .workflows
        .create(workflows::seeds::ralph_task_loop_definition(
            crate::tests::test_environment_id(),
        ))
        .expect("workflow");
    let selection = WorkflowSelection {
        workflow_id: workflow.id,
        definition_version: workflow.definition_version,
    }
    .as_token();
    let once = state
        .workflows
        .create(
            workflows::seeds::production_seeds(crate::tests::test_environment_id())
                .into_iter()
                .next()
                .expect("seed")
                .definition,
        )
        .expect("workflow");
    let once_selection = WorkflowSelection {
        workflow_id: once.id,
        definition_version: once.definition_version,
    }
    .as_token();
    for (selection, directory_launch) in [(&once_selection, true), (&selection, false)] {
        let view = launch_view(
            &state,
            &conversation,
            Some(selection),
            Some("missing"),
            "Keep this",
            "",
            "",
            "",
            "",
            "",
            "",
            &[],
            "",
        )
        .await;
        assert_eq!(view.directory_launch, directory_launch);
        if directory_launch {
            assert!(view.targets.is_empty());
            assert!(view.error.is_empty());
            assert!(!view.launch_blocked);
        } else {
            assert!(!view.error.is_empty());
            assert!(view.launch_blocked);
        }
    }
    let document = state
        .documents
        .create_task_list_from_text(
            conversation.id,
            "Review input".to_owned(),
            "# Tasks\n\n- [ ] Keep the selected task\n".to_owned(),
            None,
        )
        .expect("task list");
    let task_token = format!(
        "{}/1/{}",
        document.id.as_hex(),
        document.current().content_hash.as_str()
    );
    for selection in [&selection, &once_selection] {
        for stage in ["choose", "inputs", "review"] {
            for (representation, target) in [
                (None, "<!doctype html>"),
                (Some("navigation"), "chat-main"),
                (Some("patch"), "workflow-launch"),
            ] {
                let mut request = Request::builder()
            .uri(format!(
                "/conversations/{}/workflow?stage={stage}&workflow={selection}&brief=Preserve+this+brief&task_document={task_token}",
                conversation.id.as_hex()
            ))
            .header(
                header::COOKIE,
                format!("powerplant_session={}", token.raw().as_str()),
            );
                if let Some(representation) = representation {
                    request = request
                        .header(hypergraft::GRAFT_REQUEST, representation)
                        .header(header::ACCEPT, hypergraft::MEDIA_TYPE);
                }
                let response = app
                    .clone()
                    .oneshot(request.body(Body::empty()).expect("request"))
                    .await
                    .expect("response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = String::from_utf8(
                    to_bytes(response.into_body(), 1024 * 1024)
                        .await
                        .expect("body")
                        .to_vec(),
                )
                .expect("text");
                assert!(body.contains(target), "missing {target}");
                assert!(body.contains("Preserve this brief"));
                if stage == "review" {
                    assert!(body.contains("Workers receive selected inputs,"));
                    assert!(body.contains("workflow-start"));
                    if selection != &once_selection {
                        assert!(body.contains("Review input"));
                        assert!(body.contains(&task_token));
                    }
                }
            }
        }
    }
    for (query, error) in [
        (
            format!("stage=review&workflow={selection}&brief=Keep+this"),
            "Choose the required saved input before review.",
        ),
        (
            format!("stage=review&workflow={once_selection}&brief=Keep+this&phase=invalid"),
            "Choose a model for every model phase.",
        ),
        (
            format!("stage=review&workflow={selection}&brief=Keep+this&target=missing"),
            "The selected project is unavailable.",
        ),
        (
            "stage=inputs".to_owned(),
            "Choose a workflow before you continue.",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/conversations/{}/workflow?{query}",
                        conversation.id.as_hex()
                    ))
                    .header(
                        header::COOKIE,
                        format!("powerplant_session={}", token.raw().as_str()),
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let body = String::from_utf8(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body")
                .to_vec(),
        )
        .expect("text");
        assert!(body.contains(error));
        assert!(!body.contains("id=\"workflow-start\""));
        if query.contains("brief=") {
            assert!(body.contains("Keep this"));
        }
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/conversations/{}/workflow?stage=review&workflow=stale",
                    conversation.id.as_hex()
                ))
                .header(
                    header::COOKIE,
                    format!("powerplant_session={}", token.raw().as_str()),
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text");
    assert!(body.contains("This workflow selection is no longer available."));
    assert!(!state.sessions.busy(&token.id()));
    assert!(
        state
            .workflow_runs
            .for_conversation(&conversation.id)
            .is_empty()
    );
    assert!(
        state
            .task_loops
            .for_conversation(&conversation.id)
            .is_empty()
    );
    assert_eq!(
        state
            .conversations
            .get(&conversation.id)
            .expect("conversation"),
        conversation
    );
}
