use gitcomet_core::identity::ProductIdentity;
use gitcomet_core::platform::test_harness::named_like_test_binary;
use std::{env, ffi::OsString, time::Duration};

fn uses_temporary_profile() -> bool {
    cfg!(test) || named_like_test_binary()
}

pub(crate) fn profile_root(identity: &ProductIdentity) -> Option<OsString> {
    uses_temporary_profile().then(|| {
        let user = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_default();
        let profile = format!("{}-test-profile-{user}", identity.directory_name());
        env::temp_dir().join(profile).into()
    })
}

pub(crate) fn stale_after(normal_retention: Duration) -> Duration {
    if uses_temporary_profile() {
        Duration::from_secs(60 * 60)
    } else {
        normal_retention
    }
}
