#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interrupted;

pub(crate) fn check(should_stop: &dyn Fn() -> bool) -> Result<(), Interrupted> {
    if should_stop() {
        return Err(Interrupted);
    }
    Ok(())
}
