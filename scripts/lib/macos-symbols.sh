#!/usr/bin/env bash
# macOS symbol extraction helpers for package-macos.sh. Sourced, not executed.
# shellcheck disable=SC2154,SC2034  # repo_root, mode, arch, bin_src, symbols_abs are caller-defined; SYMBOL_INPUT is exported for the caller.
#
# Requires the caller to have defined: bin_src (the packaged binary),
# repo_root, mode (debug|release), arch, and symbols_abs (absolute store root).
# Emits the resolved dSYM input via SYMBOL_INPUT.
#
# The ordering is transactional: symbols are extracted while the build output
# is still untouched (the binary is copied into the bundle and codesigned
# after this runs), the dSYM identity is validated against the binary's UUID
# before anything is emitted, and the canonical bundle is only moved into
# place once the checks pass.

extract_macos_symbols() {
  # Read the .dSYM rather than the binary: with split-debuginfo = "packed" the
  # DWARF lives in the bundle, and the binary alone would yield symbols with no
  # line numbers.
  symbol_input="${repo_root}/target/${mode}/gitcomet.dSYM"
  if [[ ! -d "$symbol_input" ]]; then
    # Only reached when Cargo left no uplifted symlink; the bundle then sits
    # beside the hashed artifact. `|| true` so a missing deps/ does not abort
    # before the diagnostic below.
    candidates="$(find "${repo_root}/target/${mode}/deps" -maxdepth 1 -type d \
      -name 'gitcomet-*.dSYM' 2>/dev/null | sort || true)"

    if [[ -n "$candidates" ]]; then
      candidate_count="$(printf '%s\n' "$candidates" | wc -l | tr -d '[:space:]')"
      if [[ "$candidate_count" -gt 1 ]]; then
        # `cargo test` and benches leave identically-named bundles here. Picking
        # one arbitrarily would publish symbols keyed to the wrong binary's
        # UUID, and every later check would still pass.
        echo "Multiple dSYM bundles under ${repo_root}/target/${mode}/deps:" >&2
        printf '%s\n' "$candidates" >&2
        echo "Cannot tell which belongs to the gitcomet binary; clean the target directory." >&2
        exit 1
      fi
      symbol_input="$candidates"
    fi
  fi

  if [[ ! -d "$symbol_input" ]]; then
    echo "No dSYM bundle found under ${repo_root}/target/${mode}." >&2
    if [[ -d "${repo_root}/target/${mode}/gitcomet.dSYM.staged" ]]; then
      echo "A previous run was interrupted mid-move; the bundle is at" >&2
      echo "  ${repo_root}/target/${mode}/gitcomet.dSYM.staged" >&2
      echo "Rename it to gitcomet.dSYM to recover it, or delete it and rebuild." >&2
      exit 1
    fi
    echo "Build with split-debuginfo = \"packed\" (the release workflow sets" >&2
    echo "CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO) to produce one." >&2
    exit 1
  fi

  # A bundle left over from an earlier build sits at exactly the canonical path
  # and passes every later check, publishing symbols under the wrong build's
  # UUID while the shipped binary resolves to nothing. Compare identities.
  if command -v dwarfdump >/dev/null 2>&1; then
    bin_uuid="$(dwarfdump --uuid "$bin_src" 2>/dev/null | awk '{print $2}' | head -1 || true)"
    if [[ -z "$bin_uuid" ]]; then
      echo "Could not read a UUID from ${bin_src}; refusing to guess which dSYM matches." >&2
      exit 1
    fi
    if ! dwarfdump --uuid "$symbol_input" 2>/dev/null | grep -qF "$bin_uuid"; then
      echo "dSYM does not match the binary being packaged." >&2
      echo "  binary: ${bin_src} (${bin_uuid})" >&2
      echo "  dSYM:   ${symbol_input}" >&2
      dwarfdump --uuid "$symbol_input" 2>/dev/null >&2 || true
      echo "It is stale; remove it or clean target/${mode} and rebuild." >&2
      exit 1
    fi
  else
    echo "::warning title=dSYM identity unverified::dwarfdump is unavailable, so ${symbol_input} was not checked against ${bin_src}."
  fi

  # Cargo only symlinks the bundle here, into intermediates that
  # clean_target_intermediates_for_ci deletes. Every check above follows the
  # link, so the -L test is what stops it being left dangling -- which `tar`
  # archives as a broken link, exiting 0 on a few hundred bytes.
  canonical_dsym="${repo_root}/target/${mode}/gitcomet.dSYM"
  if [[ "$symbol_input" != "$canonical_dsym" || -L "$canonical_dsym" ]]; then
    real_dsym="$(cd "$symbol_input" && pwd -P)"
    target_prefix="$(cd "${repo_root}/target/${mode}" && pwd -P)/"

    if [[ "$real_dsym" != "$canonical_dsym" ]]; then
      staged_dsym="${canonical_dsym}.staged"
      rm -rf "$staged_dsym"

      # A rename within target/ is free where copying would duplicate ~183 MiB
      # just before the cleanup runs. But it removes the source, so anything
      # resolving outside target/ -- a shared build cache, another volume --
      # must be copied instead of taken out of it.
      if [[ "$real_dsym" == "$target_prefix"* ]]; then
        # Aside first: the canonical path may be the symlink to this bundle.
        mv "$real_dsym" "$staged_dsym"
      else
        echo "dSYM resolves outside ${target_prefix} (${real_dsym}); copying rather than moving it."
        cp -RL "$real_dsym" "$staged_dsym"
      fi

      rm -rf "$canonical_dsym"
      mv "$staged_dsym" "$canonical_dsym"
    fi
    symbol_input="$canonical_dsym"
  fi

  if [[ -L "$canonical_dsym" || ! -d "$canonical_dsym" ]]; then
    echo "Expected a real .dSYM directory at ${canonical_dsym}." >&2
    exit 1
  fi

  # A dSYM carries DWARF but not __eh_frame, which stays in the linked binary,
  # and dump_syms reads a single file with no macOS equivalent of the .pdb/.exe
  # re-pairing it does on Windows. Requiring CFI here would fail every macOS
  # release; the warning keeps the gap visible instead.
  #
  # --module-name must match the executable a crash report names, which is
  # Contents/MacOS/gitcomet in the bundle.
  "${repo_root}/scripts/emit-breakpad-symbols.sh" \
    --input "$symbol_input" \
    --store "$symbols_abs" \
    --arch "$arch" \
    --module-name gitcomet \
    --allow-missing-cfi

  SYMBOL_INPUT="$symbol_input"
}
