#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SELF_REL="scripts/cargo-flatten-dupes.sh"
DEFAULT_MANIFEST_PATH="$ROOT_DIR/Cargo.toml"
WHY_TREE_MAX_LINES=40

MODE=""
JSON_OUTPUT=0
INCLUDE_DEV=0
MANIFEST_PATH="$DEFAULT_MANIFEST_PATH"
FILTER_CRATE=""
TREE_EDGES="normal,build"

TMP_FILES=()
ALL_METADATA_FILE=""
HOST_METADATA_FILE=""
ANALYSIS_FILE=""
WORKSPACE_ROOT=""
WORKSPACE_MANIFEST_PATH=""
HOST_TRIPLE=""
ANALYSIS_READY=0

usage() {
  cat <<'EOF'
Usage: scripts/cargo-flatten-dupes.sh [options] <command> [crate]

Audit duplicate resolved Cargo crates, explain which requirement keeps a lower
version alive, and suggest local flattening steps where possible.

Commands:
  summary            List duplicate crates with blocker summaries.
  why <crate>        Show incoming requirements and reverse-tree excerpts for a duplicate crate.
  suggest [crate]    Emit non-mutating local fix hints for one duplicate crate or all of them.

Options:
  --manifest-path PATH  Cargo manifest to analyze. Default: workspace root Cargo.toml
  --include-dev         Include dev-dependencies in reachability and blocker analysis.
  --json                Emit machine-readable JSON for the selected command.
  -h, --help            Show this help.

Notes:
  - Analysis uses `cargo metadata --locked --offline` and does not edit files.
  - Target scope defaults to all targets. This catches platform-specific skew.
  - `suggest` reports local manifest or lockfile actions only when they can
    remove a duplicate without relying on upstream changes.

Examples:
  scripts/cargo-flatten-dupes.sh summary
  scripts/cargo-flatten-dupes.sh why rustix
  scripts/cargo-flatten-dupes.sh suggest windows
  scripts/cargo-flatten-dupes.sh --json suggest
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

cleanup() {
  if ((${#TMP_FILES[@]} == 0)); then
    return
  fi

  rm -f "${TMP_FILES[@]}"
}

trap cleanup EXIT

make_tmp() {
  local tmp
  tmp="$(mktemp)"
  TMP_FILES+=("$tmp")
  printf '%s\n' "$tmp"
}

require_tool() {
  local tool="$1"
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required."
}

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

normalize_version() {
  printf '%s\n' "${1%%+*}"
}

version_compare() {
  local left right max
  left="$(normalize_version "$1")"
  right="$(normalize_version "$2")"

  if [[ "$left" == "$right" ]]; then
    printf '0\n'
    return
  fi

  max="$(printf '%s\n%s\n' "$left" "$right" | sort -V | tail -n 1)"
  if [[ "$max" == "$left" ]]; then
    printf '1\n'
  else
    printf '%s\n' '-1'
  fi
}

version_ge() {
  [[ "$(version_compare "$1" "$2")" != "-1" ]]
}

version_gt() {
  [[ "$(version_compare "$1" "$2")" == "1" ]]
}

version_le() {
  [[ "$(version_compare "$1" "$2")" != "1" ]]
}

version_lt() {
  [[ "$(version_compare "$1" "$2")" == "-1" ]]
}

parse_req_base() {
  local raw="$1"
  local core
  local -a parts=()

  raw="${raw%%+*}"
  core="${raw%%-*}"
  IFS='.' read -r -a parts <<<"$core"

  printf '%s\t%s\t%s\t%s\n' \
    "${#parts[@]}" \
    "${parts[0]:-0}" \
    "${parts[1]:-0}" \
    "${parts[2]:-0}"
}

caret_upper_bound() {
  local count major minor patch
  IFS=$'\t' read -r count major minor patch <<<"$(parse_req_base "$1")"

  if ((major > 0)); then
    printf '%s.0.0\n' "$((major + 1))"
  elif ((minor > 0)); then
    printf '0.%s.0\n' "$((minor + 1))"
  else
    printf '0.0.%s\n' "$((patch + 1))"
  fi
}

tilde_upper_bound() {
  local count major minor patch
  IFS=$'\t' read -r count major minor patch <<<"$(parse_req_base "$1")"

  if ((count <= 1)); then
    printf '%s.0.0\n' "$((major + 1))"
  else
    printf '%s.%s.0\n' "$major" "$((minor + 1))"
  fi
}

req_clause_allows_version() {
  local clause="$1"
  local version="$2"
  local count major minor patch lower upper rest

  clause="$(trim "$clause")"
  version="$(normalize_version "$version")"

  if [[ -z "$clause" ]]; then
    return 0
  fi

  case "$clause" in
    \*)
      return 0
      ;;
    \^*)
      rest="$(trim "${clause#^}")"
      IFS=$'\t' read -r count major minor patch <<<"$(parse_req_base "$rest")"
      lower="${major}.${minor}.${patch}"
      upper="$(caret_upper_bound "$rest")"
      version_ge "$version" "$lower" && version_lt "$version" "$upper"
      return
      ;;
    \~*)
      rest="$(trim "${clause#\~}")"
      IFS=$'\t' read -r count major minor patch <<<"$(parse_req_base "$rest")"
      lower="${major}.${minor}.${patch}"
      upper="$(tilde_upper_bound "$rest")"
      version_ge "$version" "$lower" && version_lt "$version" "$upper"
      return
      ;;
    \>=*)
      rest="$(trim "${clause#>=}")"
      version_ge "$version" "$rest"
      return
      ;;
    \>*)
      rest="$(trim "${clause#>}")"
      version_gt "$version" "$rest"
      return
      ;;
    \<\=*)
      rest="$(trim "${clause#<=}")"
      version_le "$version" "$rest"
      return
      ;;
    \<*)
      rest="$(trim "${clause#<}")"
      version_lt "$version" "$rest"
      return
      ;;
    \=*)
      rest="$(trim "${clause#=}")"
      [[ "$(normalize_version "$rest")" == "$version" ]]
      return
      ;;
  esac

  if [[ "$clause" == *"*"* ]]; then
    IFS=$'\t' read -r _ major minor patch <<<"$(parse_req_base "${clause//\*/0}")"
    IFS='.' read -r -a version_parts <<<"${version%%-*}"

    case "$clause" in
      *.*.*)
        [[ "${version_parts[0]:-0}" == "$major" && "${version_parts[1]:-0}" == "$minor" && "${version_parts[2]:-0}" == "$patch" ]]
        ;;
      *.*)
        [[ "${version_parts[0]:-0}" == "$major" && "${version_parts[1]:-0}" == "$minor" ]]
        ;;
      *)
        [[ "${version_parts[0]:-0}" == "$major" ]]
        ;;
    esac
    return
  fi

  if [[ "$clause" == *"||"* ]]; then
    return 1
  fi

  IFS=$'\t' read -r count major minor patch <<<"$(parse_req_base "$clause")"
  lower="${major}.${minor}.${patch}"
  upper="$(caret_upper_bound "$clause")"
  version_ge "$version" "$lower" && version_lt "$version" "$upper"
}

req_allows_version() {
  local req="$1"
  local version="$2"
  local clause
  local -a clauses=()

  req="$(trim "$req")"
  if [[ -z "$req" || "$req" == *"||"* ]]; then
    return 1
  fi

  IFS=',' read -r -a clauses <<<"$req"
  for clause in "${clauses[@]}"; do
    clause="$(trim "$clause")"
    if ! req_clause_allows_version "$clause" "$version"; then
      return 1
    fi
  done

  return 0
}

humanize_status() {
  case "$1" in
    locally-fixable)
      printf 'locally-fixable'
      ;;
    platform-specific)
      printf 'platform-specific'
      ;;
    upstream-blocked)
      printf 'upstream-blocked'
      ;;
    source-mismatch)
      printf 'source-mismatch'
      ;;
    *)
      printf '%s' "$1"
      ;;
  esac
}

