//! Exercise asynchronous light changes and stale-job replacement using installed assets.
use overworld::{glam_mc,terrain,Runtime};
use std::time::{Duration,Instant};
fn main()->Result<(),String>{
    assert!(std::env::var_os("CSR_WORLD_DIR").is_some(),"Use an isolated test save");
    let mut r=Runtime::load_seed(Some(853111))?;
    r.set_render_distance(3);
    let feet=r.spawn_source();
    let p=r.save.origin;
    let camera=terrain::sections::CullCamera{position:glam_mc::DVec3::from_array(p)+glam_mc::DVec3::Y*2.,forward:glam_mc::Vec3::X,fov_degrees:74.,aspect:16./9.,yaw_degrees:0.,pitch_degrees:0.};
    let (x,y,z)=overworld::block_pos(p);
    let pos=(x+2,y+3,z);
    r.set_block((pos.0,pos.1-1,pos.2),Some(terrain::scene::Block::new("minecraft:stone")));
    r.set_block(pos,None);
    let settle=|r:&mut Runtime,expected:Option<u8>| {
        let start=Instant::now();let mut frames=0;let mut max=Duration::ZERO;
        loop {
            r.tick(0.05,feet,false);
            let t=Instant::now();r.frame(&camera);max=max.max(t.elapsed());frames+=1;
            let loaded=r.light.chunk_column((pos.0>>4,pos.2>>4)).is_some();
            if loaded && frames>=20 && r.stream.lighting_stats().0==0 && expected.is_none_or(|n|r.light.get_block(pos)==n){break;}
            assert!(start.elapsed()<Duration::from_secs(30),"lighting did not settle: {} light {}",r.stream.diag(),r.light.get_block(pos));
            std::thread::sleep(Duration::from_millis(5));
        }
        println!("Settled {frames} frames in {:.2}s; maximum host frame {:.2}ms",start.elapsed().as_secs_f32(),max.as_secs_f32()*1000.);
    };
    settle(&mut r,None);
    let original=r.light.get_block(pos);
    r.set_block(pos,Some(terrain::scene::Block::new("minecraft:torch")));
    settle(&mut r,Some(14));
    assert!(r.light.get_block((pos.0+1,pos.1,pos.2))>=13);
    // Supersede an in-flight removal with placement, then remove again. Stale epochs must not win.
    r.set_block(pos,None);r.frame(&camera);
    r.set_block(pos,Some(terrain::scene::Block::new("minecraft:torch")));r.frame(&camera);
    r.set_block(pos,None);
    settle(&mut r,Some(original));
    println!("PASS: worker relighting adds/removes torch light, lights neighbors and rejects superseded edits.");
    Ok(())
}
