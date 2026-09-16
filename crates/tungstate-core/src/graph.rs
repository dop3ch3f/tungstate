//! Topological ordering, and what is left when there is no such thing.
//!
//! Nothing here knows what an operation is. It is a graph over indices, so it
//! can be tested against hand-built knots with no policy in sight.

use std::collections::{BTreeMap, BTreeSet};

/// A dependency graph over `0..nodes`, where an edge `u -> v` reads
/// *u must happen before v*.
#[derive(Debug, Default, Clone)]
pub struct Graph {
    nodes: usize,
    after: BTreeMap<usize, BTreeSet<usize>>,
}

/// What a sort managed, and what defeated it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sorted {
    /// Nodes in an order that satisfies every edge between them.
    pub order: Vec<usize>,
    /// Nodes that could not be placed, which is exactly the set caught in
    /// cycles. Empty when the graph is acyclic.
    pub cyclic: BTreeSet<usize>,
}

impl Graph {
    /// An empty graph over `nodes` indices.
    #[must_use]
    pub fn new(nodes: usize) -> Self {
        Self {
            nodes,
            after: BTreeMap::new(),
        }
    }

    /// Require `before` to happen before `after`.
    ///
    /// A node is never made to wait for itself: an op whose source is also its
    /// destination is a no-op the planner should not have emitted, and a
    /// self-edge would turn that mistake into a false cycle that cannot be
    /// broken.
    pub fn require(&mut self, before: usize, after: usize) {
        if before != after {
            self.after.entry(before).or_default().insert(after);
        }
    }

    /// Order the nodes, reporting whatever is caught in a cycle.
    ///
    /// Kahn's algorithm, for two reasons worth the choice. The residual — what
    /// still has unmet dependencies when nothing more is ready — is *exactly*
    /// the set of nodes in cycles, which is what a caller that means to break
    /// them needs; a depth-first sort finds one back edge and leaves the rest
    /// to be reconstructed. And the ready set is a `BTreeSet` rather than a
    /// queue, so ties are broken by index rather than by the order edges
    /// happened to be added — which is what makes the same input give
    /// byte-identical output every time.
    #[must_use]
    pub fn sort(&self) -> Sorted {
        let mut waiting = vec![0_usize; self.nodes];
        for dependents in self.after.values() {
            for &node in dependents {
                waiting[node] += 1;
            }
        }

        let mut ready: BTreeSet<usize> = (0..self.nodes).filter(|n| waiting[*n] == 0).collect();
        let mut order = Vec::with_capacity(self.nodes);
        while let Some(&node) = ready.iter().next() {
            ready.remove(&node);
            order.push(node);
            if let Some(dependents) = self.after.get(&node) {
                for &dependent in dependents {
                    waiting[dependent] -= 1;
                    if waiting[dependent] == 0 {
                        ready.insert(dependent);
                    }
                }
            }
        }

        let placed: BTreeSet<usize> = order.iter().copied().collect();
        let cyclic = (0..self.nodes).filter(|n| !placed.contains(n)).collect();
        Sorted { order, cyclic }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(nodes: usize, edges: &[(usize, usize)]) -> Graph {
        let mut g = Graph::new(nodes);
        for &(before, after) in edges {
            g.require(before, after);
        }
        g
    }

    #[test]
    fn an_empty_graph_keeps_its_nodes_in_index_order() {
        assert_eq!(graph(3, &[]).sort().order, [0, 1, 2]);
    }

    #[test]
    fn a_chain_comes_out_in_its_own_order() {
        let sorted = graph(3, &[(2, 1), (1, 0)]).sort();
        assert_eq!(sorted.order, [2, 1, 0]);
        assert!(sorted.cyclic.is_empty());
    }

    #[test]
    fn independent_nodes_break_ties_by_index_not_by_insertion() {
        // The determinism the whole plan rests on. Edges added back to front
        // must not change the answer.
        let forwards = graph(4, &[(0, 3), (1, 3), (2, 3)]).sort();
        let backwards = graph(4, &[(2, 3), (1, 3), (0, 3)]).sort();
        assert_eq!(forwards.order, backwards.order);
        assert_eq!(forwards.order, [0, 1, 2, 3]);
    }

    #[test]
    fn a_two_node_cycle_is_reported_whole() {
        let sorted = graph(2, &[(0, 1), (1, 0)]).sort();
        assert!(sorted.order.is_empty());
        assert_eq!(sorted.cyclic, BTreeSet::from([0, 1]));
    }

    #[test]
    fn a_rotation_is_reported_whole_rather_than_one_edge_of_it() {
        // The reason for Kahn over a depth-first sort: all three come back,
        // not just the one back edge that closed the loop.
        let sorted = graph(3, &[(0, 1), (1, 2), (2, 0)]).sort();
        assert_eq!(sorted.cyclic, BTreeSet::from([0, 1, 2]));
    }

    #[test]
    fn what_can_be_placed_is_placed_even_when_something_cannot() {
        // 0 and 1 are a knot; 2 and 3 are a chain that has nothing to do
        // with it and must still come out ordered.
        let sorted = graph(4, &[(0, 1), (1, 0), (3, 2)]).sort();
        assert_eq!(sorted.order, [3, 2]);
        assert_eq!(sorted.cyclic, BTreeSet::from([0, 1]));
    }

    #[test]
    fn a_node_never_waits_for_itself() {
        let sorted = graph(2, &[(0, 0), (1, 1)]).sort();
        assert_eq!(sorted.order, [0, 1]);
        assert!(sorted.cyclic.is_empty());
    }

    #[test]
    fn a_repeated_edge_is_counted_once() {
        // Otherwise the in-degree never reaches zero and an ordinary graph
        // reads as a cycle.
        let sorted = graph(2, &[(0, 1), (0, 1), (0, 1)]).sort();
        assert_eq!(sorted.order, [0, 1]);
        assert!(sorted.cyclic.is_empty());
    }
}
