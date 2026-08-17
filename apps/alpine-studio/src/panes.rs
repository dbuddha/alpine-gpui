//! Allocation-free bounded split-view state for Alpine Studio.

use std::{error::Error, fmt};

use alpine_core::{Point, Rect, Size};

pub(crate) const MAX_PANES: usize = 4;
const MAX_NODES: usize = MAX_PANES * 2 - 1;
const DIVIDER_EXTENT: f32 = 2.0;
const MIN_COLUMN_WIDTH: f32 = 120.0;
const MIN_ROW_HEIGHT: f32 = 80.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SplitAxis {
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PaneId(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneGeometry {
    pub(crate) id: PaneId,
    pub(crate) bounds: Rect,
    pub(crate) active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Node {
    Empty,
    Leaf {
        state: u8,
    },
    Split {
        axis: SplitAxis,
        first: u8,
        second: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneState {
    id: PaneId,
    scroll_y: f32,
    occupied: bool,
}

impl PaneState {
    const EMPTY: Self = Self {
        id: PaneId(0),
        scroll_y: 0.0,
        occupied: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneError {
    CapacityReached(usize),
    LastPane,
    GeometryTooSmall,
    InvalidGeometry,
    IdentityExhausted,
    InconsistentState,
}

impl fmt::Display for PaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityReached(limit) => write!(formatter, "pane limit of {limit} was reached"),
            Self::LastPane => formatter.write_str("the final pane cannot be closed"),
            Self::GeometryTooSmall => formatter.write_str("the active pane is too small to split"),
            Self::InvalidGeometry => formatter.write_str("pane geometry is invalid"),
            Self::IdentityExhausted => formatter.write_str("pane identity is exhausted"),
            Self::InconsistentState => formatter.write_str("pane ownership is inconsistent"),
        }
    }
}

impl Error for PaneError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneLayout {
    entries: [Option<PaneGeometry>; MAX_PANES],
    len: usize,
}

impl PaneLayout {
    pub(crate) fn iter(&self) -> impl Iterator<Item = PaneGeometry> + '_ {
        self.entries[..self.len].iter().filter_map(|entry| *entry)
    }

    pub(crate) fn active(&self) -> Option<PaneGeometry> {
        self.iter().find(|entry| entry.active)
    }

    fn pane_at(&self, point: Point) -> Option<PaneGeometry> {
        self.iter().find(|entry| contains(entry.bounds, point))
    }
}

#[derive(Debug)]
pub(crate) struct PaneGrid {
    nodes: [Node; MAX_NODES],
    states: [PaneState; MAX_PANES],
    active: PaneId,
    next_id: u64,
    pane_count: usize,
}

impl PaneGrid {
    pub(crate) fn new(scroll_y: f32) -> Self {
        let mut nodes = [Node::Empty; MAX_NODES];
        let mut states = [PaneState::EMPTY; MAX_PANES];
        let id = PaneId(1);
        nodes[0] = Node::Leaf { state: 0 };
        states[0] = PaneState {
            id,
            scroll_y: finite_scroll(scroll_y),
            occupied: true,
        };
        Self {
            nodes,
            states,
            active: id,
            next_id: 2,
            pane_count: 1,
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.pane_count
    }

    #[cfg(test)]
    pub(crate) const fn active_id(&self) -> PaneId {
        self.active
    }

    #[cfg(test)]
    pub(crate) fn inject_scroll_fault(
        &mut self,
        id: PaneId,
        scroll_y: f32,
    ) -> Result<(), PaneError> {
        let state = self
            .states
            .iter_mut()
            .find(|state| state.occupied && state.id == id)
            .ok_or(PaneError::InconsistentState)?;
        state.scroll_y = scroll_y;
        Ok(())
    }

    pub(crate) fn sync_active(&mut self, scroll_y: f32) -> Result<(), PaneError> {
        if !scroll_y.is_finite() || scroll_y < 0.0 {
            return Err(PaneError::InvalidGeometry);
        }
        let state = self
            .states
            .iter_mut()
            .find(|state| state.occupied && state.id == self.active)
            .ok_or(PaneError::InconsistentState)?;
        state.scroll_y = scroll_y;
        Ok(())
    }

    pub(crate) fn scroll_for(&self, id: PaneId, active_scroll: f32) -> Result<f32, PaneError> {
        if id == self.active {
            if !active_scroll.is_finite() || active_scroll < 0.0 {
                return Err(PaneError::InvalidGeometry);
            }
            return Ok(active_scroll);
        }
        self.states
            .iter()
            .find(|state| state.occupied && state.id == id)
            .map(|state| state.scroll_y)
            .ok_or(PaneError::InconsistentState)
    }

    pub(crate) fn can_split(&self, axis: SplitAxis, bounds: Rect) -> bool {
        self.pane_count < MAX_PANES
            && self
                .layout(bounds)
                .ok()
                .and_then(|layout| layout.active())
                .is_some_and(|active| split_bounds(active.bounds, axis).is_ok())
    }

    pub(crate) fn split(
        &mut self,
        axis: SplitAxis,
        active_scroll: f32,
        bounds: Rect,
    ) -> Result<f32, PaneError> {
        if self.pane_count >= MAX_PANES {
            return Err(PaneError::CapacityReached(MAX_PANES));
        }
        self.sync_active(active_scroll)?;
        let active_geometry = self
            .layout(bounds)?
            .active()
            .ok_or(PaneError::InconsistentState)?;
        let _ = split_bounds(active_geometry.bounds, axis)?;
        let (active_node, old_state) = self
            .leaf_node(self.active)
            .ok_or(PaneError::InconsistentState)?;
        let mut vacant_nodes = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| (*node == Node::Empty).then_some(index));
        let first_node = vacant_nodes.next().ok_or(PaneError::InconsistentState)?;
        let second_node = vacant_nodes.next().ok_or(PaneError::InconsistentState)?;
        let new_state = self
            .states
            .iter()
            .position(|state| !state.occupied)
            .ok_or(PaneError::InconsistentState)?;
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(PaneError::IdentityExhausted)?;
        let id = PaneId(self.next_id);
        let scroll_y = self.states[usize::from(old_state)].scroll_y;
        self.states[new_state] = PaneState {
            id,
            scroll_y,
            occupied: true,
        };
        self.nodes[first_node] = Node::Leaf { state: old_state };
        self.nodes[second_node] = Node::Leaf {
            state: u8::try_from(new_state).map_err(|_| PaneError::InconsistentState)?,
        };
        self.nodes[active_node] = Node::Split {
            axis,
            first: u8::try_from(first_node).map_err(|_| PaneError::InconsistentState)?,
            second: u8::try_from(second_node).map_err(|_| PaneError::InconsistentState)?,
        };
        self.active = id;
        self.next_id = next_id;
        self.pane_count += 1;
        Ok(scroll_y)
    }

    pub(crate) fn focus_at(
        &mut self,
        point: Point,
        bounds: Rect,
        active_scroll: f32,
    ) -> Result<Option<f32>, PaneError> {
        self.sync_active(active_scroll)?;
        let Some(target) = self.layout(bounds)?.pane_at(point) else {
            return Ok(None);
        };
        self.focus(target.id)
    }

    pub(crate) fn focus_next(&mut self, active_scroll: f32) -> Result<Option<f32>, PaneError> {
        self.sync_active(active_scroll)?;
        let layout = self.layout(unit_bounds()?)?;
        let mut ids = [PaneId(0); MAX_PANES];
        let mut len = 0;
        for entry in layout.iter() {
            ids[len] = entry.id;
            len += 1;
        }
        let current = ids[..len]
            .iter()
            .position(|id| *id == self.active)
            .ok_or(PaneError::InconsistentState)?;
        let target = ids[(current + 1) % len];
        self.focus(target)
    }

    pub(crate) fn close_active(&mut self, active_scroll: f32) -> Result<f32, PaneError> {
        if self.pane_count == 1 {
            return Err(PaneError::LastPane);
        }
        self.sync_active(active_scroll)?;
        let (active_node, removed_state) = self
            .leaf_node(self.active)
            .ok_or(PaneError::InconsistentState)?;
        let (parent, sibling) = self
            .nodes
            .iter()
            .enumerate()
            .find_map(|(index, node)| match *node {
                Node::Split { first, second, .. } if usize::from(first) == active_node => {
                    Some((index, usize::from(second)))
                }
                Node::Split { first, second, .. } if usize::from(second) == active_node => {
                    Some((index, usize::from(first)))
                }
                Node::Empty | Node::Leaf { .. } | Node::Split { .. } => None,
            })
            .ok_or(PaneError::InconsistentState)?;
        let removed_state = usize::from(removed_state);
        let promoted = self.nodes[sibling];
        if promoted == Node::Empty {
            return Err(PaneError::InconsistentState);
        }
        self.nodes[parent] = promoted;
        self.nodes[active_node] = Node::Empty;
        self.nodes[sibling] = Node::Empty;
        self.states[removed_state] = PaneState::EMPTY;
        self.pane_count -= 1;
        let target = self.first_leaf_id(parent)?;
        let scroll_y = self
            .states
            .iter()
            .find(|state| state.occupied && state.id == target)
            .map(|state| state.scroll_y)
            .ok_or(PaneError::InconsistentState)?;
        self.active = target;
        Ok(scroll_y)
    }

    pub(crate) fn layout(&self, bounds: Rect) -> Result<PaneLayout, PaneError> {
        let mut layout = PaneLayout {
            entries: [None; MAX_PANES],
            len: 0,
        };
        self.layout_node(0, bounds, &mut layout)?;
        if layout.len != self.pane_count || layout.active().is_none() {
            return Err(PaneError::InconsistentState);
        }
        Ok(layout)
    }

    fn focus(&mut self, id: PaneId) -> Result<Option<f32>, PaneError> {
        if id == self.active {
            return Ok(None);
        }
        let scroll_y = self
            .states
            .iter()
            .find(|state| state.occupied && state.id == id)
            .map(|state| state.scroll_y)
            .ok_or(PaneError::InconsistentState)?;
        self.active = id;
        Ok(Some(scroll_y))
    }

    fn leaf_node(&self, id: PaneId) -> Option<(usize, u8)> {
        self.nodes.iter().enumerate().find_map(|(index, node)| {
            let Node::Leaf { state: state_index } = *node else {
                return None;
            };
            let state = self.states.get(usize::from(state_index))?;
            (state.occupied && state.id == id).then_some((index, state_index))
        })
    }

    fn first_leaf_id(&self, start: usize) -> Result<PaneId, PaneError> {
        let mut node = start;
        for _ in 0..MAX_NODES {
            match self
                .nodes
                .get(node)
                .copied()
                .ok_or(PaneError::InconsistentState)?
            {
                Node::Leaf { state } => {
                    let state = self
                        .states
                        .get(usize::from(state))
                        .filter(|state| state.occupied)
                        .ok_or(PaneError::InconsistentState)?;
                    return Ok(state.id);
                }
                Node::Split { first, .. } => node = usize::from(first),
                Node::Empty => return Err(PaneError::InconsistentState),
            }
        }
        Err(PaneError::InconsistentState)
    }

    fn layout_node(
        &self,
        node_index: usize,
        bounds: Rect,
        layout: &mut PaneLayout,
    ) -> Result<(), PaneError> {
        match self
            .nodes
            .get(node_index)
            .copied()
            .ok_or(PaneError::InconsistentState)?
        {
            Node::Empty => Err(PaneError::InconsistentState),
            Node::Leaf { state } => {
                if layout.len >= MAX_PANES {
                    return Err(PaneError::InconsistentState);
                }
                let state = self
                    .states
                    .get(usize::from(state))
                    .filter(|state| state.occupied)
                    .ok_or(PaneError::InconsistentState)?;
                layout.entries[layout.len] = Some(PaneGeometry {
                    id: state.id,
                    bounds,
                    active: state.id == self.active,
                });
                layout.len += 1;
                Ok(())
            }
            Node::Split {
                axis,
                first,
                second,
            } => {
                let (first_bounds, second_bounds) = split_bounds(bounds, axis)?;
                self.layout_node(usize::from(first), first_bounds, layout)?;
                self.layout_node(usize::from(second), second_bounds, layout)
            }
        }
    }
}

fn split_bounds(bounds: Rect, axis: SplitAxis) -> Result<(Rect, Rect), PaneError> {
    let origin = bounds.origin();
    let size = bounds.size();
    let (first_origin, first_size, second_origin, second_size) = match axis {
        SplitAxis::Columns => {
            let available = size.width() - DIVIDER_EXTENT;
            let first = available * 0.5;
            let second = available - first;
            if first.min(second) < MIN_COLUMN_WIDTH {
                return Err(PaneError::GeometryTooSmall);
            }
            (
                origin,
                Size::new(first, size.height()).ok_or(PaneError::InvalidGeometry)?,
                Point::new(origin.x() + first + DIVIDER_EXTENT, origin.y())
                    .ok_or(PaneError::InvalidGeometry)?,
                Size::new(second, size.height()).ok_or(PaneError::InvalidGeometry)?,
            )
        }
        SplitAxis::Rows => {
            let available = size.height() - DIVIDER_EXTENT;
            let first = available * 0.5;
            let second = available - first;
            if first.min(second) < MIN_ROW_HEIGHT {
                return Err(PaneError::GeometryTooSmall);
            }
            (
                origin,
                Size::new(size.width(), first).ok_or(PaneError::InvalidGeometry)?,
                Point::new(origin.x(), origin.y() + first + DIVIDER_EXTENT)
                    .ok_or(PaneError::InvalidGeometry)?,
                Size::new(size.width(), second).ok_or(PaneError::InvalidGeometry)?,
            )
        }
    };
    Ok((
        Rect::new(first_origin, first_size),
        Rect::new(second_origin, second_size),
    ))
}

fn contains(bounds: Rect, point: Point) -> bool {
    let origin = bounds.origin();
    let size = bounds.size();
    point.x() >= origin.x()
        && point.y() >= origin.y()
        && point.x() < origin.x() + size.width()
        && point.y() < origin.y() + size.height()
}

fn finite_scroll(scroll_y: f32) -> f32 {
    if scroll_y.is_finite() {
        scroll_y.max(0.0)
    } else {
        0.0
    }
}

fn unit_bounds() -> Result<Rect, PaneError> {
    let origin = Point::new(0.0, 0.0).ok_or(PaneError::InvalidGeometry)?;
    let size = Size::new(1_024.0, 1_024.0).ok_or(PaneError::InvalidGeometry)?;
    Ok(Rect::new(origin, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(width: f32, height: f32) -> Result<Rect, PaneError> {
        let origin = Point::new(10.0, 20.0).ok_or(PaneError::InvalidGeometry)?;
        let size = Size::new(width, height).ok_or(PaneError::InvalidGeometry)?;
        Ok(Rect::new(origin, size))
    }

    #[test]
    fn row_column_focus_and_close_preserve_exact_scroll() -> Result<(), Box<dyn Error>> {
        let bounds = bounds(800.0, 600.0)?;
        let mut panes = PaneGrid::new(11.0);
        let first = panes.active_id();
        assert_eq!(
            panes.split(SplitAxis::Columns, 11.0, bounds)?.to_bits(),
            11.0_f32.to_bits()
        );
        let second = panes.active_id();
        assert_ne!(first, second);
        let layout = panes.layout(bounds)?;
        let entries: Vec<_> = layout.iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].bounds.origin().x().to_bits(), 10.0_f32.to_bits());
        assert_eq!(
            entries[0].bounds.size().width().to_bits(),
            399.0_f32.to_bits()
        );
        assert_eq!(
            entries[1].bounds.origin().x().to_bits(),
            411.0_f32.to_bits()
        );
        assert_eq!(
            entries[1].bounds.size().width().to_bits(),
            399.0_f32.to_bits()
        );
        panes.sync_active(44.0)?;
        assert_eq!(panes.focus_next(44.0)?, Some(11.0));
        assert_eq!(panes.active_id(), first);
        assert_eq!(panes.focus_next(22.0)?, Some(44.0));
        assert_eq!(panes.active_id(), second);
        let closed_scroll = panes.close_active(55.0)?;
        assert_eq!(closed_scroll.to_bits(), 22.0_f32.to_bits());
        assert_eq!(panes.len(), 1);
        assert_eq!(panes.active_id(), first);
        Ok(())
    }

    #[test]
    fn nested_geometry_capacity_and_hit_testing_are_bounded() -> Result<(), Box<dyn Error>> {
        let bounds = bounds(1_200.0, 900.0)?;
        let mut panes = PaneGrid::new(0.0);
        panes.split(SplitAxis::Columns, 0.0, bounds)?;
        panes.split(SplitAxis::Rows, 10.0, bounds)?;
        panes.split(SplitAxis::Rows, 20.0, bounds)?;
        assert_eq!(panes.len(), MAX_PANES);
        assert!(matches!(
            panes.split(SplitAxis::Columns, 0.0, bounds),
            Err(PaneError::CapacityReached(MAX_PANES))
        ));
        let layout = panes.layout(bounds)?;
        assert_eq!(layout.iter().count(), MAX_PANES);
        for entry in layout.iter() {
            let point = Point::new(
                entry.bounds.origin().x() + 1.0,
                entry.bounds.origin().y() + 1.0,
            )
            .ok_or(PaneError::InvalidGeometry)?;
            let previous = panes.active_id();
            let result = panes.focus_at(point, bounds, 30.0)?;
            assert_eq!(panes.active_id(), entry.id);
            assert_eq!(result.is_some(), previous != entry.id);
        }
        Ok(())
    }

    #[test]
    fn invalid_transitions_are_atomic_and_descriptive() -> Result<(), Box<dyn Error>> {
        let large = bounds(800.0, 600.0)?;
        let mut panes = PaneGrid::new(f32::NAN);
        assert!(matches!(
            panes.scroll_for(panes.active_id(), f32::NAN),
            Err(PaneError::InvalidGeometry)
        ));
        assert!(matches!(panes.close_active(0.0), Err(PaneError::LastPane)));
        assert_eq!(panes.len(), 1);
        let tiny = bounds(200.0, 100.0)?;
        assert!(!panes.can_split(SplitAxis::Columns, tiny));
        assert!(!panes.can_split(SplitAxis::Rows, tiny));
        assert!(matches!(
            panes.split(SplitAxis::Columns, 0.0, tiny),
            Err(PaneError::GeometryTooSmall)
        ));
        assert_eq!(panes.len(), 1);
        assert!(panes.can_split(SplitAxis::Columns, large));
        for error in [
            PaneError::CapacityReached(MAX_PANES),
            PaneError::LastPane,
            PaneError::GeometryTooSmall,
            PaneError::InvalidGeometry,
            PaneError::IdentityExhausted,
            PaneError::InconsistentState,
        ] {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_none());
        }
        Ok(())
    }

    #[test]
    fn repeated_split_focus_close_reuses_nodes_without_identity_alias() -> Result<(), Box<dyn Error>>
    {
        let bounds = bounds(1_200.0, 900.0)?;
        let mut panes = PaneGrid::new(0.0);
        let mut previous = panes.active_id();
        for index in 0_u16..4_096 {
            let scroll = f32::from(index);
            panes.split(SplitAxis::Columns, scroll, bounds)?;
            let created = panes.active_id();
            assert_ne!(created, previous);
            assert_eq!(panes.focus_next(scroll)?, Some(scroll));
            assert_eq!(panes.active_id(), previous);
            assert_eq!(
                panes.close_active(scroll + 1.0)?.to_bits(),
                scroll.to_bits()
            );
            assert_eq!(panes.len(), 1);
            assert_eq!(panes.active_id(), created);
            previous = created;
        }
        Ok(())
    }

    #[test]
    fn corrupted_topologies_fail_closed_without_partial_mutation() -> Result<(), Box<dyn Error>> {
        let bounds = bounds(1_200.0, 900.0)?;

        let mut nested = PaneGrid::new(0.0);
        nested.split(SplitAxis::Columns, 0.0, bounds)?;
        nested.split(SplitAxis::Rows, 1.0, bounds)?;
        assert_eq!(nested.close_active(2.0)?.to_bits(), 1.0_f32.to_bits());

        let mut missing_sibling = PaneGrid::new(0.0);
        missing_sibling.split(SplitAxis::Columns, 0.0, bounds)?;
        missing_sibling.nodes[1] = Node::Empty;
        assert_eq!(
            missing_sibling.close_active(0.0),
            Err(PaneError::InconsistentState)
        );
        assert_eq!(missing_sibling.pane_count, 2);

        let mut wrong_count = PaneGrid::new(0.0);
        wrong_count.pane_count = 2;
        assert_eq!(
            wrong_count.layout(bounds),
            Err(PaneError::InconsistentState)
        );

        let mut empty = PaneGrid::new(0.0);
        empty.nodes[0] = Node::Empty;
        assert_eq!(empty.first_leaf_id(0), Err(PaneError::InconsistentState));
        let mut layout = PaneLayout {
            entries: [None; MAX_PANES],
            len: 0,
        };
        assert_eq!(
            empty.layout_node(0, bounds, &mut layout),
            Err(PaneError::InconsistentState)
        );

        let valid = PaneGrid::new(0.0);
        layout.len = MAX_PANES;
        assert_eq!(
            valid.layout_node(0, bounds, &mut layout),
            Err(PaneError::InconsistentState)
        );

        let mut cycle = PaneGrid::new(0.0);
        cycle.nodes[0] = Node::Split {
            axis: SplitAxis::Columns,
            first: 0,
            second: 0,
        };
        assert_eq!(cycle.first_leaf_id(0), Err(PaneError::InconsistentState));
        Ok(())
    }

    #[test]
    fn primitive_geometry_boundaries_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(MAX_NODES, 7);
        assert_eq!(finite_scroll(7.5).to_bits(), 7.5_f32.to_bits());
        assert_eq!(finite_scroll(-3.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(finite_scroll(f32::NAN).to_bits(), 0.0_f32.to_bits());
        assert_eq!(finite_scroll(f32::INFINITY).to_bits(), 0.0_f32.to_bits());

        let exact = bounds(242.0, 162.0)?;
        let (left, right) = split_bounds(exact, SplitAxis::Columns)?;
        assert_eq!(left.origin(), exact.origin());
        assert_eq!(left.size().width().to_bits(), 120.0_f32.to_bits());
        assert_eq!(right.origin().x().to_bits(), 132.0_f32.to_bits());
        assert_eq!(right.origin().y().to_bits(), 20.0_f32.to_bits());
        assert_eq!(right.size().width().to_bits(), 120.0_f32.to_bits());
        assert_eq!(
            split_bounds(bounds(241.0, 162.0)?, SplitAxis::Columns),
            Err(PaneError::GeometryTooSmall)
        );

        let (top, bottom) = split_bounds(exact, SplitAxis::Rows)?;
        assert_eq!(top.origin(), exact.origin());
        assert_eq!(top.size().height().to_bits(), 80.0_f32.to_bits());
        assert_eq!(bottom.origin().x().to_bits(), 10.0_f32.to_bits());
        assert_eq!(bottom.origin().y().to_bits(), 102.0_f32.to_bits());
        assert_eq!(bottom.size().height().to_bits(), 80.0_f32.to_bits());
        assert_eq!(
            split_bounds(bounds(242.0, 161.0)?, SplitAxis::Rows),
            Err(PaneError::GeometryTooSmall)
        );

        assert!(contains(
            exact,
            Point::new(10.0, 20.0).ok_or(PaneError::InvalidGeometry)?
        ));
        assert!(contains(
            exact,
            Point::new(251.5, 181.5).ok_or(PaneError::InvalidGeometry)?
        ));
        assert!(!contains(
            exact,
            Point::new(252.0, 20.0).ok_or(PaneError::InvalidGeometry)?
        ));
        assert!(!contains(
            exact,
            Point::new(10.0, 182.0).ok_or(PaneError::InvalidGeometry)?
        ));

        let mut panes = PaneGrid::new(3.0);
        assert_eq!(
            panes.scroll_for(PaneId(0), 0.0),
            Err(PaneError::InconsistentState)
        );
        let large = bounds(1_200.0, 900.0)?;
        panes.split(SplitAxis::Columns, 3.0, large)?;
        panes.split(SplitAxis::Rows, 4.0, large)?;
        panes.split(SplitAxis::Rows, 5.0, large)?;
        assert!(!panes.can_split(SplitAxis::Columns, large));
        assert!(!panes.can_split(SplitAxis::Rows, large));
        Ok(())
    }
}
