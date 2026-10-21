//! Collection membership uses the backend tree as its authority, including recovery after a rejected move.

import * as tree from "../ipc/tree";
import { alphabeticalOrders, type ProjectPlacement } from "../layout/LeftSidebar/projectCollectionDrop";
import { useTermStore } from "./termStore";

async function mutateTree(mutate: () => Promise<void>): Promise<void> {
  try {
    await mutate();
    useTermStore.setState({ treeMutationError: null });
  } catch (error) {
    useTermStore.setState({ treeMutationError: error instanceof Error ? error.message : String(error) });
    throw error;
  } finally {
    await useTermStore.getState().loadTree();
  }
}

export async function setProjectCollection(projectId: string, collectionId: string | null): Promise<void> {
  await mutateTree(() => tree.setProjectCollection(projectId, collectionId));
}

/** Membership first, so a rejected move leaves the project's position untouched. */
export async function placeProject(placement: ProjectPlacement): Promise<void> {
  await mutateTree(async () => {
    if (placement.changesCollection) await tree.setProjectCollection(placement.projectId, placement.collectionId);
    if (placement.sortOrder !== null) {
      await tree.moveNode("project", placement.projectId, null, null, null, placement.sortOrder);
    }
  });
}

/** Rewrites every member's order sequentially, then reloads once. */
export async function sortCollectionAlphabetically(collectionId: string | null): Promise<void> {
  const orders = alphabeticalOrders(useTermStore.getState().projects, collectionId);
  await mutateTree(async () => {
    for (const { id, sortOrder } of orders) await tree.moveNode("project", id, null, null, null, sortOrder);
  });
}
