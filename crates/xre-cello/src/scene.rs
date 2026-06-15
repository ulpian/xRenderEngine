//! An arena-backed [`Scene`] graph with world-matrix propagation and flat
//! draw-list extraction.
//!
//! Nodes live in a `Vec` arena keyed by [`NodeId`]; each carries a local
//! [`Transform`], a parent link, and a [`NodeKind`]. World matrices propagate
//! parent→child with a dirty flag, and [`Scene::draw_list`] hoists the visible
//! `MeshInstance`s into a flat `(world, mesh, material)` list **outside** the
//! pixel loop — the Command_Line_3D anti-lesson
//! (`RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.2). Meshes are `Arc`-shared
//! so many nodes can instance one mesh.

use std::collections::HashMap;
use std::sync::Arc;

use xre_core::math::{Mat4, Vec3, Vec4};
use xre_core::Transform;
use xre_render::{Camera, Light, Material, Mesh};

/// A handle into a [`Scene`]'s node arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

/// What a node represents in the scene.
#[derive(Clone)]
pub enum NodeKind {
    /// A pure transform pivot.
    Empty,
    /// A drawable mesh instance with a material.
    Mesh {
        /// The shared mesh geometry.
        mesh: Arc<Mesh>,
        /// The material to shade it with.
        material: Material,
    },
    /// A light.
    Light(Light),
    /// A camera.
    Camera(Camera),
}

struct Node {
    transform: Transform,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    kind: NodeKind,
    world: Mat4,
}

/// One entry of an extracted draw list.
#[derive(Clone)]
pub struct DrawItem {
    /// The node's world matrix.
    pub world: Mat4,
    /// The shared mesh.
    pub mesh: Arc<Mesh>,
    /// The material.
    pub material: Material,
}

/// An arena scene graph rooted at [`Scene::root`].
pub struct Scene {
    nodes: Vec<Node>,
    root: NodeId,
    names: HashMap<String, NodeId>,
    dirty: bool,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// A new scene containing only the identity root node.
    #[must_use]
    pub fn new() -> Self {
        let root = Node {
            transform: Transform::IDENTITY,
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Empty,
            world: Mat4::IDENTITY,
        };
        Self {
            nodes: vec![root],
            root: NodeId(0),
            names: HashMap::new(),
            dirty: false,
        }
    }

    /// The root node id.
    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// The number of nodes (including the root).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the scene has only the root node.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Add a node with `transform` and `kind` under `parent`.
    pub fn add(&mut self, parent: NodeId, transform: Transform, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            transform,
            parent: Some(parent),
            children: Vec::new(),
            kind,
            world: Mat4::IDENTITY,
        });
        if let Some(p) = self.nodes.get_mut(parent.0) {
            p.children.push(id);
        }
        self.dirty = true;
        id
    }

    /// Add a mesh instance under the root.
    pub fn add_mesh(
        &mut self,
        transform: Transform,
        mesh: Arc<Mesh>,
        material: Material,
    ) -> NodeId {
        self.add(self.root, transform, NodeKind::Mesh { mesh, material })
    }

    /// Give `id` a name for later [`Scene::find`].
    pub fn name(&mut self, id: NodeId, name: impl Into<String>) {
        self.names.insert(name.into(), id);
    }

    /// Look up a node by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// Set a node's local transform (marks the graph dirty).
    pub fn set_transform(&mut self, id: NodeId, transform: Transform) {
        if let Some(n) = self.nodes.get_mut(id.0) {
            n.transform = transform;
            self.dirty = true;
        }
    }

    /// Mutably access a node's local transform; call [`Scene::touch`] after.
    pub fn transform_mut(&mut self, id: NodeId) -> Option<&mut Transform> {
        self.dirty = true;
        self.nodes.get_mut(id.0).map(|n| &mut n.transform)
    }

    /// Flag the world matrices as needing recompute.
    pub const fn touch(&mut self) {
        self.dirty = true;
    }

    /// Reparent `id` under `new_parent`, preserving its *world* transform.
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId) {
        if id == self.root || id == new_parent {
            return;
        }
        self.update_world_matrices();
        let world = self.nodes[id.0].world;
        // Detach from the old parent.
        if let Some(old) = self.nodes[id.0].parent {
            if let Some(p) = self.nodes.get_mut(old.0) {
                p.children.retain(|&c| c != id);
            }
        }
        // Attach to the new parent and solve the local transform that keeps the
        // world transform unchanged.
        let parent_world = self.nodes[new_parent.0].world;
        let local = parent_world.inverse() * world;
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        self.nodes[id.0].parent = Some(new_parent);
        self.nodes[id.0].transform = Transform {
            translation,
            rotation,
            scale,
        };
        self.nodes[new_parent.0].children.push(id);
        self.dirty = true;
    }

    /// The world matrix of `id` (recomputing the graph first if dirty).
    pub fn world_matrix(&mut self, id: NodeId) -> Mat4 {
        self.update_world_matrices();
        self.nodes.get(id.0).map_or(Mat4::IDENTITY, |n| n.world)
    }

    /// Recompute every node's world matrix by a parent-before-child pass. Cheap
    /// no-op when nothing changed.
    pub fn update_world_matrices(&mut self) {
        if !self.dirty {
            return;
        }
        // Breadth-first from the root: parents are always processed first.
        let mut queue = vec![self.root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let parent_world = self.nodes[id.0]
                .parent
                .map_or(Mat4::IDENTITY, |p| self.nodes[p.0].world);
            let local = self.nodes[id.0].transform.to_mat4();
            self.nodes[id.0].world = parent_world * local;
            queue.extend_from_slice(&self.nodes[id.0].children);
        }
        self.dirty = false;
    }

    /// Extract the visible mesh instances as a flat draw list, optionally
    /// frustum-culling against `view_proj` (pass `None` to skip culling).
    pub fn draw_list(&mut self, view_proj: Option<Mat4>) -> Vec<DrawItem> {
        self.update_world_matrices();
        let mut out = Vec::new();
        for node in &self.nodes {
            if let NodeKind::Mesh { mesh, material } = &node.kind {
                if let Some(vp) = view_proj {
                    if !aabb_visible(mesh, node.world, vp) {
                        continue;
                    }
                }
                out.push(DrawItem {
                    world: node.world,
                    mesh: Arc::clone(mesh),
                    material: *material,
                });
            }
        }
        out
    }

    /// Collect the scene's lights (resolved to world space for directional dir /
    /// point position).
    #[must_use]
    pub fn lights(&self) -> Vec<Light> {
        self.nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Light(l) => Some(world_light(*l, n.world)),
                _ => None,
            })
            .collect()
    }
}

