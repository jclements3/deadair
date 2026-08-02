//! Loading `*.zone.ron` sources from disk.

use std::fs;
use std::path::Path;

use ron::extensions::Extensions;

use crate::error::ParamError;
use crate::source::ZoneSource;

/// RON options for zone sources: `implicit_some` lets optional record
/// fields (`pen:`, `along:`, ...) be written bare in the files.
fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(Extensions::IMPLICIT_SOME)
}

/// Parse a zone source from RON text.
pub fn parse_zone_str(text: &str) -> Result<ZoneSource, ParamError> {
    ron_options()
        .from_str(text)
        .map_err(|e| ParamError::Parse {
            path: "<str>".to_owned(),
            message: e.to_string(),
        })
}

/// Load and parse one `*.zone.ron` file.
pub fn load_zone_file(path: impl AsRef<Path>) -> Result<ZoneSource, ParamError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|e| ParamError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    ron_options()
        .from_str(&text)
        .map_err(|e| ParamError::Parse {
            path: path.display().to_string(),
            message: e.to_string(),
        })
}

/// Load every `*.zone.ron` file directly inside `dir`, sorted by file name
/// so the result order is stable across platforms.
pub fn load_all_zones(dir: impl AsRef<Path>) -> Result<Vec<ZoneSource>, ParamError> {
    let dir = dir.as_ref();
    let entries = fs::read_dir(dir).map_err(|e| ParamError::Io {
        path: dir.display().to_string(),
        message: e.to_string(),
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ParamError::Io {
            path: dir.display().to_string(),
            message: e.to_string(),
        })?;
        let path = entry.path();
        let is_zone = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".zone.ron"));
        if is_zone {
            paths.push(path);
        }
    }
    paths.sort();
    paths.into_iter().map(load_zone_file).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Biome, Feature, Species};

    #[test]
    fn minimal_zone_parses_with_defaults() {
        let src = parse_zone_str(
            r#"ZoneSource(
                name: "Min",
                seed: 7,
                size_m: (10.0, 10.0),
                ambient_biome: Mud,
            )"#,
        )
        .expect("minimal source parses");
        assert_eq!(src.name, "Min");
        assert_eq!(src.ambient_biome, Biome::Mud);
        assert!(src.features.is_empty());
        assert!(src.spawn_tables.is_empty());
        assert_eq!(src.zombie_weight, 0.0);
    }

    #[test]
    fn optional_record_fields_are_implicit_some() {
        let src = parse_zone_str(
            r#"ZoneSource(
                name: "Opt",
                seed: 1,
                size_m: (50.0, 50.0),
                ambient_biome: Grass,
                features: [ Shed(pos: (5.0, 0.0, 5.0)) ],
                friendlies: [
                    (species: Cow, pen: (pos: (1.0, 0.0, 1.0), size: (8.0, 6.0)), count: 2),
                ],
                hazards: [ (kind: Hole, pos: (3.0, 0.0, 3.0), radius_m: 1.5) ],
            )"#,
        )
        .expect("optional fields parse bare");
        assert_eq!(src.friendlies[0].species, Species::Cow);
        assert!(src.friendlies[0].pen.is_some());
        assert_eq!(src.hazards[0].radius_m, Some(1.5));
        assert!(matches!(src.features[0], Feature::Shed { .. }));
    }

    #[test]
    fn bad_ron_is_a_parse_error() {
        let err = parse_zone_str("ZoneSource(name: 3)").expect_err("must fail");
        assert!(matches!(err, ParamError::Parse { .. }));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = load_zone_file("/nonexistent/nope.zone.ron").expect_err("must fail");
        assert!(matches!(err, ParamError::Io { .. }));
    }
}
