# GitComet CLA administration

Use hosted CLA Assistant with the Harmony-based Individual CLA in `/CLA.md`.
`/CLA-ENTITY.md` is a separate route for company-owned work. `metadata` in this
directory defines the minimal hosted signing form: legal name, email and acceptance.

This file is operational documentation, not a signature record or a declaration
that the integration is already live. There is no CLA GitHub Actions workflow.

## Publication

Approve the individual and entity agreements before soliciting acceptance.
The supplied texts are review drafts, not lawyer-approved agreements. Resolve the
review checklist, including Finland as the proposed governing law and the
click-to-accept wording. Do not silently import earlier contributors as signed.
Prepare appropriate contributor privacy information for the actual collection
and storage of signing information. Keep personal details out of public issues.

After approval, remove the draft banner and change `Version: 1.0-review-draft`
to the approved version, for example `Version: 1.0`. Commit and merge the
agreements and contribution instructions using the existing review process.
Do not change `LICENSE-AGPL-3.0` or other software-license files.

Create one public GitHub Gist with exactly two files:

- `CLA.md`: the approved individual agreement, byte-for-byte identical to the repository copy.
- `metadata`: this directory's metadata file, with no `.json` extension.

You may use the separate `publish-gist.py` setup helper, or run from the repository:

```sh
gh gist create --public \
  --desc "GitComet Individual Contributor License Agreement — v1.0" \
  CLA.md .github/cla/metadata
```

Use the actual approved version in the description. The command creates a PUBLIC
Gist under the authenticated GitHub account. It does not sign the agreement.
Record the URL and Gist revision with the agreement version. Avoid editing a live
Gist just to tidy formatting: a change can require renewed acceptance.

## Connect the service

Open https://cla-assistant.io/ and sign in with a GitHub account authorized to
administer `Auto-Explore/GitComet`. Review the actual access permissions. Choose
public-repository access where available; selecting one repository in the service
is not proof that its OAuth authorization is limited to that repository.

Configure a CLA for `Auto-Explore/GitComet`, not the entire organization, and
associate the new Gist URL. Complete any app installation or organization approval
that the service requests. Do not disable the organization's general access policy.

Leave human exemptions and imported signers empty. Use verified equivalent rights
or a properly executed Entity CLA only through a documented, private procedure.
Narrow bot exemptions are operational; they do not relicense dependencies.

## Test before making the check required

Use a consenting contributor whose GitHub account has not accepted this agreement.
Have them open a small PR, inspect the bot's request, and confirm that the CLA
status is not successful before acceptance. Then have the person read and accept
it. This is a real agreement acceptance, not a fabricated test signature.

Confirm the successful PR status and the recorded agreement version and identity.
Open a later PR by the same signer to check reuse. Separately test multi-author
PRs, an unsigned co-author, and adding an unsigned author after a previous success.
Inspect bot behavior rather than assuming it can identify every rights holder.
Never remove attribution to make the check green.

Record the exact successful check name and, where available, its genuine GitHub
App source. Do not invent either. A required check needs a successful run in the
repository within the past seven days, according to GitHub's documentation.

## Enforce through a branch ruleset

Open repository Settings → Rules / Rulesets → New branch ruleset. Preserve all
existing CI and code-review requirements; add a dedicated CLA rule:

| Setting | Value |
|---|---|
| Name | Contributor license agreement |
| Target | `dev` and other branches accepting contributions directly for release |
| Bypass list | Repository admin, mode "Always allow" (for direct pushes to `dev`) |
| Require a pull request | Enabled |
| Require status checks | Enabled; select the exact CLA status observed in the test |
| Expected source | Actual producing app, if offered by GitHub |
| Enforcement | Active only after the successful test |

Do not add another approving reviewer solely for the CLA process. "Require signed
commits" is not CLA enforcement. Check compatibility separately if you use merge
queues. Review release automation and other routes that may introduce code.

Check that an unsigned PR from a non-admin is now blocked, and that signed PRs
can proceed once all other requirements pass. Admins can push and merge past the
check, so admins should sign the CLA too and keep the bypass list to that role.
A successful CLA status records agreement acceptance; it does not automatically
merge a PR or audit code ownership.

## Records and recovery

The hosted service records normal individual acceptance and updates PR statuses.
Use its dashboard to inspect signers and export CSV records. Keep exports and
executed entity agreements privately, with the exact agreement versions, relevant
PRs and evidence supporting any manual coverage import. This setup does not
implement a separate automated backup of the hosted signing database.

For a stale status, use the dashboard's Recheck PRs action. For one PR:

https://cla-assistant.io/check/Auto-Explore/GitComet?pullRequest=123

Replace `123` with the actual number. Do not fake a signed record to work around
an outage. If administration changes, verify that company maintainers still have
access to both the Gist and the service configuration.

## Primary documentation

- Harmony template: https://www.harmonyagreements.org/docs/ha-combined-v1
- Hosted CLA Assistant and metadata: https://github.com/cla-assistant/cla-assistant
- Setup interface: https://cla-assistant.io/
- Gist CLI: https://cli.github.com/manual/gh_gist_create
- Rulesets: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository
- Required checks: https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks

Documentation consulted September 30, 2026. Interface labels may vary.
