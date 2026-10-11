//! Lane layout for the commit graph.
//!
//! Commits arrive in display order (newest first, topologically sorted). Each
//! active lane "expects" the next commit that will appear in it. For every row
//! we emit a set of line segments in row-local coordinates so the frontend can
//! draw each row independently (which keeps a virtualized list cheap).

use git2::Oid;

/// Vertical anchor points inside a row.
pub const TOP: u8 = 0;
pub const MID: u8 = 1;
pub const BOTTOM: u8 = 2;

/// Color used for the uncommitted-changes (WIP) node and its connector; drawn dashed.
pub const WIP_COLOR: u16 = u16::MAX;

/// (from_lane, from_y, to_lane, to_y, color)
pub type Segment = (u16, u8, u16, u8, u16);

pub struct RowLayout {
    pub lane: u16,
    pub color: u16,
    pub segments: Vec<Segment>,
}

#[derive(Default)]
pub struct GraphBuilder {
    lanes: Vec<Option<(Oid, u16)>>,
    next_color: u16,
    pub max_lanes: usize,
}

impl GraphBuilder {
    fn alloc_color(&mut self) -> u16 {
        let c = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        c
    }

    fn free_slot(&mut self) -> usize {
        if let Some(i) = self.lanes.iter().position(|l| l.is_none()) {
            i
        } else {
            self.lanes.push(None);
            self.lanes.len() - 1
        }
    }

    fn lane_of(&self, oid: Oid) -> Option<usize> {
        self.lanes
            .iter()
            .position(|l| matches!(l, Some((o, _)) if *o == oid))
    }

    /// Adds the virtual WIP row on top of the graph, connected to HEAD (if any).
    pub fn push_wip(&mut self, head: Option<Oid>) -> RowLayout {
        let lane = self.free_slot();
        let mut segments = Vec::new();
        match head {
            Some(h) => {
                self.lanes[lane] = Some((h, WIP_COLOR));
                segments.push((lane as u16, MID, lane as u16, BOTTOM, WIP_COLOR));
            }
            None => self.lanes[lane] = None,
        }
        self.max_lanes = self.max_lanes.max(lane + 1);
        RowLayout { lane: lane as u16, color: WIP_COLOR, segments }
    }

