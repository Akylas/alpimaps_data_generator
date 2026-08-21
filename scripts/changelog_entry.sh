#!/usr/bin/env bash
#
# Print one version's section of a standard-version CHANGELOG.
#
#   scripts/changelog_entry.sh <version> [changelog]
#
# Not a Keep-a-Changelog reader. standard-version writes its own heading shapes -
#
#   ## 0.1.0 (2026-08-20)                                  first release
#   ### [0.1.1](https://…/compare/v0.1.0...v0.1.1) (2026-08-20)   patch
#   ## [0.2.0](https://…/compare/v0.1.1...v0.2.0) (2026-08-21)    minor and major
#
# - none of which match the `## [0.1.1] - 2026-08-20` that Keep-a-Changelog tools look for. That
# mismatch is why the release workflow failed with a changelog it had itself just written.
set -euo pipefail

VERSION="${1:?usage: changelog_entry.sh <version> [changelog]}"
FILE="${2:-CHANGELOG.md}"
VERSION="${VERSION#v}"

[ -f "$FILE" ] || { echo "no $FILE" >&2; exit 1; }

entry="$(awk -v raw="$VERSION" '
  BEGIN {
    # the version goes into a regex, and its dots would otherwise match any character
    v = raw
    gsub(/\./, "\\.", v)
    start = "^#{2,3} +\\[?" v "\\]?([(\\[]|$| )"
    any   = "^#{2,3} +\\[?[0-9]+\\.[0-9]+\\.[0-9]+"
  }
  # the next version heading ends this section
  found && $0 ~ any { exit }
  $0 ~ start { found = 1; next }
  found { print }
' "$FILE")"

# trim blank lines top and bottom, so the release body does not open with two empty lines
entry="$(printf '%s' "$entry" | sed -e '/./,$!d' | sed -e :a -e '/^\n*$/{$d;N;};/\n$/ba')"

if [ -z "$entry" ]; then
  # Two different situations, and only one of them is a mistake.
  #
  # No heading at all means the version was never written down - a real failure, worth stopping
  # for. A heading with nothing under it means standard-version cut a version whose commits were
  # all of types it does not list (chore, docs, ci). That happened on the 0.1.2 run: the only
  # commit since v0.1.1 was the 0.1.1 release commit itself. An empty section is not a reason to
  # abandon a release, so it becomes a body that says so.
  if grep -Eq "^#{2,3} +\[?$(printf '%s' "$VERSION" | sed 's/\./\\./g')\]?([([]|$| )" "$FILE"; then
    printf 'No changelog entries for this version.\n' >&2
    printf 'No changelog entries - see the commits for what changed.\n'
    exit 0
  fi
  echo "no entry for $VERSION in $FILE" >&2
  exit 1
fi
printf '%s\n' "$entry"
