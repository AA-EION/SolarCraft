// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors (upstream code).
// Copyright (C) 2026 SolarCraft contributors (integration).
//! IngeTrazo code inside SolarCraft, arranged so upstream changes port mechanically.
//!
//! * `upstream/rust/*.rs` are IngeTrazo's native kernels (`rust/ingetrazo_core/src`) copied
//!   **byte for byte**; only the pyo3 wrapper (`lib.rs` upstream) is left out. They are
//!   declared here at the crate root under their upstream names so their `crate::` paths
//!   resolve unchanged. Never edit them: re-sync with `tools/ingetrazo_sync.py`.
//! * `core/*.rs` are Rust ports of IngeTrazo Python modules (`core/*.py`), one file per
//!   upstream module with the same name; the pinned upstream file hash in `upstream.toml`
//!   tells the sync tool when the Python source changed and the port needs review.
#![allow(clippy::all)]

#[path = "upstream/rust/boxes.rs"] pub mod boxes;
#[path = "upstream/rust/facegeom.rs"] pub mod facegeom;
#[path = "upstream/rust/gltape.rs"] pub mod gltape;
#[path = "upstream/rust/hlr.rs"] pub mod hlr;
#[path = "upstream/rust/instances.rs"] pub mod instances;
#[path = "upstream/rust/occlusion.rs"] pub mod occlusion;
#[path = "upstream/rust/parity.rs"] pub mod parity;
#[path = "upstream/rust/weld.rs"] pub mod weld;

pub mod core {
    //! Rust ports of IngeTrazo `core/*.py`.
    pub mod orient;
    pub mod triangulate;
}

/// Upstream commit the vendored and ported code corresponds to.
pub const UPSTREAM_COMMIT: &str = include_str!("../upstream.toml");

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    /// Vendored files must stay identical to the pinned upstream bytes.
    #[test]
    fn vendored_files_match_pin() {
        let manifest = include_str!("../upstream.toml");
        let mut local = None;
        for line in manifest.lines() {
            if let Some(v) = line.strip_prefix("local = ") { local = Some(v.trim_matches('"').to_string()); }
            if let (Some(h), Some(l)) = (line.strip_prefix("sha256 = "), local.as_ref()) {
                let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(l)).unwrap();
                let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(got, h.trim_matches('"'), "{l} differs from the pinned upstream file; re-sync instead of editing");
            }
        }
    }
    #[test]
    fn kernels_link() {
        let (pts, map, _) = crate::weld::weld(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1e-9], [1.0, 0.0, 0.0]], 1.0, 1e-12);
        assert_eq!(pts.len(), 2);
        assert_eq!(map.len(), 3);
    }
}
