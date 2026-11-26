// SPDX-License-Identifier: MIT OR Apache-2.0
//! Strongly connected components of the dependency graph (iterative Tarjan).

/// The strongly connected components of a graph given as adjacency lists (`deps[n]` are the
/// nodes `n` depends on), in dependency order: every component comes after the components
/// it depends on. Iterative, so a chain of 100,000 cells cannot overflow the stack.
pub fn components(deps: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = deps.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut next = 0usize;
    // (node, position of the next edge to follow)
    let mut work: Vec<(usize, usize)> = Vec::new();

    for root in 0..n {
        if index.get(root).copied() != Some(usize::MAX) {
            continue;
        }
        work.push((root, 0));
        while let Some(&(v, ei)) = work.last() {
            if ei == 0 {
                if let Some(slot) = index.get_mut(v) {
                    *slot = next;
                }
                if let Some(slot) = low.get_mut(v) {
                    *slot = next;
                }
                next += 1;
                stack.push(v);
                if let Some(slot) = on_stack.get_mut(v) {
                    *slot = true;
                }
            }
            let edges = deps.get(v).map_or(&[][..], Vec::as_slice);
            if let Some(&w) = edges.get(ei) {
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                let w_index = index.get(w).copied().unwrap_or(usize::MAX);
                if w_index == usize::MAX {
                    // The first visit of `w` is recognised by its edge counter being 0.
                    work.push((w, 0));
                } else if on_stack.get(w).copied().unwrap_or(false) {
                    let lw = low.get(v).copied().unwrap_or(0).min(w_index);
                    if let Some(slot) = low.get_mut(v) {
                        *slot = lw;
                    }
                }
            } else {
                work.pop();
                let lv = low.get(v).copied().unwrap_or(0);
                if let Some(&(parent, _)) = work.last() {
                    let lp = low.get(parent).copied().unwrap_or(0).min(lv);
                    if let Some(slot) = low.get_mut(parent) {
                        *slot = lp;
                    }
                }
                if lv == index.get(v).copied().unwrap_or(usize::MAX) {
                    let mut comp = Vec::new();
                    while let Some(w) = stack.pop() {
                        if let Some(slot) = on_stack.get_mut(w) {
                            *slot = false;
                        }
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comp.sort_unstable();
                    out.push(comp);
                }
            }
        }
    }
    out
}

/// The shortest cycle through `start` inside `members` (a strongly connected component),
/// as the nodes in dependency order starting with `start`. A self-dependency is `[start]`.
pub fn cycle_through(deps: &[Vec<usize>], members: &[usize], start: usize) -> Vec<usize> {
    let in_comp = |n: usize| members.binary_search(&n).is_ok();
    let edges = |n: usize| deps.get(n).map_or(&[][..], Vec::as_slice);
    if edges(start).contains(&start) {
        return vec![start];
    }
    let mut parent: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(start);
    while let Some(v) = queue.pop_front() {
        for &w in edges(v) {
            if !in_comp(w) {
                continue;
            }
            if w == start {
                let mut path = vec![v];
                let mut cur = v;
                while cur != start {
                    match parent.get(&cur) {
                        Some(&p) => {
                            path.push(p);
                            cur = p;
                        }
                        None => break,
                    }
                }
                path.reverse();
                return path;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(w) {
                e.insert(v);
                queue.push_back(w);
            }
        }
    }
    vec![start]
}
