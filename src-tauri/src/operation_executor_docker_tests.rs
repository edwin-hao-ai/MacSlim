use super::*;
use crate::docker::{
    DockerBuilderCache, DockerContainer, DockerImage, DockerInventory, DockerVolume,
};
use crate::operations::{
    ConsumedPlan, DockerAction, DockerResourceKind, DockerTarget, OperationStore, ProcessMode,
};
use crate::user_error::UserError;
use std::sync::{Arc, Mutex};

fn image(id: &str, repository: &str, tag: &str, in_use: bool) -> DockerImage {
    DockerImage {
        id: id.to_owned(),
        repository: repository.to_owned(),
        tag: tag.to_owned(),
        size_bytes: 100,
        created: "2026-01-15 10:20:30 +0800 CST".to_owned(),
        dangling: false,
        in_use,
        selection_key: String::new(),
    }
}

fn container(id: &str, name: &str, running: bool) -> DockerContainer {
    DockerContainer {
        id: id.to_owned(),
        name: name.to_owned(),
        image: "nginx:latest".to_owned(),
        status: "Up 2 days".to_owned(),
        running,
        size_bytes: 50,
        created: "2026-01-15 10:20:30 +0800 CST".to_owned(),
        selection_key: String::new(),
    }
}

fn volume(name: &str, in_use: bool) -> DockerVolume {
    DockerVolume {
        name: name.to_owned(),
        driver: "local".to_owned(),
        size_bytes: 20,
        in_use,
        selection_key: String::new(),
    }
}

fn inventory(
    daemon_running: bool,
    images: Vec<DockerImage>,
    containers: Vec<DockerContainer>,
    volumes: Vec<DockerVolume>,
) -> DockerInventory {
    DockerInventory {
        daemon_running,
        images,
        containers,
        volumes,
        builder: DockerBuilderCache {
            total_bytes: 0,
            reclaimable_bytes: 0,
        },
        reclaimable_bytes: 0,
    }
}

fn image_target(id: &str, name: &str, referenced: bool) -> DockerTarget {
    DockerTarget {
        resource_type: DockerResourceKind::Image,
        id: id.to_owned(),
        name: name.to_owned(),
        size_bytes: 100,
        referenced,
        reclaimable: false,
    }
}

fn target(kind: DockerResourceKind, id: &str, name: &str, referenced: bool) -> DockerTarget {
    DockerTarget {
        resource_type: kind,
        id: id.to_owned(),
        name: name.to_owned(),
        size_bytes: 50,
        referenced,
        reclaimable: false,
    }
}

#[derive(Clone, Default)]
struct FakeDockerDomain {
    inventory: Arc<Mutex<Option<DockerInventory>>>,
    inventory_error: Arc<Mutex<Option<UserError>>>,
    inventory_calls: Arc<Mutex<usize>>,
    remove_calls: Arc<Mutex<Vec<(DockerAction, String)>>>,
    remove_error: Arc<Mutex<Option<UserError>>>,
    prune_calls: Arc<Mutex<usize>>,
}

impl FakeDockerDomain {
    fn with_inventory(inventory: DockerInventory) -> Self {
        Self {
            inventory: Arc::new(Mutex::new(Some(inventory))),
            ..Self::default()
        }
    }

    fn with_inventory_error(message: &str) -> Self {
        let domain = Self::default();
        *domain.inventory_error.lock().unwrap() = Some(UserError::from(message));
        domain
    }

    fn with_inventory_and_remove_error(inventory: DockerInventory, message: &str) -> Self {
        let domain = Self::with_inventory(inventory);
        *domain.remove_error.lock().unwrap() = Some(UserError::from(message));
        domain
    }

    fn removed(&self) -> Vec<(DockerAction, String)> {
        self.remove_calls.lock().unwrap().clone()
    }

    fn call_counts(&self) -> (usize, usize, usize) {
        (
            *self.inventory_calls.lock().unwrap(),
            self.remove_calls.lock().unwrap().len(),
            *self.prune_calls.lock().unwrap(),
        )
    }
}

impl DockerDomain for FakeDockerDomain {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>> {
        *self.inventory_calls.lock().unwrap() += 1;
        let error = self.inventory_error.lock().unwrap().clone();
        let inventory = self.inventory.lock().unwrap().clone();
        Box::pin(async move {
            if let Some(error) = error {
                return Err(error);
            }
            inventory.ok_or_else(|| UserError::from("测试未提供 Docker 清单"))
        })
    }

