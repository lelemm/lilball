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
        Vec2::new(p.x.clamp(self.left, self.right), p.y.clamp(self.top, self.bottom))
    }
}

impl Default for Bounds {
    fn default() -> Self {
        // A reasonable default standing in for a 1080p desktop.
        Self::new(0.0, 0.0, 1920.0, 1080.0)
    }
}
