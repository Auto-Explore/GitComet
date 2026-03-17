#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/generate-homebrew-formula.sh \
  --version VERSION \
  --github-repo OWNER/REPO \
  --arm-tar PATH \
  --intel-tar PATH \
  --linux-tar PATH \
  --output PATH

Generates a Homebrew formula for GitComet from macOS + Linux tarball artifacts.
USAGE
}

version=""
github_repo=""
arm_tar=""
intel_tar=""
linux_tar=""
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
    --linux-tar)
      linux_tar="${2:-}"
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

if [[ -z "$version" || -z "$github_repo" || -z "$arm_tar" || -z "$intel_tar" || -z "$linux_tar" || -z "$out_path" ]]; then
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

if [[ ! -f "$linux_tar" ]]; then
  echo "linux tarball not found: $linux_tar" >&2
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
linux_sha="$(sha256_file "$linux_tar")"
arm_name="$(basename "$arm_tar")"
intel_name="$(basename "$intel_tar")"
linux_name="$(basename "$linux_tar")"

mkdir -p "$(dirname "$out_path")"

cat > "$out_path" <<EOF2
class Gitcomet < Formula
  desc "Fast, resource-efficient Git GUI written in Rust"
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
    on_intel do
      url "https://github.com/${github_repo}/releases/download/v${version}/${linux_name}"
      sha256 "${linux_sha}"
    end
  end

  def install
    if OS.mac?
      libexec.install "GitComet.app"
      bin.install_symlink libexec/"GitComet.app/Contents/MacOS/gitcomet"
    else
      bin.install "gitcomet"
    end
  end

  def post_install
    return unless OS.mac?

    target_app = libexec/"GitComet.app"
    return unless target_app.exist?

    apps_dir = Pathname.new(File.expand_path("~/Applications"))
    apps_dir.mkpath unless apps_dir.exist?

    launcher_link = apps_dir/"GitComet.app"
    if launcher_link.exist? && !launcher_link.symlink?
      opoo "Skipping ~/Applications/GitComet.app link because a non-symlink path already exists."
      return
    end

    launcher_link.unlink if launcher_link.symlink?
    launcher_link.make_symlink(target_app)
  rescue StandardError => e
    opoo "Failed to link GitComet.app into ~/Applications: #{e}"
  end

  test do
    assert_match "Usage", shell_output("#{bin}/gitcomet --help")
  end
end
EOF2

echo "Generated Homebrew formula: $out_path"
