//! Loading `*.zone.ron` sources from disk.
//!
//! File I/O lives here and only here: `VimProp` features name their `.vim`
//! scripts by path, and [`resolve_vim_sources`] inlines each script's text
//! into [`ZoneSource::vim_sources`] at load time, so
//! [`crate::expand_zone`] stays a pure function of the source value.

use std::fs;
use std::path::Path;

use ron::extensions::Extensions;

use crate::error::ParamError;
use crate::source::{Feature, ZoneSource};

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

/// Read the `.vim` script of every `VimProp` in `source` and inline its
/// text into [`ZoneSource::vim_sources`], resolving each `src` path against
/// `assets_dir`. Idempotent; a no-op for zones without `VimProp`s.
///
/// [`load_zone_file`] calls this automatically (with the parent of the
/// zone file's directory as the assets dir, so `"props/x.vim"` resolves to
/// `assets/props/x.vim` for zones in `assets/zones/`). Call it yourself
/// when a source came from [`parse_zone_str`] instead.
pub fn resolve_vim_sources(
    source: &mut ZoneSource,
    assets_dir: impl AsRef<Path>,
) -> Result<(), ParamError> {
    let assets_dir = assets_dir.as_ref();
    let srcs: Vec<String> = source
        .features
        .iter()
        .filter_map(|f| match f {
            Feature::VimProp { src, .. } => Some(src.clone()),
            _ => None,
        })
        .collect();
    for src in srcs {
        if source.vim_sources.contains_key(&src) {
            continue;
        }
        let path = assets_dir.join(&src);
        let text = fs::read_to_string(&path).map_err(|e| ParamError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        source.vim_sources.insert(src, text);
    }
    Ok(())
}

/// Load and parse one `*.zone.ron` file, inlining any `VimProp` `.vim`
/// scripts (resolved against the parent of the file's directory — the
/// assets dir for zones under `assets/zones/`).
pub fn load_zone_file(path: impl AsRef<Path>) -> Result<ZoneSource, ParamError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|e| ParamError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    let mut source: ZoneSource =
        ron_options()
            .from_str(&text)
            .map_err(|e| ParamError::Parse {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
    let assets_dir = path
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    resolve_vim_sources(&mut source, assets_dir)?;
    Ok(source)
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
