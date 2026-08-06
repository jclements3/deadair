//! `.vim` template plumbing for the built-in feature generators (the
//! primer's §10b endgame: the object library is editable text, not Rust).
//!
//! Three pieces live here:
//!
//! - [`builtin_template`]: the built-in geometry templates, baked in from
//!   `assets/props/builtin/*.vim` via `include_str!`. Baking (rather than
//!   routing builtins through the `VimProp` loader/resolver) is deliberate:
//!   `expand_zone` must stay a pure, I/O-free function of the `ZoneSource`
//!   *value*, and it is called on programmatically-built sources all over
//!   the tests and the editor — sources that never pass through
//!   `resolve_vim_sources`. `VimProp`'s resolver exists because zone RON
//!   references arbitrary user paths; the builtins are a closed set owned
//!   by this crate, so compile-time inclusion keeps expansion infallible
//!   with respect to I/O while the `.vim` text stays ground truth in the
//!   repo (cargo tracks `include_str!` inputs, so editing a template
//!   rebuilds da-param).
//! - [`vim_with_params`]: bind generator dimensions (silo radius, mast
//!   height, ...) onto a template by rewriting its numeric `let` lines.
//!   Pure string → string, deterministic.
//! - [`VimCache`]: per-expansion compile cache keyed by final source text,
//!   so each distinct template is BSP-compiled once per [`expand_zone`]
//!   call no matter how many instances a zone places.

use std::collections::BTreeMap;
use std::rc::Rc;

use glam::Vec3;

use crate::error::ParamError;

/// Names and baked text of every built-in template. The text is ground
/// truth — edit `assets/props/builtin/*.vim`, not Rust.
const BUILTIN: &[(&str, &str)] = &[
    ("silo", include_str!("../../../assets/props/builtin/silo.vim")),
    (
        "streetlight",
        include_str!("../../../assets/props/builtin/streetlight.vim"),
    ),
    (
        "radio_mast",
        include_str!("../../../assets/props/builtin/radio_mast.vim"),
    ),
    (
        "dumpster",
        include_str!("../../../assets/props/builtin/dumpster.vim"),
    ),
    (
        "gravestone_a",
        include_str!("../../../assets/props/builtin/gravestone_a.vim"),
    ),
    (
        "gravestone_b",
        include_str!("../../../assets/props/builtin/gravestone_b.vim"),
    ),
    (
        "gravestone_c",
        include_str!("../../../assets/props/builtin/gravestone_c.vim"),
    ),
];

/// The source text of the named built-in template. Panics on an unknown
/// name — that is a programmer error in a generator, not a data error.
pub(crate) fn builtin_template(name: &str) -> &'static str {
    BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| *text)
        .unwrap_or_else(|| panic!("no builtin .vim template named {name:?}"))
}

/// The repo-relative pseudo-path used in errors for a builtin template.
pub(crate) fn builtin_path(name: &str) -> String {
    format!("assets/props/builtin/{name}.vim")
}

/// Rewrite the values of top-level `let <name> = <number>` lines in a `.vim`
/// script — the DSL's parametric-dimension mechanism — binding each `(name,
/// value)` pair. Only `let` lines whose right-hand side is a *constant*
/// numeric expression (no variable references, verified via
/// [`da_csg::dsl::numeric_refs`]) are parameter targets; derived lines like
/// `let hz = h / 2` and part bindings are untouched and re-derive from the
/// new values when the script compiles. Trailing `#` comments survive.
///
/// Pure string → string and deterministic. Errors if any requested name has
/// no matching numeric `let` line — a template/generator mismatch that must
/// surface, not silently produce default-sized geometry.
pub fn vim_with_params(src: &str, params: &[(&str, f32)]) -> Result<String, String> {
    let mut bound = vec![false; params.len()];
    let lines: Vec<String> = src
        .split('\n')
        .map(|line| {
            // Split off a trailing `#` comment; a full-line `"` comment or
            // anything not shaped `let name = <const>` falls through as-is.
            let (code, comment) = match line.find('#') {
                Some(i) => (&line[..i], &line[i..]),
                None => (line, ""),
            };
            let trimmed = code.trim_start();
            let Some(rest) = trimmed.strip_prefix("let ") else {
                return line.to_owned();
            };
            let Some(eq) = rest.find('=') else {
                return line.to_owned();
            };
            let name = rest[..eq].trim();
            let rhs = rest[eq + 1..].trim();
            let is_const = da_csg::dsl::numeric_refs(rhs).is_some_and(|refs| refs.is_empty());
            let Some(pi) = params.iter().position(|(n, _)| *n == name) else {
                return line.to_owned();
            };
            if !is_const {
                return line.to_owned();
            }
            bound[pi] = true;
            let head_len = code.len() - trimmed.len() + "let ".len() + eq + 1;
            let mut out = format!("{} {}", &code[..head_len], params[pi].1);
            if !comment.is_empty() {
                out.push_str("   ");
                out.push_str(comment);
            }
            out
        })
        .collect();
    if let Some(i) = bound.iter().position(|b| !b) {
        return Err(format!(
            "parameter `{}` not found: template has no `let {} = <number>` line",
            params[i].0, params[i].0
        ));
    }
    Ok(lines.join("\n"))
}

