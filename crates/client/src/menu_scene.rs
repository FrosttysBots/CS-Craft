//! Isolated studio preview; no gameplay entities or map lighting are shared.
use crate::{
    characters::CharacterModels,
    inventory::Inventory,
    material::WorldMaterial,
    menu::{Menu, Page},
    player_anim::{AnimInput, AnimState},
    skinning::{self, SkinnedAsset, SkinnedInstance},
};
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::render_resource::TextureFormat,
};

#[derive(Resource)]
pub struct Preview {
    pub image: Handle<Image>,
    camera: Entity,
    signature: String,
    instances: Vec<(SkinnedAsset, SkinnedInstance)>,
    anim: AnimState,
    weapon_transform: Mat4,
    weapon_pose: Vec<glam::Mat4>,
    target_height: u32,
    weapon_extent: Vec3,
}
pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(
        720,
        800,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                clear_color: Color::NONE.into(),
                is_active: false,
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            RenderLayers::layer(3),
            Tonemapping::None,
            Msaa::Sample4,
            Projection::Perspective(PerspectiveProjection {
                fov: 40.0_f32.to_radians(),
                near: 0.1,
                far: 500.0,
                ..default()
            }),
            Transform::default(),
        ))
        .id();
    commands.insert_resource(Preview {
        image,
        camera,
        signature: String::new(),
        instances: vec![],
        anim: default(),
        weapon_transform: Mat4::IDENTITY,
        weapon_pose: vec![],
        target_height: 800,
        weapon_extent: Vec3::splat(50.0),
    });
}
pub fn update(
    mut commands: Commands,
    menu: Res<Menu>,
    inventory: Res<Inventory>,
    (cases, collection): (
        Res<crate::case_ui::CaseUi>,
        Res<crate::cosmetics::Inventory>,
    ),
    view_models: Res<crate::viewmodel::ViewModels>,
    mut preview: ResMut<Preview>,
    mut models: ResMut<CharacterModels>,
    mut transforms: Query<&mut Transform>,
    mut cameras: Query<&mut Camera>,
    material_handles: Query<&MeshMaterial3d<WorldMaterial>>,
    mut materials: ResMut<Assets<WorldMaterial>>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    building:Res<crate::overworld_client::Building>,
) {
    let main = menu.page == Some(Page::Main) || (!menu.open() && building.initialized && building.inventory_open);
    let case_item = if menu.page == Some(Page::Cases) {
        cases.preview_item()
    } else {
        None
    };
    let selected = case_item.map_or(inventory.selected.as_str(), |i| i.weapon.as_str());
    let active = main || menu.page == Some(Page::Inventory) || case_item.is_some();
    if let Ok(mut camera) = cameras.get_mut(preview.camera) {
        camera.is_active = active;
    }
    let signature = if main {
        format!(
            "character:{}",
            collection.active("gloves").map_or(0, |i| i.serial)
        )
    } else if active {
        format!("{}:{}", selected, case_item.map_or(0, |i| i.serial))
    } else {
        String::new()
    };
    if preview.signature != signature {
        for (_, instance) in preview.instances.drain(..) {
            for e in instance.meshes.iter().chain(&instance.joints) {
                commands.entity(*e).despawn();
            }
        }
        let assets: Vec<_> = if main {
            models
                .counter_terrorist
                .as_ref()
                .map(|m| m.asset.clone())
                .into_iter()
                .chain(models.weapons.get("weapon_m4a1_silencer").cloned())
                .collect()
        } else {
            view_models
                .preview(selected)
                .or_else(|| models.weapons.get(selected))
                .cloned()
                .into_iter()
                .collect()
        };
        for asset in assets {
            let instance = skinning::spawn(
                &mut commands,
                &asset,
                Some(RenderLayers::layer(3)),
                Some(&mut materials),
            );
            let weapon = if main {
                "weapon_m4a1_silencer".to_string()
            } else {
                selected.to_string()
            };
            crate::skin_finish::tag(
                &mut commands,
                &instance,
                crate::skin_finish::Target::Preview(weapon.clone()),
            );
            if main && asset.model.name.contains("ctm_sas") {
                crate::decorations::tag(
                    &mut commands,
                    &asset,
                    &instance,
                    crate::skin_finish::Target::Gloves,
                    3,
                );
                if collection.active("gloves").is_some() {
                    for (index, e) in instance.meshes.iter().enumerate() {
                        if asset.model.textures[asset
                            .model
                            .skin_texture(0, asset.model.meshes[index].material)]
                        .contains("glove")
                        {
                            commands.entity(*e).insert(Visibility::Hidden);
                        }
                    }
                }
            }
            if !main {
                crate::decorations::tag(
                    &mut commands,
                    &asset,
                    &instance,
                    crate::skin_finish::Target::Preview(weapon),
                    3,
                );
            }
            preview.instances.push((asset, instance));
        }
        if !main {
            if let Some((asset, _)) = preview.instances.first() {
                let m = &asset.model;
                let hand = m.bones.iter().find(|b| b.name == "weapon_hand_R").map_or(
                    if view_models.preview(selected).is_some() {
                        glam::Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2)
                    } else {
                        glam::Mat3::IDENTITY
                    },
                    |b| glam::Mat3::from_mat4(b.pose_to_bone),
                );
                let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for p in m.meshes.iter().flat_map(|m| {
                    m.vertices
                        .iter()
                        .map(|v| crate::coords::to_bevy(hand * v.position))
                }) {
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
                let scale = 50.0 / (hi - lo).max_element().max(1.0);
                let transform = Mat4::from_scale(Vec3::splat(scale))
                    * Mat4::from_translation(-(lo + hi) * 0.5)
                    * Mat4::from_quat(crate::coords::rotation_to_bevy(hand));
                let pose = m.bones.iter().map(|b| b.pose_to_bone.inverse()).collect();
                preview.weapon_transform = transform;
                preview.weapon_pose = pose;
                preview.weapon_extent = (hi - lo) * scale;
            }
        }
        preview.signature = signature;
        preview.anim = default();
    }
    if !active {
        return;
    }
    let target_height = if main {
        800
    } else {
        if case_item.is_some() {
            (720.0 * (window.height() * 0.30).min(300.) / (window.width() * 0.60).min(720.)).round()
                as u32
        } else {
            (720.0 * (window.height() - 580.0).clamp(130.0, 310.0) / 324.0).round() as u32
        }
    };
    if preview.target_height != target_height {
        if let Some(mut img) = images.get_mut(&preview.image) {
            *img =
                Image::new_target_texture(720, target_height, TextureFormat::Rgba8UnormSrgb, None);
        }
        preview.target_height = target_height;
    }
    let camera_transform = if main {
        Transform::from_xyz(96.0, 45.0, -110.0).looking_at(Vec3::new(0.0, 36.0, 0.0), Vec3::Y)
    } else {
        let extent = preview.weapon_extent;
        let aspect = 720.0 / target_height as f32;
        let distance = (extent.x.max(extent.z) / aspect).max(extent.y) * 1.35
            / (2.0 * 20.0_f32.to_radians().tan());
        let distance = distance.max(35.0);
        Transform::from_xyz(0.0, distance * 0.14, distance).looking_at(Vec3::ZERO, Vec3::Y)
    };
    if let Ok(mut t) = transforms.get_mut(preview.camera) {
        *t = camera_transform;
    }
    if main {
        if let Some(model) = models.counter_terrorist.as_mut() {
            let pose = model
                .anims
                .menu_pose(time.elapsed_secs())
                .unwrap_or_else(|| {
                    model.anims.evaluate(
                        &mut preview.anim,
                        &AnimInput {
                            speed: 0.0,
                            move_yaw: 0.0,
                            on_ground: true,
                            ducked: false,
                            pitch: 8.0,
                            class: weapons::WeaponClass::Rifle,
                            time_since_fire: 99.0,
                            reload_progress: None,
                            air_time: 0.0,
                        },
                        time.delta_secs(),
                    )
                });
            let sway = Mat4::IDENTITY;
            for (i, (asset, instance)) in preview.instances.iter().enumerate() {
                let bones = if i == 0 {
                    pose.clone()
                } else {
                    skinning::bone_merge(&asset.model.bones, &model.asset.model.bones, &pose)
                };
                skinning::apply_pose(instance, sway, &bones, &mut transforms);
            }
        }
    } else {
        for (_, instance) in &preview.instances {
            skinning::apply_pose(
                instance,
                Mat4::from_rotation_y(if case_item.is_some() {
                    (time.elapsed_secs() * 0.4).sin() * 0.45
                } else {
                    inventory.yaw
                }) * preview.weapon_transform,
                &preview.weapon_pose,
                &mut transforms,
            );
        }
    }
    for (_, instance) in &preview.instances {
        for e in &instance.meshes {
            if let Ok(handle) = material_handles.get(*e) {
                if let Some(mut m) = materials.get_mut(&handle.0) {
                    m.params.ambient_cube = [Vec4::new(0.46, 0.49, 0.54, 1.0); 6];
                    m.params.sun_dir = Vec4::new(0.3, 0.8, 0.5, 1.0);
                    m.params.sun_color = Vec4::new(0.65, 0.60, 0.5, 1.0);
                    m.params.exposure = 1.7;
                    m.params.scope_aperture = Vec4::ZERO;
                }
            }
        }
    }
}
