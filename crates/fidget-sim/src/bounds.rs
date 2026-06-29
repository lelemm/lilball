use glam::Vec2;

/// Axis-aligned rectangle describing the area the ball may move within.
///
/// In the real app this maps to the virtual desktop (which can have negative
/// coordinates across multiple monitors). The simulation works in logical
/// pixels with the origin at the top-left and `y` growing downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Horizontal floor segment corresponding to one monitor's bottom edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BottomEdge {
    pub left: f32,
    pub right: f32,
    pub y: f32,
}

impl Bounds {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Convenience constructor from an origin and a size.
    pub fn from_size(origin: Vec2, size: Vec2) -> Self {
        Self::new(origin.x, origin.y, origin.x + size.x, origin.y + size.y)
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn center(&self) -> Vec2 {
        Vec2::new(
            (self.left + self.right) * 0.5,
            (self.top + self.bottom) * 0.5,
        )
    }

    /// Clamp a point so it stays inside the rectangle.
    pub fn clamp_point(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            p.x.clamp(self.left, self.right),
            p.y.clamp(self.top, self.bottom),
        )
    }
}

impl BottomEdge {
    pub fn new(left: f32, right: f32, y: f32) -> Self {
        Self {
            left: left.min(right),
            right: left.max(right),
            y,
        }
    }

    pub fn from_bounds(bounds: Bounds) -> Self {
        Self::new(bounds.left, bounds.right, bounds.bottom)
    }

    pub fn exposed_from_bounds(monitors: &[Bounds], fallback: Bounds) -> Vec<Self> {
        let mut edges = Vec::new();
        for (i, monitor) in monitors.iter().copied().enumerate() {
            let monitor = normalized(monitor);
            if monitor.width() <= 0.0 || monitor.height() <= 0.0 {
                continue;
            }

            let mut spans = vec![(monitor.left, monitor.right)];
            for (j, other) in monitors.iter().copied().enumerate() {
                if i == j {
                    continue;
                }
                let other = normalized(other);
                if other.width() <= 0.0 || other.height() <= 0.0 {
                    continue;
                }
                if other.top <= monitor.bottom + 1.0 && other.bottom > monitor.bottom + 1.0 {
                    subtract_span(&mut spans, other.left, other.right);
                }
            }

            edges.extend(
                spans
                    .into_iter()
                    .filter(|(left, right)| right - left > 1.0)
                    .map(|(left, right)| Self::new(left, right, monitor.bottom)),
            );
        }

        if edges.is_empty() {
            edges.push(Self::from_bounds(fallback));
        }
        edges.sort_by(|a, b| {
            a.y.total_cmp(&b.y)
                .then_with(|| a.left.total_cmp(&b.left))
                .then_with(|| a.right.total_cmp(&b.right))
        });
        merge_edges(edges)
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }
}

fn normalized(bounds: Bounds) -> Bounds {
    Bounds::new(
        bounds.left.min(bounds.right),
        bounds.top.min(bounds.bottom),
        bounds.left.max(bounds.right),
        bounds.top.max(bounds.bottom),
    )
}

fn subtract_span(spans: &mut Vec<(f32, f32)>, left: f32, right: f32) {
    let span_min = left.min(right);
    let span_max = left.max(right);
    let left = span_min;
    let right = span_max;
    if right - left <= 0.0 {
        return;
    }

    let mut remaining = Vec::with_capacity(spans.len() + 1);
    for (span_left, span_right) in spans.drain(..) {
        let overlap_left = span_left.max(left);
        let overlap_right = span_right.min(right);
        if overlap_right <= overlap_left {
            remaining.push((span_left, span_right));
            continue;
        }
        if span_left < overlap_left {
            remaining.push((span_left, overlap_left));
        }
        if overlap_right < span_right {
            remaining.push((overlap_right, span_right));
        }
    }
    *spans = remaining;
}

fn merge_edges(edges: Vec<BottomEdge>) -> Vec<BottomEdge> {
    let mut merged: Vec<BottomEdge> = Vec::with_capacity(edges.len());
    for edge in edges {
        let Some(last) = merged.last_mut() else {
            merged.push(edge);
            continue;
        };
        if (last.y - edge.y).abs() <= 0.5 && edge.left <= last.right + 0.5 {
            last.right = last.right.max(edge.right);
        } else {
            merged.push(edge);
        }
    }
    merged
}

impl Default for Bounds {
    fn default() -> Self {
        // A reasonable default standing in for a 1080p desktop.
        Self::new(0.0, 0.0, 1920.0, 1080.0)
    }
}
