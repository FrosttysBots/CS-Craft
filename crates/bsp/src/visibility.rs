//! Conservative decoding of the BSP potentially visible set (PVS).
use crate::Bsp;
impl Bsp {
    /// Invalid, absent, or out-of-world visibility means draw everything.
    pub fn cluster_pvs(&self, cluster: i16) -> Option<Vec<u8>> {
        let data = &self.visibility;
        let read = |offset: usize| -> Option<i32> { Some(i32::from_le_bytes(data.get(offset..offset+4)?.try_into().ok()?)) };
        let count = read(0)?;
        if !(1..=65536).contains(&count) || cluster < 0 || cluster as i32 >= count { return None; }
        let table_end = 4 + count as usize * 8;
        let offset = read(4 + cluster as usize * 8)?;
        if offset < table_end as i32 { return None; }
        let mut cursor = offset as usize;
        let size = (count as usize).div_ceil(8);
        let mut row = Vec::with_capacity(size);
        while row.len() < size {
            let byte = *data.get(cursor)?;
            cursor += 1;
            if byte != 0 { row.push(byte); }
            else {
                let run = *data.get(cursor)? as usize;
                cursor += 1;
                if run == 0 || row.len() + run > size { return None; }
                row.resize(row.len() + run, 0);
            }
        }
        // The camera's own cluster must always remain visible.
        row[cluster as usize / 8] |= 1 << (cluster as usize % 8);
        Some(row)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(row: &[u8]) -> Bsp {
        let mut b = Bsp::default();
        b.visibility.extend(24i32.to_le_bytes());
        for _ in 0..24 { b.visibility.extend(196i32.to_le_bytes()); b.visibility.extend((-1i32).to_le_bytes()); }
        b.visibility.extend(row);
        b
    }
    #[test] fn literal_and_zero_run() {
        assert_eq!(fixture(&[5,0,2]).cluster_pvs(0), Some(vec![5,0,0]));
    }
    #[test] fn broken_rows_fail_open() {
        for row in [&[0,0][..], &[0,4], &[0], &[1]] { assert!(fixture(row).cluster_pvs(0).is_none()); }
        assert!(fixture(&[1,1,1]).cluster_pvs(-1).is_none());
        assert!(fixture(&[1,1,1]).cluster_pvs(24).is_none());
        assert!(Bsp::default().cluster_pvs(0).is_none());
    }
}
