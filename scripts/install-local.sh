#!/usr/bin/env sh
set -eu

# scripts/install-local.sh [--from-ci [--no-build]] [ref]
#
# Builds the loomtracer CLI from a given git ref and installs it to
# ~/.local/bin (override with LOOMTRACER_INSTALL_DIR) — the same location
# scripts/install.sh uses for published releases, and where Gas Town rigs
# resolve `loomtracer` on PATH ahead of ~/.cargo/bin.
#
# Unlike `make install` (which builds whatever is currently checked out),
# this takes an explicit ref and builds it in a disposable git worktree, so
# it never depends on — or disturbs — the state of the caller's checkout.
# Use it to install a specific commit (e.g. origin/master) between tagged
# releases; scripts/install.sh only installs published GitHub releases,
# which lag master by however long it's been since the last `v*` tag.
#
# The installed binary's `--version` embeds the exact commit built
# (<cargo semver>+<short sha>, via build.rs) so drift from what a ref
# actually contains is never silent (hivemind-zdsh.7).
#
# --from-ci installs the binary CI built for that commit instead of compiling
# it here (hivemind-b3my9): .github/workflows/edge.yml publishes it to the
# `edge` release of the repo (LOOMTRACER_REPO, default alexknips/loomtracer)
# once CI passed on the commit. The download is checked against the published
# sha256 and must report the commit it is named for before it replaces the
# installed binary. Only Linux x86_64 is built. When CI has no build for the
# commit (still running, failed, older than the newest 40) the script says so
# and builds locally as before; --no-build exits 3 there instead, for a caller
# that would rather retry later than compile. Anything else that goes wrong
# (an unreachable server, a checksum that does not match, a binary that will
# not run) stops the script: it is never papered over with a local build.

usage() {
  echo "usage: install-local.sh [--from-ci [--no-build]] [ref]" >&2
}

from_ci=0
no_build=0
ref=""
for arg in "$@"; do
  case "$arg" in
    --from-ci) from_ci=1 ;;
    --no-build) no_build=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      echo "unknown option: $arg" >&2
      usage
      exit 2
      ;;
    *)
      if [ -n "$ref" ]; then
        usage
        exit 2
      fi
      ref="$arg"
      ;;
  esac
done
if [ "$no_build" = 1 ] && [ "$from_ci" != 1 ]; then
  echo "--no-build only means something with --from-ci" >&2
  usage
  exit 2
fi

ref="${ref:-origin/master}"
install_dir="${LOOMTRACER_INSTALL_DIR:-$HOME/.local/bin}"
repo="${LOOMTRACER_REPO:-alexknips/loomtracer}"

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

case "$ref" in
  origin/*) git fetch origin --quiet ;;
esac

full_sha="$(git rev-parse "$ref")"
short_sha="$(git rev-parse --short=12 "$ref")"

worktree_dir=""
download_dir=""
cleanup() {
  if [ -n "$worktree_dir" ]; then
    git -C "$repo_root" worktree remove --force "$worktree_dir" >/dev/null 2>&1 || true
    rm -rf "$worktree_dir"
  fi
  if [ -n "$download_dir" ]; then
    rm -rf "$download_dir"
  fi
}
trap cleanup EXIT INT TERM

# fetch URL FILE: 0 when FILE now holds the body, 1 when the server has no such
# file (404: the one answer that means "CI has not published this"), 2 for any
# other answer or no answer.
fetch() {
  code="$(curl -sSL -o "$2" -w '%{http_code}' "$1")" || return 2
  case "$code" in
    200) return 0 ;;
    404) return 1 ;;
    *)
      echo "GET $1 answered HTTP $code" >&2
      return 2
      ;;
  esac
}

# Installs the CI-built binary for $short_sha: 0 when it did, 1 when CI has no
# build of it for this machine. It stops the script (exit) on anything else.
# Called from an `if`, where `set -e` does not apply: every step checks itself.
install_from_ci() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64 | Linux-amd64) ;;
    *)
      echo "CI builds Linux x86_64 only; there is no CI binary for $(uname -s) $(uname -m)." >&2
      return 1
      ;;
  esac

  asset="loomtracer-linux-x86_64-${short_sha}.tar.gz"
  base_url="https://github.com/${repo}/releases/download/edge"
  download_dir="$(mktemp -d)" || exit 1

  # The checksum is looked for first: CI publishes it after the tarball, so
  # when it is there the tarball is too.
  status=0
  fetch "${base_url}/${asset}.sha256" "${download_dir}/${asset}.sha256" || status=$?
  case "$status" in
    0) ;;
    1) return 1 ;;
    *) exit 1 ;;
  esac
  if ! fetch "${base_url}/${asset}" "${download_dir}/${asset}"; then
    echo "${asset}.sha256 is published but ${asset} could not be fetched." >&2
    exit 1
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$download_dir" && sha256sum -c "${asset}.sha256") >&2 || exit 1
  else
    expected="$(awk '{print $1}' "${download_dir}/${asset}.sha256")"
    actual="$(shasum -a 256 "${download_dir}/${asset}" | awk '{print $1}')"
    if [ "$actual" != "$expected" ]; then
      echo "checksum mismatch for ${asset}" >&2
      exit 1
    fi
  fi

  tar -xzf "${download_dir}/${asset}" -C "$download_dir" || exit 1
  version="$("${download_dir}/loomtracer" --version)" || {
    echo "the CI binary for ${short_sha} does not run on this host." >&2
    exit 1
  }
  case "$version" in
    *"+${short_sha}"*) ;;
    *)
      echo "the CI binary reports '${version}', not ${short_sha}; not installing it." >&2
      exit 1
      ;;
  esac

  mkdir -p "$install_dir" || exit 1
  install -m 0755 "${download_dir}/loomtracer" "${install_dir}/loomtracer" || exit 1
  echo "installed $("${install_dir}/loomtracer" --version) (CI-built, no local build)"
  echo "  -> ${install_dir}/loomtracer"
}

if [ "$from_ci" = 1 ]; then
  echo "Looking for the CI-built binary of ${ref} (${short_sha})..."
  if install_from_ci; then
    exit 0
  fi
  if [ "$no_build" = 1 ]; then
    echo "CI has no binary for ${short_sha} (is CI still running on it?); not building (--no-build)." >&2
    exit 3
  fi
  echo "CI has no binary for ${short_sha}; building it here instead."
fi

worktree_dir="$(mktemp -d)"

echo "Checking out ${ref} (${short_sha}) into a disposable worktree..."
git worktree add --detach --quiet "$worktree_dir" "$full_sha"

echo "Building (this can take a few minutes on a cold cache)..."
# Point at a target/ dir outside the disposable worktree so repeat runs —
# e.g. a nightly dog order re-running this against a new master commit —
# reuse cached dependency builds instead of recompiling from scratch every
# time. Cargo supports a target dir outside the source tree for exactly
# this reason.
build_target_dir="${LOOMTRACER_BUILD_TARGET_DIR:-$repo_root/target}"
(cd "$worktree_dir" && \
  CARGO_TARGET_DIR="$build_target_dir" \
  LOOMTRACER_BUILD_SHA="$full_sha" \
  cargo build --release --locked --bin loomtracer)

mkdir -p "$install_dir"
install -m 0755 "${build_target_dir}/release/loomtracer" "${install_dir}/loomtracer"

echo "installed $("${install_dir}/loomtracer" --version)"
echo "  -> ${install_dir}/loomtracer"
