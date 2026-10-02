//! Daylight-detector power from the pinned 26.3 block update method.

/// `DaylightDetectorBlock.updateSignalStrength`: effective sky light and the
/// environment's sun angle in degrees are sampled at the detector position.
pub fn detector_power(effective_sky: u8, sun_angle_degrees: f32, inverted: bool) -> u8 {
    let mut light = i32::from(effective_sky.min(15));
    if inverted {
        light = 15 - light;
    } else if light > 0 {
        let mut angle = sun_angle_degrees * 0.017_453_292_f32;
        let target = if angle < std::f32::consts::PI {
            0.0_f32
        } else {
            6.283_185_5_f32
        };
        angle += (target - angle) * 0.2_f32;
        let table_index =
            ((f64::from(angle) * 10_430.378_350_470_453 + 16_384.0) as i64 & 65_535) as u32;
        let cosine = (f64::from(table_index) / 10_430.378_350_470_453).sin() as f32;
        light = ((light as f32 * cosine) + 0.5).floor() as i32;
    }
    light.clamp(0, 15) as u8
}

#[cfg(test)]
mod tests {
    use super::detector_power;

    #[test]
    fn source_formula_matches_measured_noon_night_and_inversion() {
        assert_eq!(detector_power(15, 0.0, false), 15);
        assert_eq!(detector_power(4, 180.0, false), 0);
        assert_eq!(detector_power(14, 1.0, true), 1);
        assert_eq!(detector_power(4, 180.0, true), 11);
    }
}
