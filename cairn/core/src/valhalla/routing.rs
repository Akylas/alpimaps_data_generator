//! Routing against a built Valhalla graph, in-process.
//!
//! Uses `valhalla::tyr::actor_t` through a small C shim rather than talking to
//! `valhalla_service` over HTTP, which keeps prime_server and zmq out of the picture entirely.
//! Requests and responses are Valhalla's own JSON, unchanged.
//!
//! Compiled only when a built Valhalla is found (see `build.rs`); without one every entry point
//! reports that routing is unavailable instead of failing to build.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Whether this build can route.
pub const fn available() -> bool {
    cfg!(valhalla)
}

/// Build a routing config from a template, pointed at `tile_dir`.
///
/// `actor_t` validates the *whole* config on construction, not just the parts a route needs -
/// a config without `service_limits.isochrone.max_contours` is rejected even for a plain route.
/// Rather than enumerate every key Valhalla might check, this takes the pipeline's own
/// `valhalla.json` and overrides only the tile directory.
pub fn config_from_template(template_json: &str, tile_dir: &Path) -> Result<String> {
    let mut config: serde_json::Value = serde_json::from_str(template_json)
        .map_err(|e| anyhow!("parsing valhalla config template: {e}"))?;
    let mjolnir = config
        .get_mut("mjolnir")
        .and_then(|m| m.as_object_mut())
        .ok_or_else(|| anyhow!("config template has no mjolnir section"))?;
    mjolnir.insert("tile_dir".into(), serde_json::json!(tile_dir.display().to_string()));
    // a stale extract would silently win over the directory we were asked to use
    mjolnir.remove("tile_extract");
    mjolnir.remove("traffic_extract");
    Ok(config.to_string())
}

