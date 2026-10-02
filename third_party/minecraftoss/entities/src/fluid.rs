//! Pinned 26.3 EntityFluidInteraction depth sampling for local water/lava blocks.
use glam::DVec3;
use minecraftoss_player::World;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidFrame {
    pub water_height: f64,
    pub lava_height: f64,
}

impl FluidFrame {
    /// Source-water portion of EntityFluidInteraction's eye sample. The full
    /// fluid registry and waterlogged blocks are still outside this world API.
    pub fn eye_in_water(world: &impl World, position: DVec3, eye_height: f32) -> bool {
        // `EntityFluidInteraction.update`: any water block in the eye's
        // column from the feet up whose bottom is under the eyes and whose
        // camera height (`getHeightForCamera`: a full block for a source
        // under a sturdy underside) reaches them.
        let eye_y = position.y + f64::from(eye_height);
        let (x, z) = (position.x.floor() as i32, position.z.floor() as i32);
        for y in position.y.floor() as i32..=eye_y.floor() as i32 {
            let Some(block) = world.block((x, y, z)) else { continue };
            let waterlogged = block.property("waterlogged") == Some("true");
            if block.id != "minecraft:water" && !waterlogged {
                continue;
            }
            let level = if waterlogged { 0 } else { block.property("level").and_then(|value| value.parse::<u32>().ok()).unwrap_or(0) };
            let same_above = world.block((x, y + 1, z)).is_some_and(|above| above.id == "minecraft:water" || above.property("waterlogged") == Some("true"));
            // `FlowingFluid.getHeight`: `getOwnHeight` is the amount over 9
            // as a float.
            let amount = if level == 0 || level >= 8 { 8 } else { 8 - level };
            let height = if same_above { 1.0 } else { f64::from(amount as f32 / 9.0) };
            let camera = if level == 0 && world.sturdy_underside((x, y + 1, z)) { 1.0 } else { height };
            if eye_y >= f64::from(y) && eye_y <= f64::from(y) + camera {
                return true;
            }
        }
        false
    }

    pub fn in_water(self) -> bool {
        self.water_height > 0.0
    }

    pub fn in_lava(self) -> bool {
        self.lava_height > 0.0
    }

    /// `ENTITY_FLOATABLE` contains the water fluid tag in the pinned data pack.
    pub fn floatable_height(self) -> f64 {
        self.water_height
    }

    pub fn sample(world: &impl World, position: DVec3, width: f32, height: f32) -> Self {
        let margin = 0.001;
        let half_width = f64::from(width) / 2.0;
        let min = position - DVec3::new(half_width - margin, -margin, half_width - margin);
        let max = position
            + DVec3::new(
                half_width - margin,
                f64::from(height) - margin,
                half_width - margin,
            );
        let mut result = Self::default();
        for x in min.x.floor() as i32..max.x.ceil() as i32 {
            for y in min.y.floor() as i32..max.y.ceil() as i32 {
                for z in min.z.floor() as i32..max.z.ceil() as i32 {
                    let Some(block) = world.block((x, y, z)) else {
                        continue;
                    };
                    let kind = match block.id.as_str() {
                        "minecraft:water" => 0,
                        "minecraft:lava" => 1,
                        _ => continue,
                    };
                    let same_above = world
                        .block((x, y + 1, z))
                        .is_some_and(|above| above.id == block.id);
                    let level = block
                        .property("level")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(0);
                    let depth: f32 = if same_above {
                        1.0
                    } else if level >= 8 {
                        8.0 / 9.0
                    } else {
                        (8 - level) as f32 / 9.0
                    };
                    let top = y as f64 + f64::from(depth);
                    if top < min.y {
                        continue;
                    }
                    let body_depth = top - position.y;
                    if kind == 0 {
                        result.water_height = result.water_height.max(body_depth);
                    } else {
                        result.lava_height = result.lava_height.max(body_depth);
                    }
                }
            }
        }
        result
    }
}
