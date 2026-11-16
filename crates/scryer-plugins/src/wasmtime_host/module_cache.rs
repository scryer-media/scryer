//! Process-local compiled-module registry.
//!
//! Wasmtime's engine cache persists native code across restarts. This registry
//! keeps the corresponding `Module` handles alive for the current process so
//! a plugin invocation never reparses or rehydrates the same artifact. Each
//! ready module also keeps the host's pre-instantiated world binding for it,
//! so an invocation builds only its own `Store`.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Instant;

use wasmtime::component::Component;

use super::engine;

const MAX_READY_MODULES_PER_FLAVOR: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ModuleFlavor {
    Indexer,
    Archive,
    Subtitle,
    DownloadClient,
    Notification,
    ListProvider,
}

impl ModuleFlavor {
    fn label(self) -> &'static str {
        match self {
            Self::Indexer => "indexer_component",
            Self::Archive => "archive_component",
            Self::Subtitle => "subtitle_component",
            Self::DownloadClient => "download_client_component",
            Self::Notification => "notification_component",
            Self::ListProvider => "list_provider_component",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ModuleKey {
    flavor: ModuleFlavor,
    wasm_digest: [u8; 32],
}

enum ModuleEntry {
    Ready(ReadyModule),
    Loading(Arc<ModuleWaiter>),
    Failed(String),
}

/// A compiled module and, once a host has built it, that host's linked,
/// pre-instantiated binding for it. The binding is keyed by the same digest as
/// the module and lives exactly as long as the module's slot: an evicted
/// module drops it, and different bytes have a different slot.
struct ReadyModule {
    component: Arc<Component>,
    binding: Option<Arc<dyn Any + Send + Sync>>,
}

struct ModuleWaiter {
    result: Mutex<Option<Result<Arc<Component>, String>>>,
    ready: Condvar,
}

impl ModuleWaiter {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn wait(&self) -> Result<Arc<Component>, String> {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while result.is_none() {
            result = self
                .ready
                .wait(result)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        result.clone().expect("module waiter result must be set")
    }

    fn finish(&self, result: Result<Arc<Component>, String>) {
        let mut slot = self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(result);
        self.ready.notify_all();
    }
}

#[derive(Default)]
struct ModuleRegistry {
    entries: HashMap<ModuleKey, ModuleEntry>,
    recency: VecDeque<ModuleKey>,
}

impl ModuleRegistry {
    fn touch(&mut self, key: &ModuleKey) {
        if let Some(position) = self.recency.iter().position(|item| item == key) {
            self.recency.remove(position);
        }
        self.recency.push_back(key.clone());
    }

    fn insert_ready(&mut self, key: ModuleKey, artifact: Arc<Component>) {
        self.entries.insert(
            key.clone(),
            ModuleEntry::Ready(ReadyModule {
                component: artifact,
                binding: None,
            }),
        );
        self.touch(&key);

        while self
            .recency
            .iter()
            .filter(|entry| entry.flavor == key.flavor)
            .count()
            > MAX_READY_MODULES_PER_FLAVOR
        {
            let Some(position) = self
                .recency
                .iter()
                .position(|entry| entry.flavor == key.flavor)
            else {
                break;
            };
            let oldest = self
                .recency
                .remove(position)
                .expect("LRU position must point to an entry");
            self.entries.remove(&oldest);
        }
    }
}

static MODULE_REGISTRY: LazyLock<Mutex<ModuleRegistry>> =
    LazyLock::new(|| Mutex::new(ModuleRegistry::default()));

pub(crate) fn indexer_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::Indexer, wasm)
}

pub(crate) fn archive_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::Archive, wasm)
}

pub(crate) fn subtitle_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::Subtitle, wasm)
}

pub(crate) fn download_client_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::DownloadClient, wasm)
}

pub(crate) fn notification_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::Notification, wasm)
}

pub(crate) fn list_provider_component(wasm: &[u8]) -> Result<Arc<Component>, String> {
    component_for(ModuleFlavor::ListProvider, wasm)
}

