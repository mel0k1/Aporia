use core::ops::RangeInclusive;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmSize {
    width: u32,
    height: u32,
}

impl DrmSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn is_within(
        &self,
        width_range: RangeInclusive<u32>,
        height_range: RangeInclusive<u32>,
    ) -> bool {
        width_range.contains(&self.width) && height_range.contains(&self.height)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmRect {
    x: u32,
    y: u32,
    size: DrmSize,
}

impl DrmRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            size: DrmSize::new(width, height),
        }
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn size(&self) -> DrmSize {
        self.size
    }

    pub fn width(&self) -> u32 {
        self.size.width()
    }

    pub fn height(&self) -> u32 {
        self.size.height()
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.size.width())
    }

    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.size.height())
    }

    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };

        (self.x..right).contains(&x) && (self.y..bottom).contains(&y)
    }

    pub fn contains_rect(&self, other: &Self) -> bool {
        let Some(self_right) = self.right() else {
            return false;
        };
        let Some(self_bottom) = self.bottom() else {
            return false;
        };
        let Some(other_right) = other.right() else {
            return false;
        };
        let Some(other_bottom) = other.bottom() else {
            return false;
        };

        self.x <= other.x
            && other_right <= self_right
            && self.y <= other.y
            && other_bottom <= self_bottom
    }

    pub fn set_x(&mut self, x: u32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: u32) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: u32) {
        self.size.width = width;
    }

    pub fn set_height(&mut self, height: u32) {
        self.size.height = height;
    }
}