/// Transform a light into world space using its node matrix.
fn world_light(light: Light, world: Mat4) -> Light {
    match light {
        Light::Directional {
            direction,
            color,
            intensity,
        } => Light::Directional {
            direction: world.transform_vector3(direction),
            color,
            intensity,
        },
        Light::Point {
            position,
            color,
            intensity,
            attenuation,
        } => Light::Point {
            position: world.transform_point3(position),
            color,
            intensity,
            attenuation,
        },
    }
}

/// Frustum cull a mesh's world-space AABB against `view_proj` (conservative: the
/// 8 corners are tested against all six clip planes).
fn aabb_visible(mesh: &Mesh, world: Mat4, view_proj: Mat4) -> bool {
    let aabb = mesh.aabb();
    let mvp = view_proj * world;
    let corners = [
        Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];
    let clip: Vec<Vec4> = corners
        .iter()
        .map(|c| mvp * Vec4::new(c.x, c.y, c.z, 1.0))
        .collect();
    // For each of the 6 clip planes, if all corners are outside, cull.
    // Planes (0..1 depth): -w<=x<=w, -w<=y<=w, 0<=z<=w.
    let all = |f: &dyn Fn(&Vec4) -> bool| clip.iter().all(f);
    if all(&|c| c.x < -c.w)
        || all(&|c| c.x > c.w)
        || all(&|c| c.y < -c.w)
        || all(&|c| c.y > c.w)
        || all(&|c| c.z < 0.0)
        || all(&|c| c.z > c.w)
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_render::Mesh;

    #[test]
    fn child_world_composes_with_parent() {
        let mut scene = Scene::new();
        let parent = scene.add(
            scene.root(),
            Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)),
            NodeKind::Empty,
        );
        let child = scene.add(
            parent,
            Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
            NodeKind::Empty,
        );
        let w = scene.world_matrix(child);
        let p = w.transform_point3(Vec3::ZERO);
        assert!((p - Vec3::new(10.0, 5.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn reparent_preserves_world_transform() {
        let mut scene = Scene::new();
        let a = scene.add(
            scene.root(),
            Transform::from_translation(Vec3::new(3.0, 0.0, 0.0)),
            NodeKind::Empty,
        );
        let b = scene.add(
            scene.root(),
            Transform::from_translation(Vec3::new(0.0, 7.0, 0.0)),
            NodeKind::Empty,
        );
        let before = scene.world_matrix(b).transform_point3(Vec3::ZERO);
        scene.reparent(b, a);
        let after = scene.world_matrix(b).transform_point3(Vec3::ZERO);
        assert!((before - after).length() < 1e-4, "{before:?} vs {after:?}");
    }

    #[test]
    fn draw_list_collects_mesh_instances() {
        let mut scene = Scene::new();
        let mesh = Arc::new(Mesh::cube());
        for i in 0..5 {
            scene.add_mesh(
                Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
                Arc::clone(&mesh),
                Material::default(),
            );
        }
        // Instancing: the 5 nodes plus this local handle all share one Arc.
        assert_eq!(Arc::strong_count(&mesh), 6);
        let list = scene.draw_list(None);
        assert_eq!(list.len(), 5);
        // The draw list holds its own clones of the shared geometry.
        assert_eq!(Arc::strong_count(&mesh), 11);
    }

    #[test]
    fn instancing_thousand_nodes() {
        let mut scene = Scene::new();
        let mesh = Arc::new(Mesh::uv_sphere(1.0, 4, 6));
        for i in 0..1000 {
            scene.add_mesh(
                Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
                Arc::clone(&mesh),
                Material::default(),
            );
        }
        assert_eq!(scene.draw_list(None).len(), 1000);
    }
}
