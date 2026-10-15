#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
config_path="${1:-local/server.conf}"

cd "$project_root"

if [[ "$config_path" != /* ]]; then
    config_path="$project_root/$config_path"
fi

if [[ ! -f "$config_path" ]]; then
    printf 'Config file not found: %s\n' "$config_path" >&2
    exit 2
fi

exec cargo run --release -- "$config_path"