    fn remove(
        &self,
        action: DockerAction,
        target: &DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>> {
        self.remove_calls
            .lock()
            .unwrap()
            .push((action, target.id.clone()));
        let error = self.remove_error.lock().unwrap().clone();
        Box::pin(async move { error.map_or(Ok(()), Err) })
    }

    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>> {
        *self.prune_calls.lock().unwrap() += 1;
        Box::pin(async { Ok("Total reclaimed space: 1GB".to_owned()) })
    }
}

fn consumed_docker_plan(
    store: &mut OperationStore,
    targets: Vec<DockerTarget>,
    action: DockerAction,
) -> ConsumedPlan {
    let registration = store.register_docker_resources(targets).unwrap();
    let keys: Vec<&str> = registration
        .selection_keys
        .iter()
        .map(String::as_str)
        .collect();
    let prepared = store
        .prepare_docker(&registration.snapshot_id, action, keys, "main")
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn consumed_process_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_process_snapshot(vec![crate::operations::ProcessTarget {
            identity: crate::operations::ProcessIdentity {
                pid: 1,
                name: "init".to_owned(),
                exe: "/sbin/init".to_owned(),
                start_time: 1,
            },
            protected: false,
            whitelisted: false,
        }])
        .unwrap();
    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

#[tokio::test]
async fn docker_plan_revalidates_inventory_then_dispatches_typed_removal() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![
            image_target("sha256:aaa", "nginx:latest", false),
            image_target("sha256:bbb", "redis:latest", false),
        ],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![
            image("sha256:aaa", "nginx", "latest", false),
            image("sha256:bbb", "redis", "latest", false),
        ],
        Vec::new(),
        Vec::new(),
    ));

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert_eq!(report.action, "删除镜像");
    assert_eq!(report.succeeded, vec!["nginx:latest", "redis:latest"]);
    assert!(report.failed.is_empty());
    assert_eq!(
        domain.removed(),
        vec![
            (DockerAction::RemoveImage, "sha256:aaa".to_owned()),
            (DockerAction::RemoveImage, "sha256:bbb".to_owned())
        ]
    );
    assert_eq!(*domain.prune_calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn docker_plan_rejects_stopped_daemon() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain =
        FakeDockerDomain::with_inventory(inventory(false, Vec::new(), Vec::new(), Vec::new()));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("Docker"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_rejects_vanished_target() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:bbb", "nginx", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已不存在"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_rejects_id_reuse_with_another_name() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "evil", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已被其他资源占用"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_rejects_reference_state_change() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", true)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("引用状态已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_rejects_container_now_running() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![target(DockerResourceKind::Container, "c1", "web", false)],
        DockerAction::RemoveContainer,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        Vec::new(),
        vec![container("c1", "web", true)],
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("引用状态已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_removes_every_matching_container() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![
            target(DockerResourceKind::Container, "c1", "web", true),
            target(DockerResourceKind::Container, "c2", "worker", false),
        ],
        DockerAction::RemoveContainer,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        Vec::new(),
        vec![
            container("c1", "web", true),
            container("c2", "worker", false),
        ],
        Vec::new(),
    ));

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert_eq!(report.succeeded, vec!["web", "worker"]);
    assert_eq!(
        domain.removed(),
        vec![
            (DockerAction::RemoveContainer, "c1".to_owned()),
            (DockerAction::RemoveContainer, "c2".to_owned())
        ]
    );
}

#[tokio::test]
async fn docker_plan_removes_volume_by_name() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![target(DockerResourceKind::Volume, "cache", "cache", false)],
        DockerAction::RemoveVolume,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        Vec::new(),
        Vec::new(),
        vec![volume("cache", false)],
    ));

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert_eq!(report.action, "删除卷");
    assert_eq!(report.succeeded, vec!["cache"]);
    assert_eq!(
        domain.removed(),
        vec![(DockerAction::RemoveVolume, "cache".to_owned())]
    );
}

