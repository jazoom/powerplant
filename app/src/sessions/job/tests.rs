use super::{JOB_ID_LENGTH, Job, JobId, JobStatus};
use crate::workflows::RunId;

fn job() -> std::sync::Arc<Job> {
    Job::new(
        JobId::generate().expect("job id"),
        RunId::generate().expect("run"),
        1,
    )
}

#[test]
fn generated_job_ids_round_trip() {
    let id = JobId::generate().expect("job id");
    let hex = id.as_hex();
    assert_eq!(hex.len(), JOB_ID_LENGTH);
    assert!(hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(JobId::parse(&hex), Some(id));
}

#[test]
fn rejects_malformed_job_ids() {
    assert!(JobId::parse("").is_none());
    assert!(JobId::parse("abc").is_none());
    assert!(JobId::parse(&"g".repeat(JOB_ID_LENGTH)).is_none());
    assert!(JobId::parse(&"A".repeat(JOB_ID_LENGTH)).is_none());
    assert!(JobId::parse(&format!("{}0", "a".repeat(JOB_ID_LENGTH - 1))).is_some());
}

#[test]
fn output_events_are_monotonic_and_reconstructable() {
    let job = job();
    assert_eq!(job.push_output("Hel".to_owned()), Some(1));
    assert_eq!(job.push_output("lo".to_owned()), Some(2));
    assert_eq!(job.output_up_to(0), "");
    assert_eq!(job.output_up_to(1), "Hel");
    assert_eq!(job.output_up_to(2), "Hello");
    assert_eq!(job.events_after(0).len(), 2);
    assert_eq!(job.events_after(1).len(), 1);
    assert!(job.events_after(2).is_empty());
}

#[test]
fn finish_is_idempotent() {
    let job = job();
    assert_eq!(job.finish(JobStatus::Completed, None), Some(1));
    assert_eq!(job.finish(JobStatus::Failed, Some("no")), None);
    assert_eq!(job.snapshot().status, JobStatus::Completed);
    assert!(job.push_output("late".to_owned()).is_none());
}
