#!/usr/bin/env bash
set -euo pipefail

brew_prefixes=()
if [[ -n "${HOMEBREW_PREFIX:-}" ]]; then
  brew_prefixes+=("${HOMEBREW_PREFIX}")
fi
brew_prefixes+=("/opt/homebrew" "/usr/local")

protoc_bin=""
for prefix in "${brew_prefixes[@]}"; do
  candidate="${prefix}/Cellar/protobuf/33.2/bin/protoc-33.2.0"
  if [[ -x "${candidate}" ]]; then
    protoc_bin="${candidate}"
    break
  fi
done

if [[ -z "${protoc_bin}" ]]; then
  for prefix in "${brew_prefixes[@]}"; do
    candidate="${prefix}/bin/protoc"
    if [[ -x "${candidate}" ]]; then
      protoc_bin="${candidate}"
      break
    fi
  done
fi

if [[ -z "${protoc_bin}" ]]; then
  echo "protoc-wrapper: unable to locate protoc binary" >&2
  exit 1
fi

abseil_lib=""
for prefix in "${brew_prefixes[@]}"; do
  for dir in "${prefix}"/Cellar/abseil/*/lib; do
    if [[ -f "${dir}/libabsl_die_if_null.2508.0.0.dylib" ]]; then
      abseil_lib="${dir}"
      break 2
    fi
  done
done

if [[ -n "${abseil_lib}" ]]; then
  export DYLD_FALLBACK_LIBRARY_PATH="${abseil_lib}${DYLD_FALLBACK_LIBRARY_PATH:+:${DYLD_FALLBACK_LIBRARY_PATH}}"
fi

exec "${protoc_bin}" "$@"
