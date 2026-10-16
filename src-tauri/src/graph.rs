//! コミット列 (子→親の順) からレーン配置を計算する。
//!
//! git には依存させず、ID は `Copy + Eq` なら何でもよい (テストでは整数を使う)。
//! 1 行 = 1 コミット。各行は「その行と次の行の間の線分」を持つので、
//! 表示側は可視範囲の前後 1 行だけ取れば線が途切れない。

use serde::Serialize;

/// 行 r と行 r+1 の間の線分。`from` は行 r でのレーン、`to` は行 r+1 でのレーン。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub from: u16,
    pub to: u16,
    pub color: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Layout {
    pub lane: u16,
    pub color: u16,
    pub segments: Vec<Segment>,
}

pub struct LayoutBuilder<T> {
    /// 各レーンが次に受け取る予定のコミットと色。None は空きレーン。
    active: Vec<Option<(T, u16)>>,
    rows: Vec<Layout>,
    next_color: u16,
    max_lanes: usize,
}

impl<T: Copy + Eq> Default for LayoutBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Eq> LayoutBuilder<T> {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            rows: Vec::new(),
            next_color: 0,
            max_lanes: 0,
        }
    }

    /// コミットを 1 つ追加する。親より先に子を渡すこと (topological order)。
    pub fn push(&mut self, id: T, parents: &[T]) {
        let matches: Vec<usize> = self
            .active
            .iter()
            .enumerate()
            .filter(|(_, slot)| matches!(slot, Some((expected, _)) if *expected == id))
            .map(|(i, _)| i)
            .collect();

        let (lane, color) = match matches.first() {
            Some(&first) => (first, self.active[first].unwrap().1),
            None => {
                let lane = self.alloc_lane();
                let color = self.take_color();
                self.active[lane] = Some((id, color));
                (lane, color)
            }
        };

        // このコミットに合流する他のレーンは、前の行の線分の終点をこのコミットの位置へ曲げる
        if let Some(prev) = self.rows.last_mut() {
            for seg in prev.segments.iter_mut() {
                let to = seg.to as usize;
                if to != lane && matches.contains(&to) {
                    seg.to = lane as u16;
                }
            }
        }
        for &m in &matches {
            if m != lane {
                self.active[m] = None;
            }
        }

        let mut segments = Vec::new();
        // この行で新設したレーンは fork の線で始まるので、縦線 (通過線) は描かない
        let mut created = Vec::new();
        match parents.first() {
            // 第一親は自分のレーンをそのまま引き継ぐ (直線で続く)
            Some(&first_parent) => self.active[lane] = Some((first_parent, color)),
            None => self.active[lane] = None,
        }
        for &parent in parents.iter().skip(1) {
            let existing = self
                .active
                .iter()
                .position(|slot| matches!(slot, Some((expected, _)) if *expected == parent));
            match existing {
                Some(target) if target == lane => {}
                Some(target) => segments.push(Segment {
                    from: lane as u16,
                    to: target as u16,
                    color: self.active[target].unwrap().1,
                }),
                None => {
                    let target = self.alloc_lane();
                    let color = self.take_color();
                    self.active[target] = Some((parent, color));
                    created.push(target);
                    segments.push(Segment {
                        from: lane as u16,
                        to: target as u16,
                        color,
                    });
                }
            }
        }
        for (i, slot) in self.active.iter().enumerate() {
            if created.contains(&i) {
                continue;
            }
            if let Some((_, color)) = slot {
                segments.push(Segment {
                    from: i as u16,
                    to: i as u16,
                    color: *color,
                });
            }
        }
        while matches!(self.active.last(), Some(None)) {
            self.active.pop();
        }
        self.max_lanes = self.max_lanes.max(lane + 1).max(self.active.len());
        self.rows.push(Layout {
            lane: lane as u16,
            color,
            segments,
        });
    }

    /// (各行の配置, 使ったレーン数の最大)
    pub fn finish(self) -> (Vec<Layout>, usize) {
        (self.rows, self.max_lanes)
    }

    /// 一番左の空きレーンを返す。無ければ右に増やす (グラフを詰めて描くため)。
    fn alloc_lane(&mut self) -> usize {
        if let Some(i) = self.active.iter().position(|slot| slot.is_none()) {
            return i;
        }
        self.active.push(None);
        self.active.len() - 1
    }

    fn take_color(&mut self) -> u16 {
        let c = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(from: u16, to: u16, color: u16) -> Segment {
        Segment { from, to, color }
    }

    fn sorted(mut segments: Vec<Segment>) -> Vec<Segment> {
        segments.sort_by_key(|s| (s.from, s.to, s.color));
        segments
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let mut b = LayoutBuilder::new();
        b.push(3, &[2]);
        b.push(2, &[1]);
        b.push(1, &[]);
        let (rows, max_lanes) = b.finish();
        assert_eq!(max_lanes, 1);
        assert_eq!(rows[0], Layout { lane: 0, color: 0, segments: vec![seg(0, 0, 0)] });
        assert_eq!(rows[1], Layout { lane: 0, color: 0, segments: vec![seg(0, 0, 0)] });
        assert_eq!(rows[2], Layout { lane: 0, color: 0, segments: vec![] });
    }

    #[test]
    fn merge_forks_into_second_lane_and_joins_back_at_common_parent() {
        // M -> (B, C), B -> A, C -> A
        let mut b = LayoutBuilder::new();
        b.push(10, &[2, 3]);
        b.push(2, &[1]);
        b.push(3, &[1]);
        b.push(1, &[]);
        let (rows, max_lanes) = b.finish();
        assert_eq!(max_lanes, 2);
        assert_eq!((rows[0].lane, rows[0].color), (0, 0));
        // 新設した lane 1 は fork の線 (0→1) で始まり、縦線 1→1 は無い
        assert_eq!(sorted(rows[0].segments.clone()), vec![seg(0, 0, 0), seg(0, 1, 1)]);
        assert_eq!((rows[1].lane, rows[1].color), (0, 0));
        assert_eq!(sorted(rows[1].segments.clone()), vec![seg(0, 0, 0), seg(1, 1, 1)]);
        // C は lane 1 にいて、その親 A は lane 0 でも待たれているので A の行で lane 0 に合流する
        assert_eq!((rows[2].lane, rows[2].color), (1, 1));
        assert_eq!(sorted(rows[2].segments.clone()), vec![seg(0, 0, 0), seg(1, 0, 1)]);
        assert_eq!((rows[3].lane, rows[3].color), (0, 0));
        assert!(rows[3].segments.is_empty());
    }

    #[test]
    fn second_parent_already_waited_by_another_lane_gets_a_direct_edge() {
        // 2 つの頭: X -> A, M -> (B, A), B -> A
        let mut b = LayoutBuilder::new();
        b.push(20, &[1]);
        b.push(10, &[2, 1]);
        b.push(2, &[1]);
        b.push(1, &[]);
        let (rows, max_lanes) = b.finish();
        assert_eq!(max_lanes, 2);
        // M は lane 1。第二親 A は lane 0 が待っているので lane 1 -> lane 0 の線が即座に出る
        assert_eq!(rows[1].lane, 1);
        assert_eq!(sorted(rows[1].segments.clone()), vec![seg(0, 0, 0), seg(1, 0, 0), seg(1, 1, 1)]);
        // A の行で lane 1 が lane 0 へ合流する
        assert_eq!(sorted(rows[2].segments.clone()), vec![seg(0, 0, 0), seg(1, 0, 1)]);
        assert_eq!(rows[3].lane, 0);
    }

    #[test]
    fn freed_lane_is_reused_from_the_left() {
        // 3 本に広がって 1 本目と 2 本目が閉じた後、新しい頭は lane 0 を再利用する
        let mut b = LayoutBuilder::new();
        b.push(30, &[1, 2, 3]);
        b.push(1, &[]);
        b.push(2, &[]);
        b.push(40, &[4]);
        b.push(3, &[]);
        b.push(4, &[]);
        let (rows, max_lanes) = b.finish();
        assert_eq!(max_lanes, 3);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 0);
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[4].lane, 2);
        assert_eq!(rows[5].lane, 0);
    }

    #[test]
    fn new_lanes_for_an_octopus_merge_start_with_fork_edges_only() {
        let mut b = LayoutBuilder::new();
        b.push(30, &[1, 2, 3]);
        b.push(1, &[]);
        let (rows, _) = b.finish();
        assert_eq!(sorted(rows[0].segments.clone()), vec![seg(0, 0, 0), seg(0, 1, 1), seg(0, 2, 2)]);
        // 次の行では lane 1 / 2 が普通の通過線になる
        assert_eq!(sorted(rows[1].segments.clone()), vec![seg(1, 1, 1), seg(2, 2, 2)]);
    }

    #[test]
    fn duplicate_parent_does_not_draw_an_extra_edge() {
        let mut b = LayoutBuilder::new();
        b.push(2, &[1, 1]);
        b.push(1, &[]);
        let (rows, _) = b.finish();
        assert_eq!(rows[0].segments, vec![seg(0, 0, 0)]);
    }
}
