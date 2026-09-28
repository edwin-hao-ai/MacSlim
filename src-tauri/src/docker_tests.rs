use super::*;
use crate::operations::{DockerResourceKind, LiveDockerResource, OperationStore};

fn image(id: &str, repository: &str, tag: &str, dangling: bool, in_use: bool) -> DockerImage {
    DockerImage {
        id: id.to_owned(),
        repository: repository.to_owned(),
        tag: tag.to_owned(),
        size_bytes: 1024,
        created: "2026-01-15 10:20:30 +0800 CST".to_owned(),
        dangling,
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
        size_bytes: 512,
        created: "2026-01-15 10:20:30 +0800 CST".to_owned(),
        selection_key: String::new(),
    }
}

fn volume(name: &str, in_use: bool) -> DockerVolume {
    DockerVolume {
        name: name.to_owned(),
        driver: "local".to_owned(),
        size_bytes: 256,
        in_use,
        selection_key: String::new(),
    }
}

fn builder(total: u64, reclaimable: u64) -> DockerBuilderCache {
    DockerBuilderCache {
        total_bytes: total,
        reclaimable_bytes: reclaimable,
    }
}

fn stopped_daemon_inventory() -> DockerInventory {
    build_inventory(false, Vec::new(), Vec::new(), Vec::new(), builder(0, 0))
}

#[test]
fn docker_targets_are_derived_from_inventory_rows() {
    let targets = docker_targets(&build_inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false, false)],
        vec![container("c1", "web", true)],
        vec![volume("cache", false)],
        builder(0, 0),
    ));

    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0].resource_type, DockerResourceKind::Image);
    assert_eq!(targets[0].id, "sha256:aaa");
    assert_eq!(targets[0].name, "nginx:latest");
    assert!(!targets[0].referenced);
    assert_eq!(targets[1].resource_type, DockerResourceKind::Container);
    assert_eq!(targets[1].id, "c1");
    assert_eq!(targets[1].name, "web");
    assert!(targets[1].referenced, "运行中的容器视为被引用");
    assert_eq!(targets[2].resource_type, DockerResourceKind::Volume);
    assert_eq!(targets[2].id, "cache");
    assert!(!targets[2].referenced);
}

#[test]
fn docker_reclaimable_bytes_only_counts_dangling_stopped_and_unused() {
    let result = build_inventory(
        true,
        vec![
            image("sha256:aaa", "<none>", "<none>", true, false),
            image("sha256:bbb", "nginx", "latest", false, false),
        ],
        vec![container("c1", "web", false), container("c2", "db", true)],
        vec![volume("cache", false), volume("data", true)],
        builder(0, 100),
    );

    assert_eq!(result.reclaimable_bytes, 100 + 1024 + 512 + 256);
    assert!(result.daemon_running);
}

#[test]
fn docker_stopped_daemon_reports_empty_inventory() {
    let result = stopped_daemon_inventory();

    assert!(!result.daemon_running);
    assert!(result.images.is_empty());
    assert!(result.containers.is_empty());
    assert!(result.volumes.is_empty());
    assert_eq!(result.reclaimable_bytes, 0);
}

#[test]
fn docker_register_binds_one_opaque_key_per_row_in_order() {
    let mut store = OperationStore::new();
    let mut inventory = build_inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false, false)],
        vec![container("c1", "web", true)],
        vec![volume("cache", false)],
        builder(0, 0),
    );

    let registration = register_docker_inventory(&mut store, &mut inventory).unwrap();

    assert_eq!(registration.selection_keys.len(), 3);
    assert!(registration
        .selection_keys
        .iter()
        .all(|key| key.len() == 64));
    assert_eq!(
        inventory.images[0].selection_key,
        registration.selection_keys[0]
    );
    assert_eq!(
        inventory.containers[0].selection_key,
        registration.selection_keys[1]
    );
    assert_eq!(
        inventory.volumes[0].selection_key,
        registration.selection_keys[2]
    );
    assert_ne!(inventory.images[0].selection_key, "sha256:aaa");
}