display_path() {
  local path="$1"

  if [[ -n "$WORKSPACE_ROOT" && "$path" == "$WORKSPACE_ROOT/"* ]]; then
    printf '%s\n' "${path#$WORKSPACE_ROOT/}"
  else
    printf '%s\n' "$path"
  fi
}

manifest_dep_mode() {
  local manifest="$1"
  local dep="$2"
  local snippet

  if [[ ! -f "$manifest" ]]; then
    printf 'unknown\n'
    return
  fi

  snippet="$(
    awk -v dep="$dep" '
      BEGIN {
        capturing = 0
        open_count = 0
        close_count = 0
      }
      {
        if (!capturing && $0 ~ "^[[:space:]]*" dep "[[:space:]]*=") {
          capturing = 1
        }

        if (!capturing) {
          next
        }

        print
        open_count += gsub(/\{/, "{")
        close_count += gsub(/\}/, "}")

        if (open_count == 0 || close_count >= open_count) {
          exit
        }
      }
    ' "$manifest"
  )"

  if [[ -z "$snippet" ]]; then
    printf 'unknown\n'
    return
  fi

  if grep -Eq 'workspace[[:space:]]*=[[:space:]]*true' <<<"$snippet"; then
    printf 'workspace\n'
  else
    printf 'explicit\n'
  fi
}

