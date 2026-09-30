fn main() {
    let path = std::env::args().nth(1).expect("nav path");
    let bytes = std::fs::read(&path).unwrap();
    let t = std::time::Instant::now();
    let m = nav::NavMesh::parse(&bytes).unwrap();
    println!("v{} {} places, {} areas in {:?}", m.version, m.places.len(), m.areas.len(), t.elapsed());
    let a = &m.areas[0];
    let b = m.areas.iter().max_by(|x, y| x.center().distance(a.center()).total_cmp(&y.center().distance(a.center()))).unwrap();
    let t = std::time::Instant::now();
    let p = m.find_path(a.id, b.id);
    println!("path {} -> {} ({}): {:?} steps in {:?}", a.id, b.id, a.center().distance(b.center()), p.as_ref().map(|p| p.len()), t.elapsed());
    let mut places = std::collections::BTreeMap::new();
    for ar in &m.areas { *places.entry(m.place_name(ar.place).unwrap_or("-")).or_insert(0) += 1; }
    println!("{places:?}");
}