    pub fn push(&mut self, oid: Oid, parents: &[Oid]) -> RowLayout {
        let mut segments = Vec::new();

        let expecting: Vec<usize> = self
            .lanes
            .iter()
            .enumerate()
            .filter_map(|(i, l)| matches!(l, Some((o, _)) if *o == oid).then_some(i))
            .collect();

        let (node_lane, node_color) = match expecting.first() {
            Some(&first) => {
                // The WIP connector never lends its (dashed) color to real history.
                let color = expecting
                    .iter()
                    .map(|&i| self.lanes[i].unwrap().1)
                    .find(|&c| c != WIP_COLOR);
                let color = match color {
                    Some(c) => c,
                    None => self.alloc_color(),
                };
                (first, color)
            }
            None => {
                // Branch tip: nothing above points here yet.
                let idx = self.free_slot();
                let c = self.alloc_color();
                self.lanes[idx] = Some((oid, c));
                (idx, c)
            }
        };

        // Top half: pass-through lanes and lanes converging into this commit.
        for (i, l) in self.lanes.iter().enumerate() {
            if let Some((o, c)) = l {
                if *o == oid {
                    if expecting.contains(&i) {
                        segments.push((i as u16, TOP, node_lane as u16, MID, *c));
                    }
                } else {
                    segments.push((i as u16, TOP, i as u16, BOTTOM, *c));
                }
            }
        }

        for &i in &expecting {
            if i != node_lane {
                self.lanes[i] = None;
            }
        }

        // Bottom half: edges to parents.
        if parents.is_empty() {
            self.lanes[node_lane] = None;
        }
        for (pi, p) in parents.iter().enumerate() {
            if pi == 0 {
                match self.lane_of(*p) {
                    // A side branch folds back into the older line to its left right away,
                    // which keeps the graph narrow.
                    Some(k) if k < node_lane => {
                        segments.push((node_lane as u16, MID, k as u16, BOTTOM, node_color));
                        self.lanes[node_lane] = None;
                    }
                    // Otherwise continue straight down; lanes converge when the parent is reached.
                    // This keeps the main line of history in one column.
                    _ => {
                        self.lanes[node_lane] = Some((*p, node_color));
                        segments.push((node_lane as u16, MID, node_lane as u16, BOTTOM, node_color));
                    }
                }
            } else {
                let existing = self.lane_of(*p).filter(|&k| k != node_lane);
                match existing {
                    Some(k) => {
                        let c = self.lanes[k].unwrap().1;
                        segments.push((node_lane as u16, MID, k as u16, BOTTOM, c));
                    }
                    None => {
                        let k = self.free_slot();
                        let c = self.alloc_color();
                        self.lanes[k] = Some((*p, c));
                        segments.push((node_lane as u16, MID, k as u16, BOTTOM, c));
                    }
                }
            }
        }

        while matches!(self.lanes.last(), Some(None)) {
            self.lanes.pop();
        }
        let width = self
            .lanes
            .len()
            .max(node_lane + 1)
            .max(segments.iter().map(|s| s.0.max(s.2) as usize + 1).max().unwrap_or(0));
        self.max_lanes = self.max_lanes.max(width);

        RowLayout {
            lane: node_lane as u16,
            color: node_color,
            segments,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(n: u8) -> Oid {
        Oid::from_bytes(&[n; 20]).unwrap()
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let mut g = GraphBuilder::default();
        let a = g.push(oid(3), &[oid(2)]);
        let b = g.push(oid(2), &[oid(1)]);
        let c = g.push(oid(1), &[]);
        assert_eq!((a.lane, b.lane, c.lane), (0, 0, 0));
        assert_eq!(g.max_lanes, 1);
    }

    #[test]
    fn merge_keeps_main_line_in_first_lane() {
        // 5 = tip of other branch -> 1; 4 = merge(1, 3) on main; 3 -> 1
        let mut g = GraphBuilder::default();
        g.push(oid(4), &[oid(1), oid(3)]); // lane 0, opens lane 1 for 3
        let other = g.push(oid(5), &[oid(1)]); // new tip in lane 2, parent already expected in lane 0
        assert_eq!(other.lane, 2);
        g.push(oid(3), &[oid(1)]);
        let base = g.push(oid(1), &[]);
        assert_eq!(base.lane, 0, "the main line stays in the first column");
    }

    #[test]
    fn wip_row_connects_to_head_without_stealing_color() {
        let mut g = GraphBuilder::default();
        let w = g.push_wip(Some(oid(2)));
        assert_eq!((w.lane, w.color), (0, WIP_COLOR));
        let head = g.push(oid(2), &[oid(1)]);
        assert_eq!(head.lane, 0);
        assert_ne!(head.color, WIP_COLOR);
        assert!(head.segments.contains(&(0, TOP, 0, MID, WIP_COLOR)));
        assert!(head.segments.contains(&(0, MID, 0, BOTTOM, head.color)));
    }

    #[test]
    fn merge_opens_and_closes_a_lane() {
        // 4 = merge(3, 2); 3 -> 1; 2 -> 1
        let mut g = GraphBuilder::default();
        let m = g.push(oid(4), &[oid(3), oid(2)]);
        assert_eq!(m.lane, 0);
        let a = g.push(oid(3), &[oid(1)]);
        assert_eq!(a.lane, 0);
        let b = g.push(oid(2), &[oid(1)]);
        assert_eq!(b.lane, 1);
        let base = g.push(oid(1), &[]);
        assert_eq!(base.lane, 0);
        // lane 1 converges into lane 0 at the base commit
        assert!(base.segments.contains(&(1, TOP, 0, MID, b.color)) || b.segments.iter().any(|s| s.2 == 0));
        assert_eq!(g.max_lanes, 2);
    }
}
