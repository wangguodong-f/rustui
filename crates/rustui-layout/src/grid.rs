//! [`Grid`] — rows and columns with `Auto`, fixed and fractional tracks.

use std::collections::HashMap;

use rustui_core::impl_widget_base;
use rustui_core::widget::align_child;
use rustui_core::{
    Align, ArrangeContext, EventContext, EventResult, Insets, MeasureContext, ObjectId,
    PaintContext, Rect, Size, SizeHint, UiEvent, Widget, WidgetBase,
};

use crate::style::{
    apply_fill, clamp_to, content_available, content_rect, paint_children, visible_children,
    visible_children_in, ContainerStyle,
};

/// Size of one row or column of a [`Grid`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Track {
    /// As big as the largest item in the track.
    #[default]
    Auto,
    /// Fixed size in logical pixels.
    Px(f32),
    /// Share of the space left by the other tracks; `1.0` is one whole.
    Fraction(f32),
}

impl Track {
    /// An auto sized track.
    pub const AUTO: Track = Track::Auto;

    /// A fixed track.
    pub fn px(size: f32) -> Track {
        Track::Px(size.max(0.0))
    }

    /// A fractional track.
    pub fn fr(fraction: f32) -> Track {
        Track::Fraction(fraction.max(0.0))
    }
}

/// Where a child sits inside the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
    h: Align,
    v: Align,
}

impl Placement {
    fn new(row: usize, column: usize, h: Align, v: Align) -> Self {
        Self {
            row,
            column,
            row_span: 1,
            column_span: 1,
            h,
            v,
        }
    }
}

/// Two dimensional grid of children (design document §15).
///
/// Tracks are described once for the whole grid; children are either placed explicitly with
/// [`Grid::place`] / [`Grid::place_span`] or flow automatically in child order, row by row.
///
/// ```text
/// ┌──────────┬──────┐   columns = [Auto, Px(120), Fraction(1)]
/// │ title    │  ok  │   rows    = [Auto, Auto]
/// ├──────────┴──────┤
/// │ body (span 2)   │
/// └─────────────────┘
/// ```
#[derive(Debug)]
pub struct Grid {
    base: WidgetBase,
    columns: Vec<Track>,
    rows: Vec<Track>,
    spacing: Size,
    padding: Insets,
    align: (Align, Align),
    style: ContainerStyle,
    hint: SizeHint,
    placements: HashMap<ObjectId, Placement>,
}

impl Grid {
    pub fn new() -> Self {
        Self {
            base: WidgetBase::new(),
            columns: Vec::new(),
            rows: Vec::new(),
            spacing: Size::ZERO,
            padding: Insets::ZERO,
            align: (Align::Stretch, Align::Stretch),
            style: ContainerStyle::EMPTY,
            hint: SizeHint::default(),
            placements: HashMap::new(),
        }
    }

    pub fn with_columns(mut self, columns: Vec<Track>) -> Self {
        self.columns = columns;
        self
    }

