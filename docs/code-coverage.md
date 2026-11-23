# Code Coverage Formats, Viewers, and Rust Tooling

For a Rust project, use `cargo-llvm-cov` to measure execution, keep LLVM's region detail for debugging, and export LCOV or Cobertura for the viewer that will read the result.

**Research snapshot, 23 November 2026.** This report covers source-level test coverage in Rust and the main formats a mixed-language repository will encounter. Compatibility comes from publisher documentation and parser implementations. The Rust walkthrough compares four actual exports of one test run.

[Format catalogue](#format-catalogue) · [Viewer compatibility](#viewer-compatibility) · [Popularity evidence](#what-the-popularity-evidence-shows) · [Rust tools](#measuring-rust-code) · [Worked example](#one-rust-test-run-four-views) · [GitComet findings](#what-applies-to-gitcomet)

## Choosing a report

| What you want to learn | Practical choice | Why |
| --- | --- | --- |
| Which part of a Rust expression did the tests miss? | Open `cargo llvm-cov --html`. Save full LLVM coverage JSON for a custom viewer. | LLVM retains source ranges down to columns. Two expressions on the same line can have different counts. [LLVM coverage views][llvm-cov] |
| Which lines did tests exercise in my checkout? | Export LCOV and open it with VS Code Coverage Gutters or render it with `genhtml`. | One text file can drive both an editor gutter and an offline source browser. [Coverage Gutters][gutters] · [LCOV][lcov] |
| Which Rust report can our Git host display itself? | Export Cobertura XML. | GitHub Code Quality accepts it for its pull request summary. GitLab accepts it for line annotations in merge request diffs. Those views answer different questions. [GitHub coverage reference][gh-ref] · [GitLab visualization][gl-diff] |
| What should a JVM team publish? | Keep JaCoCo XML when the destination accepts it. | JaCoCo carries instruction and branch counters that a line-only destination can discard. [JaCoCo counters][jac-counters] · [JaCoCo XML schema][jac-schema] |
| How do we compare commits or operating systems? | Use a coverage service or CI plugin with a baseline, and identify the commit and test configuration for each upload. | Codecov can group uploads by flag. Jenkins can compare modified files and lines to a reference build. [Codecov flags][cc-flags] · [Jenkins Coverage][jenkins] |

## What coverage measures

A coverage run associates execution with locations in one build. For example, a test that calls `classify(0)` proves the zero case was exercised. A line hit alone cannot tell a reviewer whether someone asserted the return value or exercised the other case. Source coverage answers “what ran?” The assertions in the test answer “what behavior did we check?” [Rust compiler coverage][rustc] · [LLVM coverage interpretation][clang]

| Metric | What counts as covered | Why it matters in a viewer |
| --- | --- | --- |
| Line | The producer records execution on a source line. | A single green line can contain an unexecuted expression. Blank lines and lines with no coverage mapping can sit outside the denominator. [LLVM metrics][clang] |
| Statement, block or region | A particular statement, basic block or mapped source range executed. | Several regions can share a line. Go weights covered blocks by the number of statements they contain. [LLVM metrics][clang] · [Go coverage][go-cover] |
| Function, method or instantiation | Control entered the function or a particular compiled instance of it. | LLVM can report the source function and its generic instantiations separately. JaCoCo counts Java methods at bytecode level. [LLVM metrics][clang] · [JaCoCo counters][jac-counters] |
| Branch or condition | An outcome was exercised. The definition of an outcome comes from the producer. | JaCoCo counts branches for `if` and `switch` and excludes exception handling from that metric. Coverage.py can observe possible and actual transitions between source lines. [JaCoCo counters][jac-counters] · [Coverage.py branches][py-branch] |
| Modified condition/decision coverage (MC/DC) | Tests demonstrate that each constituent condition can independently change the decision's outcome. | For `a && b`, seeing the overall expression both true and false leaves open whether each input independently affected the result. This needs dedicated instrumentation. [LLVM MC/DC][clang] |
| Patch coverage | The covered share of measurable new or changed lines for a specified diff. | A PR can raise overall coverage while leaving its own new code uncovered. Codecov exposes project and patch statuses separately. [Codecov checks][cc-status] |

## Format catalogue

The collection file, exchange report and rendered view serve different readers. For Rust, the instrumented executable contains the mapping back to source. Running it writes counters. Moving just the counter file to another machine loses the information needed to label those counts. [Rust compiler][rustc]

```text
Instrumented test run → .profraw files → llvm-profdata merge → .profdata
.profdata + matching instrumented binaries
    | llvm-cov export → LLVM JSON
    | llvm-cov export -format=lcov → LCOV → cargo-llvm-cov conversion → Cobertura XML
    | llvm-cov show + matching source text → annotated text or HTML
```

### Exchange reports

| Format and common name | What the file carries | Where it comes from and how to use it |
| --- | --- | --- |
| **LCOV tracefile** · `lcov.info`, `*.lcov` | Text records grouped by source file. `SF` identifies the path, `DA` a line and its execution count, and `BRDA` a branch outcome. A branch taken count of `-` means the expression was never evaluated. Function records and file totals are available. | GCC/LCOV, LLVM, Istanbul, coverage.py and Coverlet can all emit it. Good for source gutters and diffs. LCOV 2.2 introduced grouped function records, and recent LCOV understands MC/DC records. A reader built for older records can miss these details. [Tracefile specification][lcov-spec] · [Istanbul reporters][ist-reports] · [Coverage.py][pycov] · [Coverlet][coverlet] |
| **Cobertura XML** · `cobertura.xml`, `coverage.xml` | A `coverage` root with packages, classes, source filenames and `line` elements with `number` and `hits`. Conditional lines can include `condition-coverage` and details. | A common exchange format for Rust, Python, JS, PHP and .NET. Generated class/package names may stand for Rust source paths. A converter starting from line counts has little information to put in the conditional fields. [Cobertura DTD][cob-dtd] · [Rust exporter][cargo-cov] · [Python XML][py-xml] · [PHPUnit][phpunit] |
| **JaCoCo XML** · `jacoco.xml` | A `report` root. Within a source file, each `line` has an `nr` and missed/covered instruction (`mi`/`ci`) and branch (`mb`/`cb`) counts. Other counters roll up by method, class and package. | Common JVM exchange format. `ci="3"` means three distinct bytecode instructions were covered on that line. Treating it as three executions would give the wrong result. JaCoCo CLI can generate XML, CSV and HTML from execution data and class files. [JaCoCo DTD][jac-schema] · [Counter definitions][jac-counters] · [CLI][jac-cli] |
| **Clover / OpenClover XML** · `clover.xml` | File and project metrics, with typed lines and counts for constructs such as statements, methods and conditions. The set of elements varies by producer. | Found in PHP and JavaScript reporters as well as the Clover ecosystem. Check the destination's documented dialect when using PHPUnit output. [Istanbul Clover example][ist-reports] · [PHPUnit][phpunit] · [Coveralls importer][ca-reporter] |
| **OpenCover XML** · `opencover.xml` | A `CoverageSession` containing modules, source-file references, sequence points and branch points with visit counts. Sequence points can identify source spans. | A .NET exchange format. Current producers include Coverlet even though the original OpenCover project is archived. ReportGenerator and Jenkins are documented readers. [OpenCover][opencover] · [Coverlet][coverlet] · [ReportGenerator][reportgen] |
| **Sonar generic coverage XML** | `<coverage version="1">` contains files and `lineToCover` elements with a covered flag and optional total/covered branch counts. | An adapter target for SonarQube when a language-specific importer cannot consume the original. A Boolean line flag suffices for this schema. Generic test-execution XML uses a different root and answers which tests ran. [Sonar schema][sonar-generic] |

### Native, ecosystem and presentation formats

| Ecosystem or artifact | Collector and data | Normal route to a viewer |
| --- | --- | --- |
| **LLVM native profiles and coverage JSON** · Rust, Clang C/C++, Swift toolchains | `.profraw` stores runtime profile data. `llvm-profdata` merges it into indexed `.profdata`. Full `llvm-cov export` JSON identifies itself as `llvm.coverage.json.export` and can carry files, segments, functions, region ranges, expansions and branch records. Summary-only JSON drops source detail. | Use `llvm-cov show` with the profile and instrumented objects, or read the full JSON in a custom viewer. Source mapping and counters stay together in the export. [LLVM CLI][llvm-cov] · [JSON exporter][llvm-json] · [Rust compiler][rustc] |
| **GCC/gcov** · `.gcno`, `.gcda`, `.gcov`, `.gcov.json.gz` | GCC writes compile-time notes (`.gcno`) and runtime counts (`.gcda`). `gcov` resolves them to annotated source or versioned, compressed JSON with functions and lines, plus branches when requested. | `gcovr` can render detailed HTML or export Cobertura, LCOV and other formats. LCOV's `geninfo` and `genhtml` are another route. gcovr's own JSON has a separate schema from GCC's JSON. [GCC data files][gcc-files] · [gcov JSON][gcc-gcov] · [gcovr outputs][gcovr] |
| **Istanbul JSON** · `coverage-final.json` | Per-file `statementMap`, `fnMap` and `branchMap` locate code. The `s`, `f` and `b` counters refer to those maps. Summary JSON contains aggregate totals instead. | `nyc` and Istanbul reporters generate local HTML, LCOV, Cobertura and Clover. Jest supports Babel and V8 coverage providers. Vitest offers Istanbul and V8 providers. [Istanbul data model][ist-model] · [Reporters][ist-reports] · [Jest configuration][jest] · [Vitest][vitest] |
| **V8 native coverage** · Node coverage JSON | V8 records function/block ranges by offsets in a script and associates counts with ranges. Node can write this data using `NODE_V8_COVERAGE`. Mapping bundled or transpiled code to original TypeScript requires source maps. | `c8` uses V8 data and renders Istanbul-compatible reports. In an interactive browser session, DevTools can record coverage directly. [V8 internals][v8] · [Node CLI][node-v8] · [c8][c8] · [Chrome DevTools][chrome] |
| **Python coverage.py** · `.coverage`, `coverage.json` | Its SQLite database records executed lines or arcs and can preserve per-test or other contexts. The JSON report exposes executed and missing lines and, in branch mode, branch details. | `coverage html` generates annotated source. `pytest-cov` drives the same measurement from pytest and supports multiple report outputs, including Cobertura XML and LCOV. [Data API][py-data] · [JSON report][py-json] · [pytest-cov][pytest-cov] |
| **Go coverprofile and covdata** · `cover.out`, `covmeta.*`, `covcounters.*` | `go test -coverprofile=cover.out` writes source ranges, statement counts and per-block counts. `mode: set` records whether a block ran, `count` records executions and `atomic` counts safely across goroutines. Instrumented binaries can write binary covdata for integration runs. | `go tool cover -html=cover.out` opens annotated source. `go tool covdata textfmt` converts a binary coverage directory to the text profile. [Go coverage modes][go-cover] · [Profile structure][go-profile] · [Integration coverage][go-integration] |
| **JVM execution data** · `jacoco.exec` | The JaCoCo agent records which probes fired for each class ID. Turning this into line coverage requires the matching class files and line debug information. | Generate the JaCoCo HTML/XML report or inspect the session with EclEmma. A mismatch between runtime and reporting class files can make an executed class appear uncovered. [JaCoCo class IDs][jac-class] · [JaCoCo CLI][jac-cli] · [EclEmma][eclemma] |
| **.NET native and Coverlet data** · `.coverage`, Microsoft XML, Coverlet JSON | Microsoft `dotnet-coverage` collects binary `.coverage` and can convert or merge to Microsoft XML or Cobertura. Coverlet emits its own JSON, OpenCover, Cobertura and LCOV. | Open `.coverage` in Visual Studio. Use ReportGenerator for the exchange reports. Choosing an XML dialect explicitly avoids mistaking Microsoft's XML for Cobertura. [Microsoft CLI][mscov] · [Coverlet][coverlet] · [ReportGenerator][reportgen] |
| **PHP and Ruby** · PHPUnit reports and SimpleCov result sets | PHPUnit uses PCOV or Xdebug to measure PHP. Xdebug can collect branch and path coverage. Ruby's SimpleCov uses Ruby's coverage library and can collate `.resultset.json` files across test runs. | PHPUnit and SimpleCov produce local HTML. PHPUnit also writes Clover and Cobertura. Coveralls accepts SimpleCov through its reporter, whereas Codecov documents LCOV or Cobertura as its compatible route for SimpleCov. [PHPUnit][phpunit] · [SimpleCov][simplecov] · [Coveralls][ca-reporter] · [Codecov][cc-formats] |
| **Apple result bundles** · `.xcresult` | Xcode can store a test run and its coverage report in a result bundle. `xccov` also reads standalone coverage reports and archives. | Browse coverage in Xcode or pass the result bundle to `xcrun xccov view --report`. [Apple release notes][xccov] · [Testing in Xcode][xcode-testing] |
| **Service upload payloads, summaries and HTML** | Coveralls JSON describes source files and per-line coverage. Codecov has a custom JSON schema. Percentages, badges and CSV summaries can carry totals without source positions. HTML contains the presentation chosen by one renderer. | Send a service payload to its documented API, share the complete HTML directory for offline reading, and keep a structured report when another tool needs to filter, merge or annotate a diff. [Coveralls API][ca-api] · [Codecov custom schema][cc-custom] · [JaCoCo report options][jac-cli] |

A filename such as `coverage.xml` or `coverage.json` leaves the schema ambiguous. Inspect the XML root and its children, or the JSON type and record structure. For example, Cobertura and Sonar generic XML both start with `coverage`, but one contains a package/class hierarchy and the other contains `file/lineToCover`. [Cobertura DTD][cob-dtd] · [Sonar schema][sonar-generic]

Existing projects may also publish JetBrains dotCover detailed XML, NCover reports, or Scala Scoverage output. ReportGenerator lists readers for these formats, including version restrictions on NCover. Check the originating tool before selecting a converter for an older report. [ReportGenerator input formats][reportgen]

### Measurement commands by ecosystem

These examples assume the named provider or plugin is installed in the project and the normal tests pass. The [Rust walkthrough](#one-rust-test-run-four-views) shows a complete single-run workflow.

| Project | How to collect and view the result |
| --- | --- |
| GCC C/C++ | Compile and link with `--coverage`, run the project's tests, then run `gcovr --html-details coverage.html` from the source root. The source and build paths must match the captured gcov data. [Compiler setup][gcovr-compile] · [HTML output][gcovr-html] |
| JVM with Gradle | With the Java and JaCoCo plugins applied, run `./gradlew test jacocoTestReport`. Gradle generates HTML by default. Enable the XML report for a CI consumer. [Gradle plugin][gradle] |
| JavaScript or TypeScript | Run `vitest run --coverage` or `jest --coverage` in the configured project. Select an HTML reporter for local source browsing and LCOV or Cobertura for CI. [Vitest][vitest] · [Jest][jest] · [Istanbul reporters][ist-reports] |
| Python with pytest | `pytest --cov=mypackage --cov-branch --cov-report=html --cov-report=xml` collects once and writes annotated HTML and Cobertura XML. Substitute the importable package name. [pytest-cov reporting][pytest-cov] · [Options][pytest-options] |
| Go | `go test -coverprofile=cover.out ./...` collects package coverage. `go tool cover -html=cover.out` opens the source view. [Go tooling][go-cover] |
| .NET with VSTest | With `coverlet.collector` installed, `dotnet test --collect:"XPlat Code Coverage"` attaches a Cobertura XML report to the test results. Feed the XML to ReportGenerator for a local HTML view. [VSTest integration][vstest] · [ReportGenerator][reportgen] |
| PHP with PHPUnit | With source filtering and a coverage driver configured, `phpunit --coverage-html coverage --coverage-clover clover.xml` emits a local source browser and a Clover report in one run. [PHPUnit CLI][php-cli] · [Coverage drivers][phpunit] |

## Viewer compatibility

Each “Yes” means the viewer or its first-party uploader accepts the named format directly. A dash means the published inputs exclude it or the inspected parser has no handler for it. Converting one XML dialect to another counts as a separate step. The sample includes two Git hosts, two coverage services, one CI plugin, one local report generator, and one editor extension.

| Viewer and where it runs | LCOV | Cobertura XML | JaCoCo XML | Clover XML | What the user sees |
| --- | :---: | :---: | :---: | :---: | --- |
| GitHub Code Quality, GitHub Actions | — | Yes | — | — | A PR comment with branch and default-branch line coverage and per-file changes. Requires GitHub Team or Enterprise Cloud, Code Quality enabled, and an upload via `actions/upload-code-coverage`. [Setup][gh-setup] · [PR view][gh-pr] |
| GitLab, merge request diff | — | Yes | Yes | — | Line annotations in files changed by the MR. The headline percentage and its history come from a separately configured job-log expression. [Diff annotations][gl-diff] · [Percentage reporting][gl-coverage] |
| Codecov, hosted | Yes | Yes | Yes | Yes | Source coverage, pull request checks for overall project and patch coverage, and comparisons split by flag. [Accepted reports][cc-formats] · [Status checks][cc-status] |
| Coveralls, hosted | Yes | Yes | Yes | Yes | Per-file source views, builds, branch history and PR comparisons. These inputs go through the Universal Coverage Reporter, which documents the PHPUnit Clover dialect. [Reporter inputs][ca-reporter] · [Web UI][ca-ui] |
| Jenkins Coverage plugin, self-hosted CI | Yes | Yes | Yes | Yes | Annotated source, trends, modified-line coverage and quality gates relative to a reference build. Its Clover parser is listed as Open Clover. [Jenkins plugin][jenkins] |
| ReportGenerator, local or CI | Yes | Yes | Yes | Yes | A browsable HTML tree with source highlighting. It can merge reports and retain history between builds. Its documented JaCoCo input starts at 0.8.3. [Formats and usage][reportgen] |
| VS Code Coverage Gutters, local editor | Yes | Yes | Yes | Yes | Coverage beside source lines while editing. The extension watches report files and has a separate parser for each of these four formats. [Extension][gutters] · [Parser][gutters-parser] |

**Cobertura reached 7 of 7 readers, JaCoCo 6 of 7, and LCOV and Clover each reached 5 of 7.** This measures compatibility within the table. Producer support changes the practical ranking. Rust can emit LCOV and Cobertura directly through Cargo tooling, Java builds commonly have a JaCoCo integration in Gradle or Maven, and PHPUnit offers Clover alongside Cobertura. A Clover reader in an editor does little for a Rust developer whose measurement tool emits LCOV. [Rust exports][cargo-cov] · [Gradle JaCoCo][gradle] · [JaCoCo Maven plugin][jac-maven] · [PHPUnit][phpunit]

### Other ways to see coverage

| Viewer | Input | What distinguishes the view |
| --- | --- | --- |
| LLVM `llvm-cov show`, terminal or offline HTML | Merged `.profdata`, matching instrumented objects and the corresponding sources | Shows individual source regions, counts on a line, and branch outcomes when instrumented. `llvm-cov report` gives tabular totals. [LLVM command guide][llvm-cov] |
| LCOV `genhtml`, offline HTML | LCOV tracefile and matching sources | Navigable directory, file and source views. Branch display is configurable. A baseline tracefile and source diff enable differential coverage. [genhtml manual][genhtml] |
| `diff-cover`, local Git checkout or CI | A Git diff and an LCOV, Cobertura, JaCoCo or Clover report | Lists changed lines missing coverage. For example, `diff-cover lcov.info --compare-branch=origin/main --format html:diff.html` creates an offline review against the chosen base. [diff-cover usage][diff-cover] |
| RustRover, desktop IDE | A run configuration launched with coverage, or a compatible saved coverage suite | Editor gutter and Coverage tool window beside project files. Can display multiple suites together and generate HTML. [RustRover help][rustrover] |
| VS Code Test Coverage, editor Test Explorer | Coverage supplied by a test extension through VS Code's testing API | The built-in tree and editor highlights follow a coverage-enabled run. Coverage Gutters above is the route when the starting point is an existing file. [VS Code testing][vscode-test] |
| SonarQube Server, analysis dashboard | For Rust, `sonar.rust.lcov.reportPaths` or `sonar.rust.cobertura.reportPaths`. A generic XML importer is also available. | Coverage on analyzed code alongside new-code metrics. Sonar documents sequential processing with undefined conflict resolution when both Rust properties cover the same code. [Rust inputs][sonar-inputs] · [Metric definitions][sonar-metrics] |
| Azure Pipelines, build UI | `PublishCodeCoverageResults@2` accepts inputs including `.coverage`, XML and LCOV | The build has a Code Coverage tab and an HTML artifact. The task also generates its own CJSON coverage data. [Azure task reference][azure] |
| Eclipse EclEmma, Microsoft Visual Studio, Apple Xcode | Respectively JaCoCo execution data, Microsoft `.coverage`, and Xcode results with coverage | Native editor or IDE views follow the language's build artifacts. For an Xcode result bundle, `xccov view --report` also exposes the report at the command line. [EclEmma][eclemma] · [Microsoft coverage tool][mscov] · [Apple release notes][xccov] |
| Chrome DevTools Coverage, browser session | A recording of a running page's JavaScript and CSS | Highlights used and unused bytes in Sources. Each resource's total size supplies the denominator. The recorded interaction determines which bytes count as used. [Chrome DevTools][chrome] |

## What the popularity evidence shows

Registry downloads, marketplace installs and format support measure different things. A compiler feature such as Go's `-cover` ships with the toolchain, so a ranking by separately downloaded packages would miss it entirely. No source in this survey measures worldwide market share across those distribution channels. [Go coverage][go-cover]

### Rust measurement and processing tools

| Crate | Downloads in the previous 90 days | Lifetime downloads | What was downloaded |
| --- | ---: | ---: | --- |
| `cargo-llvm-cov` | 2,037,572 | 9,439,854 | Cargo frontend for LLVM source coverage. [Registry record][cr-llvm] |
| `cargo-tarpaulin` | 305,460 | 3,245,763 | Cargo test runner and coverage collector. [Registry record][cr-tarp] |
| `grcov` | 198,009 | 2,726,025 | A coverage data processor that also handles inputs from other languages. [Registry record][cr-grcov] |

**`cargo-llvm-cov` led this sample by about 6.7 times the recent downloads of Tarpaulin.** The counts were read from the crates.io API on 23 November 2026 using `recent_downloads` and `downloads`. Repeat CI downloads and library dependencies can contribute. Prebuilt binaries distributed outside crates.io do not enter the count. The result supports choosing a well-used Cargo frontend without implying that the same ratio holds among Rust projects. [Registry definition of the 90-day field][cr-schema] · [Binary installation options][cargo-cov]

### Measurement tools and viewers outside Rust

| Comparison | Publisher's measure | What it tells us |
| --- | --- | --- |
| JavaScript coverage packages, 9–15 November 2026 inclusive | npm recorded **62.99M** downloads of `@vitest/coverage-v8`, **9.32M** of `nyc`, **5.22M** of `c8` and **4.63M** of `@vitest/coverage-istanbul`. | The V8 provider led these selected packages that week. A runner, a provider and a CLI can occur in the same dependency tree. [Vitest V8][npm-vitest-v8] · [nyc][npm-nyc] · [c8][npm-c8] · [Vitest Istanbul][npm-vitest-ist] |
| Hosted PR viewers, 23 November 2026 | GitHub Marketplace showed **123,044** Codecov app installs and **5,120** Coveralls app installs. | Codecov had more installs on this particular distribution channel. The counter measures app installations. One installation can cover several repositories, and other Git hosts fall outside the count. [Codecov listing][gh-codecov] · [Coveralls listing][gh-coveralls] |
| Local viewers, 23 November 2026 | VS Code Marketplace showed **960,387 installs** of Coverage Gutters. NuGet showed **131.3M total downloads** of `dotnet-reportgenerator-globaltool`. | Both tools have substantial distribution. The editor's install counter and the package manager's cumulative downloads have different units. [VS Marketplace][gutters] · [NuGet][nuget-rg] |

## Measuring Rust code

| Tool | How it gets coverage | When to choose it |
| --- | --- | --- |
| **cargo-llvm-cov** | Drives `rustc -C instrument-coverage`, test execution, `llvm-profdata` and `llvm-cov`. Exports full LLVM JSON, LCOV, Cobertura, annotated text and HTML. The Cobertura exporter converts LLVM's LCOV output. | Default choice for a Cargo workspace. Works with `cargo test` and `cargo llvm-cov nextest`. Its `show-env` workflow also handles separately launched programs. Rust linked with C/C++ needs compatible Clang/LLVM instrumentation on the foreign code for that code to be included. [Cargo frontend][cargo-cov] · [Rust compiler][rustc] |
| **rustc + LLVM tools** | Compile with `-C instrument-coverage`, run the instrumented executable, merge the profiles and give `llvm-cov` the resulting profile and objects. | Useful for custom harnesses, target-device execution, or investigating precisely which object contributed counts. The target needs a profiler runtime. Prefer the LLVM tools supplied for the compiler that built the binaries. [Rust compiler guide][rustc] · [llvm-profdata][profdata] |
| **cargo-tarpaulin** | Runs Rust tests and collects via Linux `ptrace` or LLVM. In 0.37.6, the release's build rules select `ptrace` by default on Linux x86/x86-64 and LLVM by default elsewhere. It can generate HTML, LCOV, Cobertura XML and its own JSON. | Useful for projects with Tarpaulin configuration or a reason to use the tracing backend. The release documents line coverage and marks its branch switch unimplemented. [Tarpaulin][tarpaulin] · [Backend selection][tarp-build] · [Output writers][tarp-out] |
| **grcov** | Processes coverage gathered elsewhere. It accepts LLVM raw profiles with the matching instrumented binaries, gcov data, LCOV, Go profiles and JaCoCo reports. Outputs include LCOV, Cobertura, HTML, Coveralls and recursive JSON (`covdir`). | Useful when CI collects from several languages or a test device produces artifacts for processing on another machine. A `--branch` reporting option can only report outcomes that the input represents. [grcov inputs and outputs][grcov] |
| **kcov** | Uses debug information for compiled programs to observe execution without a coverage-instrumented rebuild. The project documents Linux, FreeBSD and macOS. It writes HTML, Cobertura-compatible XML and JSON. | A specialized option for observing a debuggable Rust executable. Source-line precision depends on the binary's debug mapping. [kcov][kcov] |

Stable Rust's source coverage covers lines, functions and regions. **Branch coverage is opt-in and requires nightly** in `cargo-llvm-cov 0.9.2`. The same release documents doctest collection as opt-in and nightly-only. A traditional GCC/gcov recipe using Rust's old `-Zprofile` flag is obsolete for current Rust. A C dependency compiled with GCC can still produce its own gcov data. [Cargo coverage options][cargo-cov] · [Branch tracking issue][rust-branch] · [Rust release notes][rust-releases] · [GCC data files][gcc-files]

MC/DC needs a separate check. The Cargo frontend still lists `--mcdc`, but Rust removed its earlier implementation. On the tested `rustc 1.102.0-nightly` build from 19 November 2026, the compiler accepted `-Zcoverage-options=branch` and rejected `-Zcoverage-options=mcdc`. The Rust project lists reimplementation as a goal. An LLVM report containing an MC/DC summary field does not establish that a Rust build collected it. [Compiler change][rust-mcdc-remove] · [Rust project goal][rust-mcdc-goal]

## One Rust test run, four views

The following commands reproduce the reporting workflow in a fresh Cargo workspace. The check used Linux x86-64, stable `rustc 1.100.0`, its LLVM 23.1.5 tools and `cargo-llvm-cov 0.9.2`. An older locally installed `0.6.23` ran the fixture's test but failed to locate the object files with this Cargo version. The pinned release completed all four exports. [Release used][cargo-release] · [Upstream multi-report workflow][cargo-merge]

```sh
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov --version 0.9.2

cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --no-report
mkdir -p target/coverage-report
cargo llvm-cov report --html --output-dir target/coverage-report
cargo llvm-cov report --lcov --output-path target/coverage-report/lcov.info
cargo llvm-cov report --cobertura --output-path target/coverage-report/cobertura.xml
cargo llvm-cov report --json --output-path target/coverage-report/llvm.json
```

The clean step starts a fresh coverage collection. Each `report` command reads the same results. Open `target/coverage-report/html/index.html` locally, point Coverage Gutters at `lcov.info`, or give the XML to a CI coverage importer. For a single local inspection, `cargo llvm-cov --open` runs the tests and opens the result. [Cargo usage][cargo-cov]

To recreate the fixture, create a library with `cargo new --lib coverage-example`. Use the following `src/lib.rs` and `tests/zero.rs`, then run the commands above from that crate's root.

```rust
pub fn classify(n: i32) -> &'static str {
    if n == 0 { "zero" } else { "nonzero" }
}

pub fn uncalled() -> bool {
    true
}
```

```rust
#[test]
fn zero() {
    assert_eq!(coverage_example::classify(0), "zero");
}
```

| Output inspected | What it recorded for `src/lib.rs` |
| --- | --- |
| LCOV | `DA:2,1` marked the `if` line hit. `DA:6,0` marked the uncalled function body missed. `LH:3` and `LF:6` gave **3 of 6 lines**. `BRF:0` meant the default run supplied no branch outcomes. |
| Cobertura | Line 2 had `hits="1"` and `branch="false"`. The file's `line-rate="0.5"` matched LCOV. A line-only reader had no evidence that `"nonzero"` was missed. |
| Full LLVM JSON | The counted segment at line 2, column 33, covering `"nonzero"`, had zero executions. The file had **4 of 8 regions** covered and **1 of 2 functions** covered. This is the detail lost in the LCOV line record. |
| LLVM HTML | The source page highlighted the `"nonzero"` literal in red inside the executed `if` line. The `"zero"` and `"nonzero"` expressions could be distinguished visually on the same line. |

A separate run with `cargo +nightly llvm-cov --branch` exercised the same test. The LCOV export contained `BRDA:2,0,0,1` and `BRDA:2,0,1,0`, followed by `BRF:2` and `BRH:1`. This run measured **one of two branch outcomes** on the line. [Rust branch option][rust-branch] · [LCOV record meanings][lcov-spec]

## Reading reports reliably in a Git viewer

- **Bind the report to a revision.** Keep the commit and test configuration beside the report. Resolve filenames against that checkout, including CI path prefixes, Windows separators and renamed files. GitLab documents how mismatched or ambiguous source paths cause diff annotations to disappear. LLVM offers path remapping for reports created on a different machine. [GitLab path matching][gl-diff] · [LLVM path options][llvm-cov]
- **Keep missing, missed and partial distinct.** A measurable line with a zero count is missed. A source line absent from a report can be non-executable, excluded or outside the build. Preserve unknown coverage until the producer provides a denominator. Show a partially covered line when region or branch evidence supports it, and retain the original metric. [LCOV specification][lcov-spec] · [JaCoCo counters][jac-counters] · [LLVM metrics][clang]
- **Merge observations before computing percentages.** Suppose two shards exercise disjoint halves of the same ten measurable lines. Each reports 50%, while the combined run covers all ten. Match the source revision, logical locations and build configuration before merging. Keep platform or feature flags where the builds cover different code. Raw profile merges also need compatible artifacts. [LLVM profile merge][profdata] · [Cargo merge workflow][cargo-merge] · [Codecov flags][cc-flags]
- **Label the comparison.** Compute patch coverage on measurable lines in the diff against a named base. A whole-file coverage delta can change because tests exercised old code elsewhere in that file. A patch containing only comments can have zero measurable lines. Showing “no measurable lines” preserves that distinction. [Codecov statuses][cc-status] · [GitHub per-file deltas][gh-ref] · [genhtml differential coverage][genhtml]
- **Choose a parser by schema and producer version.** For a Rust application, the `lcov` crate provides a record reader and merger, `quick-xml` can stream Cobertura or JaCoCo elements, and `serde_json` can deserialize the versioned LLVM export. Test the intended dialect against real files, especially legacy and grouped LCOV function records. A general XML or JSON parser handles syntax. The application still has to preserve what each counter means. [Rust LCOV reader][rs-lcov] · [quick-xml][quick-xml] · [serde_json][serde-json] · [LLVM exporter][llvm-json]

For a Rust application that needs line-level patch coverage, `patchcov` already provides a library and CLI. It reads LCOV, LLVM JSON, Cobertura, JaCoCo and Go coverprofiles, normalizes their line data, and aligns it to a Git diff. An expression-level viewer would also need the original source ranges. [patchcov library and data model][patchcov]

## What applies to GitComet

GitComet measures its own source by comparing the lines LLVM maps in its GUI and headless builds with the lines its native tests exercise. An execution in either configuration covers the line. The coverage command produces a per-crate summary, line-only LCOV, and an offline HTML source view. The contributor guide defines the measurement scope and output paths. [Contributor instructions](../CONTRIBUTING.md#measuring-test-coverage) · [Coverage workflow](../.github/workflows/source-coverage.yml)

For a GitComet diff overlay, **LCOV is the first format I would support**. The existing helper already produces it, and its per-line counts map directly to a gutter. **Cobertura is the next choice** for reports already produced by mixed-language CI. **Full LLVM JSON** earns its place when the UI can show partially covered expressions or distinguish generic instantiations. [LCOV specification][lcov-spec] · [Cobertura DTD][cob-dtd] · [LLVM export][llvm-cov]

[azure]: https://learn.microsoft.com/en-us/azure/devops/pipelines/tasks/reference/publish-code-coverage-results-v2?view=azure-pipelines
[c8]: https://github.com/bcoe/c8
[ca-api]: https://docs.coveralls.io/api-jobs-endpoint
[ca-reporter]: https://github.com/coverallsapp/coverage-reporter#supported-coverage-report-formats
[ca-ui]: https://docs.coveralls.io/app-ui
[cargo-cov]: https://github.com/taiki-e/cargo-llvm-cov/tree/v0.9.2
[cargo-merge]: https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.2/README.md#merge-coverages-generated-under-different-test-conditions
[cargo-release]: https://github.com/taiki-e/cargo-llvm-cov/releases/tag/v0.9.2
[cc-custom]: https://docs.codecov.com/docs/codecov-custom-coverage-format
[cc-flags]: https://docs.codecov.com/docs/flags
[cc-formats]: https://docs.codecov.com/docs/supported-report-formats
[cc-status]: https://docs.codecov.com/docs/commit-status
[chrome]: https://developer.chrome.com/docs/devtools/coverage/
[clang]: https://clang.llvm.org/docs/SourceBasedCodeCoverage.html#interpreting-reports
[cob-dtd]: https://github.com/cobertura/cobertura/blob/master/cobertura/src/site/htdocs/xml/coverage-04.dtd
[coverlet]: https://github.com/coverlet-coverage/coverlet
[cr-grcov]: https://crates.io/api/v1/crates/grcov
[cr-llvm]: https://crates.io/api/v1/crates/cargo-llvm-cov
[cr-schema]: https://github.com/rust-lang/crates.io/blob/main/crates/crates_io_api_types/src/lib.rs
[cr-tarp]: https://crates.io/api/v1/crates/cargo-tarpaulin
[diff-cover]: https://github.com/Bachmann1234/diff_cover
[eclemma]: https://www.eclemma.org/
[gcc-files]: https://gcc.gnu.org/onlinedocs/gcc/Gcov-Data-Files.html
[gcc-gcov]: https://gcc.gnu.org/onlinedocs/gcc/Invoking-Gcov.html
[gcovr]: https://gcovr.com/en/stable/output/index.html
[gcovr-compile]: https://gcovr.com/en/stable/guide/compiling.html
[gcovr-html]: https://gcovr.com/en/stable/output/html.html
[genhtml]: https://github.com/linux-test-project/lcov/blob/master/docs/man/genhtml.rst
[gh-codecov]: https://github.com/marketplace/codecov
[gh-coveralls]: https://github.com/marketplace/coveralls
[gh-pr]: https://docs.github.com/en/code-security/how-tos/maintain-quality-code/view-coverage-on-prs
[gh-ref]: https://docs.github.com/en/code-security/reference/code-quality/code-coverage
[gh-setup]: https://docs.github.com/en/code-security/how-tos/maintain-quality-code/set-up-code-coverage
[gl-coverage]: https://docs.gitlab.com/ci/testing/code_coverage/
[gl-diff]: https://docs.gitlab.com/ci/testing/code_coverage/coverage_visualization/
[go-cover]: https://go.dev/blog/cover
[go-integration]: https://go.dev/doc/build-cover
[go-profile]: https://pkg.go.dev/golang.org/x/tools/cover#ProfileBlock
[gradle]: https://docs.gradle.org/current/userguide/jacoco_plugin.html
[grcov]: https://github.com/mozilla/grcov/tree/v0.10.8
[gutters]: https://marketplace.visualstudio.com/items?itemName=ryanluker.vscode-coverage-gutters
[gutters-parser]: https://github.com/ryanluker/vscode-coverage-gutters/blob/master/src/files/coverageparser.ts
[ist-model]: https://github.com/istanbuljs/istanbuljs/blob/main/packages/istanbul-lib-coverage/lib/file-coverage.js
[ist-reports]: https://istanbul.js.org/docs/advanced/alternative-reporters/
[jac-class]: https://www.jacoco.org/jacoco/trunk/doc/classids.html
[jac-cli]: https://www.jacoco.org/jacoco/trunk/doc/cli.html
[jac-counters]: https://www.jacoco.org/jacoco/trunk/doc/counters.html
[jac-maven]: https://www.jacoco.org/jacoco/trunk/doc/maven.html
[jac-schema]: https://github.com/jacoco/jacoco/blob/master/org.jacoco.report/src/org/jacoco/report/xml/report.dtd
[jenkins]: https://plugins.jenkins.io/coverage/
[jest]: https://jestjs.io/docs/configuration#coverageprovider-string
[kcov]: https://github.com/SimonKagstrom/kcov
[lcov]: https://github.com/linux-test-project/lcov
[lcov-spec]: https://github.com/linux-test-project/lcov/blob/master/docs/man/geninfo.rst#tracefile-format
[llvm-cov]: https://llvm.org/docs/CommandGuide/llvm-cov.html
[llvm-json]: https://github.com/llvm/llvm-project/blob/main/llvm/tools/llvm-cov/CoverageExporterJson.cpp
[mscov]: https://learn.microsoft.com/en-us/dotnet/core/additional-tools/dotnet-coverage
[node-v8]: https://nodejs.org/api/cli.html#node_v8_coveragedir
[npm-c8]: https://api.npmjs.org/downloads/point/2026-11-09:2026-11-15/c8
[npm-nyc]: https://api.npmjs.org/downloads/point/2026-11-09:2026-11-15/nyc
[npm-vitest-ist]: https://api.npmjs.org/downloads/point/2026-11-09:2026-11-15/%40vitest%2Fcoverage-istanbul
[npm-vitest-v8]: https://api.npmjs.org/downloads/point/2026-11-09:2026-11-15/%40vitest%2Fcoverage-v8
[nuget-rg]: https://www.nuget.org/packages/dotnet-reportgenerator-globaltool/
[opencover]: https://github.com/OpenCover/opencover
[patchcov]: https://docs.rs/patchcov/0.4.1/patchcov/
[php-cli]: https://docs.phpunit.de/en/12.5/cli-options.html
[phpunit]: https://docs.phpunit.de/en/12.5/code-coverage.html
[profdata]: https://llvm.org/docs/CommandGuide/llvm-profdata.html#profdata-merge
[py-branch]: https://coverage.readthedocs.io/en/latest/branch.html
[py-data]: https://coverage.readthedocs.io/en/latest/api_coveragedata.html
[py-json]: https://coverage.readthedocs.io/en/latest/commands/cmd_json.html
[py-xml]: https://coverage.readthedocs.io/en/latest/commands/cmd_xml.html
[pycov]: https://coverage.readthedocs.io/en/latest/
[pytest-cov]: https://pytest-cov.readthedocs.io/en/latest/reporting.html
[pytest-options]: https://pytest-cov.readthedocs.io/en/latest/config.html
[quick-xml]: https://docs.rs/quick-xml/latest/quick_xml/
[reportgen]: https://github.com/danielpalme/ReportGenerator#supported-input-and-output-file-formats
[rs-lcov]: https://docs.rs/lcov/0.8.2/lcov/
[rust-branch]: https://github.com/taiki-e/cargo-llvm-cov/issues/8
[rust-mcdc-goal]: https://goals.rust-lang.org/2026/mcdc-coverage-support.html
[rust-mcdc-remove]: https://github.com/rust-lang/rust/pull/144999
[rust-releases]: https://doc.rust-lang.org/releases.html#version-1840-2025-01-09
[rustc]: https://doc.rust-lang.org/rustc/instrument-coverage.html
[rustrover]: https://www.jetbrains.com/help/rust/code-coverage.html
[serde-json]: https://docs.rs/serde_json/latest/serde_json/
[simplecov]: https://github.com/simplecov-ruby/simplecov
[sonar-generic]: https://docs.sonarsource.com/sonarqube-server/analyzing-source-code/test-coverage/generic-test-data
[sonar-inputs]: https://docs.sonarsource.com/sonarqube-server/analyzing-source-code/test-coverage/test-coverage-parameters
[sonar-metrics]: https://docs.sonarsource.com/sonarqube-server/user-guide/code-metrics/metrics-definition
[tarp-build]: https://github.com/xd009642/tarpaulin/blob/0.37.6/build.rs
[tarp-out]: https://github.com/xd009642/tarpaulin/blob/0.37.6/src/report/mod.rs
[tarpaulin]: https://github.com/xd009642/tarpaulin/tree/0.37.6
[v8]: https://v8.dev/blog/javascript-code-coverage
[vitest]: https://vitest.dev/guide/coverage.html
[vscode-test]: https://code.visualstudio.com/docs/debugtest/testing
[vstest]: https://github.com/coverlet-coverage/coverlet/blob/master/Documentation/VSTestIntegration.md
[xccov]: https://developer.apple.com/documentation/xcode-release-notes/xcode-11-release-notes/
[xcode-testing]: https://developer.apple.com/videos/play/wwdc2019/413/