/// One named part of a compiled template, meshed in darkair's Y-up frame.
pub(crate) struct PartMesh {
    /// Part tag — flows from the script's `let` bindings (da-csg).
    pub name: String,
    /// Y-up vertex positions, meters.
    pub vertices: Vec<Vec3>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// A compiled `.vim` source: per-part meshes plus the combined whole-solid
/// mesh (`VimProp` places props as a single part).
pub(crate) struct CompiledVim {
    /// One entry per distinct part name, in deterministic part-id order.
    pub parts: Vec<PartMesh>,
    /// The whole solid as one `(vertices, indices)` mesh.
    pub combined: (Vec<Vec3>, Vec<u32>),
}

/// Per-expansion compile cache, keyed by final source text: each distinct
/// template is lexed/evaluated/BSP-resolved once per [`crate::expand_zone`]
/// call. `BTreeMap` + `Rc` keep lookups deterministic and clones cheap;
/// determinism across expansions is da-csg's byte-identical-mesh guarantee.
pub(crate) struct VimCache {
    map: BTreeMap<String, Rc<CompiledVim>>,
}

impl VimCache {
    /// Fresh cache for one expansion.
    pub fn new() -> Self {
        VimCache { map: BTreeMap::new() }
    }

    /// Compile `text` (or return the already-compiled result). `origin` is
    /// the path reported in errors — a zone-relative `VimProp` src or a
    /// `builtin/...` pseudo-path.
    pub fn get_or_compile(&mut self, text: &str, origin: &str) -> Result<Rc<CompiledVim>, ParamError> {
        if let Some(hit) = self.map.get(text) {
            return Ok(Rc::clone(hit));
        }
        let compiled = da_csg::compile_vim(text).map_err(|message| ParamError::VimCompile {
            path: origin.to_owned(),
            message,
        })?;
        let parts = compiled
            .solid
            .to_meshes_yup_by_part()
            .into_iter()
            .map(|(name, vertices, indices)| PartMesh { name, vertices, indices })
            .collect();
        let entry = Rc::new(CompiledVim {
            parts,
            combined: compiled.solid.to_mesh_yup(),
        });
        self.map.insert(text.to_owned(), Rc::clone(&entry));
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_template_compiles_with_parts_on_the_ground() {
        for (name, text) in BUILTIN {
            let c = da_csg::compile_vim(text)
                .unwrap_or_else(|e| panic!("builtin {name}: {e}"));
            assert!(c.solid.volume() > 0.0, "builtin {name}: empty solid");
            let parted = c.solid.to_meshes_yup_by_part();
            assert!(!parted.is_empty(), "builtin {name}: no parts");
            let min_y = parted
                .iter()
                .flat_map(|(_, v, _)| v.iter().map(|p| p.y))
                .fold(f32::INFINITY, f32::min);
            assert!(
                min_y > -0.01 && min_y < 0.5,
                "builtin {name}: base must sit at y = 0, min_y = {min_y}"
            );
            // Part names must come from the script's `let` vocabulary, not
            // kernel defaults.
            for (part, _, _) in &parted {
                assert!(
                    !["cube", "cylinder", "sphere", "extrude", "lathe", "frustum", "bevel"]
                        .contains(&part.as_str()),
                    "builtin {name}: part {part:?} kept a kernel default name — bind it via let"
                );
            }
        }
    }

    #[test]
    fn vim_with_params_rewrites_numeric_lets_only() {
        let src = "# a comment\n\
                   let radius = 4.0         # barrel radius\n\
                   let height = 18.0\n\
                   let hz = height / 2\n\
                   let barrel = cylinder(radius, height)\n\
                   model barrel";
        let out = vim_with_params(src, &[("radius", 2.5), ("height", 9.0)]).expect("binds");
        assert!(out.contains("let radius = 2.5"), "{out}");
        assert!(out.contains("# barrel radius"), "comment survives: {out}");
        assert!(out.contains("let height = 9"), "{out}");
        // Derived and part lines are untouched.
        assert!(out.contains("let hz = height / 2"), "{out}");
        assert!(out.contains("let barrel = cylinder(radius, height)"), "{out}");
        // The rewritten script still compiles and reflects the new numbers.
        let c = da_csg::compile_vim(&out).expect("rewritten script compiles");
        let want = std::f64::consts::PI * 2.5 * 2.5 * 9.0;
        let vol = c.solid.volume();
        assert!((vol - want).abs() < want * 0.01, "vol {vol} vs {want}");
    }

    #[test]
    fn vim_with_params_is_deterministic_and_pure() {
        let src = builtin_template("silo");
        let a = vim_with_params(src, &[("radius", 4.0), ("height", 18.0)]).unwrap();
        let b = vim_with_params(src, &[("radius", 4.0), ("height", 18.0)]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn vim_with_params_errors_on_missing_or_nonnumeric_names() {
        let src = "let r = 2\nlet body = cylinder(r, 4)\nmodel body";
        let err = vim_with_params(src, &[("wall", 3.0)]).expect_err("unknown name");
        assert!(err.contains("wall"), "{err}");
        // `body` exists but is a part binding, not a numeric parameter.
        let err = vim_with_params(src, &[("body", 3.0)]).expect_err("part binding");
        assert!(err.contains("body"), "{err}");
    }

    #[test]
    fn cache_compiles_each_distinct_source_once() {
        let mut cache = VimCache::new();
        let src = "let s = cube(2)\nmodel s";
        let a = cache.get_or_compile(src, "t.vim").expect("compiles");
        let b = cache.get_or_compile(src, "t.vim").expect("cached");
        assert!(Rc::ptr_eq(&a, &b), "second lookup must hit the cache");
        assert_eq!(cache.map.len(), 1);
        let other = cache.get_or_compile("let s = cube(3)\nmodel s", "t.vim").unwrap();
        assert!(!Rc::ptr_eq(&a, &other));
        assert_eq!(cache.map.len(), 2);
    }
}