parse_args() {
  local -a positionals=()

  while (($# > 0)); do
    case "$1" in
      summary|why|suggest)
        if [[ -n "$MODE" ]]; then
          die "multiple commands given: $MODE and $1"
        fi
        MODE="$1"
        ;;
      --manifest-path)
        shift
        (($# > 0)) || die "--manifest-path requires a value"
        MANIFEST_PATH="$1"
        ;;
      --include-dev)
        INCLUDE_DEV=1
        ;;
      --json)
        JSON_OUTPUT=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      --)
        shift
        positionals+=("$@")
        break
        ;;
      -*)
        die "unknown option: $1"
        ;;
      *)
        positionals+=("$1")
        ;;
    esac
    shift
  done

  [[ -n "$MODE" ]] || die "missing command"

  case "$MODE" in
    summary)
      ((${#positionals[@]} == 0)) || die "summary does not take a crate name"
      ;;
    why)
      ((${#positionals[@]} == 1)) || die "why requires exactly one crate name"
      FILTER_CRATE="${positionals[0]}"
      ;;
    suggest)
      ((${#positionals[@]} <= 1)) || die "suggest takes zero or one crate name"
      FILTER_CRATE="${positionals[0]:-}"
      ;;
  esac
}

collect_metadata() {
  HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
  [[ -n "$HOST_TRIPLE" ]] || die "failed to determine the host target triple"

  cargo metadata --format-version 1 --locked --offline --manifest-path "$MANIFEST_PATH" >"$ALL_METADATA_FILE"
  cargo metadata --format-version 1 --locked --offline --manifest-path "$MANIFEST_PATH" --filter-platform "$HOST_TRIPLE" >"$HOST_METADATA_FILE"
}

build_analysis() {
  jq -n \
    --slurpfile all "$ALL_METADATA_FILE" \
    --slurpfile host "$HOST_METADATA_FILE" \
    --argjson include_dev "$INCLUDE_DEV" '
def kind_label($kind):
  if $kind == null then "normal" else $kind end;
def target_label($target):
  if $target == null then "all" else $target end;
def source_label($pkg):
  if $pkg.source == null then "path" else $pkg.source end;
def source_family($pkg):
  if $pkg.source == null then "path"
  elif ($pkg.source | startswith("registry+")) then "crates.io"
  elif ($pkg.source | startswith("git+")) then "git"
  else $pkg.source
  end;
def semver_capture($version):
  try (
    $version
    | capture("^(?<maj>[0-9]+)\\.(?<min>[0-9]+)\\.(?<patch>[0-9]+)(?:-(?<pre>[^+]+))?(?:\\+(?<build>.*))?$")
  ) catch {
    "maj": "0",
    "min": "0",
    "patch": "0",
    "pre": "~",
    "build": ""
  };
def semver_key($version):
  (semver_capture($version)) as $m
  | [
      ($m.maj | tonumber),
      ($m.min | tonumber),
      ($m.patch | tonumber),
      (if ($m.pre // null) == null then 1 else 0 end),
      ($m.pre // ""),
      ($m.build // "")
    ];
def allowed_dep_kinds($dep_kinds):
  [
    $dep_kinds[]?
    | select(
        (kind_label(.kind) == "normal")
        or (kind_label(.kind) == "build")
        or ($include_dev and kind_label(.kind) == "dev")
      )
  ];
def reachable_ids($metadata):
  ($metadata.resolve.nodes // []) as $nodes
  | ($nodes
      | map({
          key: .id,
          value: [
            .deps[]?
            | select((allowed_dep_kinds(.dep_kinds) | length) > 0)
            | .pkg
          ]
        })
      | from_entries) as $adj
  | def closure($frontier; $seen):
      if ($frontier | length) == 0 then ($seen | unique)
      else
        ([ $frontier[] | $adj[.][]? ] | unique) as $candidates
        | [ $candidates[] as $candidate | select($seen | index($candidate) | not) | $candidate ] as $next
        | closure($next; (($seen + $next) | unique))
      end;
    closure(($metadata.workspace_members // []); (($metadata.workspace_members // []) | unique));
($all[0]) as $meta
| ($host[0]) as $host_meta
| (reachable_ids($meta)) as $reachable
| (reachable_ids($host_meta)) as $host_reachable
| ($meta.packages | map(. as $pkg | select($reachable | index($pkg.id))) | sort_by(.name, .version)) as $packages
| ($packages | map({key: .id, value: .}) | from_entries) as $pkg_map
| (
    [
      ($meta.resolve.nodes // [])[] as $node
      | select($reachable | index($node.id))
      | ($pkg_map[$node.id]) as $from
      | $node.deps[]? as $edge
      | (allowed_dep_kinds($edge.dep_kinds)) as $allowed
      | select(($allowed | length) > 0)
      | select($reachable | index($edge.pkg))
      | ($pkg_map[$edge.pkg]) as $to
      | ($from.dependencies
          | map(select((.rename // .name) == $edge.name))) as $same_name_decls
      | ($same_name_decls
          | map(
              . as $decl
              | select(
                  if ($allowed | length) == 0 then true
                  else any(
                    $allowed[];
                    (($decl.kind // "normal") == kind_label(.kind))
                    and (($decl.target // "all") == target_label(.target))
                  )
                  end
                )
            )) as $matched_decls
      | {
          to_id: $edge.pkg,
          to_name: $to.name,
          from_id: $node.id,
          from_name: $from.name,
          from_version: $from.version,
          from_source: source_label($from),
          from_source_family: source_family($from),
          from_manifest_path: ($from.manifest_path // ""),
          from_editable: (
            $from.source == null
            and (($from.manifest_path // "") | startswith($meta.workspace_root + "/"))
          ),
          dep_name: $edge.name,
          reqs: (
            if ($matched_decls | length) > 0 then ($matched_decls | map(.req) | unique | sort)
            elif ($same_name_decls | length) > 0 then ($same_name_decls | map(.req) | unique | sort)
            else ["unknown"]
            end
          ),
          kind_labels: ($allowed | map(kind_label(.kind)) | unique | sort),
          targets: ($allowed | map(target_label(.target)) | unique | sort),
          direct_target_specific: ($allowed | any(.target != null))
        }
    ]
  ) as $incoming_edges
| ($incoming_edges | sort_by(.to_id) | group_by(.to_id) | map({key: .[0].to_id, value: .}) | from_entries) as $incoming_by_to
| {
    workspace_root: $meta.workspace_root,
    manifest_path: ($meta.workspace_root + "/Cargo.toml"),
    host_triple: "'"$HOST_TRIPLE"'",
    include_dev: $include_dev,
    target_scope: "all",
    duplicates: (
      $packages
      | sort_by(.name)
      | group_by(.name)
      | map(select((map(.id) | unique | length) > 1))
      | map(
          . as $group
          | ($group | sort_by(semver_key(.version))) as $sorted
          | ($sorted[-1]) as $candidate_pkg
          | ($sorted
              | map(
                  . as $pkg
                  | {
                      id,
                      version,
                      source: source_label($pkg),
                      source_family: source_family($pkg),
                      host_reachable: ($host_reachable | index($pkg.id) != null),
                      inbound_edges: ($incoming_by_to[$pkg.id] // [])
                    }
                )) as $versions
          | ($versions[0:-1]) as $lower_versions
          | ([ $lower_versions[]?.inbound_edges[]? ]) as $lower_blockers
          | ([ $lower_blockers[] | select(.from_editable) ]) as $local_blockers
          | ([ $lower_blockers[] | select(.from_editable | not) ]) as $upstream_blockers
          | ([ $group[] | source_family(.) ] | unique) as $source_families
          | {
              name: $group[0].name,
              status: (
                if ($source_families | length) > 1 then "source-mismatch"
                elif (($lower_versions | length) > 0 and ([ $lower_versions[] | select(.host_reachable) ] | length) == 0) then "platform-specific"
                elif (($lower_blockers | length) > 0 and ($local_blockers | length) == ($lower_blockers | length)) then "locally-fixable"
                else "upstream-blocked"
                end
              ),
              source_families: $source_families,
              candidate: {
                id: $candidate_pkg.id,
                version: $candidate_pkg.version,
                source: source_label($candidate_pkg),
                source_family: source_family($candidate_pkg)
              },
              versions: $versions,
              blockers: $lower_blockers,
              local_blockers: $local_blockers,
              upstream_blockers: $upstream_blockers
            }
        )
      | sort_by(
          (if .status == "locally-fixable" then 0 elif .status == "platform-specific" then 1 elif .status == "upstream-blocked" then 2 else 3 end),
          .name
        )
    )
  }'
}

ensure_analysis() {
  if ((ANALYSIS_READY == 1)); then
    return
  fi

  require_tool cargo
  require_tool jq
  require_tool rustc

  ALL_METADATA_FILE="$(make_tmp)"
  HOST_METADATA_FILE="$(make_tmp)"
  ANALYSIS_FILE="$(make_tmp)"
  TREE_EDGES="normal,build"
  if ((INCLUDE_DEV == 1)); then
    TREE_EDGES="normal,build,dev"
  fi

  collect_metadata
  build_analysis >"$ANALYSIS_FILE"

  WORKSPACE_ROOT="$(jq -r '.workspace_root' "$ANALYSIS_FILE")"
  WORKSPACE_MANIFEST_PATH="$(jq -r '.manifest_path' "$ANALYSIS_FILE")"
  ANALYSIS_READY=1
}

duplicate_record_or_die() {
  local crate="$1"
  local record

  record="$(jq -c --arg crate "$crate" '.duplicates[] | select(.name == $crate)' "$ANALYSIS_FILE")"
  [[ -n "$record" ]] || die "no duplicate resolved versions found for crate '$crate'"
  printf '%s\n' "$record"
}

cargo_tree_excerpt() {
  local spec="$1"
  local tree
  local line_count

  tree="$(
    cargo tree \
      -i "$spec" \
      --workspace \
      --locked \
      --offline \
      --charset ascii \
      --target all \
      --edges "$TREE_EDGES"
  )"

  line_count="$(printf '%s\n' "$tree" | wc -l | tr -d ' ')"
  if ((line_count > WHY_TREE_MAX_LINES)); then
    printf '%s\n' "$tree" | sed -n "1,${WHY_TREE_MAX_LINES}p"
    printf '...\n'
  else
    printf '%s\n' "$tree"
  fi
}

emit_summary() {
  if ((JSON_OUTPUT == 1)); then
    cat "$ANALYSIS_FILE"
    return
  fi

  local count
  count="$(jq -r '.duplicates | length' "$ANALYSIS_FILE")"
  if [[ "$count" == "0" ]]; then
    echo "No duplicate crates found for the selected graph."
    return
  fi

  printf 'Duplicate crates (target=all, edges=%s)\n' "$TREE_EDGES"
  printf '%-19s %-24s %-34s %s\n' "STATUS" "CRATE" "VERSIONS" "DETAIL"

  while IFS=$'\t' read -r status name versions detail; do
    printf '%-19s %-24s %-34s %s\n' "$status" "$name" "$versions" "$detail"
  done < <(
    jq -r '
      .duplicates[]
      | [
          .status,
          .name,
          (.versions | map(.version + " [" + .source_family + "]") | join(", ")),
          (
            if .status == "source-mismatch" then
              "sources=" + (.source_families | join(", "))
            else
              ([ .blockers[] | "\(.from_name) \(.from_version) -> \(.reqs | join(" | "))" ]
                | unique
                | .[0:2]
                | if length == 0 then "-" else join("; ") end)
            end
          )
        ]
      | @tsv
    ' "$ANALYSIS_FILE"
  )

  printf '\nUse `%s why <crate>` for reverse paths and `%s suggest <crate>` for local actions.\n' "$SELF_REL" "$SELF_REL"
}

emit_why() {
  local record_json="$1"
  local name status candidate
  local -a version_payloads=()
  local version
  local count

  if ((JSON_OUTPUT == 1)); then
    while IFS= read -r version; do
      local repeated spec tree
      repeated="$(jq -r --arg version "$version" '[.versions[] | select(.version == $version)] | length > 1' <<<"$record_json")"
      spec="$(jq -r --arg version "$version" '.versions[] | select(.version == $version) | .id' <<<"$record_json" | head -n 1)"
      if [[ "$repeated" == "true" ]]; then
        tree="reverse tree omitted because multiple source variants share version $version"
      else
        spec="$(jq -r --arg version "$version" '.versions[] | select(.version == $version) | .name? // empty' <<<"$record_json")"
        spec="${FILTER_CRATE}@${version}"
        tree="$(cargo_tree_excerpt "$spec")"
      fi
      version_payloads+=("$(jq -n --arg version "$version" --arg spec "$spec" --arg reverse_tree "$tree" '{version: $version, spec: $spec, reverse_tree: $reverse_tree}')")
    done < <(jq -r '.versions[].version' <<<"$record_json")

    if ((${#version_payloads[@]} == 0)); then
      jq -n --argjson duplicate "$record_json" '{duplicate: $duplicate, reverse_trees: []}'
    else
      printf '%s\n' "${version_payloads[@]}" | jq -s --argjson duplicate "$record_json" '{duplicate: $duplicate, reverse_trees: .}'
    fi
    return
  fi

  name="$(jq -r '.name' <<<"$record_json")"
  status="$(jq -r '.status' <<<"$record_json")"
  candidate="$(jq -r '.candidate.version + " [" + .candidate.source_family + "]"' <<<"$record_json")"

  printf '%s [%s]\n' "$name" "$(humanize_status "$status")"
  printf 'candidate: %s\n' "$candidate"
  printf 'versions: %s\n' "$(jq -r '.versions | map(.version + " [" + .source_family + "]") | join(", ")' <<<"$record_json")"

  while IFS= read -r version; do
    local source_family host_reachable repeated tree spec
    source_family="$(jq -r --arg version "$version" '.versions[] | select(.version == $version) | .source_family' <<<"$record_json" | head -n 1)"
    host_reachable="$(jq -r --arg version "$version" '.versions[] | select(.version == $version) | .host_reachable' <<<"$record_json" | head -n 1)"
    repeated="$(jq -r --arg version "$version" '[.versions[] | select(.version == $version)] | length > 1' <<<"$record_json")"

    printf '\n%s [%s] host-reachable=%s\n' "$version" "$source_family" "$host_reachable"
    echo "incoming requirements:"
    while IFS=$'\t' read -r from_name from_version reqs kinds targets editable; do
      printf '  - %s %s -> %s [kind=%s target=%s editable=%s]\n' "$from_name" "$from_version" "$reqs" "$kinds" "$targets" "$editable"
    done < <(
      jq -r --arg version "$version" '
        .versions[]
        | select(.version == $version)
        | .inbound_edges[]
        | [
            .from_name,
            .from_version,
            (.reqs | join(" | ")),
            (.kind_labels | join(",")),
            (.targets | join(",")),
            (if .from_editable then "yes" else "no" end)
          ]
        | @tsv
      ' <<<"$record_json"
    )

    echo "reverse tree excerpt:"
    if [[ "$repeated" == "true" ]]; then
      echo "  reverse tree omitted because multiple source variants share version $version"
    else
      spec="${FILTER_CRATE}@${version}"
      tree="$(cargo_tree_excerpt "$spec")"
      sed 's/^/  /' <<<"$tree"
    fi
  done < <(jq -r '.versions[].version' <<<"$record_json")
}

build_upstream_blockers_json() {
  local record_json="$1"

  jq '
    [
      .upstream_blockers[]
      | {
          from_name,
          from_version,
          reqs: (.reqs | join(" | ")),
          kinds: (.kind_labels | join(",")),
          targets: (.targets | join(",")),
          manifest_path: .from_manifest_path
        }
    ]
    | unique_by([.from_name, .from_version, .reqs, .kinds, .targets, .manifest_path])
    | sort_by(.from_name, .from_version, .reqs, .targets)
  ' <<<"$record_json"
}

build_suggest_payload() {
  local record_json="$1"
  local name status candidate_version
  local local_count upstream_count outcome reason
  local all_allow=1
  local -a all_reqs=()
  local -a local_lines=()
  local -a suggestion_jsons=()
  local req mode target_manifest via_manifest key from_name from_version reqs kinds targets current_req
  local repeated_note=""
  local upstream_blockers_json
  declare -A seen=()

  name="$(jq -r '.name' <<<"$record_json")"
  status="$(jq -r '.status' <<<"$record_json")"
  candidate_version="$(jq -r '.candidate.version' <<<"$record_json")"
  local_count="$(jq -r '.local_blockers | length' <<<"$record_json")"
  upstream_count="$(jq -r '.upstream_blockers | length' <<<"$record_json")"
  upstream_blockers_json="$(build_upstream_blockers_json "$record_json")"

  case "$status" in
    source-mismatch)
      outcome="blocked"
      reason="resolved from multiple source families: $(jq -r '.source_families | join(", ")' <<<"$record_json")"
      jq -n --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: []}'
      return
      ;;
    upstream-blocked)
      outcome="blocked"
      reason="lower-version blockers come from upstream crates"
      jq -n --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: []}'
      return
      ;;
  esac

  if [[ "$local_count" == "0" ]]; then
    outcome="blocked"
    reason="no editable first-party blocker manifests were found"
    jq -n --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: []}'
    return
  fi

  if [[ "$upstream_count" != "0" ]]; then
    outcome="blocked"
    reason="upstream crates still require a lower version, so local edits cannot fully flatten this duplicate"
    jq -n --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: []}'
    return
  fi

  while IFS= read -r req; do
    all_reqs+=("$req")
  done < <(jq -r '.local_blockers[].reqs[]' <<<"$record_json")

  for req in "${all_reqs[@]}"; do
    if ! req_allows_version "$req" "$candidate_version"; then
      all_allow=0
      break
    fi
  done

  if ((all_allow == 1)); then
    outcome="actionable"
    reason="all editable requirements already admit the highest resolved version"
    jq -n \
      --argjson duplicate "$record_json" \
      --arg outcome "$outcome" \
      --arg reason "$reason" \
      --arg crate "$name" \
      --arg version "$candidate_version" \
      '{
         duplicate: $duplicate,
         outcome: $outcome,
         reason: $reason,
         suggestions: [
           {
             type: "cargo-update",
             crate: $crate,
             version: $version,
             command: ("cargo update -p " + $crate + " --precise " + $version)
           }
         ]
       }'
    return
  fi

  while IFS=$'\t' read -r via_manifest from_name from_version reqs kinds targets; do
    mode="$(manifest_dep_mode "$via_manifest" "$name")"
    target_manifest="$via_manifest"
    if [[ "$mode" == "workspace" ]]; then
      target_manifest="$WORKSPACE_MANIFEST_PATH"
    fi

    key="${target_manifest}|${reqs}"
    if [[ -n "${seen[$key]:-}" ]]; then
      continue
    fi
    seen[$key]=1

    current_req="$reqs"
    suggestion_jsons+=("$(
      jq -n \
        --arg type "$(if [[ "$mode" == "workspace" ]]; then printf 'workspace-bump'; else printf 'manifest-bump'; fi)" \
        --arg crate "$name" \
        --arg target_manifest "$target_manifest" \
        --arg via_manifest "$via_manifest" \
        --arg from_requirement "$current_req" \
        --arg to_requirement "^${candidate_version}" \
        --arg via_package "${from_name} ${from_version}" \
        --arg kinds "$kinds" \
        --arg targets "$targets" \
        '{
           type: $type,
           crate: $crate,
           target_manifest: $target_manifest,
           via_manifest: $via_manifest,
           via_package: $via_package,
           from_requirement: $from_requirement,
           to_requirement: $to_requirement,
           kinds: $kinds,
           targets: $targets
         }'
    )")
  done < <(
    jq -r '
      .local_blockers[]
      | [
          .from_manifest_path,
          .from_name,
          .from_version,
          (.reqs | join(" | ")),
          (.kind_labels | join(",")),
          (.targets | join(","))
        ]
      | @tsv
    ' <<<"$record_json"
  )

  if ((${#suggestion_jsons[@]} == 0)); then
    outcome="blocked"
    reason="editable blockers were found, but no manifest suggestion could be constructed"
    jq -n --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: []}'
    return
  fi

  outcome="actionable"
  reason="editable blocker requirements need to move to the highest resolved version"
  printf '%s\n' "${suggestion_jsons[@]}" | jq -s --argjson duplicate "$record_json" --arg outcome "$outcome" --arg reason "$reason" --argjson blockers "$upstream_blockers_json" '{duplicate: $duplicate, outcome: $outcome, reason: $reason, blockers: $blockers, suggestions: .}'
}

emit_suggest() {
  local -a payloads=()
  local record_json payload_json

  if [[ -n "$FILTER_CRATE" ]]; then
    record_json="$(duplicate_record_or_die "$FILTER_CRATE")"
    payload_json="$(build_suggest_payload "$record_json")"
    if ((JSON_OUTPUT == 1)); then
      printf '%s\n' "$payload_json"
      return
    fi
    payloads=("$payload_json")
  else
    while IFS= read -r record_json; do
      payloads+=("$(build_suggest_payload "$record_json")")
    done < <(jq -c '.duplicates[]' "$ANALYSIS_FILE")

    if ((JSON_OUTPUT == 1)); then
      if ((${#payloads[@]} == 0)); then
        jq -n '{duplicates: []}'
      else
        printf '%s\n' "${payloads[@]}" | jq -s '{duplicates: .}'
      fi
      return
    fi
  fi

  if ((${#payloads[@]} == 0)); then
    echo "No duplicate crates found for the selected graph."
    return
  fi

  for payload_json in "${payloads[@]}"; do
    local name status outcome reason
    name="$(jq -r '.duplicate.name' <<<"$payload_json")"
    status="$(jq -r '.duplicate.status' <<<"$payload_json")"
    outcome="$(jq -r '.outcome' <<<"$payload_json")"
    reason="$(jq -r '.reason' <<<"$payload_json")"

    printf '%s [%s] %s\n' "$name" "$(humanize_status "$status")" "$outcome"
    printf '  %s\n' "$reason"

    if [[ "$(jq -r '.blockers | length' <<<"$payload_json")" != "0" ]]; then
      while IFS= read -r blocker_json; do
        local blocker_from_name blocker_from_version blocker_reqs blocker_kinds blocker_targets
        blocker_from_name="$(jq -r '.from_name' <<<"$blocker_json")"
        blocker_from_version="$(jq -r '.from_version' <<<"$blocker_json")"
        blocker_reqs="$(jq -r '.reqs' <<<"$blocker_json")"
        blocker_kinds="$(jq -r '.kinds' <<<"$blocker_json")"
        blocker_targets="$(jq -r '.targets' <<<"$blocker_json")"
        printf '  - blocker: %s %s -> %s [kind=%s target=%s]\n' \
          "$blocker_from_name" \
          "$blocker_from_version" \
          "$blocker_reqs" \
          "$blocker_kinds" \
          "$blocker_targets"
      done < <(jq -c '.blockers[]' <<<"$payload_json")
    fi

    if [[ "$(jq -r '.suggestions | length' <<<"$payload_json")" == "0" ]]; then
      echo
      continue
    fi

    while IFS= read -r suggestion_json; do
      local type crate command target_manifest via_manifest via_package from_requirement to_requirement kinds targets
      type="$(jq -r '.type' <<<"$suggestion_json")"

      case "$type" in
        cargo-update)
          command="$(jq -r '.command' <<<"$suggestion_json")"
          printf '  - run `%s`\n' "$command"
          ;;
        workspace-bump|manifest-bump)
          crate="$(jq -r '.crate' <<<"$suggestion_json")"
          target_manifest="$(jq -r '.target_manifest' <<<"$suggestion_json")"
          via_manifest="$(jq -r '.via_manifest' <<<"$suggestion_json")"
          via_package="$(jq -r '.via_package' <<<"$suggestion_json")"
          from_requirement="$(jq -r '.from_requirement' <<<"$suggestion_json")"
          to_requirement="$(jq -r '.to_requirement' <<<"$suggestion_json")"
          kinds="$(jq -r '.kinds' <<<"$suggestion_json")"
          targets="$(jq -r '.targets' <<<"$suggestion_json")"
          printf '  - edit %s: move `%s` from `%s` to `%s` (via %s in %s; kind=%s target=%s)\n' \
            "$(display_path "$target_manifest")" \
            "$crate" \
            "$from_requirement" \
            "$to_requirement" \
            "$via_package" \
            "$(display_path "$via_manifest")" \
            "$kinds" \
            "$targets"
          ;;
      esac
    done < <(jq -c '.suggestions[]' <<<"$payload_json")

    echo
  done
}

main() {
  parse_args "$@"
  ensure_analysis

  case "$MODE" in
    summary)
      emit_summary
      ;;
    why)
      emit_why "$(duplicate_record_or_die "$FILTER_CRATE")"
      ;;
    suggest)
      emit_suggest
      ;;
  esac
}

main "$@"
