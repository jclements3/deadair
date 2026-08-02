//! Vec2 and Vec3 value types used throughout deadair.

use std::ops::{Add, Sub, Mul};
use serde::{Deserialize, Serialize};

/// A 2-D vector / point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub fn new(x: f32, y: f32) -> Self { Self { x, y } }
    pub fn zero() -> Self { Self { x: 0.0, y: 0.0 } }
    pub fn length(self) -> f32 { (self.x * self.x + self.y * self.y).sqrt() }
    pub fn distance(self, other: Vec2) -> f32 { (self - other).length() }
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > 1e-9 { Self { x: self.x / len, y: self.y / len } } else { Self::zero() }
    }
    pub fn dot(self, other: Vec2) -> f32 { self.x * other.x + self.y * other.y }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, r: Vec2) -> Vec2 { Vec2 { x: self.x + r.x, y: self.y + r.y } }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, r: Vec2) -> Vec2 { Vec2 { x: self.x - r.x, y: self.y - r.y } }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, r: f32) -> Vec2 { Vec2 { x: self.x * r, y: self.y * r } }
}

/// A 3-D vector / point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } }
    pub fn zero() -> Self { Self { x: 0.0, y: 0.0, z: 0.0 } }
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }
    pub fn distance(self, other: Vec3) -> f32 { (self - other).length() }
    /// Drop the Z component.
    pub fn xy(self) -> Vec2 { Vec2::new(self.x, self.y) }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, r: Vec3) -> Vec3 {
        Vec3 { x: self.x + r.x, y: self.y + r.y, z: self.z + r.z }
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, r: Vec3) -> Vec3 {
        Vec3 { x: self.x - r.x, y: self.y - r.y, z: self.z - r.z }
    }
}
