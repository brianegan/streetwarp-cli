use super::*;

const LONDON: GPXPoint = GPXPoint {
    lat: 51.5074,
    lng: -0.1278,
    ele: None,
};
const PARIS: GPXPoint = GPXPoint {
    lat: 48.8566,
    lng: 2.3522,
    ele: None,
};

/// Assert two floats agree to within `tol`.
fn close(actual: f64, expected: f64, tol: f64) {
    assert!(
        (actual - expected).abs() < tol,
        "expected {expected}, got {actual} (tolerance {tol})"
    );
}

// The expected values below were computed independently of the `geo` crate:
// Vincenty's inverse formula on the WGS84 ellipsoid for distance, and the
// standard great-circle formulas for bearing and intermediate points. They pin
// down which model each call uses, so a `geo` upgrade that quietly switches
// from an ellipsoidal to a spherical model is caught here.

#[test]
fn distance_is_wgs84_ellipsoidal() {
    // Spherical haversine over the same pair gives 343_556.5, so this tolerance
    // is tight enough to fail if the model silently becomes spherical.
    close(get_distance(&LONDON, &PARIS), 343_923.120_090, 1e-3);
}

#[test]
fn distance_is_symmetric_and_zero_for_identical_points() {
    close(
        get_distance(&LONDON, &PARIS),
        get_distance(&PARIS, &LONDON),
        1e-6,
    );
    close(get_distance(&LONDON, &LONDON), 0.0, 1e-9);
}

#[test]
fn bearing_is_great_circle_initial_bearing() {
    close(get_bearing(&LONDON, &PARIS), 148.115_616_871_053, 1e-9);
}

#[test]
fn bearing_due_north_is_zero() {
    let origin = GPXPoint {
        lat: 0.0,
        lng: 0.0,
        ele: None,
    };
    let north = GPXPoint {
        lat: 2.0,
        lng: 0.0,
        ele: None,
    };
    close(get_bearing(&origin, &north), 0.0, 1e-9);
}

#[test]
fn interp_points_fills_factor_minus_one_interior_points() {
    let filled = interp_points(vec![LONDON, PARIS], 4);
    assert_eq!(
        filled.len(),
        3,
        "one pair at factor 4 yields 3 interior points"
    );

    // Great-circle intermediate points at 1/4, 2/4 and 3/4 of the way.
    let expected = [
        (50.849_731_994_153, 0.518_416_706_242),
        (50.188_594_877_568, 1.146_617_629_030),
        (49.524_162_899_685, 1.757_619_896_023),
    ];
    for (point, (lat, lng)) in filled.iter().zip(expected) {
        close(point.lat, lat, 1e-9);
        close(point.lng, lng, 1e-9);
    }
}

#[test]
fn interp_points_passes_through_when_factor_below_two() {
    let input = vec![LONDON, PARIS];
    assert_eq!(interp_points(input.clone(), 1), input);
    assert_eq!(interp_points(input.clone(), 0), input);
}

#[test]
fn interp_points_interpolates_elevation_only_when_both_ends_have_it() {
    let low = GPXPoint {
        lat: 0.0,
        lng: 0.0,
        ele: Some(0.0),
    };
    let high = GPXPoint {
        lat: 0.0,
        lng: 1.0,
        ele: Some(100.0),
    };
    let filled = interp_points(vec![low, high], 4);
    let elevations: Vec<_> = filled.iter().map(|p| p.ele).collect();
    assert_eq!(elevations, vec![Some(0.0), Some(25.0), Some(50.0)]);

    let no_ele = GPXPoint { ele: None, ..high };
    let filled = interp_points(vec![low, no_ele], 4);
    assert!(filled.iter().all(|p| p.ele.is_none()));
}

#[test]
fn find_distances_returns_one_fewer_than_input() {
    let points = vec![LONDON, PARIS, LONDON];
    let distances = find_distances(&points);
    assert_eq!(distances.len(), 2);
    close(distances[0], distances[1], 1e-6);
}

