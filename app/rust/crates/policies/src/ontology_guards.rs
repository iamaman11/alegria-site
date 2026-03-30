use petgraph::algo::is_cyclic_directed;
use petgraph::graphmap::DiGraphMap;

pub fn detect_is_a_cycle(edges: &[(String, String)]) -> bool {
    let mut graph = DiGraphMap::<&str, ()>::new();
    for (parent, child) in edges {
        graph.add_edge(parent.as_str(), child.as_str(), ());
    }
    is_cyclic_directed(&graph)
}
