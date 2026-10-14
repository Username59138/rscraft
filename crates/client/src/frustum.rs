use glam::{Mat4, Vec3, Vec4};

/// View frustum as six planes (xyz = normal pointing inwards, w = distance).
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extracts the planes from a view-projection matrix with 0..1 depth
    /// (Gribb–Hartmann method).
    pub fn from_view_proj(m: Mat4) -> Self {
        let [r0, r1, r2, r3] = [m.row(0), m.row(1), m.row(2), m.row(3)];
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| p / p.truncate().length());
        Self { planes }
    }

    /// Conservative test: false only if the box is entirely outside.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|plane| {
            let normal = plane.truncate();
            // The box corner furthest along the plane normal.
            let corner = Vec3::select(normal.cmpge(Vec3::ZERO), max, min);
            normal.dot(corner) + plane.w >= 0.0
        })
    }
}
