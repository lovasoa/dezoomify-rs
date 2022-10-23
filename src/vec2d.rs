use std::ops::{Add, Div, Mul, Sub};

#[derive(Debug, PartialEq, Eq, Hash, Default, Clone, Copy)]
pub struct Vec2d {
    pub x: u32,
    pub y: u32,
}

impl Vec2d {
    pub fn square(size: u32) -> Vec2d {
        Vec2d { x: size, y: size }
    }
    pub fn max<T: Into<Vec2d>>(self, other: T) -> Vec2d {
        let other = other.into();
        Vec2d {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
        }
    }
    pub fn min<T: Into<Vec2d>>(self, other: T) -> Vec2d {
        let other = other.into();
        Vec2d {
            x: self.x.min(other.x),
            y: self.y.min(other.y),
        }
    }
    pub fn ceil_div<T: Into<Vec2d>>(self, other: T) -> Vec2d {
        let other = other.into();
        let x: u32 = self.x / other.x + (self.x % other.x != 0) as u32;
        let y: u32 = self.y / other.y + (self.y % other.y != 0) as u32;
        Vec2d { x, y }
    }

    pub fn area(self) -> u64 {
        u64::from(self.x) * u64::from(self.y)
    }

    pub fn fits_inside(self, other: Vec2d) -> bool {
        self.x <= other.x && self.y <= other.y
    }
}

impl From<u32> for Vec2d {
    fn from(size: u32) -> Self { Vec2d::square(size) }
}

impl From<(u32, u32)> for Vec2d {
    fn from((x, y): (u32, u32)) -> Self { Vec2d { x, y } }
}

impl std::fmt::Display for Vec2d {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "x={} y={}", self.x, self.y)
    }
}

impl Add<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn add(self, rhs: Vec2d) -> Self::Output {
        Vec2d {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn sub(self, rhs: Vec2d) -> Self::Output {
        Vec2d {
            x: self.x.saturating_sub(rhs.x),
            y: self.y.saturating_sub(rhs.y),
        }
    }
}

impl Mul<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn mul(self, rhs: Vec2d) -> Self::Output {
        Vec2d {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
        }
    }
}

impl Mul<u32> for Vec2d {
    type Output = Vec2d;

    fn mul(self, rhs: u32) -> Self::Output {
        Vec2d {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl Div<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn div(self, rhs: Vec2d) -> Self::Output {
        Vec2d {
            x: self.x / rhs.x,
            y: self.y / rhs.y,
        }
    }
}

impl Div<u32> for Vec2d {
    type Output = Vec2d;

    fn div(self, rhs: u32) -> Self::Output {
        Vec2d {
            x: self.x / rhs,
            y: self.y / rhs,
        }
    }
}