#[test]
fn find_bearings_covers_every_point_and_repeats_the_last() {
    let points = vec![LONDON, PARIS, LONDON];
    let bearings = find_bearings(&points);
    assert_eq!(bearings.len(), points.len());
    // The final point inherits the bearing of the one before it.
    close(bearings[2].bearing, bearings[1].bearing, 1e-9);
}

#[test]
fn sample_points_by_distance_returns_requested_count() {
    // Eleven points spaced evenly along a line of longitude.
    let points: Vec<_> = (0..11)
        .map(|i| GPXPoint {
            lat: 0.0,
            lng: i as f64 * 0.1,
            ele: None,
        })
        .collect();
    let distances = find_distances(&points);
    let sample = sample_points_by_distance(&points, 5, &distances);
    assert_eq!(sample.len(), 5);
    assert_eq!(sample[0], points[0]);
    // Samples must stay in input order.
    assert!(sample.windows(2).all(|w| w[0].lng < w[1].lng));
}

#[test]
fn metadata_result_keeps_its_camel_case_wire_format() {
    // `--use-metadata` reads this JSON back and downstream consumers depend on
    // these exact keys, so the snake_case Rust fields must not leak out.
    let result = MetadataResult {
        distance: 12.5,
        frames: 2,
        gps_points: vec![SerializablePointBearing {
            lat: 51.5074,
            lng: -0.1278,
            bearing: 148.0,
            ele: Some(11.0),
        }],
        original_points: vec![LONDON],
        average_error: 0.25,
        name: "Test Route".to_string(),
        file_size_bytes: 64,
    };

    let json = serde_json::to_value(&result).expect("serialization failed");
    // `to_value` stores the object in a BTreeMap, so compare sorted names.
    let keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "averageError",
            "distance",
            "fileSizeBytes",
            "frames",
            "gpsPoints",
            "name",
            "originalPoints",
        ]
    );

    // And it round-trips, which is what --use-metadata relies on.
    let parsed: MetadataResult = serde_json::from_value(json).expect("deserialization failed");
    assert_eq!(parsed.gps_points, result.gps_points);
    assert_eq!(parsed.original_points, result.original_points);
    assert_eq!(parsed.file_size_bytes, 64);
}

#[test]
fn read_gpx_extracts_points_with_elevation() {
    let gpx = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
  <metadata><name>Test Route</name></metadata>
  <trk><trkseg>
    <trkpt lat="51.5074" lon="-0.1278"><ele>11.0</ele></trkpt>
    <trkpt lat="48.8566" lon="2.3522"></trkpt>
  </trkseg></trk>
</gpx>"#;

    let result = read_gpx(std::io::Cursor::new(gpx));
    assert_eq!(result.name.as_deref(), Some("Test Route"));
    assert_eq!(result.points.len(), 2);
    close(result.points[0].lat, 51.5074, 1e-9);
    close(result.points[0].lng, -0.1278, 1e-9);
    assert_eq!(result.points[0].ele, Some(11.0));
    assert_eq!(result.points[1].ele, None);
}

#[test]
fn read_gpx_parses_every_bundled_route() {
    // These are real ride exports. Parsing them is what actually exercises the
    // `gpx` crate against field data rather than a hand-written snippet.
    let res_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("res");
    let mut checked = 0;

    for entry in std::fs::read_dir(&res_dir).expect("could not read res/") {
        let path = entry.expect("bad dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("gpx") {
            continue;
        }

        let file = File::open(&path).expect("could not open fixture");
        let result = read_gpx(BufReader::new(file));
        assert!(
            !result.points.is_empty(),
            "{} parsed to zero points",
            path.display()
        );
        assert!(
            result
                .points
                .iter()
                .all(|p| (-90.0..=90.0).contains(&p.lat) && (-180.0..=180.0).contains(&p.lng)),
            "{} produced an out-of-range coordinate",
            path.display()
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "no .gpx fixtures found in {}",
        res_dir.display()
    );
}
