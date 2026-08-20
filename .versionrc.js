// The version lives in three files that have to agree: this package.json (which standard-version
// reads as the current version), the Cargo workspace (which every crate inherits), and
// tauri.conf.json (which names the bundles). All three are bumped together so a release cannot
// ship a binary whose reported version differs from its filename.
//
// `packageFiles` is left as the default package.json rather than the Cargo manifest, so the
// current version is read from a file standard-version understands natively.
//
// A run logs "Unable to obtain updater ... Unsupported file (Cargo.toml)" before bumping it
// anyway. That is standard-version trying its extension-based resolution first and only then
// the custom updater; the bump does happen. Noise, not failure.
const cargo = {
  filename: './cairn/Cargo.toml',
  updater: require('./tauriVersioner')
};

// tauri.conf.json has a plain top-level `version`, which the built-in JSON updater handles
const tauriConf = { filename: './cairn/src-tauri/tauri.conf.json', type: 'json' };

module.exports = {
  bumpFiles: ['package.json', cargo, tauriConf],
  types: [
    { type: 'feat', section: 'Features' },
    { type: 'fix', section: 'Bug Fixes' },
    { type: 'perf', section: 'Performance' },
    { type: 'refactor', section: 'Refactoring' },
    { type: 'docs', hidden: true },
    { type: 'test', hidden: true },
    { type: 'chore', hidden: true }
  ]
};
