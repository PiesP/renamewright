#!/usr/bin/env bash
set -euo pipefail

readonly OUTPUTS=(
  quality
  unit
  e2e
  performance
  windows
  cargo_policy
  osv
  semgrep
  codeql_actions
  codeql_rust
)

declare -A selected=()
for output in "${OUTPUTS[@]}"; do
  selected["$output"]=false
done

select_all() {
  for output in "${OUTPUTS[@]}"; do
    selected["$output"]=true
  done
}

select_product_checks() {
  selected[quality]=true
  selected[unit]=true
  selected[e2e]=true
  selected[performance]=true
  selected[windows]=true
  selected[semgrep]=true
  selected[codeql_rust]=true
}

classify_path() {
  local path=$1

  case "$path" in
    Cargo.toml|Cargo.lock|rust-toolchain.toml|deny.toml|.cargo/*)
      select_all
      ;;
    crates/*)
      select_product_checks
      if [[ "$path" == */Cargo.toml ]]; then
        selected[cargo_policy]=true
        selected[osv]=true
      fi
      ;;
    scripts/*|.github/workflows/*)
      select_all
      ;;
    .github/actions/*)
      selected[quality]=true
      selected[windows]=true
      selected[semgrep]=true
      selected[codeql_actions]=true
      ;;
    .github/codex-security/*|.github/SECURITY.md|SECURITY.md|.github/settings.yaml|.githooks/*)
      selected[quality]=true
      selected[semgrep]=true
      ;;
    *.md|docs/*|LICENSE*|.gitignore|.github/ISSUE_TEMPLATE/*|.github/pull_request_template.md)
      # Documentation can still contain credentials, so keep secrets scanning active.
      selected[semgrep]=true
      ;;
    *.png|*.jpg|*.jpeg|*.gif|*.webp|*.ico)
      ;;
    *)
      # New or unclassified project inputs are treated as product-wide changes.
      select_all
      ;;
  esac
}

collect_changed_paths() {
  local event_name=${GITHUB_EVENT_NAME:-}
  local base_sha=${WORKFLOW_BASE_SHA:-}
  local head_sha=${WORKFLOW_HEAD_SHA:-${GITHUB_SHA:-}}

  if [[ "$event_name" == "workflow_dispatch" || "$event_name" == "schedule" ]]; then
    select_all
    return
  fi

  if [[ -z "$base_sha" || -z "$head_sha" || "$base_sha" =~ ^0+$ ]]; then
    select_all
    return
  fi

  local paths_file
  paths_file=$(mktemp)
  if ! git diff --name-only -z "$base_sha" "$head_sha" > "$paths_file"; then
    rm -f "$paths_file"
    select_all
    return
  fi

  local changed=false
  while IFS= read -r -d '' path; do
    changed=true
    classify_path "$path"
  done < "$paths_file"
  rm -f "$paths_file"

  if [[ "$changed" == false ]]; then
    select_all
  fi
}

if (( $# > 0 )); then
  for path in "$@"; do
    classify_path "$path"
  done
else
  collect_changed_paths
fi

destination=${GITHUB_OUTPUT:-/dev/stdout}
for output in "${OUTPUTS[@]}"; do
  printf '%s=%s\n' "$output" "${selected[$output]}" >> "$destination"
done
