// standard-version updater for Cargo.lock.
//
// The workspace version is bumped in Cargo.toml, but the lockfile records it too - once per
// workspace member - and nothing else rewrites it, so every release left a lockfile that
// disagreed with the manifests and failed a `--locked` build.
//
// Edited as text rather than parsed and re-stringified: a lockfile is ~5000 lines of
// machine-formatted TOML, and a round trip through a serializer reorders and reflows it.
const MEMBERS = ['cairn', 'cairn-core', 'cairn-cli'];

// `[[package]]` / `name = "cairn"` / `version = "x"`: the version is the line after the name
const entry = (name) => new RegExp(`(\\[\\[package\\]\\]\\nname = "${name}"\\nversion = ")([^"]+)(")`);

module.exports.readVersion = function (contents) {
  const match = contents.match(entry(MEMBERS[0]));
  if (!match) throw new Error(`no [[package]] named "${MEMBERS[0]}" in Cargo.lock`);
  return match[2];
};

module.exports.writeVersion = function (contents, version) {
  for (const name of MEMBERS) {
    const re = entry(name);
    if (!re.test(contents)) throw new Error(`no [[package]] named "${name}" in Cargo.lock`);
    contents = contents.replace(re, `$1${version}$3`);
  }
  return contents;
};
