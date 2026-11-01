Bumps [rustls-webpki](https://github.com/rustls/webpki) from 0.103.8 to 0.103.15.
<details>
<summary>Release notes</summary>
<p><em>Sourced from <a href="https://github.com/rustls/webpki/releases">rustls-webpki's releases</a>.</em></p>
<blockquote>
<h2>v/0.103.15</h2>
<p>Fixes <code>docs.rs</code> build errors.</p>
<h2>What's Changed</h2>
<ul>
<li>Cargo: avoid all-features in docs.rs  by <a href="https://github.com/cpu"><code>@​cpu</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/530">rustls/webpki#530</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a href="https://github.com/rustls/webpki/compare/v/0.103.14..v/0.103.15">https://github.com/rustls/webpki/compare/v/0.103.14..v/0.103.15</a></p>
<h2>v/0.103.14</h2>
<h2>What's Changed</h2>
<ul>
<li>[0.103] Support stable ML-DSA by <a href="https://github.com/djc"><code>@​djc</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/523">rustls/webpki#523</a></li>
<li>[0.103] Mark ML-DSA algorithms as FIPS submitted by <a href="https://github.com/djc"><code>@​djc</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/525">rustls/webpki#525</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a href="https://github.com/rustls/webpki/compare/v/0.103.13...v/0.103.14">https://github.com/rustls/webpki/compare/v/0.103.13...v/0.103.14</a></p>
<h2>0.103.13</h2>
<ul>
<li><strong>Fix reachable panic in parsing a CRL</strong>. This was reported to us as <a href="https://github.com/rustls/webpki/security/advisories/GHSA-82j2-j2ch-gfr8">GHSA-82j2-j2ch-gfr8</a>. Users who don't use CRLs are not affected.</li>
<li>For name constraints on URI names, we incorrectly processed excluded subtrees in a way which inverted the desired meaning. See <a href="https://redirect.github.com/rustls/webpki/pull/471">rustls/webpki#471</a>. This was a case missing in the fix for <a href="https://github.com/advisories/GHSA-965h-392x-2mh5">https://github.com/advisories/GHSA-965h-392x-2mh5</a>.</li>
</ul>
<h2>What's Changed</h2>
<ul>
<li>Actually fail closed for URI matching against excluded subtrees by <a href="https://github.com/djc"><code>@​djc</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/473">rustls/webpki#473</a></li>
<li>Prepare 0.103.13 by <a href="https://github.com/ctz"><code>@​ctz</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/474">rustls/webpki#474</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a href="https://github.com/rustls/webpki/compare/v/0.103.12...v/0.103.13">https://github.com/rustls/webpki/compare/v/0.103.12...v/0.103.13</a></p>
<h2>0.103.12</h2>
<p>This release fixes two bugs in name constraint enforcement:</p>
<ul>
<li><strong>GHSA-965h-392x-2mh5</strong>: name constraints for URI names were ignored and therefore accepted. URI name constraints are now rejected unconditionally. Note this library does not provide an API for asserting URI names, and URI name constraints are otherwise not implemented.</li>
<li><strong>GHSA-xgp8-3hg3-c2mh</strong>: permitted subtree name constraints for DNS names were accepted for certificates asserting a wildcard name. This was incorrect because, given a name constraint of <code>accept.example.com</code>, <code>*.example.com</code> could feasibly allow a name of <code>reject.example.com</code> which is outside the constraint. This is very similar to <a href="https://go.dev/issue/76442">CVE-2025-61727</a>.</li>
</ul>
<p>Since name constraints are restrictions on otherwise properly-issued certificates, these bugs are reachable only after signature verification and require misissuance to exploit.</p>
<h2>What's Changed</h2>
<ul>
<li>Prepare 0.103.12 by <a href="https://github.com/djc"><code>@​djc</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/470">rustls/webpki#470</a></li>
</ul>
<p><strong>Full Changelog</strong>: <a href="https://github.com/rustls/webpki/compare/v/0.103.11...v/0.103.12">https://github.com/rustls/webpki/compare/v/0.103.11...v/0.103.12</a></p>
<h2>0.103.11</h2>
<p>In response to <a href="https://redirect.github.com/rustls/webpki/issues/464">#464</a>, we've slightly relaxed requirements for <code>anchor_from_trust_cert()</code> to ignore unknown extensions even if they're marked as critical. This only affects parsing a <code>TrustAnchor</code> from DER, for which most extensions are ignored anyway.</p>
<h2>What's Changed</h2>
<ul>
<li>Backport parsing trust anchors with unknown critical extensions to 0.103 by <a href="https://github.com/djc"><code>@​djc</code></a> in <a href="https://redirect.github.com/rustls/webpki/pull/466">rustls/webpki#466</a></li>
</ul>
<h2>0.103.10</h2>
<!-- raw HTML omitted -->
</blockquote>
<p>... (truncated)</p>
</details>
<details>
<summary>Commits</summary>
<ul>
<li><a href="https://github.com/rustls/webpki/commit/c14836d8de33c0ad0ac7dcb28fe9aade20831a4d"><code>c14836d</code></a> Cargo: version 0.103.14 -&gt; 0.103.15</li>
<li><a href="https://github.com/rustls/webpki/commit/5c9230825b933f84221936064cb12465b95b193c"><code>5c92308</code></a> Cargo: avoid all-features in docs.rs</li>
<li><a href="https://github.com/rustls/webpki/commit/10a151423089a0824b2e8a456c3be63b7d70b448"><code>10a1514</code></a> Mark ML-DSA algorithms as FIPS submitted</li>
<li><a href="https://github.com/rustls/webpki/commit/14cbaa18eaacb2ad4110206b7d02c1ad08aabb42"><code>14cbaa1</code></a> Bump version to 0.103.14</li>
<li><a href="https://github.com/rustls/webpki/commit/cbb868aab78dc13664a5e8da2ed17257a36e5e46"><code>cbb868a</code></a> Support stable ML-DSA with aws-lc-rs</li>
<li><a href="https://github.com/rustls/webpki/commit/c782fd1c29bc3ea7d6d1e9be96de95b080da8114"><code>c782fd1</code></a> Update to aws-lc-rs 1.18</li>
<li><a href="https://github.com/rustls/webpki/commit/b250e31d000b476820034631e55e90f0912a2cec"><code>b250e31</code></a> Upgrade to base64 0.23</li>
<li><a href="https://github.com/rustls/webpki/commit/99115b2730e9da4c1a734d6f8c8e7bfe91aa5657"><code>99115b2</code></a> ci: sync cargo-check-external-types nightly</li>
<li><a href="https://github.com/rustls/webpki/commit/a82bfaebf706fe5df325024f213713b8579c731e"><code>a82bfae</code></a> crl: fix clippy::question_mark finding</li>
<li><a href="https://github.com/rustls/webpki/commit/2879b2ce7a476181ac3050f73fe0835f04728e86"><code>2879b2c</code></a> Prepare 0.103.13</li>
<li>Additional commits viewable in <a href="https://github.com/rustls/webpki/compare/v/0.103.8...v/0.103.15">compare view</a></li>
</ul>
</details>
<br />


[![Dependabot compatibility score](https://dependabot-badges.githubapp.com/badges/compatibility_score?dependency-name=rustls-webpki&package-manager=cargo&previous-version=0.103.8&new-version=0.103.15)](https://docs.github.com/en/github/managing-security-vulnerabilities/about-dependabot-security-updates#about-compatibility-scores)

Dependabot will resolve any conflicts with this PR as long as you don't alter it yourself. You can also trigger a rebase manually by commenting `@dependabot rebase`.

[//]: # (dependabot-automerge-start)
[//]: # (dependabot-automerge-end)

---

<details>
<summary>Dependabot commands and options</summary>
<br />

You can trigger Dependabot actions by commenting on this PR:
- `@dependabot rebase` will rebase this PR
- `@dependabot recreate` will recreate this PR, overwriting any edits that have been made to it
- `@dependabot show <dependency name> ignore conditions` will show all of the ignore conditions of the specified dependency
- `@dependabot ignore this major version` will close this PR and stop Dependabot creating any more for this major version (unless you reopen the PR or upgrade to it yourself)
- `@dependabot ignore this minor version` will close this PR and stop Dependabot creating any more for this minor version (unless you reopen the PR or upgrade to it yourself)
- `@dependabot ignore this dependency` will close this PR and stop Dependabot creating any more for this dependency (unless you reopen the PR or upgrade to it yourself)
You can disable automated security fix PRs for this repo from the [Security Alerts page](https://github.com/example/repository/network/alerts).

</details>