/// A deliberate plugin load retries a prior compilation failure. This is used
/// for installs, updates, and explicit provider reloads; ordinary invocations
/// keep returning the retained failure instead of starting surprise work.
pub(crate) fn reset_failed_modules(wasm: &[u8]) {
    let digest = *blake3::hash(wasm).as_bytes();
    let mut registry = MODULE_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for flavor in [
        ModuleFlavor::Indexer,
        ModuleFlavor::Archive,
        ModuleFlavor::Subtitle,
        ModuleFlavor::DownloadClient,
        ModuleFlavor::Notification,
        ModuleFlavor::ListProvider,
    ] {
        let key = ModuleKey {
            flavor,
            wasm_digest: digest,
        };
        if matches!(registry.entries.get(&key), Some(ModuleEntry::Failed(_))) {
            registry.entries.remove(&key);
            tracing::debug!(
                module_flavor = flavor.label(),
                wasm_digest = %blake3::Hash::from_bytes(digest),
                "cleared retained plugin module failure for explicit reload"
            );
        }
    }
}

pub(crate) struct RehydrationArtifact {
    pub(crate) plugin_id: String,
    pub(crate) plugin_version: String,
    pub(crate) flavor: ModuleFlavor,
    pub(crate) wasm: Vec<u8>,
}

/// Register every selected artifact before spawning the single background
/// rehydration worker. A request that arrives while the worker is processing a
/// later artifact sees its `Loading` slot and waits instead of compiling.
pub(crate) fn schedule_rehydration(artifacts: Vec<RehydrationArtifact>) {
    let mut queued = Vec::new();
    {
        let mut registry = MODULE_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for artifact in artifacts {
            let key = ModuleKey {
                flavor: artifact.flavor,
                wasm_digest: *blake3::hash(&artifact.wasm).as_bytes(),
            };
            match registry.entries.get(&key) {
                Some(ModuleEntry::Ready(_)) => {
                    tracing::debug!(
                        plugin_id = artifact.plugin_id.as_str(),
                        plugin_version = artifact.plugin_version.as_str(),
                        module_flavor = artifact.flavor.label(),
                        "plugin module already rehydrated"
                    );
                }
                Some(ModuleEntry::Loading(_)) => {}
                Some(ModuleEntry::Failed(_)) => {
                    let waiter = Arc::new(ModuleWaiter::new());
                    registry
                        .entries
                        .insert(key.clone(), ModuleEntry::Loading(Arc::clone(&waiter)));
                    queued.push((key, artifact, waiter));
                }
                None => {
                    let waiter = Arc::new(ModuleWaiter::new());
                    registry
                        .entries
                        .insert(key.clone(), ModuleEntry::Loading(Arc::clone(&waiter)));
                    queued.push((key, artifact, waiter));
                }
            }
        }
    }

    if queued.is_empty() {
        return;
    }

    std::thread::Builder::new()
        .name("scryer-plugin-rehydrate".to_string())
        .spawn(move || {
            for (key, artifact, waiter) in queued {
                tracing::info!(
                    plugin_id = artifact.plugin_id.as_str(),
                    plugin_version = artifact.plugin_version.as_str(),
                    module_flavor = artifact.flavor.label(),
                    "plugin module rehydration started"
                );
                let _ = compile_registered(
                    key,
                    &artifact.wasm,
                    waiter,
                    "background rehydration",
                    Some((
                        artifact.plugin_id.as_str(),
                        artifact.plugin_version.as_str(),
                    )),
                );
            }
        })
        .expect("spawn plugin module rehydration worker");
}

fn component_for(flavor: ModuleFlavor, wasm: &[u8]) -> Result<Arc<Component>, String> {
    let key = ModuleKey {
        flavor,
        wasm_digest: *blake3::hash(wasm).as_bytes(),
    };
    component_for_key(key, wasm)
}

/// The compiled module for `wasm` and the binding `build` makes from it,
/// reused across invocations for as long as the module stays cached. `build`
/// runs only when the module's slot holds no binding of type `T` yet; it must
/// depend on nothing but the component, since every later invocation of the
/// same bytes receives what it returned. A failed build is not retained.
pub(crate) fn binding_for<T: Any + Send + Sync>(
    flavor: ModuleFlavor,
    wasm: &[u8],
    build: impl FnOnce(&Arc<Component>) -> Result<T, String>,
) -> Result<(Arc<Component>, Arc<T>), String> {
    let key = ModuleKey {
        flavor,
        wasm_digest: *blake3::hash(wasm).as_bytes(),
    };
    let component = component_for_key(key.clone(), wasm)?;
    {
        let registry = MODULE_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(ModuleEntry::Ready(ready)) = registry.entries.get(&key)
            && Arc::ptr_eq(&ready.component, &component)
            && let Some(binding) = ready.binding.clone()
            && let Ok(binding) = binding.downcast::<T>()
        {
            return Ok((component, binding));
        }
    }

    let binding = Arc::new(build(&component)?);
    let mut registry = MODULE_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(ModuleEntry::Ready(ready)) = registry.entries.get_mut(&key)
        && Arc::ptr_eq(&ready.component, &component)
    {
        ready.binding = Some(Arc::clone(&binding) as Arc<dyn Any + Send + Sync>);
    }
    Ok((component, binding))
}