#[tokio::test]
async fn docker_plan_signals_nothing_when_one_target_mismatches() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![
            image_target("sha256:aaa", "nginx:latest", false),
            image_target("sha256:bbb", "redis:latest", false),
        ],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("已不存在"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_prune_plan_only_calls_prune() {
    let mut store = OperationStore::new();
    let registration = store
        .register_docker_resources(vec![image_target("sha256:aaa", "nginx:latest", false)])
        .unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap();
    let consumed = store.consume(&prepared.operation_id, "main").unwrap();
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert_eq!(report.action, "清理 Docker");
    assert!(report.succeeded.is_empty());
    assert_eq!(report.output, "Total reclaimed space: 1GB");
    assert_eq!(*domain.prune_calls.lock().unwrap(), 1);
    assert_eq!(domain.call_counts(), (1, 0, 1));
}

#[tokio::test]
async fn docker_prune_plan_rejects_stopped_daemon() {
    let mut store = OperationStore::new();
    let registration = store.register_docker_resources(Vec::new()).unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap();
    let consumed = store.consume(&prepared.operation_id, "main").unwrap();
    let domain =
        FakeDockerDomain::with_inventory(inventory(false, Vec::new(), Vec::new(), Vec::new()));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("Docker"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_plan_records_per_target_failure_without_panicking() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![
            image_target("sha256:aaa", "nginx:latest", false),
            image_target("sha256:bbb", "redis:latest", false),
        ],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![
            image("sha256:aaa", "nginx", "latest", false),
            image("sha256:bbb", "redis", "latest", false),
        ],
        Vec::new(),
        Vec::new(),
    ));
    *domain.remove_error.lock().unwrap() = Some(UserError::from("image is being used"));

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert!(report.succeeded.is_empty());
    assert_eq!(report.failed.len(), 2);
    assert_eq!(report.failed[0].1, "image is being used");
    assert_eq!(domain.removed().len(), 2);
}

#[tokio::test]
async fn docker_plan_propagates_inventory_failure() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory_error("Cannot connect to the Docker daemon");

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert_eq!(error, "Cannot connect to the Docker daemon");
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_executor_rejects_non_docker_plan_after_consume() {
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store);
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert_eq!(error, "操作计划不是 Docker 计划");
    assert_eq!(domain.call_counts(), (0, 0, 0));
}

#[tokio::test]
async fn docker_execution_report_is_filled_from_the_real_dispatch_path() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
        DockerAction::RemoveImage,
    );
    let domain = FakeDockerDomain::with_inventory_and_remove_error(
        inventory(
            true,
            vec![image("sha256:aaa", "nginx", "latest", false)],
            Vec::new(),
            Vec::new(),
        ),
        "image is being used",
    );

    let report = execute_docker_plan(consumed, &domain).await.unwrap();

    assert_eq!(report.action, DockerAction::RemoveImage.label());
    assert!(report.succeeded.is_empty());
    assert_eq!(
        report.failed,
        vec![("nginx:latest".to_owned(), "image is being used".to_owned())]
    );
    assert!(report.output.is_empty());
    assert_eq!(domain.call_counts(), (1, 1, 0));
}

fn consumed_prune_plan(store: &mut OperationStore, targets: Vec<DockerTarget>) -> ConsumedPlan {
    let registration = store.register_docker_resources(targets).unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::Prune,
            Vec::<&str>::new(),
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

#[tokio::test]
async fn docker_prune_rejects_inventory_with_new_reclaimable_resource() {
    let mut store = OperationStore::new();
    let consumed = consumed_prune_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![
            image("sha256:aaa", "nginx", "latest", false),
            image("sha256:bbb", "redis", "latest", false),
        ],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("清单已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_prune_rejects_inventory_with_vanished_resource() {
    let mut store = OperationStore::new();
    let consumed = consumed_prune_plan(
        &mut store,
        vec![
            image_target("sha256:aaa", "nginx:latest", false),
            image_target("sha256:bbb", "redis:latest", false),
        ],
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("清单已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_prune_rejects_inventory_with_changed_reference_state() {
    let mut store = OperationStore::new();
    let consumed = consumed_prune_plan(
        &mut store,
        vec![image_target("sha256:aaa", "nginx:latest", false)],
    );
    let domain = FakeDockerDomain::with_inventory(inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", true)],
        Vec::new(),
        Vec::new(),
    ));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("清单已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}

#[tokio::test]
async fn docker_prune_rejects_inventory_with_changed_size() {
    let mut store = OperationStore::new();
    let mut registered = image_target("sha256:aaa", "nginx:latest", false);
    registered.size_bytes = 100;
    let consumed = consumed_prune_plan(&mut store, vec![registered]);
    let mut grown = image("sha256:aaa", "nginx", "latest", false);
    grown.size_bytes = 4096;
    let domain =
        FakeDockerDomain::with_inventory(inventory(true, vec![grown], Vec::new(), Vec::new()));

    let error = execute_docker_plan(consumed, &domain).await.unwrap_err();

    assert!(error.contains("清单已变化"), "{}", error);
    assert_eq!(domain.call_counts(), (1, 0, 0));
}
