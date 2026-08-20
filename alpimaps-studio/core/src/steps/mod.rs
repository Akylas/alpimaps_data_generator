pub mod download;
pub mod external;
pub mod options;
pub mod planetiler;
pub mod state;

use serde::{Deserialize, Serialize};

/// Identifies a step in the build graph. Only the two planetiler steps exist in the spike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepId {
    DownloadOsm,
    ElevationTiles,
    Basemap,
    Routes,
    TerrainRgb,
    Hillshade,
    ValhallaTiles,
    ValhallaPackage,
}

/// Every step, in a stable order for listing.
pub const ALL_STEPS: [StepId; 8] = [
    StepId::DownloadOsm,
    StepId::ElevationTiles,
    StepId::Basemap,
    StepId::Routes,
    StepId::TerrainRgb,
    StepId::Hillshade,
    StepId::ValhallaTiles,
    StepId::ValhallaPackage,
];

impl StepId {
    pub fn label(self) -> &'static str {
        match self {
            StepId::Basemap => "Basemap",
            StepId::Routes => "Routes",
            StepId::TerrainRgb => "Terrain RGB",
            StepId::Hillshade => "Hillshade",
            StepId::ValhallaTiles => "Valhalla tiles",
            StepId::ValhallaPackage => "Valhalla package",
            StepId::DownloadOsm => "Download OSM",
            StepId::ElevationTiles => "Elevation tiles",
        }
    }

    /// What the step does, in the words someone debugging a build would want.
    ///
    /// The prose lives next to the graph rather than in the UI: the app, the docs tab and any
    /// later front end all read it from here, so there is one description of a step and not
    /// three that drift apart.
    pub fn summary(self) -> &'static str {
        match self {
            StepId::DownloadOsm => {
                "Fetches the area's OSM extract, resolved through Geofabrik's index rather than \
                 a guessed URL - their ids are what the area name has to match (`rhone-alpes`, \
                 `france`). One copy feeds the basemap, the routes and the Valhalla graph, which \
                 is why it is a step of its own rather than planetiler's `--download`."
            }
            StepId::ElevationTiles => {
                "Runs `valhalla_build_elevation` with `-d`, so the .hgt tiles land decompressed. \
                 The Valhalla graph bakes elevation in during its own build, so these have to \
                 exist before the tiles are built - and the terrain step reads the same files \
                 afterwards."
            }
            StepId::Basemap => {
                "Planetiler over the OSM extract, with the bundled OpenMapTiles fork or a YAML \
                 schema. The top zoom dominates the size: z14 is about 71% of a rhone-alpes \
                 basemap."
            }
            StepId::Routes => {
                "The same planetiler run restricted to the route layer - hiking and cycling \
                 relations, which is why it comes out a fraction of the basemap's size. It is \
                 separate because the mobile app ships and updates it separately."
            }
            StepId::TerrainRgb => {
                "Terrarium-packed elevation from the sources in sources.json, lowest priority \
                 first, blended over `blur` metres at each source's coverage edge. The map draws \
                 hillshade and 3D terrain from these; there is no contour archive any more."
            }
            StepId::Hillshade => {
                "The same renderer packed the mapbox way (0.1 m per step instead of 1 m). It \
                 exists because older archives are named `_hillshade` and are still read."
            }
            StepId::ValhallaTiles => {
                "`valhalla_build_tiles` over the OSM extract, using the configured valhalla.json. \
                 Slow, and shared between areas: it is usually built once for a parent area \
                 covering everything you route in, so routes do not stop at a border."
            }
            StepId::ValhallaPackage => {
                "Packs the graph tiles covering one area into the .vtiles archive the phone \
                 downloads. The tile list comes from a .poly, from a tilemask, or from an \
                 existing package."
            }
        }
    }

    /// What the step needs before it can run, beyond the steps it depends on.
    pub fn reads(self) -> &'static str {
        match self {
            StepId::DownloadOsm => "the network",
            StepId::ElevationTiles => "valhalla.json, for the bounds to cover",
            StepId::Basemap | StepId::Routes => "the OSM extract, the planetiler jar, Java 21+",
            StepId::TerrainRgb | StepId::Hillshade => "sources.json and the elevation tiles",
            StepId::ValhallaTiles => "the OSM extract, valhalla.json, the elevation tiles",
            StepId::ValhallaPackage => "the Valhalla graph, and a shape or tile list",
        }
    }

    /// The `alpimaps` subcommand that runs this step.
    pub fn command(self) -> &'static str {
        match self {
            StepId::DownloadOsm => "download",
            StepId::ElevationTiles => "elevation",
            StepId::Basemap => "basemap",
            StepId::Routes => "routes",
            StepId::TerrainRgb => "terrain",
            StepId::Hillshade => "hillshade",
            StepId::ValhallaTiles => "valhalla-tiles",
            StepId::ValhallaPackage => "package",
        }
    }

    /// What must have run first.
    pub fn deps(self) -> &'static [StepId] {
        match self {
            StepId::DownloadOsm | StepId::ElevationTiles => &[],
            StepId::Basemap | StepId::Routes => &[StepId::DownloadOsm],
            StepId::TerrainRgb | StepId::Hillshade => &[StepId::ElevationTiles],
            StepId::ValhallaTiles => &[StepId::DownloadOsm, StepId::ElevationTiles],
            StepId::ValhallaPackage => &[StepId::ValhallaTiles],
        }
    }

    /// Whether the app can run this step itself, rather than it still being a shell script.
    pub fn is_implemented(self) -> bool {
        true
    }

    /// Steps that write into the same planetiler temp tree and therefore must not overlap.
    ///
    /// Two planetiler processes sharing a tmpdir delete each other's sort chunks
    /// (`NoSuchFileException: data/tmp/feature.db/chunk8`). The runner gives each run its own
    /// directory, but the graph also refuses to schedule these concurrently.
    pub fn is_planetiler(self) -> bool {
        matches!(self, StepId::Basemap | StepId::Routes)
    }
}

