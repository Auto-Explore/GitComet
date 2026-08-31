use super::*;

#[test]
fn worker_command_prioritizes_cancel_git_operation_over_queued_hook_output() {
    let repo_id = RepoId(7);
    let operation_id = gitcomet_core::git_operation::GitOperationId(91);
    let (tx, rx) = std::sync::mpsc::channel();

    for sequence in 0..4 {
        tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
            crate::msg::InternalMsg::GitOperationEvent {
                repo_id,
                operation_id,
                event: gitcomet_core::git_operation::GitOperationEvent::Output {
                    chunks: vec![gitcomet_core::git_operation::GitOutputChunk {
                        stream: gitcomet_core::git_operation::GitOutputStream::Stdout,
                        text: format!("noisy hook output {sequence}\n"),
                    }],
                },
            },
        ))))
        .expect("send hook output");
    }
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CancelGitOperation {
        repo_id,
        operation_id,
    })))
    .expect("send cancellation");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    assert!(
        matches!(
            command,
            StoreWorkerCommand::Msg(msg)
                if matches!(
                    *msg,
                    Msg::CancelGitOperation {
                        repo_id: got_repo,
                        operation_id: got_operation,
                    } if got_repo == repo_id && got_operation == operation_id
                )
        ),
        "Stop must overtake already-queued hook output"
    );
}

#[test]
fn worker_command_prioritizes_close_repo_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_prioritizes_close_repo_over_queued_open_error() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::RepoOpenedErr {
            repo_id,
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/not-a-repo"),
            },
            error: Error::new(ErrorKind::NotARepository),
        },
    ))))
    .expect("send open error");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred open error");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred open error"),
    }
}

#[test]
fn worker_command_prioritizes_close_repos_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepos {
        repo_ids: vec![repo_id],
        activate_after: None,
    })))
    .expect("send close repos");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::CloseRepos {
                    repo_ids,
                    activate_after: None,
                } if repo_ids == vec![repo_id]
            ));
        }
        _ => panic!("expected close repos command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred tags result");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_prioritizes_tab_switch_over_queued_background_result() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::SetActiveRepo {
        repo_id,
    })))
    .expect("send tab switch");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::SetActiveRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected tab switch first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("deferred tags result");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            ));
        }
        _ => panic!("expected deferred tags result"),
    }
}

#[test]
fn worker_command_keeps_queued_open_repo_before_close_repo() {
    let repo_id = RepoId(7);
    let path = PathBuf::from("/tmp/repo");
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::OpenRepo(
        path.clone(),
    ))))
    .expect("send open");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("next command");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::OpenRepo(got) if got == path));
        }
        _ => panic!("expected open repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued close");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command second"),
    }
}

#[test]
fn worker_command_prioritizes_open_repo_over_queued_background_result() {
    let repo_id = RepoId(7);
    let path = PathBuf::from("/tmp/repo");
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::Internal(
        crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        },
    ))))
    .expect("send background result");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::OpenRepo(
        path.clone(),
    ))))
    .expect("send open");
    tx.send(StoreWorkerCommand::Msg(Box::new(Msg::CloseRepo {
        repo_id,
    })))
    .expect("send close");

    let mut deferred = std::collections::VecDeque::new();
    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued open");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::OpenRepo(got) if got == path));
        }
        _ => panic!("expected open repo command first"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("queued close");
    match command {
        StoreWorkerCommand::Msg(msg) => {
            assert!(matches!(*msg, Msg::CloseRepo { repo_id: got } if got == repo_id));
        }
        _ => panic!("expected close repo command second"),
    }

    let command = recv_next_worker_command(&rx, &mut deferred).expect("background result");
    assert!(matches!(
        command,
        StoreWorkerCommand::Msg(msg)
            if matches!(
                *msg,
                Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
                    repo_id: got,
                    ..
                }) if got == repo_id
            )
    ));
}

#[test]
fn guarded_effect_sender_wraps_repository_load_messages() {
    let repo_id = RepoId(7);
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = StoreWorkerSender::new(
        tx,
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
        StoreInstanceId::next(),
    );
    let guarded = sender.with_repo_load_guard(repo_id, 3, CancellationToken::new());

    guarded.send_effect_or_log(
        Msg::Internal(crate::msg::InternalMsg::TagsLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
        "guarded effect sender test",
    );

    let command = rx.recv_timeout(Duration::from_secs(1)).expect("message");
    match command {
        StoreWorkerCommand::Msg(msg) => match *msg {
            Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
                repo_id: got_repo_id,
                load_epoch,
                message,
            }) => {
                assert_eq!(got_repo_id, repo_id);
                assert_eq!(load_epoch, 3);
                assert!(matches!(
                    *message,
                    crate::msg::InternalMsg::TagsLoaded {
                        repo_id: got_inner_repo_id,
                        ..
                    } if got_inner_repo_id == repo_id
                ));
            }
            other => panic!("expected guarded load message, got {other:?}"),
        },
        _ => panic!("expected worker message"),
    }
}
