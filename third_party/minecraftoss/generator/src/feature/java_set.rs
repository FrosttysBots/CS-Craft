//! `java.util.HashSet<BlockPos>` iteration order.
//!
//! Tree placement and several features iterate hash sets of positions and
//! draw randomness per element, so results depend on the JDK's bucket order:
//! a power-of-two table (initially 16, grown at 3/4 load), insertion order
//! within a bucket, and red-black tree bins for long chains once the table
//! reaches 64. Adapted from the Stage 1 engine's `java_hashset.rs`; hashes
//! compare as signed ints like `HashMap.putTreeVal`.

use minecraftoss_core::BlockPos;

/// `HashMap.hash(BlockPos)`: `Vec3i.hashCode` spread by its high half.
pub fn spread(pos: BlockPos) -> i32 {
    let h = pos.y.wrapping_add(pos.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(pos.x);
    h ^ ((h as u32) >> 16) as i32
}

#[derive(Clone, Debug)]
struct Node {
    pos: BlockPos,
    parent: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    red: bool,
}

#[derive(Clone, Debug, Default)]
struct Tree {
    nodes: Vec<Node>,
    root: Option<usize>,
}

fn less(a: BlockPos, b: BlockPos) -> bool {
    // Equal hashes fall back to identity order in Java; position order here.
    (spread(a), a) < (spread(b), b)
}

impl Tree {
    /// `TreeNode.treeify`: inserts in chain order, then moves the root to the front.
    fn build(order: &mut Vec<BlockPos>) -> Self {
        let mut tree = Self::default();
        for &pos in order.iter() {
            tree.insert_node(pos);
        }
        tree.move_root_front(order);
        tree
    }

    fn move_root_front(&self, order: &mut Vec<BlockPos>) {
        let Some(root) = self.root else { return };
        let pos = self.nodes[root].pos;
        if let Some(i) = order.iter().position(|&p| p == pos) {
            order.remove(i);
            order.insert(0, pos);
        }
    }

    /// `putTreeVal`: the new node follows its parent in the chain.
    fn insert(&mut self, pos: BlockPos, order: &mut Vec<BlockPos>) {
        match self.insert_node(pos) {
            Some(parent) => {
                let parent_pos = self.nodes[parent].pos;
                let i = order.iter().position(|&p| p == parent_pos).expect("tree parent in chain");
                order.insert(i + 1, pos);
            }
            None => order.push(pos),
        }
        self.move_root_front(order);
    }

    fn insert_node(&mut self, pos: BlockPos) -> Option<usize> {
        let mut parent = None;
        let mut cursor = self.root;
        let mut go_left = false;
        while let Some(i) = cursor {
            parent = Some(i);
            go_left = less(pos, self.nodes[i].pos);
            cursor = if go_left { self.nodes[i].left } else { self.nodes[i].right };
        }
        let index = self.nodes.len();
        self.nodes.push(Node { pos, parent, left: None, right: None, red: parent.is_some() });
        match parent {
            Some(p) => {
                if go_left {
                    self.nodes[p].left = Some(index);
                } else {
                    self.nodes[p].right = Some(index);
                }
                self.balance(index);
            }
            None => self.root = Some(index),
        }
        parent
    }

    fn rotate_left(&mut self, node: usize) {
        let right = self.nodes[node].right.expect("right child");
        self.nodes[node].right = self.nodes[right].left;
        if let Some(c) = self.nodes[right].left {
            self.nodes[c].parent = Some(node);
        }
        let parent = self.nodes[node].parent;
        self.nodes[right].parent = parent;
        match parent {
            Some(p) if self.nodes[p].left == Some(node) => self.nodes[p].left = Some(right),
            Some(p) => self.nodes[p].right = Some(right),
            None => self.root = Some(right),
        }
        self.nodes[right].left = Some(node);
        self.nodes[node].parent = Some(right);
    }

    fn rotate_right(&mut self, node: usize) {
        let left = self.nodes[node].left.expect("left child");
        self.nodes[node].left = self.nodes[left].right;
        if let Some(c) = self.nodes[left].right {
            self.nodes[c].parent = Some(node);
        }
        let parent = self.nodes[node].parent;
        self.nodes[left].parent = parent;
        match parent {
            Some(p) if self.nodes[p].left == Some(node) => self.nodes[p].left = Some(left),
            Some(p) => self.nodes[p].right = Some(left),
            None => self.root = Some(left),
        }
        self.nodes[left].right = Some(node);
        self.nodes[node].parent = Some(left);
    }

    fn balance(&mut self, mut node: usize) {
        while let Some(parent) = self.nodes[node].parent {
            if !self.nodes[parent].red {
                break;
            }
            let grand = self.nodes[parent].parent.expect("red parent has a parent");
            let parent_is_left = self.nodes[grand].left == Some(parent);
            let uncle = if parent_is_left { self.nodes[grand].right } else { self.nodes[grand].left };
            if let Some(u) = uncle.filter(|&u| self.nodes[u].red) {
                self.nodes[parent].red = false;
                self.nodes[u].red = false;
                self.nodes[grand].red = true;
                node = grand;
                continue;
            }
            if parent_is_left {
                if self.nodes[parent].right == Some(node) {
                    node = parent;
                    self.rotate_left(node);
                }
                let parent = self.nodes[node].parent.expect("parent");
                let grand = self.nodes[parent].parent.expect("grandparent");
                self.nodes[parent].red = false;
                self.nodes[grand].red = true;
                self.rotate_right(grand);
            } else {
                if self.nodes[parent].left == Some(node) {
                    node = parent;
                    self.rotate_right(node);
                }
                let parent = self.nodes[node].parent.expect("parent");
                let grand = self.nodes[parent].parent.expect("grandparent");
                self.nodes[parent].red = false;
                self.nodes[grand].red = true;
                self.rotate_left(grand);
            }
        }
        let root = self.root.expect("root");
        self.nodes[root].red = false;
    }
}

const NONE: u32 = u32::MAX;

/// A chain link in `JavaHashSet::entries`.
#[derive(Clone, Debug)]
struct Entry {
    pos: BlockPos,
    next: u32,
}

/// A bin that grew into a red-black tree: its chain order and the tree.
#[derive(Clone, Debug)]
struct TreeBin {
    bin: usize,
    order: Vec<BlockPos>,
    tree: Tree,
}

/// A `HashSet<BlockPos>` that reproduces the JDK iteration order.
///
/// Ordinary bins are linked chains in one entry arena (no allocation per
/// bin; growing relinks entries in place). The rare bins that turn into
/// trees keep their chain as a vector beside the tree.
#[derive(Clone, Debug)]
pub struct JavaHashSet {
    heads: Vec<u32>,
    tails: Vec<u32>,
    entries: Vec<Entry>,
    trees: Vec<TreeBin>,
    len: usize,
}

impl Default for JavaHashSet {
    fn default() -> Self {
        Self::new()
    }
}

/// `JavaHashSet::iter`.
pub struct Iter<'a> {
    set: &'a JavaHashSet,
    bin: usize,
    link: u32,
    tree: Option<std::slice::Iter<'a, BlockPos>>,
}

