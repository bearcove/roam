#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail=false

check_forbidden_outside_allowlist() {
  local pattern="$1"
  local label="$2"
  shift 2
  local allow_files=("$@")

  local matches
  matches="$(rg -n "$pattern" Sources Tests || true)"
  if [[ -z "$matches" ]]; then
    return
  fi

  local disallowed=""
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    local file="${line%%:*}"
    local allowed=false
    for allow in "${allow_files[@]}"; do
      if [[ "$file" == "$allow" ]]; then
        allowed=true
        break
      fi
    done
    if [[ "$allowed" == false ]]; then
      disallowed+="$line"$'\n'
    fi
  done <<< "$matches"

  if [[ -n "$disallowed" ]]; then
    echo "hardening check failed: disallowed $label found outside allowlist"
    echo "$disallowed"
    fail=true
  fi
}

check_forbidden_outside_allowlist \
  '@preconcurrency import' \
  '@preconcurrency import' \
  'Sources/TelexRuntime/Transport.swift' \
  'Tests/TelexRuntimeTests/TransportTests.swift'

check_forbidden_outside_allowlist \
  '@unchecked Sendable' \
  '@unchecked Sendable' \
  'Sources/TelexRuntime/Binding.swift' \
  'Sources/TelexRuntime/Channel.swift' \
  'Sources/TelexRuntime/Driver.swift' \
  'Sources/TelexRuntime/ShmBipBuffer.swift' \
  'Sources/TelexRuntime/ShmGuest.swift' \
  'Sources/TelexRuntime/ShmRegion.swift' \
  'Sources/TelexRuntime/ShmTransport.swift' \
  'Sources/TelexRuntime/Transport.swift' \
  'Sources/shm-guest-client/main.swift'

check_forbidden_outside_allowlist \
  '(^|[^[:alnum:]_])(Unmanaged|Unsafe[A-Za-z0-9_]*|withUnsafe[A-Za-z0-9_]*)' \
  'unsafe APIs' \
  'Sources/TelexRuntime/Postcard.swift' \
  'Sources/TelexRuntime/ShmAtomics.swift' \
  'Sources/TelexRuntime/ShmBipBuffer.swift' \
  'Sources/TelexRuntime/ShmBootstrap.swift' \
  'Sources/TelexRuntime/ShmGuest.swift' \
  'Sources/TelexRuntime/ShmRegion.swift' \
  'Sources/TelexRuntime/ShmTransport.swift' \
  'Tests/TelexRuntimeTests/ShmBootstrapTests.swift' \
  'Tests/TelexRuntimeTests/ShmGuestRuntimeTests.swift'

if [[ "$fail" == true ]]; then
  exit 1
fi

echo "hardening check passed"
