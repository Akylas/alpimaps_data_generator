# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

### [0.1.2](https://github.com/Akylas/cairn/compare/v0.1.1...v0.1.2) (2026-10-12)


### Features

* **brand:** add the Cairn icon and logo ([3043e9d](https://github.com/Akylas/cairn/commit/3043e9d40a9dccce669b441c9483d124c3dbf638))
* **cairn:** bathymap step, PMTiles output, remote terrain, concurrent runs ([6b0ec94](https://github.com/Akylas/cairn/commit/6b0ec94a9eb4375f9b3bdae7fdfafc951af0ae09))
* **cairn:** clip to the area's own boundary, fetching it when absent ([f2ad95c](https://github.com/Akylas/cairn/commit/f2ad95ce3ecbe5adc62b84cce9fb7259185cd648))
* **cairn:** expose --drop_duplicate_names ([6d8484d](https://github.com/Akylas/cairn/commit/6d8484de33196cff20599559208c5aa7524ca063)), closes [farfromrefug/planetiler-openmaptiles#10](https://github.com/farfromrefug/planetiler-openmaptiles/issues/10)
* **cairn:** expose --poi_custom_ranks, on in the measured preset ([0c6226e](https://github.com/Akylas/cairn/commit/0c6226e3205f69e27f5d9fe7b4be745b1a71905c))
* **cairn:** expose --poi_landmarks and --poi_guideposts ([7b8752a](https://github.com/Akylas/cairn/commit/7b8752aff4b99c440d83110c339f84e8be647bb7))
* **cairn:** expose --poi_trees ([2f0a7f7](https://github.com/Akylas/cairn/commit/2f0a7f793edecc109a9db8871383fb974ea3f0ca))
* **cairn:** expose --power_lines ([5f61fe3](https://github.com/Akylas/cairn/commit/5f61fe3aafc44135464dee9e6e03baffa354dc1e))
* **cairn:** expose the new planetiler flags, drop three that no longer exist ([36ccd55](https://github.com/Akylas/cairn/commit/36ccd55ee0ee245271338d9c78c989a466a2f760))
* **cairn:** make the measured presets mirror the README build commands ([7d1ac4b](https://github.com/Akylas/cairn/commit/7d1ac4be1103c04fef2a4d04d060256a5c1d54db))
* **cairn:** reproduce the README builds by default, for every step ([56ecee2](https://github.com/Akylas/cairn/commit/56ecee22eeafbe97cc88ac0e649261217f047942))
* **cairn:** stronger section headers in the output tab, and a tested delete ([1c97b0a](https://github.com/Akylas/cairn/commit/1c97b0ae01b28b7666f4df252480b24e25592e33))
* **ci:** build Cairn without releasing it ([5d06a4a](https://github.com/Akylas/cairn/commit/5d06a4a63a6143c4ebe0d0c6aef15536b0d33d98))
* **site:** add the project website ([161f1c6](https://github.com/Akylas/cairn/commit/161f1c63cfd392b058d3fd4abd19fe690a72e508))


### Bug Fixes

* **bundle:** carry Linux libraries in the app package too, and stop the CLI overwriting the app ([2be5d3d](https://github.com/Akylas/cairn/commit/2be5d3d589213daaa17c2aad0ab2d1ccd801553b))
* **cairn:** a run that does nothing, and progress nobody can see ([6da3307](https://github.com/Akylas/cairn/commit/6da33071dd86f31c4222a9a1a743949231846411))
* **cairn:** show the defaults in the form, and fold hillshade into terrain ([a67582e](https://github.com/Akylas/cairn/commit/a67582efa023113718c051926b2506638c72063a))
* **cairn:** the app had no Tauri capability, so no event ever reached it ([1b5892d](https://github.com/Akylas/cairn/commit/1b5892d8c02382ae2dab3219325223f9943c5e75))
* **cairn:** the app ran a three-week-old jar and never said so ([af24e84](https://github.com/Akylas/cairn/commit/af24e84ac8b02542c919d6b84a20ab5b707bbc81))
* **ci:** boost new enough for Valhalla, and scripts that run on the bash macOS has ([2615849](https://github.com/Akylas/cairn/commit/261584917f0fe5561b28d5ae9fcba23b2bc77148))
* **ci:** build Linux on 22.04 so one build runs on 22.04 and 24.04 ([fa4e7d6](https://github.com/Akylas/cairn/commit/fa4e7d651d5a7fd9fc497482479b3e80557e6c16))
* **ci:** build protobuf too, and finish the Valhalla build on 22.04 ([e2d4737](https://github.com/Akylas/cairn/commit/e2d473736453d384995b68fea01b59407a5eb904))
* **ci:** give Valhalla a toolchain and the tools it actually checks for ([2938ad8](https://github.com/Akylas/cairn/commit/2938ad8bf6c22fce661f620818f0954a6873fadc))
* commit the routing module, which .gitignore was hiding ([1a3ed5e](https://github.com/Akylas/cairn/commit/1a3ed5e775e593a5da86ba333d7d6c66200754f1))
* link the libraries CMake names, whatever their suffix ([0db9618](https://github.com/Akylas/cairn/commit/0db9618f5dc5327ad3b5c98a9b2176fb22ebbdca))
* **release:** read the changelog standard-version actually writes ([93bfc3a](https://github.com/Akylas/cairn/commit/93bfc3a5b4859b54f2f65a3caa64999ea6927148))
* **release:** stop cutting versions that have nothing in them ([c2dc378](https://github.com/Akylas/cairn/commit/c2dc378225a939c2fbe2d1e2b8391fcc3475f7c0))


### Performance

* **ci:** build the two Valhalla targets that ship, not all 26 ([a494112](https://github.com/Akylas/cairn/commit/a4941124ddbc939095cb5cceb22c1260eb641ad3))
* **ci:** cache so that unchanged dependencies are not rebuilt ([af85060](https://github.com/Akylas/cairn/commit/af85060eaf0aeb4de7904e4685b58fd60b6e9a77))


### Refactoring

* flatten the repo for the Cairn release ([4cacde6](https://github.com/Akylas/cairn/commit/4cacde64e9d2ac8989f2e6026266bcc3fdce48e8))

### 0.1.1 (2026-08-20)


### Features

* **cli:** run every build step from a terminal ([d251eac](https://github.com/Akylas/alpimaps_data_generator/commit/d251eac245cd76bf545f67091406f3551318b71f))
* **release:** ship the CLI on its own alongside the app ([21e5ab7](https://github.com/Akylas/alpimaps_data_generator/commit/21e5ab7e9c32667efcc9ea6f41fc22f0c406207a))
* rename to Cairn, and split the jar into its own release ([a0b6ba1](https://github.com/Akylas/alpimaps_data_generator/commit/a0b6ba197a4b91567c14df4081e2671250e99373))
* **studio:** a CLI that replaces the scripts, not one that drives the app ([bdfccd6](https://github.com/Akylas/alpimaps_data_generator/commit/bdfccd6169c6021bbe550875b241657ff818a2cf))
* **studio:** design pass over the whole app ([f37d466](https://github.com/Akylas/alpimaps_data_generator/commit/f37d4664f7d24d988f7987d954f07bc6bce7ed54))
* **studio:** desktop app for the tile pipeline ([ededc38](https://github.com/Akylas/alpimaps_data_generator/commit/ededc38fac22682f7a3d4500a986a7a4eef20f0e))
* **studio:** explain steps and modes from one source, and package the tools ([83e865b](https://github.com/Akylas/alpimaps_data_generator/commit/83e865b820b2f22d081725bf0b8f541da8b12c74))
* **studio:** make the bundle self-contained, and prove it by building one ([0d213a8](https://github.com/Akylas/alpimaps_data_generator/commit/0d213a8d15e0483236d9c377a8d52c90ab2c7cad))
* **studio:** read the in-app CLI reference from the binary ([2f85d7b](https://github.com/Akylas/alpimaps_data_generator/commit/2f85d7b12cd8b4350edf9c4624aabe604d21eb85))
* **studio:** rework the map view and make build steps legible ([cc95c96](https://github.com/Akylas/alpimaps_data_generator/commit/cc95c960d88bd43adca5928dac22ca09a6736be4))
* **studio:** skip steps whose output is already there ([eec52c8](https://github.com/Akylas/alpimaps_data_generator/commit/eec52c83da8988835d909f24ca77c17536a5b831))
* **studio:** wire every step, find the jar, document both front ends ([4da6732](https://github.com/Akylas/alpimaps_data_generator/commit/4da6732043dd6fef94cf06b1a59d7f4a4f8d8178))


### Bug Fixes

* **studio:** configurable Valhalla config, per-step options, routing package picker ([fa93ba3](https://github.com/Akylas/alpimaps_data_generator/commit/fa93ba39385b8fe06e2cc9f63543ec7798e0046f))
* **studio:** fetch the elevation tiles the graph and the terrain need ([4384372](https://github.com/Akylas/alpimaps_data_generator/commit/438437231c8dd1a03cb6f0a7f620cf39a823d956))
* **studio:** give the map view the whole window and clarify compare ([bed83c6](https://github.com/Akylas/alpimaps_data_generator/commit/bed83c68b7364661190e907a5b980d81fbf5851d))
* **studio:** open on Output again, not the tab left behind while testing ([d7b03c9](https://github.com/Akylas/alpimaps_data_generator/commit/d7b03c94499f37d762b6bda189038c3664f6ac20))
* **studio:** stop blocking the UI thread, and read the docs like the CLI ([56b440c](https://github.com/Akylas/alpimaps_data_generator/commit/56b440c0784081df56ce78d0fc1537f6299c1470))
* use our own tippecanoe with optimized mbtiles ([f888212](https://github.com/Akylas/alpimaps_data_generator/commit/f88821220e9d2be0aa4bb6b757297b1d194ccf47))


### Performance

* **studio:** 216 MB bundle down to 156 MB, and a check that runs it ([fea4eaf](https://github.com/Akylas/alpimaps_data_generator/commit/fea4eaf30e8a25bac57368318e61380d320ca9e1))