impl Iterator for Iter<'_> {
    type Item = BlockPos;

    fn next(&mut self) -> Option<BlockPos> {
        loop {
            if let Some(tree) = &mut self.tree {
                if let Some(&pos) = tree.next() {
                    return Some(pos);
                }
                self.tree = None;
                self.bin += 1;
                self.link = NONE;
                continue;
            }
            if self.link != NONE {
                let entry = &self.set.entries[self.link as usize];
                self.link = entry.next;
                if self.link == NONE {
                    self.bin += 1;
                }
                return Some(entry.pos);
            }
            if self.bin >= self.set.heads.len() {
                return None;
            }
            if let Some(t) = self.set.tree_of(self.bin) {
                self.tree = Some(self.set.trees[t].order.iter());
                continue;
            }
            self.link = self.set.heads[self.bin];
            if self.link == NONE {
                self.bin += 1;
            }
        }
    }
}

impl JavaHashSet {
    pub fn new() -> Self {
        Self { heads: vec![NONE; 16], tails: vec![NONE; 16], entries: Vec::new(), trees: Vec::new(), len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn bucket(&self, pos: BlockPos) -> usize {
        spread(pos) as u32 as usize & (self.heads.len() - 1)
    }

    fn tree_of(&self, bin: usize) -> Option<usize> {
        if self.trees.is_empty() {
            return None;
        }
        self.trees.iter().position(|t| t.bin == bin)
    }

    fn chain_contains(&self, bin: usize, pos: BlockPos) -> bool {
        let mut link = self.heads[bin];
        while link != NONE {
            let entry = &self.entries[link as usize];
            if entry.pos == pos {
                return true;
            }
            link = entry.next;
        }
        false
    }

    fn chain_len(&self, bin: usize) -> usize {
        let (mut link, mut n) = (self.heads[bin], 0);
        while link != NONE {
            n += 1;
            link = self.entries[link as usize].next;
        }
        n
    }

    /// Appends an arena entry to the end of a chain in `heads`/`tails`.
    fn link(entries: &mut [Entry], heads: &mut [u32], tails: &mut [u32], bin: usize, index: u32) {
        entries[index as usize].next = NONE;
        if tails[bin] == NONE {
            heads[bin] = index;
        } else {
            entries[tails[bin] as usize].next = index;
        }
        tails[bin] = index;
    }

    fn push_entry(&mut self, bin: usize, pos: BlockPos) {
        let index = self.entries.len() as u32;
        self.entries.push(Entry { pos, next: NONE });
        Self::link(&mut self.entries, &mut self.heads, &mut self.tails, bin, index);
    }

    pub fn contains(&self, pos: BlockPos) -> bool {
        let b = self.bucket(pos);
        match self.tree_of(b) {
            Some(t) => self.trees[t].order.contains(&pos),
            None => self.chain_contains(b, pos),
        }
    }

    /// `HashSet.add`: false when already present.
    pub fn insert(&mut self, pos: BlockPos) -> bool {
        let b = self.bucket(pos);
        if let Some(t) = self.tree_of(b) {
            let bin = &mut self.trees[t];
            if bin.order.contains(&pos) {
                return false;
            }
            bin.tree.insert(pos, &mut bin.order);
        } else {
            if self.chain_contains(b, pos) {
                return false;
            }
            let before = self.chain_len(b);
            self.push_entry(b, pos);
            if before >= 8 {
                // treeifyBin: small tables grow instead.
                if self.heads.len() < 64 {
                    self.resize();
                } else {
                    let mut order = Vec::with_capacity(before + 1);
                    let mut link = self.heads[b];
                    while link != NONE {
                        order.push(self.entries[link as usize].pos);
                        link = self.entries[link as usize].next;
                    }
                    self.heads[b] = NONE;
                    self.tails[b] = NONE;
                    let tree = Tree::build(&mut order);
                    self.trees.push(TreeBin { bin: b, order, tree });
                }
            }
        }
        self.len += 1;
        if self.len > self.heads.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    fn resize(&mut self) {
        let cap = self.heads.len();
        let mut heads = vec![NONE; cap * 2];
        let mut tails = vec![NONE; cap * 2];
        let old_trees = std::mem::take(&mut self.trees);
        let mut trees = Vec::new();
        for i in 0..cap {
            if let Some(t) = old_trees.iter().position(|t| t.bin == i) {
                let TreeBin { order, tree, .. } = old_trees[t].clone();
                let (mut lo, mut hi) = (Vec::new(), Vec::new());
                for pos in order {
                    if spread(pos) as u32 as usize & cap == 0 {
                        lo.push(pos);
                    } else {
                        hi.push(pos);
                    }
                }
                // TreeNode.split: a half of at most six untreeifies; a half
                // that got everything keeps the old tree and chain; otherwise
                // each large half is treeified again.
                let (lo_len, hi_len) = (lo.len(), hi.len());
                let mut old_tree = Some(tree);
                for (target, mut half, other) in [(i, lo, hi_len), (i + cap, hi, lo_len)] {
                    if half.len() > 6 {
                        let tree = if other == 0 { old_tree.take().expect("one half keeps the tree") } else { Tree::build(&mut half) };
                        trees.push(TreeBin { bin: target, order: half, tree });
                    } else {
                        for pos in half {
                            let index = self.entries.len() as u32;
                            self.entries.push(Entry { pos, next: NONE });
                            Self::link(&mut self.entries, &mut heads, &mut tails, target, index);
                        }
                    }
                }
                continue;
            }
            let mut link = self.heads[i];
            while link != NONE {
                let next = self.entries[link as usize].next;
                let pos = self.entries[link as usize].pos;
                let target = if spread(pos) as u32 as usize & cap == 0 { i } else { i + cap };
                Self::link(&mut self.entries, &mut heads, &mut tails, target, link);
                link = next;
            }
        }
        self.heads = heads;
        self.tails = tails;
        self.trees = trees;
    }

    /// Removes an element; the chain order of the others is kept
    /// (`HashIterator.remove` never moves a tree root).
    pub fn remove(&mut self, pos: BlockPos) -> bool {
        let b = self.bucket(pos);
        if let Some(t) = self.tree_of(b) {
            let bin = &mut self.trees[t];
            let Some(i) = bin.order.iter().position(|&p| p == pos) else {
                return false;
            };
            bin.order.remove(i);
            // The JDK deletes from the red-black tree without untreeifying;
            // a rebuild over the remaining chain approximates its shape.
            if bin.order.is_empty() {
                self.trees.remove(t);
            } else {
                bin.tree = Tree::build_keep_order(&bin.order);
            }
            self.len -= 1;
            return true;
        }
        let (mut previous, mut link) = (NONE, self.heads[b]);
        while link != NONE {
            let next = self.entries[link as usize].next;
            if self.entries[link as usize].pos == pos {
                if previous == NONE {
                    self.heads[b] = next;
                } else {
                    self.entries[previous as usize].next = next;
                }
                if self.tails[b] == link {
                    self.tails[b] = previous;
                }
                self.len -= 1;
                return true;
            }
            previous = link;
            link = next;
        }
        false
    }

    /// The first element in iteration order, removed (`iterator.next(); iterator.remove()`).
    pub fn pop_first(&mut self) -> Option<BlockPos> {
        let first = self.iter().next()?;
        self.remove(first);
        Some(first)
    }

    /// Elements in JDK iteration order.
    pub fn iter(&self) -> Iter<'_> {
        Iter { set: self, bin: 0, link: NONE, tree: None }
    }

    pub fn to_vec(&self) -> Vec<BlockPos> {
        self.iter().collect()
    }
}

impl Tree {
    fn build_keep_order(order: &[BlockPos]) -> Self {
        let mut tree = Self::default();
        for &pos in order {
            tree.insert_node(pos);
        }
        tree
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Default)]
    struct Bin {
        order: Vec<BlockPos>,
        tree: Option<Tree>,
    }

    /// The earlier one-vector-per-bin implementation, as a reference.
    #[derive(Clone, Debug)]
    pub struct OldJavaHashSet {
        bins: Vec<Bin>,
        len: usize,
    }

    impl Default for OldJavaHashSet {
        fn default() -> Self {
            Self::new()
        }
    }

    #[allow(dead_code)]
    impl OldJavaHashSet {
        pub fn new() -> Self {
            Self { bins: vec![Bin::default(); 16], len: 0 }
        }

        pub fn len(&self) -> usize {
            self.len
        }

        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        fn bucket(&self, pos: BlockPos) -> usize {
            spread(pos) as u32 as usize & (self.bins.len() - 1)
        }

        pub fn contains(&self, pos: BlockPos) -> bool {
            self.bins[self.bucket(pos)].order.contains(&pos)
        }

        /// `HashSet.add`: false when already present.
        pub fn insert(&mut self, pos: BlockPos) -> bool {
            let b = self.bucket(pos);
            if self.bins[b].order.contains(&pos) {
                return false;
            }
            let bin = &mut self.bins[b];
            if let Some(tree) = &mut bin.tree {
                tree.insert(pos, &mut bin.order);
            } else {
                let before = bin.order.len();
                bin.order.push(pos);
                if before >= 8 {
                    // treeifyBin: small tables grow instead.
                    if self.bins.len() < 64 {
                        self.resize();
                    } else {
                        let bin = &mut self.bins[b];
                        bin.tree = Some(Tree::build(&mut bin.order));
                    }
                }
            }
            self.len += 1;
            if self.len > self.bins.len() * 3 / 4 {
                self.resize();
            }
            true
        }

        fn resize(&mut self) {
            let old = std::mem::take(&mut self.bins);
            let cap = old.len();
            let mut next = vec![Bin::default(); cap * 2];
            for (i, bin) in old.into_iter().enumerate() {
                let tree = bin.tree;
                for pos in bin.order {
                    let target = if spread(pos) as u32 as usize & cap == 0 { i } else { i + cap };
                    next[target].order.push(pos);
                }
                if let Some(tree) = tree {
                    // TreeNode.split: a half of at most six untreeifies; a half
                    // that got everything keeps the old tree and chain; otherwise
                    // each large half is treeified again.
                    let (lo, hi) = (next[i].order.len(), next[i + cap].order.len());
                    let mut old_tree = Some(tree);
                    for (target, count, other) in [(i, lo, hi), (i + cap, hi, lo)] {
                        if count > 6 {
                            next[target].tree = if other == 0 {
                                old_tree.take()
                            } else {
                                Some(Tree::build(&mut next[target].order))
                            };
                        }
                    }
                }
            }
            self.bins = next;
        }

        /// Removes an element; the chain order of the others is kept
        /// (`HashIterator.remove` never moves a tree root).
        pub fn remove(&mut self, pos: BlockPos) -> bool {
            let b = self.bucket(pos);
            let bin = &mut self.bins[b];
            let Some(i) = bin.order.iter().position(|&p| p == pos) else {
                return false;
            };
            bin.order.remove(i);
            if bin.tree.is_some() {
                // The JDK deletes from the red-black tree without untreeifying;
                // a rebuild over the remaining chain approximates its shape.
                bin.tree = (!bin.order.is_empty()).then(|| Tree::build_keep_order(&bin.order));
            }
            self.len -= 1;
            true
        }

        /// The first element in iteration order, removed (`iterator.next(); iterator.remove()`).
        pub fn pop_first(&mut self) -> Option<BlockPos> {
            let first = self.bins.iter().find_map(|b| b.order.first().copied())?;
            self.remove(first);
            Some(first)
        }

        /// Elements in JDK iteration order.
        pub fn iter(&self) -> impl Iterator<Item = BlockPos> + '_ {
            self.bins.iter().flat_map(|b| b.order.iter().copied())
        }

        pub fn to_vec(&self) -> Vec<BlockPos> {
            self.iter().collect()
        }
    }

