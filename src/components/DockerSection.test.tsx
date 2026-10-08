import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  dockerInventory: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    dockerInventory: mocks.dockerInventory,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
  };
});

vi.mock("@/i18n", async () => {
  const { fakeI18n, passthroughT } = await import("@/i18n/fake-i18n");
  return { useI18n: () => fakeI18n({ t: passthroughT }) };
});

import DockerSection from "@/components/DockerSection";

const inventory = () => ({
  snapshot_id: "snap-docker",
  expires_at_ms: 1_700_000_000_000,
  value: {
    daemon_running: true,
    images: [
      {
        selection_key: "key-image",
        id: "sha256:abc",
        repository: "nginx",
        tag: "latest",
        size_bytes: 500,
        created: "2026-01-01",
        dangling: false,
        in_use: false,
      },
    ],
    containers: [
      {
        selection_key: "key-container",
        id: "cid-1",
        name: "web",
        image: "nginx",
        status: "exited",
        running: false,
        size_bytes: 10,
        created: "2026-01-01",
      },
    ],
    volumes: [
      {
        selection_key: "key-volume",
        name: "cache-vol",
        driver: "local",
        size_bytes: 20,
        in_use: false,
      },
    ],
    builder: { total_bytes: 30, reclaimable_bytes: 30 },
    reclaimable_bytes: 560,
  },
});

const prepared = {
  operation_id: "op-docker-1",
  kind: "docker",
  expires_at_ms: 1_700_000_600_000,
  item_count: 1,
  estimated_bytes: 500,
  summary_key: "opSummary.dockerRemoveImage",
  summary_params: [
    ["count", "1"],
    ["size", "500 B"],
  ],
};

const dockerResult = {
  kind: "docker",
  value: {
    action: "remove_image",
    succeeded: ["nginx:latest"],
    failed: [],
    output: "",
    reclaimed_bytes: null,
  },
};

const confirmDialog = () => screen.findByTestId("operation-confirm-validity");
const confirmRun = async () =>
  fireEvent.click(await screen.findByRole("button", { name: "opConfirm.confirm" }));

describe("DockerSection broker flow", () => {
  beforeEach(() => {
    mocks.dockerInventory.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.dockerInventory.mockImplementation(() => Promise.resolve(inventory()));
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue(dockerResult);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("deletes a resource by key with a typed action and refetches", async () => {
    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));
    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "docker",
      snapshot_id: "snap-docker",
      action: "remove_image",
      target_keys: ["key-image"],
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-docker-1");
    await waitFor(() => expect(mocks.dockerInventory).toHaveBeenCalledTimes(2));
  });

  it("keeps raw image ids, container ids and volume names off the wire", async () => {
    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));

    await confirmDialog();
    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(Object.keys(request).sort()).toEqual([
      "action",
      "snapshot_id",
      "target_keys",
      "type",
    ]);
    const payload = JSON.stringify(request);
    expect(payload).not.toContain("sha256:abc");
    expect(payload).not.toContain("nginx");
  });

  it("prunes without any target keys", async () => {
    mocks.prepareOperation.mockResolvedValue({ ...prepared, operation_id: "op-prune" });
    mocks.executeOperation.mockResolvedValue({
      kind: "docker",
      value: { action: "prune", succeeded: [], failed: [], output: "Total reclaimed space: 0B", reclaimed_bytes: null },
    });

    render(() => <DockerSection />);
    fireEvent.click(await screen.findByRole("button", { name: "docker.pruneAll" }));
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "docker",
      snapshot_id: "snap-docker",
      action: "prune",
      target_keys: [],
    });
  });

  it("keeps the prune confirmation gate in place", async () => {
    render(() => <DockerSection />);
    fireEvent.click(await screen.findByRole("button", { name: "docker.pruneAll" }));

    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("warns that a prune cannot be undone", async () => {
    mocks.prepareOperation.mockResolvedValue({ ...prepared, operation_id: "op-prune" });

    render(() => <DockerSection />);
    fireEvent.click(await screen.findByRole("button", { name: "docker.pruneAll" }));

    expect(await screen.findByText("docker.pruneIrreversible")).toBeTruthy();
    expect(screen.getByText("opConfirm.irreversible")).toBeTruthy();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("demands the irreversible confirmation above 10GB of backend estimate", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      estimated_bytes: 11 * 1024 * 1024 * 1024,
    });

    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));

    expect(await screen.findByText("opConfirm.largeWarning")).toBeTruthy();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("never deletes a resource when the user cancels the confirmation", async () => {
    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));
    await confirmDialog();

    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("shows the backend summary before deleting a resource", async () => {
    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));

    expect(await screen.findByText("opSummary.dockerRemoveImage")).toBeTruthy();
    expect(screen.getByText("opConfirm.items")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated")).toBeTruthy();
  });

  it("reports a changed docker inventory and refetches", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "docker_inventory_changed",
      message: "Docker 资源清单已变化，请重新扫描后再清理",
      params: [],
    });

    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));

    await screen.findByText("opError.stale");
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await waitFor(() => expect(mocks.dockerInventory).toHaveBeenCalledTimes(2));
  });

  it("ignores a result tagged as another operation kind", async () => {
    mocks.executeOperation.mockResolvedValue({
      kind: "cache",
      value: { reports: [], deleted_bytes: 0, reclaimed_bytes: null, success_count: 0, fail_count: 0 },
    });

    render(() => <DockerSection />);
    fireEvent.click(await screen.findByTitle("docker.deleteImage"));
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(screen.queryByText("docker.done")).toBeNull();
  });
});
