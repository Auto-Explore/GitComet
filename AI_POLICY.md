# GitComet AI Policy

You may use AI tools to write code, tests and documentation for GitComet, but a human must review every change before it is merged.

This policy covers everyone who contributes, maintainers included, and every kind of contribution: code, tests, documentation, commit messages, pull requests, issues and review comments. An AI tool is any generative model or agent, from inline code completion to a coding agent that edits files and runs commands.

## Human review

No change merges into GitComet until a human has reviewed all of it.

- **The submitter reviews first.** Before you ask anyone else to review a change, read every line of it, including the tests and documentation an AI tool wrote. You understand what each part does and why, and can explain it to a reviewer without asking the tool again.
- **The submitter verifies.** Run the builds and checks in [Testing](CONTRIBUTING.md#testing) that apply to the change. State what you verified and identify any applicable checks you could not run, with the reason. Check that the tests an AI tool wrote can fail when the behavior they test breaks.
- **A maintainer reviews each pull request.** A maintainer reads a contributor's pull request before merging it. A maintainer's own review satisfies this requirement for changes they author. AI review tools may help the reviewer find problems, but an AI review never counts as the review, and only a human approves or rejects a change.
- **Keep the change focused.** Remove edits the task does not need, such as unrelated refactors, reformatting or comments that restate the code.

## Agents

A coding agent may edit files, run commands and draft commits on your machine. A human decides what leaves it.

- **No autonomous submissions.** An agent may open pull requests or issues, post comments, push changes or merge only after a human has reviewed the relevant content or changes and approved the action. Pull requests and issues an agent opens on its own are closed.
- **Automation is separate.** Rule-based automation, such as dependency updates, builds, tests and CLA checks, falls outside this policy. Any AI-generated contributions produced through those systems remain covered. Dependabot's pull requests still get a maintainer's review before they merge.

## Disclosure

- **Contributions.** When an AI tool generates a meaningful part of the code or text you submit, disclose that use and name the tool in the pull request description or alongside the contribution, including when you have edited the output. Routine spelling and grammar corrections and trivial inline completions need no disclosure.

## Copyright and licensing

You are responsible for AI output the same way you are for code you type yourself, and the [CLA](CLA.md) applies to it unchanged.

- **Tool terms.** Use a tool whose terms let you contribute its output under AGPL-3.0-only and grant the rights the CLA describes.
- **Recognizable code.** If an AI tool's output reproduces existing code or documentation, such as a recognizable function from another project or its license header, treat it as third-party material under [Third-party material](CONTRIBUTING.md#third-party-material-and-employer-owned-contributions), or remove it. Asking a tool to rewrite copyrighted code does not remove its copyright.

## Security and private data

- **Protect private data.** Do not give AI tools secrets, credentials, signing keys, or any other private data. This includes data exposed through prompts, attachments, files an agent can read, and command output supplied to it.
- **New dependencies.** Check that each crate an AI tool adds exists, is the crate you meant, and is maintained, and declare it as [Workspace layout](CONTRIBUTING.md#workspace-layout) describes.
- **Risky code.** Avoid `unsafe` code whenever possible. When `unsafe` is necessary, explain why and document the conditions required for it to be safe. Review `unsafe` blocks, Git and shell command construction, path handling and network access with extra care.

## Communication

Pull request descriptions and issues may be drafted with an AI tool, but you edit them until they are accurate and short. Answer reviewers using your own understanding and reasoning. AI may help with grammar, translation and clarity, but you must verify that the wording accurately expresses your answer.

## When a contribution does not follow this policy

A maintainer may ask for changes, or close a pull request without a full review, when its submitter cannot explain it, when it shows no sign of review, such as invented APIs, unrelated edits or tests that cannot fail, or when an agent submitted it alone. A maintainer may stop accepting contributions from an account that keeps submitting unreviewed AI output.

## Design rule

AI can accelerate writing a change, but careful human review is still required. The person using AI must review and understand the resulting contribution before asking anyone else to review it.

This policy draws on the AI policies of [Fedora](https://docs.fedoraproject.org/en-US/council/policy/ai-contribution-policy/), [LLVM](https://llvm.org/docs/AIToolPolicy.html), the [Linux kernel](https://docs.kernel.org/process/coding-assistants.html), the [Apache Software Foundation](https://www.apache.org/legal/generative-tooling.html) and [Home Assistant](https://contributors.home-assistant.io/start/ai-policy/).