    /// The arena implementation against the reference, operation by
    /// operation, including chains long enough to become trees.
    #[test]
    fn matches_the_reference_implementation() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = |bound: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % bound
        };
        for round in 0..300 {
            let (mut new, mut old) = (JavaHashSet::new(), OldJavaHashSet::new());
            let colliding = round % 3 == 0;
            for _ in 0..(50 + next(400)) {
                let pos = if colliding {
                    // Equal hashes: x = c - 961 z keeps (31 z) * 31 + x fixed.
                    let z = next(40) as i32 - 20;
                    BlockPos::new(7 - 961 * z, 0, z)
                } else {
                    BlockPos::new(next(24) as i32 - 12, next(24) as i32 - 12, next(24) as i32 - 12)
                };
                match next(10) {
                    0..=5 => assert_eq!(new.insert(pos), old.insert(pos)),
                    6 | 7 => assert_eq!(new.remove(pos), old.remove(pos)),
                    8 => assert_eq!(new.pop_first(), old.pop_first()),
                    _ => assert_eq!(new.contains(pos), old.contains(pos)),
                }
                assert_eq!(new.len(), old.len());
                assert_eq!(new.to_vec(), old.to_vec(), "round {round}");
            }
        }
    }

    #[test]
    fn small_sets_follow_bucket_then_insertion_order() {
        let mut set = JavaHashSet::new();
        let positions = [BlockPos::new(3, 0, 0), BlockPos::new(1, 0, 0), BlockPos::new(19, 0, 0), BlockPos::new(2, 0, 0)];
        for p in positions {
            assert!(set.insert(p));
        }
        assert!(!set.insert(BlockPos::new(1, 0, 0)));
        // Buckets by hash & 15: x=1 -> 1, x=2 -> 2, x=3 and x=19 -> 3 in insertion order.
        let order: Vec<i32> = set.iter().map(|p| p.x).collect();
        assert_eq!(order, vec![1, 2, 3, 19]);
    }

    #[test]
    fn negative_hashes_use_unsigned_buckets() {
        let pos = BlockPos::new(-1, -5, -3);
        let h = spread(pos);
        let mut set = JavaHashSet::new();
        set.insert(pos);
        assert_eq!(set.bucket(pos), h as u32 as usize & 15);
        assert!(set.contains(pos));
        assert_eq!(set.pop_first(), Some(pos));
        assert!(set.is_empty());
    }
}
