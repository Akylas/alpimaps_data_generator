pub mod options;
pub mod planetiler;

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
        matches!(
            self,
            StepId::Basemap | StepId::Routes | StepId::TerrainRgb | StepId::ValhallaPackage
        )
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

    /// Hillshade, the OSM download and valhalla_build_tiles are still shell scripts. The flag
    /// must stay honest: a step claiming to be implemented that silently does nothing is worse
    /// than one that says it is not wired.
    #[test]
    fn implemented_flag_matches_what_the_runner_handles() {
        for step in [StepId::Basemap, StepId::Routes, StepId::TerrainRgb, StepId::ValhallaPackage] {
            assert!(step.is_implemented(), "{step:?} is wired into the runner");
        }
        for step in [StepId::DownloadOsm, StepId::ElevationTiles, StepId::Hillshade, StepId::ValhallaTiles] {
            assert!(!step.is_implemented(), "{step:?} is still a shell script");
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
