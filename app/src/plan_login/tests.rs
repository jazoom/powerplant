use super::{PendingPlan, PlanLogin};
use crate::providers::ProviderKind;

fn pending(code: &str) -> PendingPlan {
    PendingPlan {
        kind: ProviderKind::Xai,
        verification_uri: "https://accounts.x.ai/connect".to_owned(),
        user_code: code.to_owned(),
        error: None,
    }
}

#[test]
fn a_stale_generation_cannot_change_the_current_login() {
    let login = PlanLogin::new();
    let stale = login.begin();
    login.set_pending(stale, pending("OLD-CODE"));

    let current = login.begin();
    login.set_pending(current, pending("NEW-CODE"));
    login.set_pending(stale, pending("STALE-CODE"));
    login.set_error(stale, "stale error".to_owned());
    login.finish(stale);

    assert_eq!(login.snapshot(), Some(pending("NEW-CODE")));
}

#[test]
fn a_current_operation_excludes_a_new_generation() {
    let login = std::sync::Arc::new(PlanLogin::new());
    let generation = login.begin();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let operation_login = login.clone();
    let operation = std::thread::spawn(move || {
        operation_login.apply_if_current(generation, || {
            entered_tx.send(()).expect("entered");
            release_rx.recv().expect("release");
        })
    });
    entered_rx.recv().expect("operation start");

    let (attempting_tx, attempting_rx) = std::sync::mpsc::channel();
    let (begun_tx, begun_rx) = std::sync::mpsc::channel();
    let begin_login = login.clone();
    let begin = std::thread::spawn(move || {
        attempting_tx.send(()).expect("attempting begin");
        begun_tx.send(begin_login.begin())
    });
    attempting_rx.recv().expect("begin attempt");
    assert!(
        begun_rx
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err()
    );

    release_tx.send(()).expect("release operation");
    assert_eq!(operation.join().expect("operation"), Some(()));
    assert_ne!(begun_rx.recv().expect("new generation"), generation);
    begin.join().expect("begin").expect("generation send");
}

#[tokio::test]
async fn a_new_generation_aborts_the_previous_task() {
    struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let login = PlanLogin::new();
    let generation = login.begin();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _notify = NotifyOnDrop(Some(dropped_tx));
        let _ = started_tx.send(());
        std::future::pending::<()>().await;
    });
    login.attach_task(generation, task);
    started_rx.await.expect("task start");

    login.begin();

    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("task abort timeout")
        .expect("task drop notification");
}