fn component_for_key(key: ModuleKey, wasm: &[u8]) -> Result<Arc<Component>, String> {
    let flavor = key.flavor;
    enum Resolution {
        Wait(Arc<ModuleWaiter>),
        Compile(Arc<ModuleWaiter>),
    }

    let resolution = {
        let mut registry = MODULE_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match registry.entries.get(&key) {
            Some(ModuleEntry::Ready(ready)) => {
                let artifact = ready.component.clone();
                registry.touch(&key);
                tracing::debug!(
                    module_flavor = flavor.label(),
                    wasm_digest = %blake3::Hash::from_bytes(key.wasm_digest),
                    "plugin module memory-cache hit"
                );
                return Ok(artifact);
            }
            Some(ModuleEntry::Loading(waiter)) => Resolution::Wait(Arc::clone(waiter)),
            Some(ModuleEntry::Failed(error)) => return Err(error.clone()),
            None => {
                let waiter = Arc::new(ModuleWaiter::new());
                registry
                    .entries
                    .insert(key.clone(), ModuleEntry::Loading(Arc::clone(&waiter)));
                Resolution::Compile(waiter)
            }
        }
    };

    match resolution {
        Resolution::Wait(waiter) => {
            tracing::debug!(
                module_flavor = flavor.label(),
                wasm_digest = %blake3::Hash::from_bytes(key.wasm_digest),
                "waiting for shared plugin module compilation"
            );
            waiter.wait()
        }
        Resolution::Compile(waiter) => compile_registered(key, wasm, waiter, "demand", None),
    }
}

