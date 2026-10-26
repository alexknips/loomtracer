#!/usr/bin/env sh
set -eu

# scripts/cell-update.sh [--from-ci [--no-build]] [ref]
#
# Updates a self-hosted Loomtracer cell (docker-compose.yml) to a given git
# ref by building the server image from source and tagging it with the
# commit sha — never `:latest`, which only moves on a tagged release (see
# .github/workflows/release.yml) and can silently lag master for weeks
# (hivemind-zdsh.7). Health-checks the new container before declaring
# success and prints the sha `/v1/version` reports back, so a caller can
# confirm the cell is actually running what it was just built from.
#
# The sha-tagged image is also pinned as LOOMTRACER_IMAGE in the compose
# project's env file (default: `.env` next to the first compose file). That
# file is what anything else reconciling the cell with `docker compose up -d`
# — a systemd timer, a cron job, you next week — reads, so the update
# survives it instead of being recreated back onto `:latest`
# (hivemind-2mgj). The pin is written before the restart, so no reconciler
# tick can slip in between, and it is kept only if the new container comes
# up healthy: on any failure the env file is put back exactly as it was, and
# a reconciler rolls the cell back to the previous image.
#
# --from-ci pulls the image CI built for the commit instead of compiling it
# here (hivemind-b3my9): .github/workflows/edge.yml pushes
# ghcr.io/<repo>:<sha> once CI passed on the commit and publishes the image's
# digest on the repo's `edge` release. The digest read from there is what is
# pulled and what is pinned (`ghcr.io/<repo>:<sha>@sha256:...`), so the cell
# runs exactly the image CI recorded; the /v1/version check below still has to
# report the sha. When CI has no image for the commit (still running, failed,
# older than the newest 40) the script says so and builds locally as before;
# --no-build exits 3 there instead, for a caller that would rather retry later
# than compile. Anything else that goes wrong (an unreachable server, a digest
# that is not one, a pull that fails) stops the script before it touches the
# env file: it is never papered over with a local build.
#
# NOT wired to run automatically on every merge: that's an explicit
# non-goal (see hivemind-zdsh.7's DO/NOT). This is the mechanism a
# scheduled dog order or a human invokes deliberately.
#
# Environment:
#   LOOMTRACER_REPO          GitHub repo whose CI-built image --from-ci pulls
#                          (default: alexknips/loomtracer).
#   LOOMTRACER_COMPOSE_FILE  compose file(s), colon-separated like COMPOSE_FILE
#                          (default: docker-compose.yml). Pass every file the
#                          cell is reconciled with, override files included.
#   LOOMTRACER_ENV_FILE      env file to pin LOOMTRACER_IMAGE in and hand to
#                          compose (default: `.env` next to the first compose
#                          file, which is where compose looks by itself).

