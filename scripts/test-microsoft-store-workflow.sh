#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_dir"

stop_with_diagnostic() {
  printf 'ERROR: %s\n' "$1" >&2
  exit 2
}

for dependency in jq ruby shellcheck; do
  command -v "$dependency" >/dev/null 2>&1 ||
    stop_with_diagnostic "Required command not found: $dependency"
done

actionlint_bin="${ACTIONLINT:-}"
if [[ -z "$actionlint_bin" ]]; then
  actionlint_bin="$(command -v actionlint || true)"
fi
[[ -n "$actionlint_bin" && -x "$actionlint_bin" ]] ||
  stop_with_diagnostic 'Required command not found: actionlint. Install it from https://github.com/rhysd/actionlint/releases or set ACTIONLINT to its executable path.'

store_scripts=(
  scripts/microsoft-store-fetch-listings.sh
  scripts/microsoft-store-listing-languages.sh
  scripts/microsoft-store-listing-metadata.sh
  scripts/microsoft-store-normalize-json.sh
  scripts/microsoft-store-prepare-metadata.sh
  scripts/test-microsoft-store-credentials.sh
  scripts/test-microsoft-store-fetch-listings.sh
  scripts/test-microsoft-store-listing-languages.sh
  scripts/test-microsoft-store-workflow.sh
)

for script in "${store_scripts[@]}"; do
  bash -n "$script"
done
shellcheck "${store_scripts[@]}"

scripts/test-microsoft-store-listing-languages.sh
scripts/test-microsoft-store-fetch-listings.sh

# ShellCheck is run directly above. Here actionlint validates workflow structure,
# expressions, reusable-workflow inputs, event configuration, and action metadata.
"$actionlint_bin" -shellcheck="" \
  .github/workflows/deploy-microsoft-store.yml \
  .github/workflows/deployment-ci.yml \
  .github/workflows/release-manual-main.yml

# Also ShellCheck every embedded command in the Store workflow. SC2129 is a
# non-functional grouping style suggestion in the input-normalization step.
"$actionlint_bin" -ignore 'SC2129' .github/workflows/deploy-microsoft-store.yml

ruby <<'RUBY'
require "yaml"

workflow_path = ".github/workflows/deploy-microsoft-store.yml"
workflow = YAML.load_file(workflow_path)
deploy = workflow.fetch("jobs").fetch("deploy")
steps = deploy.fetch("steps")

raise "Microsoft Store runner must be pinned to ubuntu-24.04" unless deploy.fetch("runs-on") == "ubuntu-24.04"

dotnet_step = steps.find { |step| step["name"] == "Install .NET 9 for Microsoft Store CLI" }
raise "missing pinned .NET setup action" unless dotnet_step&.fetch("uses") == "actions/setup-dotnet@a98b56852c35b8e3190ac28c8c2271da59106c68"
raise "Microsoft Store CLI requires .NET 9" unless dotnet_step.fetch("with").fetch("dotnet-version") == "9.0.x"

cli_step = steps.find { |step| step["name"] == "Install Microsoft Store CLI" }
raise "missing pinned Microsoft Store CLI action" unless cli_step&.fetch("uses") == "microsoft/microsoft-store-apppublisher@cc9910a8d59f2eb55cbb83df0a3800cf3b5300e0"
raise "Microsoft Store CLI version must be pinned" unless cli_step.fetch("with").fetch("version") == "v0.3.9"

package_check = steps.find { |step| step["name"] == "Wait for hosted MSI URLs to become reachable" }
raise "hosted package check must validate the MSI/OLE signature" unless package_check&.fetch("run").include?("d0cf11e0a1b11ae1")

prepare_index = steps.index { |step| step["name"] == "Prepare Microsoft Store listing metadata" }
package_index = steps.index { |step| step["name"] == "Update Microsoft Store draft submission packages" }
metadata_index = steps.index { |step| step["name"] == "Update Microsoft Store draft submission metadata" }
publish_index = steps.index { |step| step["name"] == "Publish Microsoft Store submission" }
raise "missing Microsoft Store submission steps" unless [prepare_index, package_index, metadata_index, publish_index].all?
raise "listing metadata must be prepared before the draft is mutated" unless prepare_index < package_index
raise "packages must be updated before metadata" unless package_index < metadata_index
raise "metadata must be attempted before publish" unless metadata_index < publish_index

mutating_steps = [package_index, metadata_index, publish_index].map { |index| steps.fetch(index) }
mutating_steps.each do |step|
  condition = step.fetch("if")
  raise "missing dry-run mutation guard for #{step.fetch("name")}" unless condition.include?("dry_run != 'true'")
  raise "missing Store preflight mutation guard for #{step.fetch("name")}" unless condition.include?("store_preflight != 'true'")
end

prepare_step = steps.fetch(prepare_index)
if prepare_step.fetch("run").match?(/msstore submission (update|updateMetadata|publish)/)
  raise "read-only Store preflight contains a mutating Store CLI command"
end

metadata_step = steps.fetch(metadata_index)
raise "optional listing metadata must not block package publishing" unless metadata_step["continue-on-error"] == true
raise "publish must remain a strict release gate" if steps.fetch(publish_index)["continue-on-error"]

manual_workflow = YAML.load_file(".github/workflows/release-manual-main.yml")
manual_jobs = manual_workflow.fetch("jobs")
preflight_job = manual_jobs.fetch("preflight_microsoft_store")
live_job = manual_jobs.fetch("deploy_microsoft_store")
raise "release must run a read-only Store preflight" unless preflight_job.fetch("with").fetch("store_preflight") == true
raise "live Store submission must disable preflight mode" unless live_job.fetch("with").fetch("store_preflight") == false
raise "live Store submission must require the preflight" unless live_job.fetch("needs").include?("preflight_microsoft_store")
raise "live Store submission condition must require successful preflight" unless live_job.fetch("if").include?("needs.preflight_microsoft_store.result == 'success'")
RUBY

printf '%s\n' 'Microsoft Store workflow tests passed.'
