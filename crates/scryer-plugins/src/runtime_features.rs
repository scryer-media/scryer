use std::collections::HashSet;

pub const PLUGIN_REQUIRED_FEATURE_SIMD128: &str = "simd128";
pub const PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD: &str = "relaxed-simd";

/// WASI targets, spelled exactly as the catalog spells an artifact's `runtime`.
pub const PLUGIN_RUNTIME_TARGET_WASIP1: &str = "wasm32-wasip1";
pub const PLUGIN_RUNTIME_TARGET_WASIP2: &str = "wasm32-wasip2";

/// The WASI targets this host can actually instantiate.
///
/// This is a declaration, not a probe: unlike a wasm feature — which either
/// compiles on this CPU or does not — a WASI target is a property of the
/// loader that is linked into this binary. The components-only runtime loads
/// `wasm32-wasip2` components and nothing else; the Preview 1 command and
/// Extism runtimes were deleted with the component migration, so
/// `wasm32-wasip1` is deliberately absent.
///
/// Adding Preview 3 later is a one-line change here plus the wasmtime bump —
/// the catalog wire format, the renderer, and the selection code stay as they
/// are, because both halves of an artifact's requirement (`runtime` and
/// `required_features`) are matched as opaque capability tokens against the
/// set this module returns.
const SUPPORTED_PLUGIN_RUNTIME_TARGETS: &[&str] = &[PLUGIN_RUNTIME_TARGET_WASIP2];

const SIMD128_PROBE_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/simd128_probe.wasm"));
const RELAXED_SIMD_PROBE_WASM: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/relaxed_simd_probe.wasm"));

/// Every capability token this host satisfies: WASI targets and wasm features
/// in one namespace.
///
/// A catalog artifact is runnable here when its `runtime` token is in this set
/// and its `required_features` are a subset of it. Target names
/// (`wasm32-wasip*`) and feature names (`simd128`, `relaxed-simd`) cannot
/// collide, so one set expresses both axes and a new axis — a future target, a
/// future wasm feature — needs no new plumbing.
pub fn detect_plugin_runtime_capabilities() -> HashSet<String> {
    let mut capabilities = detect_supported_plugin_required_features();
    capabilities.extend(supported_plugin_runtime_targets());
    capabilities
}

/// The WASI target tokens this host declares support for.
pub fn supported_plugin_runtime_targets() -> HashSet<String> {
    SUPPORTED_PLUGIN_RUNTIME_TARGETS
        .iter()
        .map(|target| (*target).to_string())
        .collect()
}

pub fn detect_supported_plugin_required_features() -> HashSet<String> {
    detect_supported_plugin_required_features_with(supports_plugin_module)
}

fn detect_supported_plugin_required_features_with(
    mut supports: impl FnMut(&[u8]) -> bool,
) -> HashSet<String> {
    let mut features = HashSet::new();
    if !supports(SIMD128_PROBE_WASM) {
        return features;
    }

    features.insert(PLUGIN_REQUIRED_FEATURE_SIMD128.to_string());
    if supports(RELAXED_SIMD_PROBE_WASM) {
        features.insert(PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD.to_string());
    }
    features
}

fn supports_plugin_module(wasm: &[u8]) -> bool {
    compile_plugin_module(wasm).is_ok()
}

fn compile_plugin_module(wasm: &[u8]) -> wasmtime::Result<()> {
    wasmtime::Module::new(crate::wasmtime_host::engine::shared_engine(), wasm).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_simd_probe_degrades_to_baseline() {
        let features = detect_supported_plugin_required_features_with(|_| false);
        assert!(features.is_empty());
    }

    #[test]
    fn relaxed_simd_requires_simd128() {
        let mut calls = 0;
        let features = detect_supported_plugin_required_features_with(|_| {
            calls += 1;
            calls == 2
        });
        assert!(features.is_empty());
    }

    #[test]
    fn simd128_can_be_reported_without_relaxed_simd() {
        let mut calls = 0;
        let features = detect_supported_plugin_required_features_with(|_| {
            calls += 1;
            calls == 1
        });
        assert_eq!(
            features,
            HashSet::from([PLUGIN_REQUIRED_FEATURE_SIMD128.to_string()])
        );
    }

    #[test]
    fn reports_simd128_and_relaxed_simd_when_both_probes_pass() {
        let features = detect_supported_plugin_required_features_with(|_| true);
        assert_eq!(
            features,
            HashSet::from([
                PLUGIN_REQUIRED_FEATURE_SIMD128.to_string(),
                PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD.to_string(),
            ])
        );
    }

    #[test]
    fn wasmtime_runtime_accepts_probe_modules() {
        compile_plugin_module(SIMD128_PROBE_WASM).expect("simd128 probe should compile");
        compile_plugin_module(RELAXED_SIMD_PROBE_WASM).expect("relaxed-simd probe should compile");
    }

    #[test]
    fn detected_features_use_catalog_feature_tokens() {
        let features = detect_supported_plugin_required_features();
        for feature in &features {
            assert!(
                matches!(
                    feature.as_str(),
                    PLUGIN_REQUIRED_FEATURE_SIMD128 | PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD
                ),
                "unexpected feature token {feature}"
            );
        }
        if features.contains(PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD) {
            assert!(features.contains(PLUGIN_REQUIRED_FEATURE_SIMD128));
        }
    }

    #[test]
    fn declared_targets_are_the_components_only_runtime() {
        let targets = supported_plugin_runtime_targets();
        assert!(targets.contains(PLUGIN_RUNTIME_TARGET_WASIP2));
        assert!(
            !targets.contains(PLUGIN_RUNTIME_TARGET_WASIP1),
            "the Preview 1 command runtime was deleted; declaring it would make the client \
             download wasip1 artifacts it cannot instantiate"
        );
    }

    #[test]
    fn capabilities_carry_targets_and_features_in_one_namespace() {
        let capabilities = detect_plugin_runtime_capabilities();
        assert!(capabilities.contains(PLUGIN_RUNTIME_TARGET_WASIP2));
        for feature in detect_supported_plugin_required_features() {
            assert!(capabilities.contains(&feature));
        }
        for capability in &capabilities {
            assert!(
                matches!(
                    capability.as_str(),
                    PLUGIN_RUNTIME_TARGET_WASIP2
                        | PLUGIN_REQUIRED_FEATURE_SIMD128
                        | PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD
                ),
                "unexpected capability token {capability}"
            );
        }
    }
}