#[test]
fn docker_live_resource_lookup_is_scoped_by_kind() {
    let inventory = build_inventory(
        true,
        vec![image("sha256:aaa", "nginx", "latest", false, true)],
        vec![container("c1", "web", true)],
        vec![volume("cache", false)],
        builder(0, 0),
    );

    assert_eq!(
        find_live_resource(&inventory, DockerResourceKind::Image, "sha256:aaa"),
        Some(LiveDockerResource {
            id: "sha256:aaa".to_owned(),
            name: "nginx:latest".to_owned(),
            referenced: true,
        })
    );
    assert_eq!(
        find_live_resource(&inventory, DockerResourceKind::Container, "sha256:aaa"),
        None
    );
    assert_eq!(
        find_live_resource(&inventory, DockerResourceKind::Volume, "cache"),
        Some(LiveDockerResource {
            id: "cache".to_owned(),
            name: "cache".to_owned(),
            referenced: false,
        })
    );
}

#[test]
fn docker_action_labels_and_resource_types_are_total() {
    assert_eq!(DockerAction::RemoveImage.label(), "删除镜像");
    assert_eq!(DockerAction::RemoveContainer.label(), "删除容器");
    assert_eq!(DockerAction::RemoveVolume.label(), "删除卷");
    assert_eq!(DockerAction::Prune.label(), "清理 Docker");
    assert_eq!(
        DockerAction::RemoveImage.resource_type(),
        Some(DockerResourceKind::Image)
    );
    assert_eq!(
        DockerAction::RemoveContainer.resource_type(),
        Some(DockerResourceKind::Container)
    );
    assert_eq!(
        DockerAction::RemoveVolume.resource_type(),
        Some(DockerResourceKind::Volume)
    );
    assert_eq!(DockerAction::Prune.resource_type(), None);
    assert_eq!(DockerResourceKind::Image.label(), "镜像");
    assert_eq!(DockerResourceKind::Container.label(), "容器");
    assert_eq!(DockerResourceKind::Volume.label(), "卷");
}

#[test]
fn docker_human_size_parsing_covers_units() {
    assert_eq!(parse_human_size("0B"), 0);
    assert_eq!(parse_human_size("0"), 0);
    assert_eq!(parse_human_size("512B"), 512);
    assert_eq!(parse_human_size("1.5KB"), 1536);
    assert_eq!(parse_human_size("2MB"), 2 * 1024 * 1024);
    assert_eq!(parse_human_size("1.5 GB"), 1536 * 1024 * 1024);
    assert_eq!(parse_human_size("unknown"), 0);
}

#[test]
fn docker_image_parsing_ignores_malformed_lines() {
    let parsed = parse_image_lines("sha256:aaa|nginx|latest|2MB|2026-01-15|", "");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "aaa");
    assert_eq!(parsed[0].repository, "nginx");
    assert_eq!(parsed[0].tag, "latest");
    assert_eq!(parsed[0].size_bytes, 2 * 1024 * 1024);
    assert!(!parsed[0].dangling);
    assert!(!parsed[0].in_use);
}

#[test]
fn docker_image_parsing_marks_dangling_and_container_reference() {
    let dangling = parse_image_lines("sha256:bbb|<none>|<none>|0B|2026-01-15|", "");
    assert!(dangling[0].dangling);

    let used = parse_image_lines("sha256:ccc|redis|latest|10MB|2026-01-15|", "redis:latest\n");
    assert!(used[0].in_use);
}

#[test]
fn docker_container_parsing_extracts_state_and_size() {
    let parsed = parse_container_lines(
        "c1|web|nginx:latest|Up 2 days|running|2MB (virtual 150MB)|2026-01-15\nc2|db|mysql|exited|exited|0B|2026-01-16",
    );

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, "c1");
    assert_eq!(parsed[0].name, "web");
    assert!(parsed[0].running);
    assert_eq!(parsed[0].size_bytes, 2 * 1024 * 1024);
    assert_eq!(parsed[1].id, "c2");
    assert!(!parsed[1].running);
}

#[test]
fn docker_volume_parsing_marks_reference_from_container_mounts() {
    let parsed = parse_volume_lines(
        "cache|local\ndata|local\nscratch|local",
        "/cache\n/var/lib/data",
    );

    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].name, "cache");
    assert!(parsed[0].in_use);
    assert!(parsed[1].in_use);
    assert!(!parsed[2].in_use);
}

#[test]
fn docker_system_operator_stays_the_production_implementation() {
    fn assert_domain<T: crate::operation_executor::DockerDomain>() {}
    assert_domain::<SystemDocker>();
}