    pub fn with_rows(mut self, rows: Vec<Track>) -> Self {
        self.rows = rows;
        self
    }

    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = Size::new(spacing, spacing);
        self
    }

    pub fn with_gaps(mut self, horizontal: f32, vertical: f32) -> Self {
        self.spacing = Size::new(horizontal, vertical);
        self
    }

    pub fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_align(mut self, h: Align, v: Align) -> Self {
        self.align = (h, v);
        self
    }

    pub fn with_style(mut self, style: ContainerStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_fill(mut self) -> Self {
        self.hint = SizeHint::fill();
        self
    }

    /// Registers a child without an explicit placement (auto flow).
    pub fn with_child(mut self, id: ObjectId) -> Self {
        self.base.add_child(id);
        self
    }

    /// Registers a child at `(row, column)`.
    pub fn with_placed(mut self, id: ObjectId, row: usize, column: usize) -> Self {
        self.place_child(id, row, column);
        self
    }

    /// Registers a child spanning several rows / columns.
    pub fn place_child(&mut self, id: ObjectId, row: usize, column: usize) -> &mut Self {
        self.base.add_child(id);
        self.placements
            .insert(id, Placement::new(row, column, self.align.0, self.align.1));
        self
    }

    /// Registers a child spanning `row_span` rows and `column_span` columns.
    pub fn place_span(
        &mut self,
        id: ObjectId,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
    ) -> &mut Self {
        self.base.add_child(id);
        let mut placement = Placement::new(row, column, self.align.0, self.align.1);
        placement.row_span = row_span.max(1);
        placement.column_span = column_span.max(1);
        self.placements.insert(id, placement);
        self
    }

    /// Overrides the alignment of one child inside its cell.
    pub fn set_align_of(&mut self, id: ObjectId, h: Align, v: Align) {
        if let Some(placement) = self.placements.get_mut(&id) {
            placement.h = h;
            placement.v = v;
        }
    }

    pub fn set_columns(&mut self, columns: Vec<Track>) {
        self.columns = columns;
    }

    pub fn set_rows(&mut self, rows: Vec<Track>) {
        self.rows = rows;
    }

    /// Appends a child (auto placement).
    pub fn push(&mut self, id: ObjectId) -> &mut Self {
        self.base.add_child(id);
        self
    }

    /// Resolves the track lists and the placement of every child.
    ///
    /// Called identically by `measure` and `arrange`, so both passes agree on the geometry.
    fn plan(&self, children: &[ObjectId]) -> (Vec<Track>, Vec<Track>, Vec<(ObjectId, Placement)>) {
        let mut columns = self.columns.clone();
        if columns.is_empty() {
            columns.push(Track::Auto);
        }
        let column_count = columns.len();

        let mut items: Vec<(ObjectId, Placement)> = Vec::new();
        let mut used_rows = 0usize;
        let (mut cursor_row, mut cursor_column) = (0usize, 0usize);
        for id in children {
            if let Some(placement) = self.placements.get(id) {
                items.push((*id, *placement));
                used_rows = used_rows.max(placement.row + placement.row_span);
                continue;
            }
            if cursor_column >= column_count {
                cursor_column = 0;
                cursor_row += 1;
            }
            items.push((
                *id,
                Placement::new(cursor_row, cursor_column, self.align.0, self.align.1),
            ));
            used_rows = used_rows.max(cursor_row + 1);
            cursor_column += 1;
        }

        let mut rows = self.rows.clone();
        rows.resize(used_rows.max(rows.len()), Track::Auto);
        (columns, rows, items)
    }

    /// Natural size of every child, measured (top-down) or read back from the store.
    fn measure_children(
        children: &[ObjectId],
        ctx: &MeasureContext<'_>,
        available: Size,
    ) -> HashMap<ObjectId, Size> {
        children
            .iter()
            .map(|id| (*id, ctx.measure_child(*id, available)))
            .collect()
    }

    /// Track sizes for one axis.
    ///
    /// `spanned` lists `(first_track, span, natural_size)` for every child on that axis and
    /// `available` is the constraint of that axis (`None` when it is unbounded).
    fn resolve(
        tracks: &[Track],
        spanned: &[(usize, usize, f32)],
        available: Option<f32>,
        spacing: f32,
    ) -> Vec<f32> {
        let count = tracks.len();
        if count == 0 {
            return Vec::new();
        }
        let mut sizes = vec![0.0f32; count];

        // 1. Fixed tracks, then auto tracks seeded with the single-span items' natural size.
        for (index, track) in tracks.iter().enumerate() {
            if let Track::Px(px) = track {
                sizes[index] = *px;
            }
        }
        for &(first, span, natural) in spanned {
            if span != 1 || first >= count {
                continue;
            }
            if matches!(tracks[first], Track::Auto) {
                sizes[first] = sizes[first].max(natural);
            }
        }

        // 2. Items spanning several tracks push the auto tracks they cover.
        for &(first, span, natural) in spanned {
            if span <= 1 || first >= count {
                continue;
            }
            let span = span.min(count - first);
            let covered: f32 = sizes[first..first + span].iter().sum();
            let deficit = natural - covered;
            if deficit <= 0.0 {
                continue;
            }
            let targets: Vec<usize> = (first..first + span)
                .filter(|i| matches!(tracks[*i], Track::Auto))
                .collect();
            if targets.is_empty() {
                continue;
            }
            let share = deficit / targets.len() as f32;
            for index in targets {
                sizes[index] += share;
            }
        }

        // 3. Fractional tracks: share of what the other tracks leave over, or — without a
        //    constraint — whatever their content needs.
        let fractions: f32 = tracks
            .iter()
            .map(|t| match t {
                Track::Fraction(f) => *f,
                _ => 0.0,
            })
            .sum();
        if fractions > 0.0 {
            match available {
                Some(available) => {
                    // Only the tracks that are *not* fractional compete for the space.
                    let occupied: f32 = tracks
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| !matches!(t, Track::Fraction(_)))
                        .map(|(i, _)| sizes[i])
                        .sum::<f32>();
                    let gaps = spacing * (count.saturating_sub(1)) as f32;
                    let free = (available - occupied - gaps).max(0.0);
                    for (index, track) in tracks.iter().enumerate() {
                        if let Track::Fraction(f) = track {
                            sizes[index] = free * f / fractions;
                        }
                    }
                }
                None => {
                    for (index, track) in tracks.iter().enumerate() {
                        if matches!(track, Track::Fraction(_)) {
                            sizes[index] = spanned
                                .iter()
                                .filter(|(first, span, _)| *first <= index && index < first + span)
                                .map(|(_, _, natural)| *natural)
                                .fold(0.0f32, f32::max);
                        }
                    }
                }
            }
        }
        sizes
    }

    /// Turns an axis size into a constraint, `None` when it is unbounded.
    fn bounded(value: f32) -> Option<f32> {
        if value.is_finite() {
            Some(value)
        } else {
            None
        }
    }

    /// Total size of a resolved track list including the gaps.
    fn total(sizes: &[f32], spacing: f32) -> f32 {
        let sum: f32 = sizes.iter().sum();
        sum + spacing * (sizes.len().saturating_sub(1)) as f32
    }

    /// Offset of every track start (relative to the content origin).
    fn offsets(sizes: &[f32], spacing: f32) -> Vec<f32> {
        let mut offsets = Vec::with_capacity(sizes.len());
        let mut cursor = 0.0f32;
        for size in sizes {
            offsets.push(cursor);
            cursor += size + spacing;
        }
        offsets
    }
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}

