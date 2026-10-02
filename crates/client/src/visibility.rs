//! PVS rejection for static map batches, followed by Bevy's frustum culling.
use bevy::prelude::*;
use bevy::mesh::VertexAttributeValues;
#[derive(Component)]
pub struct MapClusters(Vec<usize>);

impl MapClusters {
    /// Include every leaf touching the full batch bounds, not just its centre.
    /// This deliberately overestimates visibility for large surfaces/props.
    pub fn mesh(bsp: &bsp::Bsp, mesh: &Mesh) -> Self {
        let Some(VertexAttributeValues::Float32x3(vertices)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return Self(Vec::new()); };
        let (mut lo, mut hi) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
        for p in vertices {
            let p = crate::coords::to_source(Vec3::from_array(*p));
            lo = lo.min(p); hi = hi.max(p);
        }
        lo -= glam::Vec3::splat(8.0); hi += glam::Vec3::splat(8.0);
        let mut clusters: Vec<_> = bsp.leafs.iter().filter(|leaf| leaf.cluster >= 0).filter(|leaf| {
            (0..3).all(|i| hi[i] >= leaf.mins[i] as f32 && lo[i] <= leaf.maxs[i] as f32)
        }).map(|leaf| leaf.cluster as usize).collect();
        clusters.sort_unstable(); clusters.dedup();
        Self(clusters)
    }
    fn visible(&self, row: Option<&[u8]>) -> bool {
        let Some(row) = row else { return true; };
        self.0.is_empty() || self.0.iter().any(|&c| row.get(c / 8).is_none_or(|b| b & (1 << (c % 8)) != 0))
    }
}

pub fn update(level: Res<crate::LoadedLevel>, local: Res<crate::player::LocalPlayer>,
    mut previous: Local<Option<i16>>, mut batches: Query<(&MapClusters, &mut Visibility)>) {
    let bsp = &level.0.bsp;
    let cluster = bsp.leafs.get(bsp.point_leaf(local.render_eye)).map_or(-1, |l| l.cluster);
    if *previous == Some(cluster) { return; }
    *previous = Some(cluster);
    let row = if std::env::var_os("CSR_NO_PVS").is_some() { None } else { bsp.cluster_pvs(cluster) };
    for (clusters, mut visibility) in &mut batches {
        *visibility = if clusters.visible(row.as_deref()) { Visibility::Inherited } else { Visibility::Hidden };
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn batches_keep_any_visible_leaf_and_fail_open() {
        assert!(MapClusters(vec![1,8]).visible(Some(&[0,1])));
        assert!(!MapClusters(vec![1,8]).visible(Some(&[1,0])));
        assert!(MapClusters(vec![]).visible(Some(&[0])));
        assert!(MapClusters(vec![99]).visible(Some(&[0])));
        assert!(MapClusters(vec![1]).visible(None));
    }
}