/// Expand a selection into a runnable, dependency-ordered plan.
///
/// Dependencies are pulled in automatically, duplicates collapse, and the result is a
/// topological order. Selecting only `ValhallaPackage` therefore still builds the tiles it
/// needs rather than failing halfway.
pub fn plan(requested: &[StepId]) -> Vec<StepId> {
    let mut ordered: Vec<StepId> = Vec::new();
    let mut visiting: Vec<StepId> = Vec::new();

    fn visit(step: StepId, ordered: &mut Vec<StepId>, visiting: &mut Vec<StepId>) {
        if ordered.contains(&step) || visiting.contains(&step) {
            // already placed, or a cycle - the graph is acyclic today, and refusing to recurse
            // keeps a future edit from hanging the UI instead of erroring
            return;
        }
        visiting.push(step);
        for dep in step.deps() {
            visit(*dep, ordered, visiting);
        }
        visiting.retain(|s| *s != step);
        ordered.push(step);
    }

    for step in requested {
        visit(*step, &mut ordered, &mut visiting);
    }
    ordered
}

/// Emitted by every step, native or subprocess, so the UI has one shape to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StepEvent {
    Started { step: StepId, area: String },
    Phase { step: StepId, name: String },
    Progress { step: StepId, label: String, percent: u8 },
    Log { step: StepId, line: String },
    Finished { step: StepId, ok: bool, elapsed: Option<String>, outputs: Vec<String> },
    /// Nothing ran: this step was already built. `reason` is shown as-is.
    Skipped { step: StepId, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_pulls_in_dependencies() {
        let p = plan(&[StepId::ValhallaPackage]);
        assert_eq!(
            p,
            vec![
                StepId::DownloadOsm,
                StepId::ElevationTiles,
                StepId::ValhallaTiles,
                StepId::ValhallaPackage
            ]
        );
    }

    #[test]
    fn plan_orders_dependencies_first() {
        for step in ALL_STEPS {
            let p = plan(&[step]);
            for (i, s) in p.iter().enumerate() {
                for dep in s.deps() {
                    let at = p.iter().position(|x| x == dep).expect("dep present");
                    assert!(at < i, "{dep:?} must precede {s:?} in {p:?}");
                }
            }
        }
    }

    #[test]
    fn plan_deduplicates_shared_dependencies() {
        let p = plan(&[StepId::Basemap, StepId::Routes]);
        assert_eq!(p, vec![StepId::DownloadOsm, StepId::Basemap, StepId::Routes]);
        assert_eq!(p.iter().filter(|s| **s == StepId::DownloadOsm).count(), 1);
    }

    #[test]
    fn plan_of_everything_contains_everything_once() {
        let p = plan(&ALL_STEPS);
        assert_eq!(p.len(), ALL_STEPS.len());
        for step in ALL_STEPS {
            assert!(p.contains(&step), "{step:?} missing");
        }
    }

    #[test]
    fn empty_selection_is_an_empty_plan() {
        assert!(plan(&[]).is_empty());
    }

    /// Every step runs from the app now. The flag has to stay honest: one claiming to be
    /// implemented that silently does nothing is worse than one that says it is not wired.
    #[test]
    fn every_step_is_wired_into_the_runner() {
        for step in ALL_STEPS {
            assert!(step.is_implemented(), "{step:?} is not wired into the runner");
        }
    }

    /// The two planetiler steps are the pair that corrupted each other's sort chunks when run
    /// concurrently, so they must be identifiable as such.
    #[test]
    fn planetiler_steps_are_flagged() {
        assert!(StepId::Basemap.is_planetiler() && StepId::Routes.is_planetiler());
        assert!(!StepId::TerrainRgb.is_planetiler());
    }
}
