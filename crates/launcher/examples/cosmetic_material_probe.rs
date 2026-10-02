fn main(){
 let (_,fs)=level::mount_game(None).unwrap();
 for path in ["models/weapons/customization/knife_butterfly/knife_butterfly_masks","models/weapons/customization/knife_butterfly/knife_butterfly_ao","models/weapons/customization/knife_butterfly/knife_butterfly_surface","models/weapons/customization/paints/anodized_air/fade"]{
  let v=vtf::Vtf::parse(&fs.read(&vmt::vtf_path(path)).unwrap()).unwrap();
  let pixels=v.rgba8(0,0,0).unwrap();
  let mut alpha=std::collections::BTreeMap::new();for p in pixels.chunks_exact(4){*alpha.entry(p[3]).or_insert(0usize)+=1;}let mut alpha:Vec<_>=alpha.into_iter().collect();alpha.sort_by_key(|(_,n)|std::cmp::Reverse(*n));println!("alpha {:?}",&alpha[..alpha.len().min(12)]);
  let mut hist=std::collections::BTreeMap::new();
  for p in pixels.chunks_exact(4){*hist.entry([p[0]/16,p[1]/16,p[2]/16,p[3]/16]).or_insert(0usize)+=1;}
  let mut hist:Vec<_>=hist.into_iter().collect();hist.sort_by_key(|(_,n)|std::cmp::Reverse(*n));
  println!("{path} {}x{} frames={} {:?}",v.width,v.height,v.frames,&hist[..hist.len().min(12)]);
 }
}