usage() {
  echo "usage: cell-update.sh [--from-ci [--no-build]] [ref]" >&2
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
repo="${LOOMTRACER_REPO:-alexknips/loomtracer}"
compose_files="${LOOMTRACER_COMPOSE_FILE:-docker-compose.yml}"
first_compose_file="${compose_files%%:*}"
env_file="${LOOMTRACER_ENV_FILE:-$(dirname "$first_compose_file")/.env}"
port="${LOOMTRACER_PORT:-8080}"
health_url="${LOOMTRACER_HEALTH_URL:-http://localhost:${port}/v1/health}"
version_url="${LOOMTRACER_VERSION_URL:-http://localhost:${port}/v1/version}"
health_retries="${LOOMTRACER_HEALTH_RETRIES:-30}"

# The positional parameters become the `-f` list from here on; $ref is read.
set --
old_ifs="$IFS"
IFS=:
for compose_file in $compose_files; do
  [ -n "$compose_file" ] || continue
  set -- "$@" -f "$compose_file"
done
IFS="$old_ifs"

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

case "$ref" in
  origin/*) git fetch origin --quiet ;;
esac

full_sha="$(git rev-parse "$ref")"
short_sha="$(git rev-parse --short=12 "$ref")"
image="loomtracer:${short_sha}"

# The env file as it was before this run, so a failed update can put it back
# byte for byte; and whether it existed at all, so a file this script created
# is removed again rather than left behind empty.
env_backup="$(mktemp)"
env_new="$(mktemp)"
env_existed=0
[ ! -f "$env_file" ] || env_existed=1
pinned=0
verified=0

restore_env_file() {
  if [ "$env_existed" = 1 ]; then
    cat "$env_backup" > "$env_file"
  else
    rm -f "$env_file"
  fi
}

finish() {
  status=$?
  trap - EXIT
  if [ "$pinned" = 1 ] && [ "$verified" != 1 ]; then
    restore_env_file
    echo "restored ${env_file} to its state before this update." >&2
  fi
  rm -f "$env_backup" "$env_new"
  exit "$status"
}
trap finish EXIT
trap 'exit 1' HUP INT TERM

# Sets $image to the CI-built image of $short_sha, pulled by the digest CI
# published: 0 when it did, 1 when CI has no image of it. It stops the script
# (exit) on anything else, and nothing it does touches the env file.
pull_ci_image() {
  base_url="https://github.com/${repo}/releases/download/edge"
  digest_file="$(mktemp)" || exit 1

  # fetch: 200 -> the body is in the file; 404 -> CI has not published this
  # (the one answer that means "build it here"); anything else -> stop.
  code="$(curl -sSL -o "$digest_file" -w '%{http_code}' \
    "${base_url}/loomtracer-cell-image-${short_sha}.digest")" || {
    rm -f "$digest_file"
    exit 1
  }
  case "$code" in
    200) ;;
    404)
      rm -f "$digest_file"
      return 1
      ;;
    *)
      echo "GET ${base_url}/loomtracer-cell-image-${short_sha}.digest answered HTTP ${code}" >&2
      rm -f "$digest_file"
      exit 1
      ;;
  esac

  digest="$(tr -d ' \t\r\n' < "$digest_file")"
  rm -f "$digest_file"
  case "$digest" in
    sha256:????????????????????????????????????????????????????????????????) ;;
    *)
      echo "the published digest for ${short_sha} is not a sha256 digest: '${digest}'" >&2
      exit 1
      ;;
  esac

  ci_image="ghcr.io/$(printf '%s' "$repo" | tr '[:upper:]' '[:lower:]'):${short_sha}@${digest}"
  echo "Pulling the CI-built ${ci_image}..."
  docker pull "$ci_image" || {
    echo "could not pull ${ci_image}; nothing was changed." >&2
    exit 1
  }
  image="$ci_image"
}

built_by="built locally"
ci_image_pulled=0
if [ "$from_ci" = 1 ]; then
  echo "Looking for the CI-built image of ${ref} (${short_sha})..."
  # Not `if pull_ci_image`: set -e does not apply inside a function called as a
  # condition, and the pull's own checks above must stop the script.
  pull_status=0
  pull_ci_image || pull_status=$?
  if [ "$pull_status" = 0 ]; then
    ci_image_pulled=1
    built_by="CI-built"
  elif [ "$no_build" = 1 ]; then
    echo "CI has no image for ${short_sha} (is CI still running on it?); not building (--no-build)." >&2
    exit 3
  else
    echo "CI has no image for ${short_sha}; building it here instead."
  fi
fi

if [ "$ci_image_pulled" != 1 ]; then
  echo "Building ${image} from ${ref} (${full_sha})..."
  docker build --build-arg "GIT_SHA=${full_sha}" -t "$image" -f Dockerfile "$repo_root"
fi

echo "Pinning LOOMTRACER_IMAGE=${image} in ${env_file}..."
[ "$env_existed" != 1 ] || cp "$env_file" "$env_backup"
awk -v line="LOOMTRACER_IMAGE=${image}" '
  /^[ \t]*(export[ \t]+)?LOOMTRACER_IMAGE[ \t]*=/ { if (!seen) print line; seen = 1; next }
  { print }
  END { if (!seen) print line }
' "$env_backup" > "$env_new"
pinned=1
# Written in place, not renamed over: an existing env file keeps its mode
# and owner (the umask only shapes a file created here, which holds the
# cell's secrets and so starts owner-only).
(umask 077 && cat "$env_new" > "$env_file")

echo "Restarting cell with ${image}..."
LOOMTRACER_IMAGE="$image" docker compose "$@" --env-file "$env_file" up -d --no-deps loomtracer

echo "Waiting for ${health_url} to report healthy..."
attempt=0
until curl -fsS "$health_url" >/dev/null 2>&1; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge "$health_retries" ]; then
    echo "cell did not become healthy after ${health_retries} attempts." >&2
    echo "leaving the container running for inspection — it was NOT rolled back;" >&2
    echo "anything reconciling with 'docker compose up -d' will move it back to the previous image." >&2
    echo "check logs with: docker compose $* --env-file ${env_file} logs loomtracer" >&2
    exit 1
  fi
  sleep 2
done

deployed_version="$(curl -fsS "$version_url")"
echo "Cell healthy."
echo "  built:    ${short_sha} (${built_by})"
echo "  deployed: ${deployed_version}"

case "$deployed_version" in
  *"$short_sha"*) verified=1 ;;
  *)
    echo "WARNING: /v1/version does not mention ${short_sha} — the running" >&2
    echo "container may not be the one just built. Investigate before trusting it." >&2
    exit 1
    ;;
esac

echo "  pinned:   LOOMTRACER_IMAGE=${image} in ${env_file}"
