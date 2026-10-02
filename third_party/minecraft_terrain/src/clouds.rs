//! 26.3 fancy-cloud cells: 12 blocks wide, 4 blocks thick, opaque mask pixels.
use crate::{
    mesh::{ChunkMesh, Vertex},
    pack::{PackStack, ResourceId},
};
use anyhow::Result;

pub struct CloudMask {
    cells: Vec<bool>,
    width: i32,
    height: i32,
}
impl CloudMask {
    pub fn from_pack(packs: &PackStack) -> Result<Self> {
        let id = ResourceId::parse("minecraft:environment/clouds")?;
        let image = packs
            .texture(&id)?
            .map(|bytes| image::load_from_memory_with_format(&bytes, image::ImageFormat::Png))
            .transpose()?
            .map(|image| image.to_rgba8());
        if let Some(image) = image {
            Ok(Self {
                cells: image.pixels().map(|p| p[3] >= 10).collect(),
                width: image.width() as i32,
                height: image.height() as i32,
            })
        } else {
            Ok(Self {
                cells: vec![false],
                width: 1,
                height: 1,
            })
        }
    }
    fn occupied(&self, x: i32, z: i32) -> bool {
        let x = x.rem_euclid(self.width);
        let z = z.rem_euclid(self.height);
        self.cells[(z * self.width + x) as usize]
    }
    pub fn center(&self, x: f32, z: f32, time: f64) -> (i32, i32) {
        (
            ((x + time as f32 * 0.03) / 12.0).floor() as i32,
            ((z + 3.96) / 12.0).floor() as i32,
        )
    }
    pub fn build(&self, center: (i32, i32), camera_y: f32) -> ChunkMesh {
        self.build_options(center,camera_y,86,false)
    }
    pub fn build_options(&self,center:(i32,i32),camera_y:f32,radius:i32,flat:bool)->ChunkMesh {
        let mut mesh = ChunkMesh::default();
        let below = camera_y < 192.33;
        let above = camera_y > 196.33;
        // The 26.3 comparison client defaults to cloudRange=64 chunks.
        let radius = radius.clamp(8,86);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dz * dz > radius * radius {
                    continue;
                }
                let (x, z) = (center.0 + dx, center.1 + dz);
                if !self.occupied(x, z) {
                    continue;
                }
                let x0 = x as f32 * 12.0;
                let x1 = x0 + 12.0;
                let z0 = z as f32 * 12.0;
                let z1 = z0 + 12.0;
                let y0 = 192.33;
                let y1 = if flat {y0} else {y0 + 4.0};
                if !below || flat {
                    quad(
                        &mut mesh,
                        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
                        1.0,
                    );
                }
                if !above || flat {
                    quad(
                        &mut mesh,
                        [[x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y0, z0]],
                        0.7,
                    );
                }
                if flat {continue;}
                if !self.occupied(x, z - 1) {
                    quad(
                        &mut mesh,
                        [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
                        0.8,
                    );
                }
                if !self.occupied(x, z + 1) {
                    quad(
                        &mut mesh,
                        [[x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]],
                        0.8,
                    );
                }
                if !self.occupied(x - 1, z) {
                    quad(
                        &mut mesh,
                        [[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]],
                        0.9,
                    );
                }
                if !self.occupied(x + 1, z) {
                    quad(
                        &mut mesh,
                        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
                        0.9,
                    );
                }
            }
        }
        mesh
    }
}

fn quad(mesh: &mut ChunkMesh, corners: [[f32; 3]; 4], shade: f32) {
    let start = mesh.vertices.len() as u32;
    for position in corners {
        mesh.vertices.push(Vertex {
            position,
            uv: [0.0, 0.0],
            color: [shade, shade, shade, 1.0],
            sky_light: 15.0,
            block_light: 0.0,
        });
    }
    mesh.indices
        .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    mesh.faces += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_clouds_are_flat_and_fancy_clouds_have_sides() {
        let mask=CloudMask{cells:vec![true,false,false,false],width:2,height:2};
        let fast=mask.build_options((0,0),80.,8,true);
        let fancy=mask.build_options((0,0),80.,8,false);
        assert!(!fast.indices.is_empty());
        assert!(fast.vertices.iter().all(|v|v.position[1]==192.33));
        assert!(fancy.vertices.iter().any(|v|v.position[1]>196.));
        assert!(fancy.indices.len()>fast.indices.len());
    }
}
