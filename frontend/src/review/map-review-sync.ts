/**
 * 地图联动审核（Map-Linked Review）纯函数计算层：
 * - 计算提案（或整条依赖链）在主地图 `IncidenceGraph` 上的高亮子图与 1 跳邻接上下文；
 * - 支持从主地图上的节点/连线 `domain_id` 反向定位到对应的提案与卡片条目。
 */
import type { IncidenceGraph, IncidenceNode } from "../graph/projection";
import { SEMANTIC_PROPOSAL_KINDS } from "../semantic/labels";
import { dependencyGroups, keyOf, titleOf, type PendingChangeSet } from "./proposal-model";

export interface ProposalMapScope {
  nodeIds: string[];
  relationIds: string[];
  description: string;
}

function collectTouchedDomainIds(proposal: PendingChangeSet): Set<string> {
  const ids = new Set<string>();
  for (const stagedId of Object.values(proposal.result.staged_ids ?? {})) {
    if (typeof stagedId === "string" && stagedId) ids.add(stagedId);
  }
  for (const obj of proposal.proposal.proposed_objects ?? []) {
    if (obj.target_id) ids.add(obj.target_id);
  }
  for (const rel of proposal.proposal.proposed_relations ?? []) {
    if (rel.target_id) ids.add(rel.target_id);
  }
  return ids;
}

/**
 * 计算主地图在地图联动审核模式下应高亮的节点与边：
 * 包含所选提案（或所在依赖链全部提案）直接创建/修改/退役的对象，以及它们在主地图上的 1 跳上下文邻居。
 */
export function computeProposalMapScope(
  graph: IncidenceGraph,
  items: PendingChangeSet[],
  activeItem: PendingChangeSet,
  includeDependencyChain: boolean,
): ProposalMapScope {
  const activeGroup = includeDependencyChain
    ? dependencyGroups(items).find((g) => g.some((c) => keyOf(c) === keyOf(activeItem))) ?? [activeItem]
    : [activeItem];

  const touchedDomains = new Set<string>();
  for (const member of activeGroup) {
    for (const id of collectTouchedDomainIds(member)) touchedDomains.add(id);
  }

  const nodeById = new Map<string, IncidenceNode>(graph.nodes.map((n) => [n.id, n]));
  const primaryEntities = new Set<string>();
  const primaryRelations = new Set<string>();

  for (const node of graph.nodes) {
    const matched = (node.domain_id && touchedDomains.has(node.domain_id)) || touchedDomains.has(node.id);
    if (!matched) continue;
    if (node.element_kind === "relation") primaryRelations.add(node.id);
    else primaryEntities.add(node.id);
  }

  // Group incidence edges by relation vertex id
  const relEndpoints = new Map<string, string[]>();
  for (const edge of graph.edges) {
    const src = nodeById.get(edge.source);
    const dst = nodeById.get(edge.target);
    if (!src || !dst) continue;
    if (src.element_kind === "relation" && dst.element_kind !== "relation") {
      if (edge.role === "evidence" || dst.domain_kind === "SourceSpan") continue;
      const list = relEndpoints.get(src.id) ?? [];
      list.push(dst.id);
      relEndpoints.set(src.id, list);
    } else if (dst.element_kind === "relation" && src.element_kind !== "relation") {
      if (edge.role === "evidence" || src.domain_kind === "SourceSpan") continue;
      const list = relEndpoints.get(dst.id) ?? [];
      list.push(src.id);
      relEndpoints.set(dst.id, list);
    }
  }

  const inScopeNodes = new Set<string>(primaryEntities);
  const inScopeRelations = new Set<string>(primaryRelations);

  for (const [relNodeId, endpoints] of relEndpoints) {
    const relIsPrimary = primaryRelations.has(relNodeId);
    const anyEndpointPrimary = endpoints.some((ep) => primaryEntities.has(ep));
    if (relIsPrimary || anyEndpointPrimary) {
      inScopeRelations.add(relNodeId);
      const relDomain = nodeById.get(relNodeId)?.domain_id;
      if (relDomain) inScopeRelations.add(relDomain);
      inScopeNodes.add(relNodeId);
      for (const ep of endpoints) {
        if (nodeById.get(ep)?.domain_kind !== "SourceSpan") {
          inScopeNodes.add(ep);
        }
      }
    }
  }

  const label = activeGroup.length > 1
    ? `地图联动审核：依赖组（${activeGroup.length} 份提案）· 当前「${titleOf(activeItem)}」`
    : `地图联动审核：「${titleOf(activeItem)}」v${activeItem.version}`;

  return {
    nodeIds: [...inScopeNodes],
    relationIds: [...inScopeRelations],
    description: label,
  };
}

/**
 * 给定地图对象 `domainId`（以及可选 `changeSetId`），定位它在待审队列中的提案 key 与审核卡片 target key。
 */
export function findProposalTargetForDomainObject(
  items: PendingChangeSet[],
  domainId: string | null | undefined,
  preferredChangeSetId?: string | null,
): { changeSetKey: string; focusTarget: string | null } | null {
  const ordered = preferredChangeSetId
    ? [
        ...items.filter((i) => i.changeSetId === preferredChangeSetId),
        ...items.filter((i) => i.changeSetId !== preferredChangeSetId),
      ]
    : items;

  if (!domainId) {
    const first = ordered[0];
    return first ? { changeSetKey: keyOf(first), focusTarget: null } : null;
  }

  for (const item of ordered) {
    const stagedIds = item.result.staged_ids ?? {};
    for (const obj of item.proposal.proposed_objects ?? []) {
      const stagedId = stagedIds[obj.local_ref];
      if (stagedId === domainId || obj.target_id === domainId) {
        if (SEMANTIC_PROPOSAL_KINDS.has(obj.kind)) {
          return { changeSetKey: keyOf(item), focusTarget: `semantic:${obj.local_ref}` };
        }
        if (obj.kind === "source_unit") {
          return { changeSetKey: keyOf(item), focusTarget: `unit:${obj.local_ref}` };
        }
      }
    }
    for (const rel of item.proposal.proposed_relations ?? []) {
      const stagedId = stagedIds[rel.local_ref];
      if (stagedId === domainId || rel.target_id === domainId) {
        return { changeSetKey: keyOf(item), focusTarget: `relation:${rel.local_ref}` };
      }
    }
  }

  if (preferredChangeSetId) {
    const match = items.find((i) => i.changeSetId === preferredChangeSetId);
    if (match) return { changeSetKey: keyOf(match), focusTarget: null };
  }
  return null;
}
