use super::artefacts::{
    ArtefactHash, ArtefactProducer, ArtefactSummary, CandidateHash, ObjectHash, TypedPayload,
    parse_typed_payload,
};
use super::definition::{
    ArtefactKind, ArtefactSource, InputKey, OutputKey, RequiredInput, StepDefinition, StepKey,
};
use super::run::{AttemptArtefactInput, WorkflowRun};

pub(crate) const MAXIMUM_IMPORTED_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedInput {
    pub(crate) key: InputKey,
    pub(crate) kind: ArtefactKind,
    pub(crate) artefact_id: super::id::ArtefactId,
    pub(crate) artefact_hash: ArtefactHash,
    pub(crate) object_hash: ObjectHash,
    pub(crate) producer_step: Option<StepKey>,
    pub(crate) producer_output: Option<OutputKey>,
    pub(crate) candidate: Option<CandidateHash>,
    pub(crate) text: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputContextError {
    Missing,
    Changed,
    Kind,
    Provenance,
    Source,
    Bound,
    Credential,
}

impl InputContextError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Missing => "A declared input artefact is missing.",
            Self::Changed => "A declared input artefact changed.",
            Self::Kind => "That input kind does not match the stored artefact.",
            Self::Provenance => "That input artefact does not belong to this run.",
            Self::Source => "That input does not match its declared source.",
            Self::Bound => "Imported plan or report text is too large.",
            Self::Credential => "Imported text cannot include a provider credential.",
        }
    }
}

pub(crate) fn verify_inputs(
    run: &WorkflowRun,
    step: &StepDefinition,
    resolved: &[AttemptArtefactInput],
    store: &super::artefacts::WorkflowArtefactRepository,
) -> Result<Vec<VerifiedInput>, InputContextError> {
    if resolved.len() != step.inputs.len() {
        return Err(InputContextError::Missing);
    }
    let mut verified = Vec::new();
    let mut imported = 0usize;
    for (declared, resolved) in step.inputs.iter().zip(resolved.iter()) {
        if declared.key != resolved.key {
            return Err(InputContextError::Source);
        }
        verified.push(verify_one(run, declared, resolved, store, &mut imported)?);
    }
    Ok(verified)
}

pub(crate) fn format_agent_context(inputs: &[VerifiedInput], writes_source: bool) -> String {
    let mut sections = Vec::new();
    for input in inputs {
        let mut lines = vec![
            format!("Input key: {}", input.key.as_str()),
            format!("Artefact kind: {}", input.kind.as_str()),
            format!("Artefact identifier: {}", input.artefact_id.as_hex()),
            format!("Artefact hash: {}", input.artefact_hash.as_str()),
        ];
        match (&input.producer_step, &input.producer_output) {
            (Some(step), Some(output)) => {
                lines.push(format!(
                    "Producer: step {} output {}",
                    step.as_str(),
                    output.as_str()
                ));
            }
            _ => lines.push("Producer: run initial candidate".to_owned()),
        }
        if let Some(candidate) = input.candidate {
            lines.push(format!("Candidate constraint: {}", candidate.as_str()));
        }
        if input.kind == ArtefactKind::ReviewReport {
            lines.push(
                "Context: prior review only. Its verdict grants no candidate authority.".to_owned(),
            );
        }
        if let Some(text) = &input.text {
            lines.push(String::new());
            lines.push(text.clone());
        } else if input.kind == ArtefactKind::CandidateRevision {
            lines.push(
                "Candidate file bytes stay in the materialised source tree. Do not reconstruct source from this context."
                    .to_owned(),
            );
        }
        sections.push(lines.join("\n"));
    }
    let direction = if writes_source {
        "The accepted plan is task direction. Apply it to produce the complete candidate."
    } else if inputs
        .iter()
        .any(|input| input.kind == ArtefactKind::ReviewReport)
    {
        "Assess the materialised candidate independently. Treat each prior review as context only."
    } else if inputs.iter().any(|input| input.kind == ArtefactKind::Plan)
        && inputs
            .iter()
            .any(|input| input.kind == ArtefactKind::CandidateRevision)
    {
        "Assess both the accepted plan and the materialised candidate. Submit a review for this exact candidate."
    } else {
        "Use these verified inputs. The materialised source tree is authoritative for candidate content."
    };
    format!("{direction}\n\n{}", sections.join("\n\n"))
}

