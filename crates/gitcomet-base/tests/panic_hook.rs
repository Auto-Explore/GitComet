use std::cell::Cell;
use std::sync::Once;

use gitcomet_base::panic::{ContainedPanic, contain, note_contained};

static HOOK: Once = Once::new();

thread_local! {
    static UNCONTAINED: Cell<usize> = const { Cell::new(0) };
}

fn install_noting_hook() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if note_contained(info) {
                return;
            }
            UNCONTAINED.with(|count| count.set(count.get() + 1));
            previous(info);
        }));
    });
}

#[test]
fn a_noted_panic_carries_its_message_and_location() {
    install_noting_hook();
    let line = line!() + 1;
    let result = contain(|| panic!("boom {}", 7));
    assert_eq!(
        result,
        Err(ContainedPanic {
            message: "boom 7".to_owned(),
            location: Some(format!("{}:{line}", file!())),
        })
    );
    assert_eq!(UNCONTAINED.with(Cell::get), 0);
}

#[test]
fn an_outer_containment_still_notes_after_an_inner_one_caught() {
    install_noting_hook();
    let mut line = 0;
    let outer = contain(|| {
        let inner = contain(|| panic!("inner"));
        assert_eq!(
            inner.map_err(|panic| panic.message),
            Err("inner".to_owned())
        );
        line = line!() + 1;
        panic!("outer");
    });
    assert_eq!(
        outer,
        Err(ContainedPanic {
            message: "outer".to_owned(),
            location: Some(format!("{}:{line}", file!())),
        })
    );
    assert_eq!(UNCONTAINED.with(Cell::get), 0);
}

#[test]
fn a_panic_after_contain_returns_reaches_the_previous_hook() {
    install_noting_hook();
    let _ = contain(|| panic!("contained"));
    let _ = std::panic::catch_unwind(|| panic!("not contained"));
    assert_eq!(UNCONTAINED.with(Cell::get), 1);
}

#[test]
fn a_second_panic_before_contain_takes_the_first_is_reported() {
    install_noting_hook();
    let result = contain(|| {
        let _ = std::panic::catch_unwind(|| panic!("first"));
        panic!("second");
    });
    assert_eq!(
        result,
        Err(ContainedPanic {
            message: "second".to_owned(),
            location: None,
        })
    );
    assert_eq!(UNCONTAINED.with(Cell::get), 1);
}
