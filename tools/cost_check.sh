#!/bin/sh
# cost_check.sh - zero-cost gate.
#
# SheikhEmu must stay 100% free: no paid services, no accounts, no ads,
# no analytics, no tracking. This gate fails when:
#   - a crate declares an external dependency (path deps are internal
#     and allowed), or
#   - code references a paid/analytics/tracking service.
#
# Exit status: 0 = pass, 1 = violations found.

set -u
cd "$(dirname "$0")/.."

status=0

echo "== cost gate =="

# 1. No external crate dependencies outside the D-27/D-40 whitelist.
#
#    D-27 (owner-approved 2026-11-13): the only external crates of
#    the whole project were `winit` and `softbuffer`, allowed ONLY in
#    crates/se-frontend. D-40/M4.3 (owner-approved 2026-11-14) added
#    exactly ONE more name to that same single manifest: `cpal` (the
#    audio output backend, owner pre-registered in principle by D-27
#    and activated by the approved M4 plan). The explicit frontend
#    statement is therefore {winit, softbuffer, cpal} and nothing
#    else. Every other crate stays stdlib-only, and even se-frontend
#    may not declare anything else external. Path dependencies are
#    internal and always allowed.
frontend_whitelist="winit softbuffer cpal"

for manifest in crates/*/Cargo.toml; do
  case "$manifest" in
    crates/se-frontend/Cargo.toml) whitelist="$frontend_whitelist" ;;
    *) whitelist="" ;;
  esac
  section=""
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "[dependencies]") section="deps" ;;
      \[*) section="other" ;;
      *)
        trimmed=$(printf '%s' "$line" | tr -d '[:space:]')
        if [ "$section" = "deps" ] && [ -n "$trimmed" ]; then
          case "$trimmed" in
            *path=*) : ;; # internal workspace dependency: allowed
            *)
              name=${trimmed%%=*}
              allowed=0
              for ok in $whitelist; do
                [ "$name" = "$ok" ] && allowed=1
              done
              if [ "$allowed" -eq 0 ]; then
                echo "FAIL: external dependency in $manifest: $line"
                status=1
              fi
              ;;
          esac
        fi
        ;;
    esac
  done < "$manifest"
done

# 2. No paid / analytics / tracking references in code.
if grep -rinE '(api\.stripe\.com|firebase|google-analytics|gtag\(|adsense|admob|crashlytics|sentry\.io|applovin|appodeal|unity-ads)' \
    crates/ tools/ --exclude='cost_check.sh' >/dev/null 2>&1; then
  echo "FAIL: paid/analytics/tracking reference found:"
  grep -rinE '(api\.stripe\.com|firebase|google-analytics|gtag\(|adsense|admob|crashlytics|sentry\.io|applovin|appodeal|unity-ads)' \
    crates/ tools/ --exclude='cost_check.sh' || true
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "PASS: no external dependencies, no paid or tracking services"
fi
exit "$status"
