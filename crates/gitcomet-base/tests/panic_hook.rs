use gitcomet_base::panic::test_support::uncontained_panics;
use gitcomet_base::panic::{ContainedPanic, PanicLocation, contain};

#[test]
fn a_noted_panic_carries_its_message_and_location() {
    let line = line!() + 1;
    let (result, uncontained) = uncontained_panics(|| contain(|| panic!("boom {}", 7)));
    assert_eq!(
        result,
        Err(ContainedPanic {
            message: "boom 7".to_owned(),
            location: Some(PanicLocation {
                file: file!().to_owned(),
                line,
            }),
        })
    );
    assert_eq!(uncontained, 0);
}

#[test]
fn an_outer_containment_still_notes_after_an_inner_one_caught() {
    let mut line = 0;
    let (outer, uncontained) = uncontained_panics(|| {
        contain(|| {
            let inner = contain(|| panic!("inner"));
            assert_eq!(
                inner.map_err(|panic| panic.message),
                Err("inner".to_owned())
            );
            line = line!() + 1;
            panic!("outer");
        })
    });
    assert_eq!(
        outer,
        Err(ContainedPanic {
            message: "outer".to_owned(),
            location: Some(PanicLocation {
                file: file!().to_owned(),
                line,
            }),
        })
    );
    assert_eq!(uncontained, 0);
}

#[test]
fn a_panic_after_contain_returns_reaches_the_previous_hook() {
    let ((), uncontained) = uncontained_panics(|| {
        let _ = contain(|| panic!("contained"));
        let _ = std::panic::catch_unwind(|| panic!("not contained"));
    });
    assert_eq!(uncontained, 1);
}

#[test]
fn a_second_panic_before_contain_takes_the_first_is_reported() {
    let (result, uncontained) = uncontained_panics(|| {
        contain(|| {
            let _ = std::panic::catch_unwind(|| panic!("first"));
            panic!("second");
        })
    });
    assert_eq!(
        result,
        Err(ContainedPanic {
            message: "second".to_owned(),
            location: None,
        })
    );
    assert_eq!(uncontained, 1);
}
