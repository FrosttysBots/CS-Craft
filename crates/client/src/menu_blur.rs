//! GPU menu backdrop. Pipeline setup adapted from Bevy 0.19 custom_post_processing
//! (Bevy contributors, MIT OR Apache-2.0); effect only runs on the final scene camera.
use bevy::{
    core_pipeline::{Core3dSystems, FullscreenShader, schedule::Core3d},
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        view::ViewTarget,
    },
};

const SHADER_ASSET_PATH: &str = "embedded://client/shaders/menu_blur.wgsl";

pub struct PostProcessPlugin;

impl Plugin for PostProcessPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/menu_blur.wgsl");
        app.add_systems(Update, update.after(crate::menu::input));
        app.add_plugins((
            ExtractComponentPlugin::<PostProcessSettings>::default(),
            UniformComponentPlugin::<PostProcessSettings>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.add_systems(RenderStartup, init_post_process_pipeline);
        render_app.add_systems(
            Core3d,
            post_process_system.in_set(Core3dSystems::PostProcess),
        );
    }
}

#[derive(Default)]
struct PostProcessBindGroupCache {
    cached: Option<(TextureViewId, BindGroup)>,
}

fn post_process_system(
    view: ViewQuery<(
        &ViewTarget,
        &PostProcessSettings,
        &DynamicUniformIndex<PostProcessSettings>,
    )>,
    post_process_pipeline: Option<Res<PostProcessPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    settings_uniforms: Res<ComponentUniforms<PostProcessSettings>>,
    mut cache: Local<PostProcessBindGroupCache>,
    mut ctx: RenderContext,
) {
    let Some(post_process_pipeline) = post_process_pipeline else {
        return;
    };

    let (view_target, settings, settings_index) = view.into_inner();
    if settings.intensity <= 0. {
        return;
    }

    let Some(pipeline) = pipeline_cache.get_render_pipeline(post_process_pipeline.pipeline_id)
    else {
        return;
    };

    let Some(settings_binding) = settings_uniforms.uniforms().binding() else {
        return;
    };

    let post_process = view_target.post_process_write();

    let bind_group = match &mut cache.cached {
        Some((texture_id, bind_group)) if post_process.source.id() == *texture_id => bind_group,
        cached => {
            let bind_group = ctx.render_device().create_bind_group(
                "post_process_bind_group",
                &pipeline_cache.get_bind_group_layout(&post_process_pipeline.layout),
                &BindGroupEntries::sequential((
                    post_process.source,
                    &post_process_pipeline.sampler,
                    settings_binding.clone(),
                )),
            );

            let (_, bind_group) = cached.insert((post_process.source.id(), bind_group));
            bind_group
        }
    };

    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("post_process_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

    render_pass.set_pipeline(pipeline);
    render_pass.set_bind_group(0, bind_group, &[settings_index.index()]);
    render_pass.draw(0..3, 0..1);
}

#[derive(Resource)]
struct PostProcessPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

fn init_post_process_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "post_process_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PostProcessSettings>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let shader = asset_server.load(SHADER_ASSET_PATH);
    let vertex_state = fullscreen_shader.to_vertex_state();
    let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("post_process_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: vertex_state,
        fragment: Some(FragmentState {
            shader,
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(PostProcessPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
pub struct PostProcessSettings {
    intensity: f32,
}

fn update(
    menu: Res<crate::menu::Menu>,
    options: Res<crate::settings::Settings>,
    mut views: Query<&mut PostProcessSettings>,
) {
    for mut settings in &mut views {
        settings.intensity = if menu.in_match && menu.open() && options.menu_blur {
            2.0
        } else {
            0.
        };
    }
}
