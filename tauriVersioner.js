// standard-version updater for the Studio's Cargo manifest.
//
// Adapted from gps-mocker-rs. One difference matters: this app is a Cargo *workspace*, so the
// version lives under `[workspace.package]` and the crates inherit it with
// `version.workspace = true`. Writing `[package].version` instead would bump a key nothing
// reads, and every build would keep shipping the old number.
const TOML = require('@tauri-apps/toml');

function holder(parsed) {
  if (parsed.workspace && parsed.workspace.package && 'version' in parsed.workspace.package) {
    return parsed.workspace.package;
  }
  if (parsed.package && 'version' in parsed.package) {
    return parsed.package;
  }
  throw new Error('no [workspace.package].version or [package].version to read');
}

module.exports.readVersion = function (contents) {
  return holder(TOML.parse(contents)).version;
};

module.exports.writeVersion = function (contents, version) {
  const parsed = TOML.parse(contents);
  holder(parsed).version = version;
  return TOML.stringify(parsed);
};
