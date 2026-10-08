import type { Dict } from "./zh-CN";

export const en: Dict = {
  common: {
    appName: "MacSlim",
    tagline: "One-click Mac optimization",
    scan: "Scan",
    rescan: "Rescan",
    cancel: "Cancel",
    confirm: "Confirm",
    close: "Close",
    back: "Back",
    save: "Save",
    remove: "Remove",
    add: "Add",
    loading: "Loading...",
    scanning: "Scanning...",
    optimizing: "Optimizing...",
    cleaning: "Cleaning...",
    version: "Version",
    notice_irreversible: "This action cannot be undone",
  },

  // Result wording (honest across views).
  // `reclaimed` only appears when measured; `null` (unmeasured) never
  // masquerades as freed space — it says "deleted / moved to Trash, not yet freed".
  result: {
    reclaimed: "Measured {size} reclaimed",
    deletedUnmeasured: "Deleted {count} items (space freed unmeasured)",
    trashedPending: "Moved to Trash: {size} — not yet freed",
  },

  nav: {
    scan: "Smart Scan",
    process: "Processes",
    applications: "Applications",
    cache: "Cache",
    uninstaller: "Uninstaller",
    history: "History",
    settings: "Settings",
  },

  welcome: {
    title: "Welcome to MacSlim",
    subtitle:
      "A one-click Mac system maintenance tool.\nCleans residual processes and developer caches to keep your Mac snappy.",
    featureProcessTitle: "Process Cleanup",
    featureProcessDesc: "Find zombies, idle, and resource-hog processes",
    featureCacheTitle: "Cache Cleanup",
    featureCacheDesc: "NPM · Docker · Xcode · Homebrew",
    featureSafetyTitle: "Safe & Auditable",
    featureSafetyDesc: "Rule-driven · native commands · path allowlist",
    cta: "Start Scanning",
    footer: "Runs 100% locally · no data uploaded · no AI APIs",
  },

  health: {
    title: "System Health",
    subtitle: "Live CPU / Memory / Disk",
    reading: "Reading...",
    normal: "Normal",
    warning: "Needs attention",
    critical: "Under pressure",
    cpu: "CPU",
    memory: "Memory",
    disk: "Disk",
  },

  scan: {
    processListTitle: "Optimizable Processes",
    itemsCount: "{count} items",
    noProcesses: "No processes to optimize — your Mac is healthy",
    oneClick: "Optimize",
    selectedCount: "{count} selected",
    killSuccess: "Terminated {count} processes",
    killPartial: ", {failed} failed",
    whitelistAdded: "{name} added to whitelist; hidden from future scans",
    confirmProtectedTitle: "Selection contains protected processes",
    confirmProtectedMessage:
      "Protected processes (app main processes, multi-process family components, started under 10 minutes ago) can only be force-terminated — the whole process tree gets SIGKILLed.",
    forceTerminate: "Force terminate anyway",
    protectedHint: "Protected:",
    scanFailed: "Scan failed: {error}",
    optimizeFailed: "Optimization failed: {error}",
    whitelistTooltip: "Whitelist (never scan this process again)",
  },

  process: {
    totalRows: "Total",
    totalMemory: "Memory",
    totalCpu: "CPU",
    selectedRows: "Selected",
    viewTree: "Tree",
    viewFlat: "Flat",
    viewTreeTitle: "Tree view: grouped by parent/child",
    viewFlatTitle: "Flat view: sorted by CPU / memory",
    portsOnlyOn: "Ports only",
    portsOnlyOff: "Show all processes",
    portsOnlyTitle: "Only processes listening on ports",
    searchPlaceholder: "Search name, PID, path or port...",
    columnName: "Process",
    columnUptime: "Uptime",
    columnPid: "PID",
    columnAction: "Actions",
    expand: "Expand",
    collapse: "Collapse",
    protectedCheckboxTitle: "Protected: asks for confirmation",
    protectedDefault: "Protected process",
    listeningPorts: "Listening on: {ports}",
    noMatch: "No process matches \"{query}\"",
    noProcesses: "No visible processes",
    terminateSelected: "Terminate selected ({count})",
    protectedHint: "Protected rows can only be force-terminated (with confirmation)",
    masTerminateUnsupported:
      "The App Store build runs in the system sandbox and cannot terminate other apps. This page still shows per-process memory and CPU usage.",
    whitelistLocked: "Whitelisted processes are never terminated",
    failedTitle: "These processes could not be terminated:",
    confirmProtectedTitle: "Force-terminate protected processes?",
    confirmProtectedMessage:
      "These processes are flagged as protected. Force termination may crash apps, lose data, or destabilize the system:",
    forceTerminate: "Force terminate anyway",
    killSuccess: "Terminated {count} processes",
    killPartial: ", {failed} failed",

    // Sent from the backend as i18n keys (ProcessInfo.reason_key and friends).
    // Placeholder names must stay in sync with zh-CN.ts.
    /** The "protected (...)" fragment appended to the scan-row reason line. */
    protectedInline: "protected ({reason})",
    /** Listening-ports note. {ports} reads like "3000/8080" or "3000/8080/9000 and 5 more". */
    portsNote: "ports {ports} (a service is running here — please confirm)",
    /** Trailing count when more than three ports are listed. */
    portsMore: "and {count} more",

    reason: {
      zombie: "Zombie process (exited, waiting for its parent to reap it)",
      zombieVetoedParentOfOthers:
        "Zombie process (it is the parent of another process), not selected by default",
      zombieVetoedMultiProcessComponent:
        "Zombie process (part of a known multi-process app), not selected by default",
      zombieVetoedYoungProcess:
        "Zombie process (started less than 10 minutes ago), not selected by default",
      highCpu: "High CPU usage",
      highMemory: "High memory usage",
      idle: "Running for {uptime_min} minutes · idle for a long time",
      devToolHighMemory: "Developer tool process, using a lot of memory",
      devTool: "Developer tool process (review it yourself)",
      vetoedHighUsageMultiProcess:
        "Component of a multi-process app — clearing it will likely crash the app",
      vetoedHighUsageMainProcess:
        "Main process of an app — clearing it will likely crash the app",
    },
    protect: {
      parentOfOthers: "It is the parent of another process (an app's main process)",
      multiProcessComponent:
        "Part of a known multi-process app — that is how those apps are built",
      youngProcess:
        "Started less than 10 minutes ago — you may be using it right now",
      whitelisted: "On your whitelist — not recommended to terminate",
    },
    status: {
      idle: "Idle",
      run: "Running",
      sleep: "Sleeping",
      stop: "Stopped",
      zombie: "Zombie",
      tracing: "Traced",
      dead: "Dead",
      wakekill: "Waking (kill)",
      waking: "Waking",
      parked: "Parked",
      lockBlocked: "Lock blocked",
      diskSleep: "Disk sleep",
      unknown: "Unknown",
    },
    name: {
      webkitWebContent: "WebKit Web Content",
      webkitNetworking: "WebKit Networking",
      webkitGpu: "WebKit GPU",
    },
  },

  app: {
    appCount: "Apps",
    hideSystem: "Hide system apps",
    searchPlaceholder: "Search app name, bundle ID or path...",
    systemBadge: "System",
    whitelistBadge: "Whitelist",
    cautionBadge: "Caution",
    childProcesses: "Children",
    listening: "Listening: {ports}",
    mainBadge: "Main",
    quit: "Quit",
    quitTitle: "Let the app quit on its own (it can save first)",
    forceQuit: "Force Quit",
    forceQuitTitle:
      "Terminate the whole process tree (irreversible, unsaved work is lost)",
    quitSuccess: "Asked {name} to quit ({count} apps confirmed quit)",
    quitFailed: "{name} did not respond to the quit request: {error}",
    forceQuitSuccess: "Force-terminated {count} processes of {name}",
    whitelistLocked: "This app has whitelisted processes and will not be terminated",
    noMatch: "No app matches the filters",
    noRunning: "No running apps",
    confirmForceTitle: "Force quit {name}?",
    confirmForceMessage:
      "This app has protected processes. Force quitting kills the whole process tree and cannot be undone.",
    confirmForceCounts: "{protected} protected, {whitelisted} whitelisted processes.",
    confirmForce: "Force quit anyway",
  },

  kind: {
    zombie: "Zombie",
    idle: "Idle",
    hog: "Resource Hog",
    dev: "Dev Tool",
    system: "System",
    foreground: "Foreground",
  },

  risk: {
    safe: "Safe",
    low: "Low Risk",
    dev: "Dev",
    notice: "Caution",
  },

  safety: {
    all: "All",
    safe: "Safe to Clean",
    checkFirst: "Check First",
  },

  opError: {
    stale: "The operation expired or the snapshot was refreshed — rescan and try again: {error}",
    history:
      "The action ran, but the history entry could not be written — check the History tab: {error}",
    forceOnly: "Protected processes can only be force-terminated: {error}",
    whitelisted:
      "Whitelisted processes are never terminated — remove it in Settings first: {error}",
    failed: "Action failed: {error}",
  },

  // 错误 / 结果消息：一枚后端 ErrorCode 对应一条词条（`error.<code>`）。
  // 后端发 code + 插值参数，译文只在这里；`internal` 刻意不在这里 ——
  // 它是 `From<String>` 的兜底，正常流程不该出现。
  error: {
    admin_privileges_required:
      "Administrator privileges are required to delete this folder: {reason}",
    app_ambiguous_name:
      "Apps with the same name cannot be told apart — uninstall them separately",
    app_bundle_id_changed: "The bundle id of {app} changed — rescan",
    app_child_not_in_app:
      "The child process {process} of {app} does not belong to that app — rescan",
    app_child_selection_mismatch:
      "The child-process selection keys do not match the snapshot",
    app_gone: "The app {app} no longer exists — rescan",
    app_name_changed: "The name of {app} changed — rescan",
    app_no_quittable_process: "The target app has no process to quit — rescan",
    app_no_terminable_process:
      "The app {app} has no process to terminate — rescan",
    app_selected_twice: "The same app was selected twice",
    app_selection_count_mismatch:
      "The app selection key count does not match the snapshot",
    folder_access_panel_failed:
      "Could not open the folder picker. Check macOS automation permissions in System Settings.",
    folder_access_bookmark_failed:
      "Could not create an access credential for the selected folder. It may have been moved or deleted.",
    folder_access_store_failed: "Could not save the folder authorization. Please try again.",
    authorization_cancelled: "The authorization was cancelled",
    blocking_channel_plan_mismatch:
      "The blocking channel only accepts process and app-termination plans",
    cache_ancestor_check_failed: "Could not inspect the path: {reason}",
    cache_busy_app_skipped:
      "{app} is running, so the cleanup was skipped to avoid damage",
    cache_command_exit_nonzero:
      "The cleanup command exited abnormally: {output}",
    cache_command_failed: "The cleanup command could not be started: {reason}",
    cache_docker_action_mismatch:
      "The Docker cache action does not match the item",
    cache_item_missing_path:
      "The cache item has no usable cleanup path: {reason}",
    cache_ownership_changed: "The owner of the cache item changed",
    cache_pip_broken:
      "The pip environment is broken (its shebang points at an uninstalled Python). Run `python3 -m pip cache purge` by hand, or reinstall pip. Details: {reason}",
    cache_selection_count_mismatch:
      "The cache selection key count does not match the snapshot",
    delete_failed: "Deletion failed: {reason}",
    docker_cli_failed: "The Docker command failed: {reason}",
    docker_id_reused:
      "The Docker {kind} id {id} was taken by another resource — rescan",
    docker_inventory_changed:
      "The Docker inventory changed — rescan before cleaning",
    docker_not_running: "Docker is not running — start it first",
    docker_prune_rejects_selection:
      "Docker prune does not take resource selections",
    docker_prune_rejects_target: "Docker prune does not take resource targets",
    docker_referenced_changed:
      "The reference state of the Docker {kind} {name} changed — rescan",
    docker_resource_gone: "The Docker {kind} {name} is already gone — rescan",
    docker_selection_count_mismatch:
      "The {label} selection key count does not match the snapshot",
    docker_selection_type_mismatch:
      "The Docker selection has the wrong resource kind",
    history_storage_unavailable:
      "Local storage is unavailable, so the history entry was not written ({operation} · {target}) — the result is not audited",
    history_write_failed:
      "The action failed and the history entry could not be written either ({operation} · {target}): {failure} — check the History tab",
    kill_already_gone: "The process was already gone",
    kill_failed: "Failed: {reason}",
    kill_permission_denied:
      "Permission denied — usually a root or system process, which MacSlim should not even see",
    kill_respawned:
      "The original process was terminated, but a supervisor immediately restarted it as PID {new_pid} ({name}). Stop it from its launcher (launchd agent / pm2 / nvm / Cursor / VS Code), or add the process name to the whitelist to hide it.",
    kill_still_alive:
      "SIGKILL was sent but the system still reports the process as alive — it may be a zombie or kernel-protected",
    kill_terminated: "Terminated",
    move_to_trash_failed: "Moving the item to the Trash failed: {reason}",
    no_docker_targets: "No Docker resources to delete",
    no_quittable_app_targets: "No app targets to quit",
    no_terminable_process_targets: "No process targets to terminate",
    no_uninstallable_app_targets: "No app targets to uninstall",
    operation_id_mismatch: "The plan does not match the requested operation id",
    operation_lock_broken: "The operation store lock is no longer usable",
    operation_missing_snapshot: "The operation has no snapshot attached",
    operation_owner_empty: "The operation owner is empty",
    operation_owner_mismatch: "This operation belongs to another window",
    operation_snapshot_stale: "The snapshot behind this operation has expired",
    operation_time_invalid: "The operation timestamp is invalid",
    operation_used_or_expired: "The operation was already used or has expired",
    path_not_whitelisted:
      "The path is not on the whitelist — refusing to delete: {path}",
    plan_kind_mismatch: "The plan is not a {kind} plan",
    pre_delete_recheck_failed:
      "The pre-delete re-check failed — refusing to delete: {path}",
    process_execution_failed: "The process operation failed: {reason}",
    process_gone: "The process (PID {pid}) is already gone — rescan",
    process_identity_changed:
      "The process identity changed (PID {pid}) — rescan",
    process_pid_reused: "PID {pid} was reused by another process — rescan",
    process_plan_requires_blocking:
      "Process plans must run on the blocking channel",
      process_termination_unsupported:
        "The App Store build runs in the system sandbox and cannot terminate other apps. Cache cleaning and app uninstall are unaffected.",
    process_protection_changed:
      "The protection state changed (PID {pid}) — rescan",
    protected_force_only: "Protected processes can only be force-terminated",
    random_id_failed: "Could not generate a random id",
    refuse_symlink: "Refusing to clean a symlink: {reason}",
    refuse_symlink_ancestor:
      "Refusing to clean a path that has a symlinked ancestor",
    residue_batch_empty: "The residue batch is empty",
    residue_batch_missing_app_key:
      "A residue batch is missing its app selection key",
    residue_gone: "The residue {path} is already gone — rescan",
    residue_not_in_app:
      "The residue {path} does not belong to that app — rescan",
    residue_not_in_selected_app:
      "A selected residue does not belong to the selected app",
    residue_path_changed: "The residue path {path} changed — rescan",
    residue_path_duplicated: "Duplicate residue path: {path}",
    residue_path_out_of_scope:
      "The residue path {path} is outside the allowed cleanup scope — refused",
    residue_recheck_count_mismatch:
      "The residue re-check returned a different count — rescan",
    residue_scan_failed: "Scanning residues failed: {reason}",
    residue_selection_count_mismatch:
      "The residue selection key count does not match the snapshot",
    residue_snapshot_empty: "The residue snapshot is empty",
    residue_unknown_app_key:
      "The residue snapshot references an unknown app selection key",
    result_not_cache_summary: "The result is not a cache-clean summary",
    result_not_kill_report: "The result is not a process-kill report",
    selection_duplicated: "The same item was selected twice",
    selection_empty: "The selection is empty",
    selection_key_generation_failed:
      "Could not generate a random selection key",
    selection_missing: "The selected item no longer exists",
    snapshot_dedicated_entry_required:
      "This snapshot kind has to be registered through its dedicated path",
    snapshot_payload_mismatch: "The snapshot kind does not match its payload",
    snapshot_stale: "The snapshot does not exist or has expired",
    stale_missing_canonical:
      "The stale cache item has no canonical-path snapshot",
    stale_missing_ownership: "The stale cache item has no ownership snapshot",
    stale_path_changed: "The stale path changed",
    stale_path_kind_invalid:
      "The stale node_modules path is of an invalid kind",
    stale_path_missing_parent: "The stale path has no parent directory",
    stale_path_not_dir: "The stale node_modules path is not a directory",
    stale_path_not_node_modules: "The stale path must point at node_modules",
    stale_path_outside_projects:
      "The stale path is outside the allowed project roots",
    system_app_cannot_uninstall: "System apps cannot be uninstalled",
    whitelisted_app_not_quit: "Whitelisted processes are not asked to quit",
    whitelisted_process_cannot_terminate:
      "Whitelisted processes are never terminated",
  },

  opConfirm: {
    title: "Confirm this cleanup",
    items: "{count} targets involved",
    estimated: "Estimated reclaim: {size}",
    validity: "Valid for {minutes} more minutes — rescan after it expires",
    irreversible:
      "This action cannot be undone: nothing goes to the Trash, you will have to download / rebuild / pull it again.",
    largeWarning: "This reclaims more than 10 GB — make sure you really want to delete it.",
    confirm: "Run it",
  },

  // Scan stage progress. Stage names come straight from the backend, no mapping here.
  scanProgress: {
    current: "Scanning: {stage}",
    working: "Scanning…",
    count: "{done}/{total} done",
    found: "{size} found",
  },

  cache: {
    title: "Developer Caches",
    subtitle: "NPM / Docker / Xcode / Homebrew / Cargo and more",
    // MAS edition: Docker needs the CLI and Homebrew needs `brew`; neither
    // is reachable from the sandbox, so naming them here would be a promise
    // this build cannot keep. These six are exactly what the grant list offers.
    titleMas: "App & Developer Caches",
    subtitleMas: "App caches / logs / Xcode / npm / Cargo / Trash",
    scanning: "Scanning caches...",
    freeable: "Reclaimable",
    cleaningStage: "Reclaiming space",
    cleaningLive: "Cleaning {count} items, reclaiming up to {size}",
    cleanCta: "Clean {size} ({count})",
    preparingCta: "Calculating…",
    preparingHint: "Measuring actual reclaimable space item by item — takes ~15s with many items",
    clean: "Clean",
    cleanSuccess: "Cleanup complete — freed {size}",
    // Honest wording when reclamation could not be measured: report what was
    // deleted, never claim it as freed.
    cleanSuccessDeleted: "Cleanup complete — deleted {size} (space freed unmeasured)",
    cleanSuccessDetail: "{count} succeeded, {failed} failed",
    releaseLabel: "Freed this run",
    deletedUnmeasuredLabel: "Deleted (space freed unmeasured)",
    successItems: "{count} succeeded",
    failItems: "{count} failed",
    groupCount: "{count} items · {size}",
    noAccess:
      "No folder has been authorized yet, so no cache is visible. Authorize a folder and scan again.",
    noCleanable: "We checked the folders you authorized — nothing there is safe to clean.",
    noItems: "No caches to clean. Your Mac is spotless.",
    partialFail: "Some items failed to clean",
    notifyTitle: "MacSlim cleanup complete",
    notifyBody: "Freed {size} across {count} items",
    notifyBodyUnmeasured: "Cleaned {count} items; freed space could not be measured",

    // Sent from the backend as i18n keys (CacheItem.label_key / description_key).
    // Placeholder names must stay in sync with zh-CN.ts.
    item: {
      npmCache: "Global NPM cache",
      pnpmStore: "PNPM Store",
      yarnCache: "Global Yarn cache",
      dockerReclaimableImages: "Reclaimable Docker images",
      dockerBuildCache: "Docker build cache",
      dockerStoppedContainers: "Stopped containers ({count})",
      dockerUnreferencedVolumes: "Unreferenced Docker volumes",
      dockerStaleImages: "Docker images unused for 3 months ({count})",
      staleNodeModules: "node_modules untouched for 6 months",
      homebrewCache: "Homebrew package cache",
      xcodeDerivedData: "Xcode DerivedData",
      xcodeSimulatorCaches: "iOS Simulator caches",
      xcodeIosDeviceSupport: "iOS device support files",
      xcodeArchives: "Xcode Archives",
      cocoapodsCache: "CocoaPods cache",
      cargoRegistryCache: "Cargo download cache",
      pipCache: "Pip cache",
      goBuildCache: "Go build cache",
      appCache: "{app} cache",
      appCacheRunning: "{app} cache (running)",
      appLogs: "App logs",
      crashReports: "Crash reports",
      trash: "Trash",
    },
    desc: {
      npmCache:
        "Packages npm has downloaded. The next install fetches them again.",
      pnpmStore:
        "pnpm's global store. Packages no project references can be pruned.",
      yarnCache: "Packages Yarn has downloaded.",
      dockerReclaimableImages:
        "Dangling images and image layers no container references.",
      dockerBuildCache: "Intermediate layers produced by docker build.",
      dockerStoppedContainers: "Exited containers and the layers they wrote.",
      dockerUnreferencedVolumes: "Anonymous volumes that no container uses.",
      dockerStaleImages:
        "Non-dangling images, referenced by no container, created or pulled more than 90 days ago.",
      staleNodeModules: "Target: {path}",
      homebrewCache:
        "Downloaded bottle files and older package versions. Installed tools are untouched.",
      xcodeDerivedData:
        "Intermediate Xcode build products. Xcode regenerates them when the project reopens.",
      xcodeSimulatorCaches: "Temporary CoreSimulator caches.",
      xcodeIosDeviceSupport:
        "iOS symbol files used for debugging on a physical device.",
      xcodeArchives:
        "Archive history, which may include production builds. Confirm before cleaning.",
      cocoapodsCache: "Specs and pods CocoaPods has downloaded.",
      cargoRegistryCache:
        "Downloaded .crate archives. Only the cache is removed, extracted sources are untouched.",
      pipCache: "Wheels and source distributions pip has downloaded.",
      goBuildCache: "Go build output cache. Downloaded modules are untouched.",
      appCache: "App-local cache. The app rebuilds it when it next runs.",
      devToolPnpm: "pnpm cache. Dependencies download again after cleanup.",
      devToolPlaywright:
        "Playwright browser binaries. They download again after cleanup.",
      devToolGoBuild: "Go build cache. The next build is slower after cleanup.",
      devToolHomebrew: "Bottle files Homebrew has downloaded.",
      appLogs: "Log files written by apps. Usually not worth keeping.",
      crashReports:
        "Diagnostic files written when an app crashes. Usually no longer useful for debugging.",
      trash: "Files that were deleted but not emptied from the Trash.",
    },
  },

  history: {
    title: "Operation History",
    subtitle:
      "Local log of all process and cache cleanup operations. Metadata only, nothing uploaded.",
    empty: "No operations yet",
    opCache: "Cache cleanup",
    opProcess: "Process termination",
    opAppTerminate: "Forced app quit",
    opAppGracefulQuit: "Graceful app quit",
    opUninstall: "App uninstall",
    opDocker: "Docker cleanup",
    // 结构化计数的本地化渲染（见 HistoryView）。
    // 后端同时下发拼好的中文 target/detail 作兜底，
    // 这里给双语界面用。
    target: {
      cache: "{count} cache items",
      process: "{count} processes",
      app_terminate: "{count} processes",
      app_graceful_quit: "{count} apps",
      uninstall: "{count} apps",
      docker: "{count} Docker items",
      cache_one: "1 cache item",
      process_one: "1 process",
      app_terminate_one: "1 process",
      app_graceful_quit_one: "1 app",
      uninstall_one: "1 app",
      docker_one: "1 Docker item",
    },
    detail: {
      cache: "{ok} succeeded, {fail} failed",
      process: "{ok} succeeded, {fail} failed",
      app_terminate: "{ok} succeeded, {fail} failed",
      app_graceful_quit: "{ok} quit, {fail} failed",
      uninstall: "{ok} moved, {fail} failed",
      docker: "{ok} succeeded, {fail} failed",
    },
    // Right-hand counter column, honest wording (see HistoryView):
    // uninstall = moved to Trash (space not yet freed, so no green +);
    // deleted-but-unmeasured = "deleted" only, never claimed as freed.
    trashed: "{size} → Trash",
    deletedUnmeasured: "Deleted {size} (unmeasured)",
  },

  // Operation summaries (from PreparedOperation.summary_key / summary_params).
  // Conditional variants get their own key: no "possibly empty" fragment inside
  // a template — that never survives English word order.
  opSummary: {
    cache: "Ready to clean {count} cache items, reclaiming about {size}",
    processForce: "Ready to force-terminate {count} processes",
    processGraceful: "Ready to gracefully quit {count} processes",
    appTerminate:
      "Ready to terminate {processes} processes across {apps} apps (the whole process tree is killed — unsaved work is lost)",
    appGracefulQuit:
      "Ready to gracefully quit {apps} apps, {processes} processes in total (each app is asked to quit, so save first)",
    uninstall:
      "Ready to uninstall {apps} apps and {residues} leftover items, reclaiming about {size}",
    uninstallQuitFirst:
      "Ready to uninstall {apps} apps and {residues} leftover items, reclaiming about {size}; running apps are quit first",
    dockerPrune:
      "Ready to clean {count} reclaimable Docker resources (dangling images, stopped containers, unreferenced volumes), reclaiming about {size}. This cannot be undone",
    dockerRemoveImage:
      "Ready to remove {count} Docker images, reclaiming about {size}",
    dockerRemoveContainer:
      "Ready to remove {count} Docker containers, reclaiming about {size}",
    dockerRemoveVolume:
      "Ready to remove {count} Docker volumes, reclaiming about {size}",
  },

  // The 16 scan stages (mirrored one-for-one from the backend stage table).
  scanStage: {
    npmCache: "NPM cache",
    pnpmCache: "PNPM cache",
    yarnCache: "Yarn cache",
    dockerImagesAndContainers: "Docker images and containers",
    dockerUnusedImages: "Unused Docker images",
    staleNodeModules: "Idle node_modules",
    homebrewCache: "Homebrew cache",
    xcodeCache: "Xcode cache",
    cocoapodsCache: "CocoaPods cache",
    cargoCache: "Cargo cache",
    pipCache: "Pip cache",
    goModuleCache: "Go module cache",
    appCache: "App caches",
    appLogs: "App logs",
    crashReports: "Crash reports",
    trash: "Trash",
  },

  // Cache-page group badges (the `__i18n__:`-marked entries in CATEGORY_LABELS).
  cacheCategory: {
    system: "System",
  },

  // Relative times and the absolute-date fallback (history / whitelist lists).
  timeFormat: {
    justNow: "just now",
    minutesAgo: "{min} min ago",
    hoursAgo: "{h} h ago",
    daysAgo: "{day} d ago",
  },

  access: {
    title: "Authorize folders to clean",
    subtitle:
      "The App Store edition runs inside the system sandbox and can only read folders you authorize yourself. One-time authorization stays valid, and you can revoke it at any time.",
    grant: "Authorize",
    granting: "Pick a folder in the dialog...",
    revoke: "Revoke",
    grantedHint: "Authorized folders take effect on the next scan.",
    target: {
      userCaches: "App caches",
      userLogs: "App logs",
      xcode: "Xcode build cache",
      npm: "npm cache",
      cargo: "Rust toolchain cache",
      trash: "Trash",
    },
  },

  settings: {
    fda: {
      title: "Full Disk Access",
      granted: "Granted — all cleanup and process features are available",
      needUserCache:
        "Cache cleanup and process management need Full Disk Access. Add MacSlim in System Settings → Privacy & Security → Full Disk Access.",
      needSystem:
        "Process enumeration needs Full Disk Access. Add MacSlim in System Settings → Privacy & Security → Full Disk Access.",
      needBoth:
        "Cache cleanup and process management need Full Disk Access. Add MacSlim in System Settings → Privacy & Security → Full Disk Access.",
      openSettings: "Open System Settings",
      manualHint:
        "If the button does nothing, open System Settings → Privacy & Security → Full Disk Access manually, drag MacSlim in, and turn it on.",
      openFailed:
        "Could not open System Settings automatically. Go to System Settings → Privacy & Security → Full Disk Access manually.",
      // App Store build only. Do not mention granting access — it does not
      // help here. And never label this a "lite/limited" edition: App Store
      // review guideline 4.0 lists "not enough functionality" as the top
      // removal reason, and users read a self-applied label as a bait.
      sandboxed:
        "The App Store edition runs inside the system sandbox and cannot read your user cache folders. That is a platform limit — granting access does not change it. Process monitoring, system health, app size analysis and uninstall are unaffected.",
    },
    general: "General",
    generalDesc: "Behavior toggles",
    autostart: "Launch at login",
    autostartDesc:
      "Start MacSlim at macOS login and minimize to the menu bar",
    notifyClean: "Cleanup notifications",
    notifyCleanDesc: "Show a macOS notification when cleanup completes",
    cleanupSound: "Cleanup sound",
    cleanupSoundDesc: "Play a chime when cleanup finishes; turn off for silent mode",
    notifyRequestPrompt: "Notifications not permitted — click to request",
    language: "Interface Language",
    languageDesc: "Switch between Chinese and English (restart required)",
    languageAuto: "System",
    languageZh: "中文",
    languageEn: "English",
    updates: "Check for Updates",
    updatesDesc: "Manually check whether a newer MacSlim is available",
    updatesCheck: "Check",
    updatesChecking: "Checking...",
    updatesLatest: "You are on the latest version ({version})",
    updatesAvailable: "Version {version} available",
    updatesDownload: "Download & Restart",
    updatesDownloading: "Downloading...",
    updatesError: "Check failed: {error}",
    whitelist: "Custom Whitelist",
    whitelistCount:
      "{count} items · whitelisted items are never scanned or cleaned",
    whitelistEmpty:
      "No custom whitelist yet. Click the shield icon in the scan list to add.",
    whitelistKindProcess: "Process name",
    whitelistKindPath: "Cache path",
    whitelistKindProcessPH: "e.g. Chrome",
    whitelistKindPathPH: "e.g. ~/.npm",
    whitelistNotePH: "Note (optional)",
    whitelistBadgeProcess: "Process",
    whitelistBadgePath: "Path",
    about: "About MacSlim",
    aboutLine1:
      "{version} · rule-driven · local storage · open-source friendly",
    aboutLine2: "No data uploaded · no LLM · no ads",
    aboutDb: "Data: ~/Library/Application Support/MacSlim/macslim.db",
    aboutCli: "CLI: ~/MacSlim/src-tauri/target/debug/macslim-cli",
  },

  placeholder: {
    processTitle: "Processes",
    processDesc:
      "Advanced filters, quick whitelist, port inspection. Coming in M3.",
    cacheTitle: "Cache",
    cacheDesc:
      "Deep clean NPM / Docker / Xcode / Homebrew. Coming in M2.",
    settingsTitle: "Settings",
    settingsDesc:
      "Launch at login, auto monitoring, cleanup thresholds. Coming in M3.",
    buildingDesc: "This module is under construction and will be available in a future release.",
  },

  uninstaller: {
    scanning: "Scanning installed apps...",
    scanComplete: "Scan complete",
    noApps: "No uninstallable apps found",
    appSize: "App size",
    residueSize: "Residue size",
    totalSize: "Total size",
    search: "Search app name or Bundle ID...",
    sandboxNotice: "The App Store edition runs inside the macOS sandbox and cannot remove apps — a platform limit that no permission lifts. This page still shows how much space each app occupies, largest first.",
    hideSystem: "Hide system apps",
    systemApp: "Core system app, not recommended to uninstall",
    selectedCount: "{count} apps selected",
    estimatedFree: "Estimated free {size}",
    uninstallSelected: "Uninstall selected",
    confirmTitle: "Confirm uninstall",
    confirmMessage: "Files will be moved to Trash. This cannot be undone after emptying Trash.",
    confirmLargeTitle: "Large uninstall confirmation",
    confirmLargeMessage: "Total size exceeds 10GB, please confirm again",
    appRunning: "App is running",
    quitAndUninstall: "Quit & Uninstall",
    forceQuitAndUninstall: "Force Quit & Uninstall",
    cancel: "Cancel",
    confirm: "Confirm Uninstall",
    uninstalling: "Uninstalling...",
    complete: "Uninstall complete",
    freedSpace: "Moved to Trash — {size}",
    cleanedFiles: "{count} files cleaned",
    failedFiles: "{count} files failed",
    residueIncomplete: "Residue scan may be incomplete",
    devToolData: "Developer tool data",
    selectAll: "Select all",
    deselectAll: "Deselect all",
    quitFailedButRemoved: "The app was removed, but quitting it failed: {error}",
  },

  docker: {
    title: "Docker Cache & Resources",
    subtitleOff: "Docker on macOS runs inside a Linux VM, so host directory sizes don't reliably map to image/volume/container usage. We use the docker CLI for accurate scanning.",
    subtitleOn: "Using docker CLI to identify reclaimable images, containers, volumes, and build cache.",
    notRunning: "Docker is not running or not installed. Start Docker Desktop to see reclaimable items.",
    loading: "Loading Docker status...",
    refresh: "Refresh",
    pruneAll: "Clean Reclaimable",
    pruneIrreversible: "Removes every dangling image, stopped container, build cache and unreferenced volume; nothing can be restored afterwards.",
    pruneDone: "Reclaimable Docker resources were cleaned",
    reclaimable: "Reclaimable",
    images: "Images",
    containers: "Containers",
    volumes: "Volumes",
    buildCache: "Build Cache",
    dangling: "{count} dangling",
    stopped: "{count} stopped",
    unused: "{count} unused",
    danglingBadge: "Dangling",
    inUseBadge: "In Use",
    runningBadge: "Running",
    stoppedBadge: "Stopped",
    deleteImage: "Delete image",
    deleteContainer: "Delete container",
    deleteContainerForce: "Force delete container",
    deleteVolume: "Delete volume",
    deleteVolumeDisabled: "In use, cannot delete",
    done: "Completed {count} items",
    partialFail: "{count} items failed: {error}",
  },
};
