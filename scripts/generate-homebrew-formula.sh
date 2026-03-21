#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/generate-homebrew-formula.sh \
  --version VERSION \
  --github-repo OWNER/REPO \
  --arm-tar PATH \
  --intel-tar PATH \
  --linux-arm-tar PATH \
  --linux-intel-tar PATH \
  --output PATH

Generates a Homebrew formula for GitComet from macOS + Linux tarball artifacts.
USAGE
}

version=""
github_repo=""
arm_tar=""
intel_tar=""
linux_arm_tar=""
linux_intel_tar=""
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
    --arm-tar)
      arm_tar="${2:-}"
      shift 2
      ;;
    --intel-tar)
      intel_tar="${2:-}"
      shift 2
      ;;
    --linux-arm-tar)
      linux_arm_tar="${2:-}"
      shift 2
      ;;
    --linux-intel-tar)
      linux_intel_tar="${2:-}"
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

if [[ -z "$version" || -z "$github_repo" || -z "$arm_tar" || -z "$intel_tar" || -z "$linux_arm_tar" || -z "$linux_intel_tar" || -z "$out_path" ]]; then
  echo "All arguments are required." >&2
  usage
  exit 2
fi

if ! [[ "$github_repo" =~ ^[^/]+/[^/]+$ ]]; then
  echo "Invalid --github-repo '$github_repo'. Expected OWNER/REPO." >&2
  exit 2
fi

if [[ ! -f "$arm_tar" ]]; then
  echo "arm tarball not found: $arm_tar" >&2
  exit 1
fi

if [[ ! -f "$intel_tar" ]]; then
  echo "intel tarball not found: $intel_tar" >&2
  exit 1
fi

if [[ ! -f "$linux_arm_tar" ]]; then
  echo "linux arm tarball not found: $linux_arm_tar" >&2
  exit 1
fi

if [[ ! -f "$linux_intel_tar" ]]; then
  echo "linux intel tarball not found: $linux_intel_tar" >&2
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

arm_sha="$(sha256_file "$arm_tar")"
intel_sha="$(sha256_file "$intel_tar")"
linux_arm_sha="$(sha256_file "$linux_arm_tar")"
linux_intel_sha="$(sha256_file "$linux_intel_tar")"
arm_name="$(basename "$arm_tar")"
intel_name="$(basename "$intel_tar")"
linux_arm_name="$(basename "$linux_arm_tar")"
linux_intel_name="$(basename "$linux_intel_tar")"

mkdir -p "$(dirname "$out_path")"

cat > "$out_path" <<EOF2
class GitcometCli < Formula
  desc "GitComet command-line binary"
  homepage "https://github.com/${github_repo}"
  version "${version}"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/${github_repo}/releases/download/v${version}/${arm_name}"
      sha256 "${arm_sha}"
    end

    on_intel do
      url "https://github.com/${github_repo}/releases/download/v${version}/${intel_name}"
      sha256 "${intel_sha}"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/${github_repo}/releases/download/v${version}/${linux_arm_name}"
      sha256 "${linux_arm_sha}"
    end

    on_intel do
      url "https://github.com/${github_repo}/releases/download/v${version}/${linux_intel_name}"
      sha256 "${linux_intel_sha}"
    end
  end

  def install
    bin.install "gitcomet"
  end

  test do
    assert_match "Usage", shell_output("#{bin}/gitcomet --help")
  end
end
EOF2

echo "Generated Homebrew formula: $out_path"