impl_widget_base!(Grid);
impl rustui_core::Layout for Grid {}

impl Widget for Grid {
    fn kind(&self) -> &'static str {
        "Grid"
    }

    fn size_hint(&self) -> SizeHint {
        self.hint
    }

    fn is_opaque(&self) -> bool {
        self.style.background.a > 0.0
    }

    fn measure(&mut self, ctx: &MeasureContext<'_>, available: Size) -> Size {
        let inner = content_available(available, self.padding);
        let children = visible_children_in(ctx, self.base.children());
        let sizes = Self::measure_children(&children, ctx, inner);
        let (columns, rows, items) = self.plan(&children);

        let column_spans: Vec<(usize, usize, f32)> = items
            .iter()
            .map(|(id, p)| {
                (
                    p.column,
                    p.column_span,
                    sizes.get(id).map(|s| s.width).unwrap_or(0.0),
                )
            })
            .collect();
        let row_spans: Vec<(usize, usize, f32)> = items
            .iter()
            .map(|(id, p)| {
                (
                    p.row,
                    p.row_span,
                    sizes.get(id).map(|s| s.height).unwrap_or(0.0),
                )
            })
            .collect();

        let column_sizes = Self::resolve(
            &columns,
            &column_spans,
            Self::bounded(inner.width),
            self.spacing.width,
        );
        let row_sizes = Self::resolve(
            &rows,
            &row_spans,
            Self::bounded(inner.height),
            self.spacing.height,
        );
        let size = Size::new(
            Self::total(&column_sizes, self.spacing.width),
            Self::total(&row_sizes, self.spacing.height),
        );
        let size = apply_fill(
            clamp_to(size.expand(self.padding), available),
            available,
            self.hint,
        );
        self.base.set_measured(size);
        size
    }

    fn arrange(&mut self, ctx: &ArrangeContext<'_>, rect: Rect) {
        self.base.set_bounds(rect);
        let content = content_rect(Rect::new(0.0, 0.0, rect.width, rect.height), self.padding);
        let children = visible_children(ctx.store(), self.base.children());
        if children.is_empty() {
            return;
        }
        let sizes: HashMap<ObjectId, Size> = children
            .iter()
            .map(|id| (*id, crate::style::measured_size(ctx, *id)))
            .collect();
        let (columns, rows, items) = self.plan(&children);

        let column_spans: Vec<(usize, usize, f32)> = items
            .iter()
            .map(|(id, p)| {
                (
                    p.column,
                    p.column_span,
                    sizes.get(id).map(|s| s.width).unwrap_or(0.0),
                )
            })
            .collect();
        let row_spans: Vec<(usize, usize, f32)> = items
            .iter()
            .map(|(id, p)| {
                (
                    p.row,
                    p.row_span,
                    sizes.get(id).map(|s| s.height).unwrap_or(0.0),
                )
            })
            .collect();

        let column_sizes = Self::resolve(
            &columns,
            &column_spans,
            Self::bounded(content.width),
            self.spacing.width,
        );
        let row_sizes = Self::resolve(
            &rows,
            &row_spans,
            Self::bounded(content.height),
            self.spacing.height,
        );
        let column_offsets = Self::offsets(&column_sizes, self.spacing.width);
        let row_offsets = Self::offsets(&row_sizes, self.spacing.height);

        for (id, placement) in &items {
            let first_column = placement.column.min(column_sizes.len().saturating_sub(1));
            let first_row = placement.row.min(row_sizes.len().saturating_sub(1));
            let column_span = placement
                .column_span
                .min(column_sizes.len().saturating_sub(first_column));
            let row_span = placement
                .row_span
                .min(row_sizes.len().saturating_sub(first_row));
            let width: f32 = column_sizes[first_column..first_column + column_span]
                .iter()
                .copied()
                .sum::<f32>()
                + self.spacing.width * (column_span.saturating_sub(1)) as f32;
            let height: f32 = row_sizes[first_row..first_row + row_span]
                .iter()
                .copied()
                .sum::<f32>()
                + self.spacing.height * (row_span.saturating_sub(1)) as f32;
            let cell = Rect::new(
                content.left() + column_offsets[first_column],
                content.top() + row_offsets[first_row],
                width,
                height,
            );
            let natural = sizes.get(id).copied().unwrap_or(Size::ZERO);
            ctx.arrange_child(*id, align_child(cell, natural, placement.h, placement.v));
        }
    }

    fn event(&mut self, _ctx: &mut EventContext<'_>, _event: &UiEvent) -> EventResult {
        EventResult::Unhandled
    }

    fn paint(&mut self, ctx: &mut PaintContext<'_>) {
        let bounds = self.base.bounds();
        self.style.paint_begin(ctx, bounds);
        paint_children(ctx, self.base.children());
        self.style.paint_end(ctx, bounds);
    }
}