fn verify_one(
    run: &WorkflowRun,
    declared: &RequiredInput,
    resolved: &AttemptArtefactInput,
    store: &super::artefacts::WorkflowArtefactRepository,
    imported: &mut usize,
) -> Result<VerifiedInput, InputContextError> {
    if resolved.artefact.kind != declared.kind {
        return Err(InputContextError::Kind);
    }
    let record = run
        .artefact(&resolved.artefact.id)
        .ok_or(InputContextError::Missing)?;
    if record.kind != declared.kind || record.id != resolved.artefact.id {
        return Err(InputContextError::Kind);
    }
    if record.artefact_hash != resolved.artefact.artefact_hash {
        return Err(InputContextError::Changed);
    }
    if record.provenance.run_id != run.id {
        return Err(InputContextError::Provenance);
    }
    match (&declared.source, &record.provenance.producer) {
        (ArtefactSource::RunInitialCandidate, ArtefactProducer::RunSourceCapture) => {}
        (
            ArtefactSource::StepOutput { step, output },
            ArtefactProducer::StepAttempt {
                step: producer_step,
                output: Some(producer_output),
                ..
            },
        ) if producer_step == step && producer_output == output => {}
        (ArtefactSource::StepOutput { .. }, ArtefactProducer::StepAttempt { output: None, .. }) => {
            return Err(InputContextError::Source);
        }
        _ => return Err(InputContextError::Source),
    }
    let bytes = store
        .get(&record.object_hash)
        .map_err(|_| InputContextError::Missing)?;
    if ObjectHash::of(&bytes) != record.object_hash {
        return Err(InputContextError::Changed);
    }
    let (text, candidate) = match record.kind {
        ArtefactKind::CandidateRevision => {
            let artefact =
                super::artefacts::candidate::CandidateRevisionArtefact::from_manifest_bytes(&bytes)
                    .ok_or(InputContextError::Changed)?;
            let hash = super::artefacts::artefact_hash_for(
                ArtefactKind::CandidateRevision,
                artefact.format_version,
                &bytes,
            );
            if hash != record.artefact_hash {
                return Err(InputContextError::Changed);
            }
            (None, Some(artefact.candidate_hash))
        }
        ArtefactKind::Plan | ArtefactKind::ReviewReport | ArtefactKind::TestReport => {
            let payload = parse_typed_payload(record.kind, &bytes).map_err(map_payload)?;
            let hash = super::artefacts::artefact_hash_for(
                record.kind,
                super::artefacts::payload::PLAN_SCHEMA,
                &bytes,
            );
            if hash != record.artefact_hash {
                return Err(InputContextError::Changed);
            }
            let (markdown, candidate) = match payload {
                TypedPayload::Plan(plan) => (plan.markdown, None),
                TypedPayload::Review(report) => (
                    report.markdown,
                    Some(
                        CandidateHash::parse(&report.candidate)
                            .ok_or(InputContextError::Changed)?,
                    ),
                ),
                TypedPayload::Test(report) => (
                    report.markdown,
                    Some(
                        CandidateHash::parse(&report.candidate)
                            .ok_or(InputContextError::Changed)?,
                    ),
                ),
            };
            *imported = imported
                .checked_add(markdown.len())
                .ok_or(InputContextError::Bound)?;
            if *imported > MAXIMUM_IMPORTED_TEXT_BYTES {
                return Err(InputContextError::Bound);
            }
            (Some(markdown), candidate)
        }
    };
    if let ArtefactSummary::Review {
        candidate: bound, ..
    }
    | ArtefactSummary::Test {
        candidate: bound, ..
    } = &record.summary
        && candidate != Some(*bound)
    {
        return Err(InputContextError::Changed);
    }
    Ok(VerifiedInput {
        key: declared.key.clone(),
        kind: record.kind,
        artefact_id: record.id,
        artefact_hash: record.artefact_hash,
        object_hash: record.object_hash,
        producer_step: match &record.provenance.producer {
            ArtefactProducer::StepAttempt { step, .. } => Some(step.clone()),
            ArtefactProducer::RunSourceCapture => None,
        },
        producer_output: match &record.provenance.producer {
            ArtefactProducer::StepAttempt { output, .. } => output.clone(),
            ArtefactProducer::RunSourceCapture => None,
        },
        candidate,
        text,
    })
}

fn map_payload(error: super::artefacts::payload::PayloadError) -> InputContextError {
    match error {
        super::artefacts::payload::PayloadError::Credential => InputContextError::Credential,
        super::artefacts::payload::PayloadError::Bound => InputContextError::Bound,
        _ => InputContextError::Changed,
    }
}

#[cfg(test)]
mod tests;