/// Load `valhalla.json` from disk and point it at `tile_dir`.
pub fn config_from_file(template: &Path, tile_dir: &Path) -> Result<String> {
    let text = std::fs::read_to_string(template)
        .map_err(|e| anyhow!("reading {}: {e}", template.display()))?;
    config_from_template(&text, tile_dir)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRequest {
    /// `[lon, lat]` pairs, in visiting order.
    pub locations: Vec<[f64; 2]>,
    /// Valhalla costing model: `auto`, `pedestrian`, `bicycle`, ...
    pub costing: String,
}

impl RouteRequest {
    pub fn to_json(&self) -> String {
        let locations: Vec<_> = self
            .locations
            .iter()
            .map(|[lon, lat]| serde_json::json!({ "lon": lon, "lat": lat }))
            .collect();
        serde_json::json!({
            "locations": locations,
            "costing": self.costing,
            "directions_options": { "units": "kilometers" }
        })
        .to_string()
    }
}

#[cfg(valhalla)]
mod ffi {
    use std::os::raw::{c_char, c_void};
    extern "C" {
        pub fn valhalla_actor_create(config_json: *const c_char, error: *mut *mut c_char) -> *mut c_void;
        pub fn valhalla_actor_route(
            handle: *mut c_void,
            request_json: *const c_char,
            error: *mut *mut c_char,
        ) -> *mut c_char;
        pub fn valhalla_actor_destroy(handle: *mut c_void);
        pub fn valhalla_string_free(text: *mut c_char);
    }
}

#[cfg(valhalla)]
pub struct Router {
    handle: *mut std::os::raw::c_void,
}

// actor_t owns its own graph reader and is not shared between threads here; the raw pointer is
// only ever touched through &mut self.
#[cfg(valhalla)]
unsafe impl Send for Router {}

#[cfg(valhalla)]
impl Router {
    pub fn new(config_json: &str) -> Result<Self> {
        use std::ffi::{CStr, CString};
        let config = CString::new(config_json)?;
        let mut error: *mut std::os::raw::c_char = std::ptr::null_mut();
        let handle = unsafe { ffi::valhalla_actor_create(config.as_ptr(), &mut error) };
        if handle.is_null() {
            let message = if error.is_null() {
                "valhalla actor creation failed".to_string()
            } else {
                let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().to_string();
                unsafe { ffi::valhalla_string_free(error) };
                text
            };
            return Err(anyhow!(message));
        }
        Ok(Self { handle })
    }

    /// Open using a `valhalla.json` template pointed at `tile_dir`.
    pub fn open(template: &Path, tile_dir: &Path) -> Result<Self> {
        Self::new(&config_from_file(template, tile_dir)?)
    }

    /// Run a route request, returning Valhalla's JSON response verbatim.
    pub fn route(&mut self, request_json: &str) -> Result<String> {
        use std::ffi::{CStr, CString};
        let request = CString::new(request_json)?;
        let mut error: *mut std::os::raw::c_char = std::ptr::null_mut();
        let raw = unsafe { ffi::valhalla_actor_route(self.handle, request.as_ptr(), &mut error) };
        if raw.is_null() {
            let message = if error.is_null() {
                "routing failed".to_string()
            } else {
                let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().to_string();
                unsafe { ffi::valhalla_string_free(error) };
                text
            };
            return Err(anyhow!(message));
        }
        let response = unsafe { CStr::from_ptr(raw) }.to_string_lossy().to_string();
        unsafe { ffi::valhalla_string_free(raw) };
        Ok(response)
    }
}

#[cfg(valhalla)]
impl Drop for Router {
    fn drop(&mut self) {
        unsafe { ffi::valhalla_actor_destroy(self.handle) };
    }
}

#[cfg(not(valhalla))]
pub struct Router;

#[cfg(not(valhalla))]
impl Router {
    pub fn new(_config_json: &str) -> Result<Self> {
        Err(anyhow!("this build has no Valhalla; rebuild with VALHALLA_DIR set"))
    }
    pub fn open(_template: &Path, _tile_dir: &Path) -> Result<Self> {
        Self::new("")
    }
    pub fn route(&mut self, _request_json: &str) -> Result<String> {
        Err(anyhow!("this build has no Valhalla"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_repointed_at_the_tile_directory() {
        let template = r#"{"mjolnir":{"tile_dir":"/old","tile_extract":"/old.tar"},
                           "service_limits":{"isochrone":{"max_contours":4}}}"#;
        let json = config_from_template(template, Path::new("/data/tiles")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["mjolnir"]["tile_dir"], "/data/tiles");
        // everything else survives, including the keys the actor validates
        assert_eq!(parsed["service_limits"]["isochrone"]["max_contours"], 4);
    }

    /// A leftover `tile_extract` would take precedence over the directory being asked for.
    #[test]
    fn stale_extract_is_removed() {
        let template = r#"{"mjolnir":{"tile_dir":"/old","tile_extract":"/old.tar"}}"#;
        let json = config_from_template(template, Path::new("/data/tiles")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["mjolnir"].get("tile_extract").is_none());
    }

    #[test]
    fn rejects_a_template_without_mjolnir() {
        assert!(config_from_template(r#"{"loki":{}}"#, Path::new("/tiles")).is_err());
    }

    #[test]
    fn request_serialises_lon_lat_pairs() {
        let request = RouteRequest {
            locations: vec![[5.72, 45.18], [6.86, 45.83]],
            costing: "pedestrian".into(),
        };
        let parsed: serde_json::Value = serde_json::from_str(&request.to_json()).unwrap();
        assert_eq!(parsed["locations"][0]["lon"], 5.72);
        assert_eq!(parsed["locations"][1]["lat"], 45.83);
        assert_eq!(parsed["costing"], "pedestrian");
    }

    /// Without Valhalla the wrapper must fail cleanly rather than fail to build.
    #[cfg(not(valhalla))]
    #[test]
    fn reports_unavailable_without_valhalla() {
        assert!(!available());
        assert!(Router::new("{}").is_err());
    }

    #[cfg(valhalla)]
    #[test]
    fn reports_available_with_valhalla() {
        assert!(available());
    }
}
