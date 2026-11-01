import unittest

import perf_workloads


local = perf_workloads.module("test_local_performance_driver", "local-performance.py")


def repository_trait(status, policy):
    return f"""pub trait GitRepository: Send + Sync {{
    fn status(&self{status}) -> Result<RepoStatus>;
    fn set_remote_url_with_output(
        &self,
        _name: &str,
        _url: &str,
        _kind: RemoteUrlKind,
        {policy}
    ) -> Result<CommandOutput> {{
        unimplemented!()
    }}
}}
"""


class RepositoryApiTests(unittest.TestCase):
    def test_legacy_repository_selects_legacy_adapter(self):
        mode = local.repository_api_mode(repository_trait("", ""))
        self.assertEqual(mode, "legacy")
        self.assertIn('cargo:rustc-cfg=gitcomet_probe_legacy_repository_api',
                      local.repository_api_build_script(mode))

    def test_current_repository_selects_current_adapter(self):
        mode = local.repository_api_mode(repository_trait(
            ", cancellation: &CancellationToken",
            "_remote_url_policy: crate::remote_url::RemoteUrlPolicy,"))
        self.assertEqual(mode, "current")
        script = local.repository_api_build_script(mode)
        self.assertIn('cargo:rustc-check-cfg=cfg(gitcomet_probe_legacy_repository_api)', script)
        self.assertNotIn('cargo:rustc-cfg=', script)

    def test_mixed_unknown_and_duplicate_signatures_are_rejected(self):
        legacy = repository_trait("", "")
        cases = [
            repository_trait(", cancellation: &CancellationToken", ""),
            repository_trait("", "_remote_url_policy: crate::remote_url::RemoteUrlPolicy,"),
            repository_trait(", cancellation: Option<&CancellationToken>", ""),
            legacy.replace("Result<RepoStatus>", "Result<Option<RepoStatus>>"),
            legacy.replace("    fn status", "    fn renamed_status"),
            legacy.replace("    fn status", "    fn status(&self) -> Result<RepoStatus>;\n    fn status"),
        ]
        for source in cases:
            with self.subTest(source=source), self.assertRaisesRegex(ValueError, "Unsupported GitRepository probe API"):
                local.repository_api_mode(source)

    def test_build_script_rejects_unknown_api_mode(self):
        with self.assertRaisesRegex(ValueError, "Unknown repository API mode"):
            local.repository_api_build_script("guess")

    def test_process_status_does_not_select_repository_api(self):
        source = repository_trait("", "") + "\nfn run() { command.status().unwrap(); }\n"
        self.assertEqual(local.repository_api_mode(source), "legacy")