fn compile_registered(
    key: ModuleKey,
    wasm: &[u8],
    waiter: Arc<ModuleWaiter>,
    source: &'static str,
    plugin: Option<(&str, &str)>,
) -> Result<Arc<Component>, String> {
    let started = Instant::now();
    // Every surviving flavor is a WASI Preview 2 component, so they all compile
    // on the one async engine.
    let engine = engine::shared_async_engine();
    engine::prepare_cache_cleanup();
    let (cache_hits_before, cache_misses_before) = engine::cache_statistics();
    let result = Component::from_binary(engine, wasm)
        .map(Arc::new)
        .map_err(|error| format!("failed to compile plugin component: {error:#}"));

    let registered_waiter = {
        let mut registry = MODULE_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(ModuleEntry::Loading(registered_waiter)) = registry.entries.remove(&key) else {
            unreachable!("plugin module registry entry changed while compiling");
        };
        match &result {
            Ok(artifact) => registry.insert_ready(key.clone(), artifact.clone()),
            Err(error) => {
                registry
                    .entries
                    .insert(key.clone(), ModuleEntry::Failed(error.clone()));
            }
        }
        registered_waiter
    };
    debug_assert!(Arc::ptr_eq(&waiter, &registered_waiter));
    let (cache_hits_after, cache_misses_after) = engine::cache_statistics();
    let disk_cache_hits = cache_hits_after.saturating_sub(cache_hits_before);
    let disk_cache_misses = cache_misses_after.saturating_sub(cache_misses_before);
    let cache_state = if disk_cache_misses > 0 {
        "cold_compile"
    } else if disk_cache_hits > 0 {
        "disk_cache_hit"
    } else {
        "cache_state_unavailable"
    };
    let (plugin_id, plugin_version) = plugin.unwrap_or(("", ""));

    match &result {
        Ok(_) => tracing::info!(
            plugin_id,
            plugin_version,
            module_flavor = key.flavor.label(),
            wasm_digest = %blake3::Hash::from_bytes(key.wasm_digest),
            duration_ms = started.elapsed().as_millis() as u64,
            cache_state,
            disk_cache_hits,
            disk_cache_misses,
            source,
            "plugin module ready"
        ),
        Err(error) => tracing::warn!(
            plugin_id,
            plugin_version,
            module_flavor = key.flavor.label(),
            wasm_digest = %blake3::Hash::from_bytes(key.wasm_digest),
            duration_ms = started.elapsed().as_millis() as u64,
            cache_state,
            disk_cache_hits,
            disk_cache_misses,
            source,
            error = %error,
            "plugin module compilation failed"
        ),
    }
    registered_waiter.finish(result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_registry_reuses_the_same_bounded_slot() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        let first = indexer_component(&wasm).expect("first component must compile");
        let second = indexer_component(&wasm).expect("second component must reuse cache");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn bindings_are_built_once_per_module_and_never_for_other_bytes() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Binding(usize);
        let builds = AtomicUsize::new(0);
        let build = |_: &Arc<Component>| -> Result<Binding, String> {
            Ok(Binding(builds.fetch_add(1, Ordering::SeqCst)))
        };
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");
        let changed =
            wat::parse_str("(component (core module))").expect("component WAT must parse");

        let (first_component, first) =
            binding_for(ModuleFlavor::Notification, &wasm, build).expect("first binding");
        let (second_component, second) =
            binding_for(ModuleFlavor::Notification, &wasm, build).expect("reused binding");
        let (changed_component, changed) =
            binding_for(ModuleFlavor::Notification, &changed, build).expect("changed binding");
        let (_, other_flavor) =
            binding_for(ModuleFlavor::ListProvider, &wasm, build).expect("other flavor binding");

        assert!(Arc::ptr_eq(&first_component, &second_component));
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first_component, &changed_component));
        assert_eq!((first.0, changed.0, other_flavor.0), (0, 1, 2));
        assert_eq!(builds.load(Ordering::SeqCst), 3);

        let failed = binding_for(
            ModuleFlavor::Subtitle,
            &wasm,
            |_| -> Result<Binding, String> { Err("refused".into()) },
        );
        assert!(failed.is_err());
        let (_, after_failure) =
            binding_for(ModuleFlavor::Subtitle, &wasm, build).expect("a failure is not retained");
        assert_eq!(after_failure.0, 3);
    }

    #[test]
    fn concurrent_requests_share_one_module_slot() {
        let wasm = Arc::new(wat::parse_str("(component)").expect("component WAT must parse"));
        let first_wasm = Arc::clone(&wasm);
        let second_wasm = Arc::clone(&wasm);

        let first = std::thread::spawn(move || archive_component(&first_wasm));
        let second = std::thread::spawn(move || archive_component(&second_wasm));
        let first = first
            .join()
            .expect("first request must not panic")
            .expect("compile");
        let second = second
            .join()
            .expect("second request must not panic")
            .expect("compile");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn registry_uses_a_per_flavor_lru() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");
        let component = indexer_component(&wasm).expect("component must compile");
        let mut registry = ModuleRegistry::default();
        let indexer_keys = (0..=MAX_READY_MODULES_PER_FLAVOR)
            .map(|index| ModuleKey {
                flavor: ModuleFlavor::Indexer,
                wasm_digest: [index as u8; 32],
            })
            .collect::<Vec<_>>();

        for key in indexer_keys.iter().take(MAX_READY_MODULES_PER_FLAVOR) {
            registry.insert_ready(key.clone(), Arc::clone(&component));
        }
        registry.touch(&indexer_keys[0]);
        registry.insert_ready(
            indexer_keys[MAX_READY_MODULES_PER_FLAVOR].clone(),
            Arc::clone(&component),
        );
        assert!(registry.entries.contains_key(&indexer_keys[0]));
        assert!(!registry.entries.contains_key(&indexer_keys[1]));

        for index in 0..MAX_READY_MODULES_PER_FLAVOR {
            registry.insert_ready(
                ModuleKey {
                    flavor: ModuleFlavor::Archive,
                    wasm_digest: [index as u8; 32],
                },
                Arc::clone(&component),
            );
        }
        assert_eq!(
            registry
                .entries
                .keys()
                .filter(|key| key.flavor == ModuleFlavor::Indexer)
                .count(),
            MAX_READY_MODULES_PER_FLAVOR
        );
        assert_eq!(
            registry
                .entries
                .keys()
                .filter(|key| key.flavor == ModuleFlavor::Archive)
                .count(),
            MAX_READY_MODULES_PER_FLAVOR
        );
    }
}
