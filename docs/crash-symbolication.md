# Crash Symbolication

GitComet ships stripped binaries, so a crash report is a list of hex addresses
until it is resolved against debug symbols. Release CI publishes those symbols to
a **Breakpad symbol store** — static files served over HTTP, keyed by each
build's debug ID — so a dump can be symbolized long after the build that
produced it.

- [`scripts/install-dump-syms.sh`](../scripts/install-dump-syms.sh) installs a pinned
  [`mozilla/dump_syms`](https://github.com/mozilla/dump_syms) release.
- [`scripts/emit-breakpad-symbols.sh`](../scripts/emit-breakpad-symbols.sh) converts a
  binary, `.pdb` or `.dSYM` into a `.sym` file, writes it into the store layout, and
  fails the build if the result is empty.
- [`scripts/symbolicate-minidump.sh`](../scripts/symbolicate-minidump.sh) resolves a
  minidump against the published store.
- [`.github/workflows/build-release-artifacts.yml`](../.github/workflows/build-release-artifacts.yml)
  runs all of it per release.

## The Two Kinds of Crash Report

**GitComet's own crash log.** Written by the panic hook in
[`crates/gitcomet/src/crashlog.rs`](../crates/gitcomet/src/crashlog.rs) whenever a
Rust panic occurs:

| OS | Location |
| --- | --- |
| Windows | `%LOCALAPPDATA%\gitcomet\crashes\panic-<pid>-<ms>.log` |
| macOS | `~/Library/Logs/gitcomet/crashes/panic-<pid>-<ms>.log` |
| Linux | `${XDG_STATE_HOME:-~/.local/state}/gitcomet/crashes/panic-<pid>-<ms>.log` |

The `location=<file>#L<line>` line in that file is **always usable without any
symbols** — it is compile-time information baked into the binary. Ask for this
file first; it often identifies the panic outright.

The backtrace below it depends on the platform. Linux and macOS ship with
`strip = "debuginfo"`, keeping the symbol table, so frames carry function names
(no line numbers). **Windows frames stay `<unknown>`**: MSVC keeps symbols in the
`.pdb`, which is not shipped, and `Backtrace::force_capture` resolves through
dbghelp against that file. On Windows, rely on the `location=` line and on
symbolicating the WER minidump.

**Operating-system dumps.** Windows Error Reporting minidumps, and any minidump a
future in-process crash handler writes. These need the symbol store.

Note that some crashes never reach the panic hook — a double panic aborts before
the hook runs, and a `__fastfail` bypasses every user-mode handler. If a process
died but wrote no crash log, that absence is itself a clue.

## Symbolicating a Minidump

```sh
cargo install minidump-stackwalk --locked
scripts/symbolicate-minidump.sh path/to/crash.dmp
```

The store URL defaults to `https://apt.gitcomet.dev/symbols/`; set
`GITCOMET_SYMBOLS_URL` only to point at a staging store or a local mirror.

Symbols are fetched lazily over HTTP and cached under
`${XDG_CACHE_HOME:-~/.cache}/gitcomet-symbols`, so only the modules a crash
actually touches are downloaded, and only once per build.

Reading the output: a `Crash reason` of `EXCEPTION_STACK_BUFFER_OVERRUN` with
`FAST_FAIL_FATAL_APP_EXIT` and an `int 0x29` instruction is **not** a buffer
overrun — it is a deliberate Rust `abort`, reached by `std::process::abort`, a
double panic, a panic crossing an `extern "C"` boundary, or an allocation
failure. Frames marked *"Found by: stack scanning"* are guesses.

## What CI Produces

Each release build emits `<name>/<debug-id>/<name>.sym` and uploads it as a
`symbols-<target_platform>` artifact. The `publish_symbols` job merges all
targets and uploads them to Azure Blob Storage.

The debug ID survives stripping on every platform — ELF `.note.gnu.build-id`,
the PE debug directory's PDB GUID+Age, and Mach-O `LC_UUID` — which is why
shipped binaries stay stripped and remain symbolizable.

Symbols are built with `--inlines`. GitComet uses fat LTO, so without inline
records most frames collapse into whichever outer function survived inlining.
The cost is real: on x86_64 Linux the `.sym` is ~239 MiB (~43 MiB gzipped)
against ~37 MiB without inline records. Storage is a few cents a month; drop the
flag in `emit-breakpad-symbols.sh` if that ever changes.

### Generating symbols locally

`[profile.release]` carries no debug info: the release jobs enable it on the
build step so `cargo bench`, which inherits that profile, is not taxed with
fat-LTO debug info it discards. Reproducing a build's symbols by hand means
opting in the same way — and the input differs per platform, because each one
keeps its debug info somewhere else.

**Linux** — DWARF is in the binary, so it must not be stripped before extraction:

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=none \
  cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin gitcomet
scripts/emit-breakpad-symbols.sh --input target/release/gitcomet --store /tmp/symstore
```

**macOS** — `dsymutil` must run, and the input is the bundle, not the binary:

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed \
  cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin gitcomet
scripts/emit-breakpad-symbols.sh --input target/release/gitcomet.dSYM \
  --store /tmp/symstore --arch "$(uname -m)" --allow-missing-cfi
```

**Windows** — the input is the `.pdb`; `strip` is irrelevant on MSVC:

```pwsh
$env:CARGO_PROFILE_RELEASE_DEBUG = "line-tables-only"
cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin gitcomet
bash scripts/emit-breakpad-symbols.sh --input target/release/gitcomet.pdb --store /tmp/symstore
```

Pointing the script at a stripped binary, or at a macOS binary instead of its
`.dSYM`, is rejected by the guards rather than yielding symbols that resolve
nothing — the message names stripping as the likely cause, so check the input
first.

### Unwind data on macOS

`emit-breakpad-symbols.sh` requires STACK (CFI) records, because without them a
stackwalker falls back to scanning — the "Found by: stack scanning" frames that
make a report unreadable. macOS is the exception and runs with
`--allow-missing-cfi`: `dsymutil` puts DWARF in the `.dSYM` but leaves
`__eh_frame` in the linked binary, and `dump_syms` reads a single file with no
macOS equivalent of the `.pdb`/`.exe` re-pairing it performs on Windows.

The macOS job therefore emits a `::warning` when STACK records are absent. If
that warning appears on the first release, macOS stacks are unwinding by
scanning; recovering CFI would mean also dumping the linked binary and merging
its STACK records into the same debug ID.

For debuggers that cannot read Breakpad `.sym`, CI also keeps the Windows `.pdb`
(`windows-debug-symbols-*`) and the macOS `.dSYM`, tarred so the bundle wrapper
survives the artifact round-trip (`macos-dsym-*`).

Note what these do **not** contain. Everything here is built at
`debug = "line-tables-only"`, which carries function names, file, line and
inline frames but no variables, parameters or types — measured on a probe build,
zero `DW_TAG_variable` / `DW_TAG_formal_parameter` / `DW_TAG_structure_type`
against 37 / 62 / 34 at `debug = 2`. So WinDbg and lldb will resolve a stack but
cannot inspect state. Raising the level would not help the store either: a
Breakpad `.sym` generated from a full-debug build is the same size with the same
record counts, because the format has nowhere to put types. And release builds
are `opt-level = 3` with fat LTO, where even full debug info yields
`<optimized out>` for most parameters and "No locals." The Linux raw DWARF is not
archived — it is ~203 MiB per build and the `.sym` covers `minidump-stackwalk`.

## Where the Store Lives

No dedicated infrastructure: symbols reuse the storage account that already
serves the APT repository and the Windows installer, under a `symbols/` prefix
in the `$web` static-website container.

| Setting | Value |
| --- | --- |
| Storage account | `APT_STORAGE_ACCOUNT` (currently `aegitcometaptsa`) |
| Container | `APT_STORAGE_CONTAINER` (currently `$web`) |
| Blob prefix | `symbols` |
| Public URL | `https://apt.gitcomet.dev/symbols/` |
| Credential | `AZURE_STORAGE_ACCOUNT_KEY` secret, via `--account-key` |

Because `APT_STORAGE_PREFIX` is empty, the APT repository lives at the root of
that same container, and `deploy-apt-repo.yml` mirrors every blob it finds
through the runner on each release. It therefore skips the `symbols/` prefix
explicitly — without that, each release would download and re-upload the whole
symbol store one `az` call at a time, and the stale-blob sweep at the end of
that job would be free to delete it. Anything else added to this container needs
the same treatment.

This mirrors how `deploy-windows-installer.yml` publishes the MSI to the same
container under a `windows/` prefix.

To move symbols to their own account later, set `SYMBOLS_STORAGE_ACCOUNT` (which
takes precedence over `APT_STORAGE_ACCOUNT`) **and** update the default URL in
`scripts/symbolicate-minidump.sh`. A shell script cannot read repository
variables, so the read side of the store lives in the repo rather than in CI
config — the two must move together.

Changing only the account would keep publishing to the new store while every
lookup still resolved against the old host, and because the `$web` container
answers unknown blobs with the site error document, that surfaces as unresolved
frames rather than an obvious failure.

If the account key or storage account is missing, `publish_symbols` skips with a
notice rather than failing, so forks and unconfigured repositories still build
releases.

Because this is a static-website container, a request for an unknown symbol is
answered by the site's error document rather than a bare 404 body. That is fine
as long as the response status stays 404 — worth confirming after the first
publish, since a 200 would hand `minidump-stackwalk` an HTML page in place of a
symbol file.

## Limits

- **Symbols cannot be produced retroactively.** Any release built before this
  landed has no symbols and never will.
- **Linux and macOS have no minidump writer.** The store pays off immediately on
  Windows, where WER writes dumps. On the other platforms it improves crash
  *logs* via the retained symbol table; producing real minidumps there would
  need an in-process crash handler.
