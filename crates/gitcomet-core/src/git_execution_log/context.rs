use super::RepoLogKey;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiSurface {
    Button,
    Menu,
    Shortcut,
    CommandPalette,
    Action,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemOrigin {
    Startup,
    RepositoryWatcher,
    Activation,
    AutomaticRefresh,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Origin {
    Ui {
        surface: UiSurface,
        action: Arc<str>,
    },
    Extension {
        command: Arc<str>,
    },
    System(SystemOrigin),
    Unknown,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ui { surface, action } => write!(f, "{surface:?}: {action}"),
            Self::Extension { command } => write!(f, "Extension: {command}"),
            Self::System(origin) => write!(f, "System: {origin:?}"),
            Self::Unknown => f.write_str("Unknown caller"),
        }
    }
}

impl Origin {
    pub(super) fn bounded(self) -> Self {
        let bounded =
            |text: &str| super::output::bounded_text(&super::output::sanitize(text), 1024).into();
        match self {
            Self::Ui { surface, action } => Self::Ui {
                surface,
                action: bounded(&action),
            },
            Self::Extension { command } => Self::Extension {
                command: bounded(&command),
            },
            other => other,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionContext {
    pub action_id: u64,
    pub origin: Origin,
    pub repository: Option<RepoLogKey>,
    pub parent_execution: Option<u64>,
}

impl ExecutionContext {
    pub fn new(origin: Origin) -> Self {
        static NEXT_ACTION: AtomicU64 = AtomicU64::new(1);
        Self {
            action_id: NEXT_ACTION.fetch_add(1, Ordering::Relaxed),
            origin,
            repository: None,
            parent_execution: None,
        }
    }

    pub fn ui(surface: UiSurface, action: impl Into<Arc<str>>) -> Self {
        Self::new(Origin::Ui {
            surface,
            action: action.into(),
        })
    }

    pub fn for_repository(mut self, repository: RepoLogKey) -> Self {
        self.repository = Some(repository);
        self
    }

    pub fn attach(&self) -> ContextScope {
        attach(Some(self))
    }

    pub fn wrap<T>(self, run: impl FnOnce() -> T) -> impl FnOnce() -> T {
        move || {
            let _scope = self.attach();
            run()
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<ExecutionContext>> = const { RefCell::new(None) };
}

pub struct ContextScope {
    previous: Option<ExecutionContext>,
    _thread: PhantomData<Rc<()>>,
}

impl Drop for ContextScope {
    fn drop(&mut self) {
        CURRENT.with(|slot| slot.replace(self.previous.take()));
    }
}

pub fn current() -> Option<ExecutionContext> {
    CURRENT.with(|slot| slot.borrow().clone())
}

pub fn attach(context: Option<&ExecutionContext>) -> ContextScope {
    ContextScope {
        previous: CURRENT.with(|slot| slot.replace(context.cloned())),
        _thread: PhantomData,
    }
}

pub fn wrap_task<T>(run: impl FnOnce() -> T) -> impl FnOnce() -> T {
    let context = current();
    move || {
        let _scope = attach(context.as_ref());
        run()
    }
}
