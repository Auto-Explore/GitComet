#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/generate-homebrew-cask.sh \
  --version VERSION \
  --github-repo OWNER/REPO \
  --arm-dmg PATH \
  --intel-dmg PATH \
  --output PATH

Generates a Homebrew cask for GitComet from macOS DMG artifacts.
USAGE
}

version=""
github_repo=""
arm_dmg=""
intel_dmg=""
out_path=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      version="${2:-}"
      shift 2
      ;;
    --github-repo)
      github_repo="${2:-}"
      shift 2
      ;;
    --arm-dmg)
      arm_dmg="${2:-}"
      shift 2
      ;;
    --intel-dmg)
      intel_dmg="${2:-}"
      shift 2
      ;;
    --output)
      out_path="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$version" || -z "$github_repo" || -z "$arm_dmg" || -z "$intel_dmg" || -z "$out_path" ]]; then
  echo "All arguments are required." >&2
  usage
  exit 2
fi

if ! [[ "$github_repo" =~ ^[^/]+/[^/]+$ ]]; then
  echo "Invalid --github-repo '$github_repo'. Expected OWNER/REPO." >&2
  exit 2
fi

if [[ ! -f "$arm_dmg" ]]; then
  echo "arm DMG not found: $arm_dmg" >&2
  exit 1
fi

if [[ ! -f "$intel_dmg" ]]; then
  echo "intel DMG not found: $intel_dmg" >&2
  exit 1
fi

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
    return
  fi
  echo "No SHA256 tool found (sha256sum or shasum required)." >&2
  exit 1
}

arm_sha="$(sha256_file "$arm_dmg")"
intel_sha="$(sha256_file "$intel_dmg")"

mkdir -p "$(dirname "$out_path")"

cat > "$out_path" <<EOF2
cask "gitcomet" do
  arch arm: "arm64", intel: "x86_64"

  version "${version}"
  sha256 arm: "${arm_sha}", intel: "${intel_sha}"

  url "https://github.com/${github_repo}/releases/download/v#{version}/gitcomet-v#{version}-macos-#{arch}.dmg"
  name "GitComet"
  desc "Fast, resource-efficient Git GUI written in Rust"
  homepage "https://github.com/${github_repo}"

  depends_on macos: ">= :ventura"

  app "GitComet.app"

  caveats do
    <<~EOS
      Optional CLI:
        brew install gitcomet-cli
    EOS
  end
end
EOF2

echo "Generated Homebrew cask: $out_path"
