# Security policy

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub:
[Security → Report a vulnerability](https://github.com/Akylas/cairn/security/advisories/new),
or email dev@akylas.fr.

Include what you found, how to reproduce it, and the version (`cairn --version`, or the app's
about box). You will get an acknowledgement within a week, and we will agree a disclosure date
with you once the fix is understood.

## Scope

Cairn is a local tool: it runs builds on your machine, serves tiles on `127.0.0.1`, and
downloads data from public sources (Geofabrik, elevation mirrors, GitHub releases for the
planetiler jar). Things we want to hear about include:

* the local tile server being reachable from other hosts, or letting a web page read local files;
* path traversal or command injection through area names, polygons, options or pass-through
  planetiler flags;
* downloads that are not verified, or a build artifact that runs untrusted code;
* the release bundles carrying a library with a known vulnerability.

Free-form planetiler flags are passed through verbatim by design; running a flag you do not
trust is the same as running any command you do not trust.

## Supported versions

Only the latest release receives fixes while the project is pre-1.0.
