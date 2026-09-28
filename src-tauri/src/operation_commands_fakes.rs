use super::*;
use crate::user_error::UserError;

// ===== fakes =====

#[derive(Clone, Default)]
pub(crate) struct FakeWhitelist {
    pub(crate) names: Arc<Vec<String>>,
}

impl FakeWhitelist {
    pub(crate) fn new(names: &[&str]) -> Self {
        Self {
            names: Arc::new(names.iter().map(|name| (*name).to_owned()).collect()),
        }
    }
}

impl ProcessWhitelist for FakeWhitelist {
    fn is_process_whitelisted(&self, name: &str) -> bool {
        self.names.iter().any(|entry| entry == name)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingCache {
    pub(crate) calls: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CacheCleaner for RecordingCache {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        self.calls
            .lock()
            .unwrap()
            .push(items.iter().map(|item| item.id.clone()).collect());
        Box::pin(async { clean_summary() })
    }
}

pub(crate) struct RecordingUninstall {
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
}

impl UninstallDomain for RecordingUninstall {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<crate::operations::InstalledAppIdentity>, UserError>> {
        let observed = bundle_paths
            .iter()
            .map(|path| crate::operations::InstalledAppIdentity {
                bundle_path: path.clone(),
                app_name: "Alpha".to_owned(),
                bundle_id: "com.example.Alpha".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            })
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        let observed = paths
            .iter()
            .map(|path| LiveResidue {
                path: path.clone(),
                exists: true,
            })
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, _app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        Box::pin(async { Ok(()) })
    }

    fn remove(
        &self,
        app: &crate::operations::InstalledAppIdentity,
        _residues: &[crate::operations::ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        self.calls.lock().unwrap().push(app.bundle_path.clone());
        Box::pin(async move { uninstall_report("Alpha") })
    }
}

pub(crate) struct RecordingDocker {
    pub(crate) inventory: DockerInventory,
    pub(crate) removed: Arc<Mutex<Vec<String>>>,
    pub(crate) pruned: Arc<Mutex<usize>>,
}

impl RecordingDocker {
    pub(crate) fn new(inventory: DockerInventory) -> Self {
        Self {
            inventory,
            removed: Arc::new(Mutex::new(Vec::new())),
            pruned: Arc::new(Mutex::new(0)),
        }
    }
}

impl DockerDomain for RecordingDocker {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>> {
        let snapshot = DockerInventory {
            daemon_running: true,
            images: Vec::new(),
            containers: Vec::new(),
            volumes: Vec::new(),
            builder: DockerBuilderCache {
                total_bytes: 0,
                reclaimable_bytes: 0,
            },
            reclaimable_bytes: 0,
        };
        let _ = snapshot;
        Box::pin(async move { Ok(self.build_inventory()) })
    }

    fn remove(
        &self,
        action: DockerAction,
        target: &crate::operations::DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>> {
        self.removed.lock().unwrap().push(target.id.clone());
        let _ = action;
        Box::pin(async { Ok(()) })
    }

    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>> {
        *self.pruned.lock().unwrap() += 1;
        Box::pin(async { Ok("Total reclaimed space: 0B".to_owned()) })
    }
}

impl RecordingDocker {
    pub(crate) fn build_inventory(&self) -> DockerInventory {
        DockerInventory {
            daemon_running: self.inventory.daemon_running,
            images: self.inventory.images.clone(),
            containers: self.inventory.containers.clone(),
            volumes: self.inventory.volumes.clone(),
            builder: DockerBuilderCache {
                total_bytes: self.inventory.builder.total_bytes,
                reclaimable_bytes: self.inventory.builder.reclaimable_bytes,
            },
            reclaimable_bytes: self.inventory.reclaimable_bytes,
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingObserver {
    pub(crate) observed: Arc<Mutex<Vec<Vec<u32>>>>,
    pub(crate) live: Vec<LiveProcess>,
}

impl ProcessObserver for RecordingObserver {
    fn observe(&mut self, pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        self.observed.lock().unwrap().push(pids.to_vec());
        Ok(self.live.clone())
    }
}

#[derive(Default)]
pub(crate) struct RecordingSignaller {
    pub(crate) calls: Arc<Mutex<Vec<(u32, ProcessMode)>>>,
}

impl ProcessSignaller for RecordingSignaller {
    fn terminate(
        &mut self,
        target: &crate::operations::ProcessTarget,
        mode: ProcessMode,
    ) -> KillOutcome {
        self.calls.lock().unwrap().push((target.identity.pid, mode));
        KillOutcome::Success
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingHistory {
    pub(crate) entries: Arc<Mutex<Vec<OperationHistoryEntry>>>,
}

impl HistorySink for RecordingHistory {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError> {
        self.entries.lock().unwrap().push(entry.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct FailingHistory {
    pub(crate) attempts: Arc<Mutex<Vec<OperationHistoryEntry>>>,
}

impl FailingHistory {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl HistorySink for FailingHistory {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError> {
        self.attempts.lock().unwrap().push(entry.clone());
        Err(UserError::new(
            crate::user_error::ErrorCode::INTERNAL,
            "历史数据库不可写",
        ))
    }
}

pub(crate) struct Harness {
    pub(crate) store: Mutex<OperationStore>,
    pub(crate) cache: RecordingCache,
    pub(crate) uninstall: RecordingUninstall,
    pub(crate) docker: RecordingDocker,
    pub(crate) app_quit: RecordingAppQuitter,
    pub(crate) history: RecordingHistory,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self::with_inventory(docker_inventory_fixture())
    }

    pub(crate) fn with_inventory(inventory: DockerInventory) -> Self {
        Self {
            store: Mutex::new(OperationStore::new()),
            cache: RecordingCache::default(),
            uninstall: RecordingUninstall {
                calls: Arc::new(Mutex::new(Vec::new())),
            },
            docker: RecordingDocker::new(inventory),
            app_quit: RecordingAppQuitter::with_live(Vec::new()),
            history: RecordingHistory::default(),
        }
    }

    pub(crate) fn domains(
        &self,
    ) -> DomainServices<'_, RecordingCache, RecordingUninstall, RecordingAppQuitter, RecordingDocker>
    {
        DomainServices {
            cache: &self.cache,
            uninstall: &self.uninstall,
            app_quit: &self.app_quit,
            docker: &self.docker,
        }
    }
}

#[derive(Default)]
pub(crate) struct RecordingAppQuitter {
    pub(crate) live: Vec<InstalledAppIdentity>,
    pub(crate) quit_names: Arc<Mutex<Vec<String>>>,
    pub(crate) failures: Vec<UserError>,
}

impl RecordingAppQuitter {
    pub(crate) fn with_live(live: Vec<InstalledAppIdentity>) -> Self {
        Self {
            live,
            quit_names: Arc::new(Mutex::new(Vec::new())),
            failures: Vec::new(),
        }
    }
}

impl crate::operation_executor::AppQuitter for RecordingAppQuitter {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> crate::operation_executor::DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>>
    {
        let observed: Vec<InstalledAppIdentity> = self
            .live
            .iter()
            .filter(|app| bundle_paths.contains(&app.bundle_path))
            .cloned()
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(
        &self,
        app: &InstalledAppIdentity,
    ) -> crate::operation_executor::DomainFuture<'_, Result<(), UserError>> {
        self.quit_names.lock().unwrap().push(app.app_name.clone());
        let failure = self
            .failures
            .iter()
            .find(|name| **name == app.app_name)
            .cloned();
        Box::pin(async move {
            match failure {
                Some(message) => Err(message),
                None => Ok(()),
            }
        })
    }
}

pub(crate) struct TerminationProbe {
    pub(crate) observed: Arc<Mutex<Vec<Vec<u32>>>>,
    pub(crate) calls: Arc<Mutex<Vec<(u32, ProcessMode)>>>,
    observer: RecordingObserver,
    signaller: RecordingSignaller,
}

impl TerminationProbe {
    pub(crate) fn new(live: Vec<LiveProcess>) -> Self {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        Self {
            observer: RecordingObserver {
                observed: Arc::clone(&observed),
                live,
            },
            signaller: RecordingSignaller {
                calls: Arc::clone(&calls),
            },
            observed,
            calls,
        }
    }

    pub(crate) fn services(&mut self) -> ProcessServices<'_> {
        ProcessServices {
            observer: &mut self.observer,
            signaller: &mut self.signaller,
        }
    }
}
