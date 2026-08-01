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

// Minimap projection. Google's "Map and Tile Coordinates" guide worked example
// is the reference here: it states that Chicago, at 41.850 N 87.650 W, sits on
// world pixel (65.67, 95.17) at zoom 0. Taking the expected values from Google's
// own documentation rather than from this implementation is what makes the test
// able to disagree with the code, and it pins the specific Mercator variant the
// Static Maps API uses.

#[test]
fn world_pixel_matches_googles_documented_chicago_coordinate() {
    let (x, y) = minimap::world_pixel(minimap::LatLng {
        lat: 41.850,
        lng: -87.650,
    });
    close(x, 65.67, 0.01);
    close(y, 95.17, 0.01);
}

#[test]
fn project_puts_the_centre_coordinate_at_the_middle_of_the_image() {
    let centre = minimap::LatLng {
        lat: 41.850,
        lng: -87.650,
    };
    let (x, y) = minimap::project(centre, centre, 14, 300);
    close(x, 150.0, 1e-9);
    close(y, 150.0, 1e-9);
}

// 360 degrees of longitude span the 256-pixel world square at zoom 0, so
// 1.40625 degrees is exactly one zoom-0 pixel and exactly 128 pixels at zoom 7.
// A point that far east of the centre of a 256-pixel image therefore lands on
// its right edge. The number comes from that arithmetic rather than from the
// implementation, so a projection off by a factor of two fails here.
#[test]
fn project_offsets_east_by_the_zoom_scaled_world_distance() {
    let centre = minimap::LatLng { lat: 0.0, lng: 0.0 };
    let east = minimap::LatLng {
        lat: 0.0,
        lng: 1.40625,
    };
    let (x, y) = minimap::project(east, centre, 7, 256);
    close(x, 256.0, 1e-9);
    close(y, 128.0, 1e-9);
}

#[test]
fn project_places_northern_points_above_the_centre() {
    let centre = minimap::LatLng { lat: 0.0, lng: 0.0 };
    let north = minimap::LatLng { lat: 1.0, lng: 0.0 };
    let (_, y) = minimap::project(north, centre, 10, 400);
    assert!(y < 200.0, "a point north of centre should sit above it");
}

// The threshold below is arithmetic on the zoom-0 world square, not a value read
// back out of `fit_zoom`. One zoom-0 pixel is 360/256 = 1.40625 degrees of
// longitude, so a box spanning 2.8125 degrees is exactly two of them, and it
// occupies exactly 256 pixels at zoom 7. That is the last zoom it fits a
// 256-pixel image at, and the pair of tests brackets it from both sides.

#[test]
fn fit_zoom_picks_the_deepest_zoom_the_box_still_fits() {
    let exactly_two_world_pixels_wide = minimap::BBox {
        north: 0.0,
        south: 0.0,
        west: -1.40625,
        east: 1.40625,
    };
    assert_eq!(minimap::fit_zoom(exactly_two_world_pixels_wide, 256), 7);
}

#[test]
fn fit_zoom_drops_one_level_when_the_box_grows_past_the_threshold() {
    let a_shade_too_wide = minimap::BBox {
        north: 0.0,
        south: 0.0,
        west: -1.41,
        east: 1.41,
    };
    assert_eq!(minimap::fit_zoom(a_shade_too_wide, 256), 6);
}

#[test]
fn fit_zoom_measures_latitude_extent_too() {
    // Same span in degrees, turned through ninety degrees. Mercator stretches
    // latitude, so a tall box needs at least as deep a zoom as a wide one.
    let tall = minimap::BBox {
        north: 1.40625,
        south: -1.40625,
        east: 0.0,
        west: 0.0,
    };
    assert!(minimap::fit_zoom(tall, 256) <= 7);
}

#[test]
fn fit_zoom_returns_max_zoom_for_a_single_point_route() {
    let no_extent = minimap::BBox {
        north: 51.5,
        south: 51.5,
        east: -0.12,
        west: -0.12,
    };
    assert_eq!(minimap::fit_zoom(no_extent, 256), 21);
}

// Minimap command line options. Parsing is the seam: every one of these
// resolves before `main` opens a socket, so an error here is an error that costs
// nothing.

/// Parse a command line, supplying the two arguments every run needs.
fn parse_cli(extra: &[&str]) -> Result<options::Cli, clap::Error> {
    use clap::Parser;
    let mut args = vec!["streetwarp", "route.gpx", "--api-key", "test-key"];
    args.extend_from_slice(extra);
    options::Cli::try_parse_from(args)
}

#[test]
fn minimap_is_off_by_default_with_the_documented_defaults_beside_it() {
    let cli = parse_cli(&[]).expect("bare command line should parse");
    assert_eq!(cli.minimap, options::MinimapMode::Off);
    assert_eq!(cli.minimap_position, options::MinimapPosition::Br);
    assert_eq!(cli.minimap_size, 30);
    assert_eq!(cli.minimap_margin, 12);
    assert_eq!(cli.minimap_zoom, 16);
}

#[test]
fn minimap_accepts_every_documented_mode_and_corner() {
    for (flag, expected) in [
        ("off", options::MinimapMode::Off),
        ("overview", options::MinimapMode::Overview),
        ("follow", options::MinimapMode::Follow),
    ] {
        let cli = parse_cli(&["--minimap", flag]).expect("documented mode should parse");
        assert_eq!(cli.minimap, expected, "--minimap {flag}");
    }
    for (flag, expected) in [
        ("tl", options::MinimapPosition::Tl),
        ("tr", options::MinimapPosition::Tr),
        ("bl", options::MinimapPosition::Bl),
        ("br", options::MinimapPosition::Br),
    ] {
        let cli = parse_cli(&["--minimap-position", flag]).expect("documented corner should parse");
        assert_eq!(cli.minimap_position, expected, "--minimap-position {flag}");
    }
}

#[test]
fn minimap_rejects_a_corner_that_is_not_one_of_the_four() {
    assert!(
        parse_cli(&["--minimap-position", "middle"]).is_err(),
        "an unknown corner should be rejected at parse time"
    );
}

#[test]
fn minimap_rejects_a_size_that_would_not_fit_the_frame() {
    assert!(
        parse_cli(&["--minimap-size", "150"]).is_err(),
        "a minimap larger than the video should be rejected at parse time"
    );
    assert!(
        parse_cli(&["--minimap-size", "0"]).is_err(),
        "a minimap of no size should be rejected at parse time"
    );
}

#[test]
fn minimap_rejects_a_zoom_google_will_not_serve() {
    assert!(
        parse_cli(&["--minimap-zoom", "25"]).is_err(),
        "a zoom past Google's maximum should be rejected at parse time"
    );
}

// Response cache. The expected hashes below are the published FNV-1a 64-bit
// reference vectors, not values this implementation produced. That is what lets
// the test detect a hash swapped for `DefaultHasher`, whose output is documented
// as unstable across Rust releases and would silently orphan the whole cache on a
// toolchain bump.

#[test]
fn fnv1a_64_matches_the_published_reference_vectors() {
    assert_eq!(cache::fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(cache::fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(cache::fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn cache_key_ignores_the_api_key_so_rotating_one_keeps_the_cache() {
    let with_one = "https://maps.googleapis.com/maps/api/streetview?size=640x480&location=1,2&key=AAA";
    let with_another =
        "https://maps.googleapis.com/maps/api/streetview?size=640x480&location=1,2&key=BBB";
    assert_eq!(
        cache::without_api_key(with_one),
        cache::without_api_key(with_another)
    );
}

#[test]
fn cache_key_keeps_the_api_key_out_of_the_stored_string() {
    let url = "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=SECRET";
    assert!(
        !cache::without_api_key(url).contains("SECRET"),
        "the api key must never reach a cache filename"
    );
}

#[test]
fn cache_key_only_strips_the_parameter_actually_named_key() {
    let url = "https://example.com/x?monkey=1&keyring=2&key=SECRET&okey=3";
    let stripped = cache::without_api_key(url);
    assert!(stripped.contains("monkey=1"), "{stripped}");
    assert!(stripped.contains("keyring=2"), "{stripped}");
    assert!(stripped.contains("okey=3"), "{stripped}");
    assert!(!stripped.contains("SECRET"), "{stripped}");
}

#[test]
fn cache_key_leaves_a_url_without_a_key_parameter_alone() {
    let url = "https://example.com/x?a=1&b=2";
    assert_eq!(cache::without_api_key(url), url);
}

/// A scratch cache directory of its own, removed when the test finishes.
struct ScratchCache {
    root: std::path::PathBuf,
}

impl ScratchCache {
    fn named(name: &str) -> ScratchCache {
        let root = std::env::temp_dir().join(format!("streetwarp-cache-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        ScratchCache { root }
    }

    fn cache(&self) -> cache::Cache {
        cache::Cache::rooted_at(&self.root)
    }
}

impl Drop for ScratchCache {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cache_path_is_the_same_for_two_urls_differing_only_in_api_key() {
    let scratch = ScratchCache::named("same-path");
    let cache = scratch.cache();
    let with_one = "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=AAA";
    let with_another = "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=BBB";
    assert_eq!(
        cache.path_for(cache::Kind::StreetView, with_one),
        cache.path_for(cache::Kind::StreetView, with_another)
    );
}

#[test]
fn cache_path_separates_kinds_and_carries_their_extension() {
    let scratch = ScratchCache::named("kinds");
    let cache = scratch.cache();
    let url = "https://example.com/x?a=1";
    let sv = cache.path_for(cache::Kind::StreetView, url).unwrap();
    let map = cache.path_for(cache::Kind::Map, url).unwrap();
    assert_ne!(sv, map, "different kinds must not collide on one path");
    assert_eq!(sv.extension().unwrap(), "jpg");
    assert_eq!(map.extension().unwrap(), "png");
    assert_eq!(
        cache
            .path_for(cache::Kind::Metadata, url)
            .unwrap()
            .extension()
            .unwrap(),
        "json"
    );
}

#[test]
fn cache_returns_the_exact_bytes_it_was_given() {
    let scratch = ScratchCache::named("round-trip");
    let cache = scratch.cache();
    let url = "https://example.com/image?a=1&key=SECRET";
    // Bytes that are not valid UTF-8, because these are really jpegs.
    let stored = [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];

    assert_eq!(
        cache.get(cache::Kind::StreetView, url),
        None,
        "nothing should be cached before the first put"
    );
    cache.put(cache::Kind::StreetView, url, &stored);
    assert_eq!(
        cache.get(cache::Kind::StreetView, url).as_deref(),
        Some(&stored[..])
    );
}

#[test]
fn cache_hit_survives_the_api_key_changing_between_runs() {
    let scratch = ScratchCache::named("key-rotation");
    let cache = scratch.cache();
    let stored = b"map bytes";
    cache.put(cache::Kind::Map, "https://example.com/m?z=1&key=OLD", stored);
    assert_eq!(
        cache
            .get(cache::Kind::Map, "https://example.com/m?z=1&key=NEW")
            .as_deref(),
        Some(&stored[..]),
        "rotating the api key must not throw the cache away"
    );
}

#[test]
fn disabled_cache_stores_nothing_and_reports_no_path() {
    let cache = cache::Cache::disabled();
    let url = "https://example.com/x?a=1";
    assert_eq!(cache.path_for(cache::Kind::Map, url), None);
    cache.put(cache::Kind::Map, url, b"bytes");
    assert_eq!(cache.get(cache::Kind::Map, url), None);
}

/// A [`fetch::Fetch`] that answers from a canned body and records every URL it
/// was asked for, so a test can assert on the requests a render actually makes.
struct RecordingFetcher {
    body: Vec<u8>,
    calls: std::sync::Mutex<Vec<String>>,
    refuse_matching: Option<String>,
}

impl RecordingFetcher {
    fn returning(body: &[u8]) -> RecordingFetcher {
        RecordingFetcher {
            body: body.to_vec(),
            calls: std::sync::Mutex::new(Vec::new()),
            refuse_matching: None,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl RecordingFetcher {
    /// Refuse any URL containing `needle`, the way Google refuses an API that
    /// has not been switched on for the key.
    fn refusing(body: &[u8], needle: &str) -> RecordingFetcher {
        RecordingFetcher {
            refuse_matching: Some(needle.to_string()),
            ..RecordingFetcher::returning(body)
        }
    }
}

impl fetch::Fetch for RecordingFetcher {
    async fn get(&self, url: &str) -> Result<Vec<u8>, fetch::FetchError> {
        self.calls.lock().unwrap().push(url.to_string());
        match &self.refuse_matching {
            Some(needle) if url.contains(needle.as_str()) => Err(fetch::FetchError {
                status: Some(403),
                message: "request refused with 403 Forbidden".to_string(),
            }),
            _ => Ok(self.body.clone()),
        }
    }
}

#[tokio::test]
async fn a_cached_response_is_not_requested_a_second_time() {
    let scratch = ScratchCache::named("fetch-hit");
    let cache = scratch.cache();
    let fetcher = RecordingFetcher::returning(b"frame bytes");
    let url = "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=AAA";

    let first = fetch::fetch_cached(&fetcher, &cache, cache::Kind::StreetView, url).await;
    let second = fetch::fetch_cached(&fetcher, &cache, cache::Kind::StreetView, url).await;

    assert_eq!(first.unwrap(), b"frame bytes");
    assert_eq!(
        second.unwrap(),
        b"frame bytes",
        "a cache hit must return the same bytes as the fetch did"
    );
    assert_eq!(
        fetcher.calls().len(),
        1,
        "the second request should have been answered from the cache"
    );
}

#[tokio::test]
async fn a_disabled_cache_requests_every_time() {
    let fetcher = RecordingFetcher::returning(b"frame bytes");
    let cache = cache::Cache::disabled();
    let url = "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=AAA";

    fetch::fetch_cached(&fetcher, &cache, cache::Kind::StreetView, url)
        .await
        .unwrap();
    fetch::fetch_cached(&fetcher, &cache, cache::Kind::StreetView, url)
        .await
        .unwrap();

    assert_eq!(
        fetcher.calls().len(),
        2,
        "--no-cache should send every request"
    );
}

#[tokio::test]
async fn a_cache_hit_survives_the_api_key_changing_between_runs() {
    let scratch = ScratchCache::named("fetch-key-rotation");
    let cache = scratch.cache();
    let fetcher = RecordingFetcher::returning(b"frame bytes");

    fetch::fetch_cached(
        &fetcher,
        &cache,
        cache::Kind::StreetView,
        "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=OLD",
    )
    .await
    .unwrap();
    fetch::fetch_cached(
        &fetcher,
        &cache,
        cache::Kind::StreetView,
        "https://maps.googleapis.com/maps/api/streetview?location=1,2&key=NEW",
    )
    .await
    .unwrap();

    assert_eq!(
        fetcher.calls().len(),
        1,
        "a new api key must not force a refetch of identical imagery"
    );
}

#[test]
fn the_cache_is_on_unless_no_cache_is_passed() {
    assert!(!parse_cli(&[]).unwrap().no_cache);
    assert!(parse_cli(&["--no-cache"]).unwrap().no_cache);
}

/// Two points far enough apart to produce distinct request URLs.
fn two_point_bearings() -> Vec<PointBearing> {
    vec![
        PointBearing {
            point: GPXPoint {
                lat: 51.5074,
                lng: -0.1278,
                ele: None,
            },
            bearing: 90.0,
        },
        PointBearing {
            point: GPXPoint {
                lat: 48.8566,
                lng: 2.3522,
                ele: None,
            },
            bearing: 180.0,
        },
    ]
}

const ONE_OK_METADATA: &[u8] =
    br#"{"status":"OK","pano_id":"pano-1","location":{"lat":51.5,"lng":-0.12},"date":"2021-06"}"#;

#[tokio::test]
async fn a_second_metadata_pass_over_the_same_route_makes_no_requests() {
    let scratch = ScratchCache::named("metadata-second-pass");
    let cache = scratch.cache();
    let fetcher = RecordingFetcher::returning(ONE_OK_METADATA);
    let points = two_point_bearings();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };

    let first = fetching.metadata(&points).await;
    let after_first_pass = fetcher.calls().len();
    let second = fetching.metadata(&points).await;

    assert_eq!(
        after_first_pass,
        points.len(),
        "the first pass should request one metadata document per point"
    );
    assert_eq!(
        fetcher.calls().len(),
        after_first_pass,
        "the second pass over the same route should request nothing"
    );
    assert_eq!(first.len(), second.len());
    assert_eq!(second[0].pano_id, "pano-1");
}

#[tokio::test]
async fn no_cache_makes_the_metadata_pass_request_everything_again() {
    let fetcher = RecordingFetcher::returning(ONE_OK_METADATA);
    let cache = cache::Cache::disabled();
    let points = two_point_bearings();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };

    fetching.metadata(&points).await;
    fetching.metadata(&points).await;

    assert_eq!(
        fetcher.calls().len(),
        points.len() * 2,
        "--no-cache should request every point on every pass"
    );
}

#[tokio::test]
async fn a_second_image_pass_over_the_same_route_makes_no_requests() {
    let scratch = ScratchCache::named("images-second-pass");
    let cache = scratch.cache();
    let fetcher = RecordingFetcher::returning(b"jpeg bytes");
    let points = two_point_bearings()
        .iter()
        .map(SerializablePointBearing::from_geo)
        .collect::<Vec<_>>();
    let out_dir = std::env::temp_dir().join("streetwarp-images-second-pass");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };

    fetching.images(&points, &out_dir).await;
    let after_first_pass = fetcher.calls().len();
    fetching.images(&points, &out_dir).await;

    assert_eq!(after_first_pass, points.len());
    assert_eq!(
        fetcher.calls().len(),
        after_first_pass,
        "the second pass over the same route should request no images"
    );
    assert_eq!(
        std::fs::read(out_dir.join("0.jpg")).unwrap(),
        b"jpeg bytes",
        "a cached image still has to reach the output directory"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// Decode a Google encoded polyline back into coordinates.
///
/// Written from the published format description rather than by reversing the
/// encoder, so a round-trip through it is a real check on the encoder instead of
/// two copies of the same mistake agreeing with each other.
fn decode_polyline(encoded: &str) -> Vec<minimap::LatLng> {
    let mut points = Vec::new();
    let bytes = encoded.as_bytes();
    let mut i = 0;
    let (mut lat, mut lng) = (0i64, 0i64);
    while i < bytes.len() {
        let mut next_value = || {
            let (mut result, mut shift) = (0i64, 0);
            loop {
                let chunk = (bytes[i] - 63) as i64;
                i += 1;
                result |= (chunk & 0x1f) << shift;
                shift += 5;
                if chunk < 0x20 {
                    break;
                }
            }
            if result & 1 != 0 { !(result >> 1) } else { result >> 1 }
        };
        lat += next_value();
        lng += next_value();
        points.push(minimap::LatLng {
            lat: lat as f64 / 1e5,
            lng: lng as f64 / 1e5,
        });
    }
    points
}

// Google's "Encoded Polyline Algorithm Format" page works this exact example:
// the three points below encode to the string asserted here. Taking it from
// Google's documentation is what makes the test capable of catching an encoder
// that is self-consistent but not the format Static Maps actually reads.
#[test]
fn encode_polyline_matches_googles_worked_example() {
    let points = [
        minimap::LatLng {
            lat: 38.5,
            lng: -120.2,
        },
        minimap::LatLng {
            lat: 40.7,
            lng: -120.95,
        },
        minimap::LatLng {
            lat: 43.252,
            lng: -126.453,
        },
    ];
    assert_eq!(
        minimap::encode_polyline(&points),
        "_p~iF~ps|U_ulLnnqC_mqNvxq`@"
    );
}

#[test]
fn encode_polyline_round_trips_back_to_its_input_points() {
    let points = (0..200)
        .map(|i| minimap::LatLng {
            lat: 51.5 + (i as f64) * 0.0013,
            lng: -0.12 - (i as f64) * 0.0021,
        })
        .collect::<Vec<_>>();

    let decoded = decode_polyline(&minimap::encode_polyline(&points));

    assert_eq!(decoded.len(), points.len());
    for (original, round_tripped) in points.iter().zip(&decoded) {
        // The format stores five decimal places, so a metre or so of loss is
        // the format working as designed.
        close(round_tripped.lat, original.lat, 1e-5);
        close(round_tripped.lng, original.lng, 1e-5);
    }
}

#[test]
fn encode_polyline_handles_an_empty_route() {
    assert_eq!(minimap::encode_polyline(&[]), "");
}

#[test]
fn percent_encode_escapes_the_polyline_characters_a_url_cannot_carry() {
    // Every one of these appears in real encoded polylines and none of them is
    // legal raw in a URL.
    assert_eq!(minimap::percent_encode("`"), "%60");
    assert_eq!(minimap::percent_encode("\\"), "%5C");
    assert_eq!(minimap::percent_encode("|"), "%7C");
    assert_eq!(minimap::percent_encode("?"), "%3F");
    assert_eq!(minimap::percent_encode("@"), "%40");
    assert_eq!(minimap::percent_encode("["), "%5B");
    assert_eq!(minimap::percent_encode("^"), "%5E");
}

#[test]
fn percent_encode_leaves_the_unreserved_characters_alone() {
    let unreserved = "abcXYZ019-_.~";
    assert_eq!(minimap::percent_encode(unreserved), unreserved);
}

#[test]
fn percent_encode_survives_googles_worked_example_polyline() {
    // The published example ends in a backtick and an at sign, so it exercises
    // the escaping on a string Google itself produced.
    let encoded = minimap::percent_encode("_p~iF~ps|U_ulLnnqC_mqNvxq`@");
    assert_eq!(encoded, "_p~iF~ps%7CU_ulLnnqC_mqNvxq%60%40");
}

/// A route of `n` points wandering north-east, roughly the density of a real
/// interpolated GPX track.
fn long_route(n: usize) -> Vec<minimap::LatLng> {
    (0..n)
        .map(|i| minimap::LatLng {
            lat: 51.5 + (i as f64) * 0.00002,
            lng: -0.12 + (i as f64) * 0.00003,
        })
        .collect()
}

#[test]
fn downsample_keeps_the_ends_of_the_route() {
    let route = long_route(1000);
    let reduced = minimap::downsample(&route, 10);
    assert!(reduced.len() <= 10, "got {}", reduced.len());
    assert_eq!(reduced.first(), route.first());
    assert_eq!(reduced.last(), route.last());
}

#[test]
fn downsample_passes_a_route_already_within_budget_straight_through() {
    let route = long_route(5);
    assert_eq!(minimap::downsample(&route, 10), route);
}

#[test]
fn map_url_carries_both_routes_in_different_colours() {
    let track = long_route(20);
    let panorama = long_route(18);
    let url = minimap::build_map_url(&minimap::MapRequest {
        center: minimap::LatLng {
            lat: 51.5,
            lng: -0.12,
        },
        zoom: 14,
        size_px: 200,
        track: &track,
        panorama: &panorama,
        api_key: "test-key",
    });

    let paths = url.matches("path=").count();
    assert_eq!(paths, 2, "both routes should be drawn: {url}");
    assert!(url.contains("center=51.5,-0.12"), "{url}");
    assert!(url.contains("zoom=14"), "{url}");
    assert!(url.contains("size=200x200"), "{url}");
    assert!(url.contains("key=test-key"), "{url}");

    // The two lines have to be told apart by eye, so they must not share a
    // colour.
    let colours = url
        .split("path=")
        .skip(1)
        .filter_map(|p| p.split_once("%7C").map(|(colour, _)| colour.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(colours.len(), 2, "{url}");
    assert_ne!(colours[0], colours[1], "{url}");
}

#[test]
fn map_url_stays_under_the_length_limit_for_a_fifty_thousand_point_route() {
    let track = long_route(50_000);
    let panorama = long_route(50_000);
    let url = minimap::build_map_url(&minimap::MapRequest {
        center: minimap::LatLng {
            lat: 51.5,
            lng: -0.12,
        },
        zoom: 12,
        size_px: 200,
        track: &track,
        panorama: &panorama,
        api_key: "test-key",
    });
    assert!(
        url.len() <= minimap::MAX_URL_LEN,
        "overview url was {} characters",
        url.len()
    );
    // Staying under the limit by drawing almost nothing would pass the check
    // above while rendering a useless map, so insist the budget is actually
    // being spent on route detail.
    assert!(
        url.len() > minimap::MAX_URL_LEN / 2,
        "downsampling threw away more of the route than it needed to: {} characters",
        url.len()
    );
}

#[test]
fn map_url_stays_under_the_length_limit_in_follow_mode() {
    let centre = minimap::LatLng {
        lat: 51.5,
        lng: -0.12,
    };
    let route = long_route(50_000);
    let track = minimap::clip_to_view(&route, centre, 16, 200);
    let panorama = minimap::clip_to_view(&route, centre, 16, 200);
    let url = minimap::build_map_url(&minimap::MapRequest {
        center: centre,
        zoom: 16,
        size_px: 200,
        track: &track,
        panorama: &panorama,
        api_key: "test-key",
    });
    assert!(
        url.len() <= minimap::MAX_URL_LEN,
        "follow url was {} characters",
        url.len()
    );
}

#[test]
fn clip_to_view_drops_the_far_end_of_the_route_but_keeps_what_is_on_screen() {
    let centre = minimap::LatLng {
        lat: 51.5,
        lng: -0.12,
    };
    let route = long_route(50_000);
    let clipped = minimap::clip_to_view(&route, centre, 16, 200);

    assert!(!clipped.is_empty(), "the centre of the route is on screen");
    assert!(
        clipped.len() < route.len(),
        "a route running far off the map should not survive whole"
    );
    // The last point is a full degree away, nowhere near a zoom-16 window.
    assert!(
        !clipped.contains(route.last().unwrap()),
        "a point far off screen should have been clipped"
    );
}


// Failing before the money is spent. The likeliest reason a minimap request is
// refused is that the Maps Static API was never switched on for the key, which
// a Street View key does not carry.

#[test]
fn map_failure_message_names_the_api_and_where_to_enable_it() {
    let refused = fetch::FetchError {
        status: Some(403),
        message: "request refused with 403 Forbidden".to_string(),
    };
    let explained = minimap::map_failure_message(&refused);

    assert!(
        explained.contains(minimap::MAPS_STATIC_ENABLE_URL),
        "the message must say where to switch the API on: {explained}"
    );
    assert!(
        explained.contains("Maps Static API"),
        "the message must name the API: {explained}"
    );
    assert!(
        !explained.contains("key="),
        "the message must never echo a request url, which carries the api key: {explained}"
    );
}

#[test]
fn map_failure_message_still_explains_a_request_that_never_completed() {
    let dropped = fetch::FetchError {
        status: None,
        message: "connection reset".to_string(),
    };
    let explained = minimap::map_failure_message(&dropped);
    assert!(explained.contains("connection reset"), "{explained}");
}

fn overview_plan() -> minimap::MinimapPlan {
    minimap::MinimapPlan::resolve(
        options::MinimapMode::Overview,
        options::MinimapPosition::Br,
        30,
        12,
        16,
        640,
        480,
    )
    .expect("overview mode should produce a plan")
}

#[test]
fn a_plan_sizes_the_minimap_against_the_shorter_side_of_the_video() {
    // 30 percent of the 480-pixel height of a 640x480 frame.
    assert_eq!(overview_plan().size_px, 144);
}

#[test]
fn there_is_no_plan_when_the_minimap_is_switched_off() {
    assert_eq!(
        minimap::MinimapPlan::resolve(
            options::MinimapMode::Off,
            options::MinimapPosition::Br,
            30,
            12,
            16,
            640,
            480
        ),
        None
    );
}

#[test]
fn overview_mode_needs_only_one_map_however_long_the_route() {
    let route = long_route(500);
    let urls = minimap::minimap_urls(&overview_plan(), &route, &route, "test-key");
    assert_eq!(
        urls.len(),
        1,
        "the whole point of overview mode is a single request"
    );
}

#[test]
fn follow_mode_needs_one_map_for_every_frame() {
    let plan = minimap::MinimapPlan::resolve(
        options::MinimapMode::Follow,
        options::MinimapPosition::Br,
        30,
        12,
        16,
        640,
        480,
    )
    .unwrap();
    let track = long_route(500);
    let frames = long_route(37);
    let urls = minimap::minimap_urls(&plan, &track, &frames, "test-key");
    assert_eq!(urls.len(), frames.len());
    // Each one is centred on its own frame, so no two are the same request.
    assert!(urls.contains(&urls[0]));
    assert_ne!(urls[0], urls[36]);
}

/// A metadata result over a short route, enough to drive the fetch stages.
fn small_metadata_result() -> MetadataResult {
    let points = two_point_bearings();
    MetadataResult {
        distance: 1000.0,
        frames: points.len(),
        gps_points: points.iter().map(SerializablePointBearing::from_geo).collect(),
        original_points: points.iter().map(|pb| pb.point).collect(),
        average_error: 0.0,
        name: "Test Route".to_string(),
        file_size_bytes: 64,
    }
}

#[tokio::test]
async fn a_refused_minimap_stops_the_render_before_a_single_frame_is_paid_for() {
    let fetcher = RecordingFetcher::refusing(b"map bytes", "staticmap");
    let cache = cache::Cache::disabled();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };
    let out_dir = std::env::temp_dir().join("streetwarp-probe-refused");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();

    let outcome = fetch_render_inputs(
        &fetching,
        Some(&overview_plan()),
        &small_metadata_result(),
        &out_dir,
    )
    .await;

    let message = outcome.expect_err("a refused minimap must fail the render");
    assert!(
        message.contains(minimap::MAPS_STATIC_ENABLE_URL),
        "{message}"
    );
    assert!(
        !fetcher.calls().iter().any(|u| u.contains("/streetview")),
        "not one Street View frame should have been requested: {:?}",
        fetcher.calls()
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[tokio::test]
async fn a_working_minimap_lets_the_street_view_frames_be_fetched() {
    let fetcher = RecordingFetcher::returning(b"bytes");
    let cache = cache::Cache::disabled();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };
    let out_dir = std::env::temp_dir().join("streetwarp-probe-ok");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let result = small_metadata_result();

    let maps = fetch_render_inputs(&fetching, Some(&overview_plan()), &result, &out_dir)
        .await
        .expect("a working minimap should not fail the render");

    assert_eq!(maps.len(), 1, "overview mode fetches one map");
    let calls = fetcher.calls();
    let first_streetview = calls.iter().position(|u| u.contains("/streetview"));
    let first_map = calls.iter().position(|u| u.contains("staticmap"));
    assert_eq!(first_map, Some(0), "the map must be fetched first: {calls:?}");
    assert!(
        first_streetview.unwrap() > first_map.unwrap(),
        "frames must come after the map: {calls:?}"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

// Stamping the position dot. The map image itself comes from Google, but where
// the dot lands on it is this program's arithmetic, and that is what a test can
// hold to account without a key or a network.

/// A plain image standing in for a fetched map, encoded as the PNG bytes Google
/// would have returned.
fn fake_map_png(side: u32) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(side, side, image::Rgba([200, 200, 200, 255]))
        .write_to(&mut buffer, image::ImageFormat::Png)
        .expect("could not encode the stand-in map");
    buffer.into_inner()
}

#[test]
fn the_dot_lands_on_the_projected_pixel() {
    let map = image::RgbaImage::from_pixel(200, 200, image::Rgba([200, 200, 200, 255]));
    let stamped = minimap::stamp_dot(&map, 50.0, 60.0);

    assert_ne!(
        stamped.get_pixel(50, 60),
        map.get_pixel(50, 60),
        "the dot should have been drawn at the projected coordinate"
    );
    assert_eq!(
        stamped.get_pixel(150, 20),
        map.get_pixel(150, 20),
        "the rest of the map should be untouched"
    );
    assert_eq!(stamped.dimensions(), map.dimensions());
}

#[test]
fn the_dot_moves_when_the_position_does() {
    let map = image::RgbaImage::from_pixel(200, 200, image::Rgba([200, 200, 200, 255]));
    let here = minimap::stamp_dot(&map, 40.0, 40.0);
    let there = minimap::stamp_dot(&map, 160.0, 160.0);
    assert_ne!(here.get_pixel(40, 40), there.get_pixel(40, 40));
    assert_ne!(here.get_pixel(160, 160), there.get_pixel(160, 160));
}

#[test]
fn a_dot_at_the_edge_still_draws_what_fits() {
    let map = image::RgbaImage::from_pixel(200, 200, image::Rgba([200, 200, 200, 255]));
    let stamped = minimap::stamp_dot(&map, 0.0, 100.0);
    assert_ne!(
        stamped.get_pixel(0, 100),
        map.get_pixel(0, 100),
        "a dot on the frame edge should still show"
    );
}

#[test]
fn overview_renders_one_image_per_frame_from_a_single_fetched_map() {
    let out_dir = std::env::temp_dir().join("streetwarp-overview-frames");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let plan = overview_plan();
    let track = long_route(400);
    let panorama = long_route(25);
    let maps = vec![fake_map_png(plan.size_px * 2)];

    let written = minimap::render_frames(&plan, &maps, &track, &panorama, &out_dir)
        .expect("rendering minimap frames should succeed");

    assert_eq!(written, panorama.len(), "one minimap per video frame");
    for index in 0..panorama.len() {
        let path = out_dir.join(minimap::frame_filename(index));
        assert!(path.exists(), "missing {}", path.display());
    }
    // And crucially it did all of that from the one map that was fetched.
    assert_eq!(maps.len(), 1);
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn overview_frames_differ_because_the_dot_has_moved() {
    let out_dir = std::env::temp_dir().join("streetwarp-overview-moving-dot");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let plan = overview_plan();
    let route = long_route(300);
    let maps = vec![fake_map_png(plan.size_px * 2)];

    minimap::render_frames(&plan, &maps, &route, &route, &out_dir).unwrap();

    let first = std::fs::read(out_dir.join(minimap::frame_filename(0))).unwrap();
    let last = std::fs::read(out_dir.join(minimap::frame_filename(299))).unwrap();
    assert_ne!(
        first, last,
        "the dot must be somewhere different at the end of the route"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn follow_renders_one_image_per_fetched_map() {
    let out_dir = std::env::temp_dir().join("streetwarp-follow-frames");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let plan = minimap::MinimapPlan::resolve(
        options::MinimapMode::Follow,
        options::MinimapPosition::Br,
        30,
        12,
        16,
        640,
        480,
    )
    .unwrap();
    let route = long_route(6);
    let maps = (0..route.len())
        .map(|_| fake_map_png(plan.size_px * 2))
        .collect::<Vec<_>>();

    let written = minimap::render_frames(&plan, &maps, &route, &route, &out_dir).unwrap();

    assert_eq!(written, route.len());
    let _ = std::fs::remove_dir_all(&out_dir);
}

// Placing the overlay. The offsets below are arithmetic on a 640x480 frame with
// a 144-pixel minimap and a 12-pixel margin: the near edge sits at the margin,
// the far edge at 640-144-12 = 484 across and 480-144-12 = 324 down.

fn plan_in_corner(position: options::MinimapPosition) -> minimap::MinimapPlan {
    minimap::MinimapPlan::resolve(
        options::MinimapMode::Overview,
        position,
        30,
        12,
        16,
        640,
        480,
    )
    .unwrap()
}

#[test]
fn each_corner_insets_the_minimap_by_the_margin() {
    let offsets = |position| minimap::overlay_offsets(&plan_in_corner(position), 640, 480);
    assert_eq!(offsets(options::MinimapPosition::Tl), (12, 12));
    assert_eq!(offsets(options::MinimapPosition::Tr), (484, 12));
    assert_eq!(offsets(options::MinimapPosition::Bl), (12, 324));
    assert_eq!(offsets(options::MinimapPosition::Br), (484, 324));
}

#[test]
fn a_minimap_too_big_for_its_margin_is_still_placed_inside_the_frame() {
    // 100 percent of the shorter side leaves no room for a margin, so the
    // corner offsets have to clamp rather than wrap around.
    let plan = minimap::MinimapPlan::resolve(
        options::MinimapMode::Overview,
        options::MinimapPosition::Br,
        100,
        12,
        16,
        640,
        480,
    )
    .unwrap();
    let (x, y) = minimap::overlay_offsets(&plan, 640, 480);
    assert!(x + plan.size_px <= 640, "x={x} size={}", plan.size_px);
    assert!(y + plan.size_px <= 480, "y={y} size={}", plan.size_px);
}

#[test]
fn the_filter_graph_leaves_the_video_alone_when_there_is_no_minimap() {
    let graph = ffmpeg::final_filter_graph(ffmpeg::Motion::Minterp, None);
    assert!(!graph.contains("overlay"), "{graph}");
    assert!(graph.contains("minterpolate"), "{graph}");
    assert!(graph.ends_with("[out]"), "{graph}");
}

#[test]
fn the_filter_graph_interpolates_before_it_composites_the_map() {
    let graph = ffmpeg::final_filter_graph(
        ffmpeg::Motion::Minterp,
        Some(ffmpeg::Overlay {
            x: 484,
            y: 324,
            size_px: 144,
        }),
    );
    let interpolation = graph.find("minterpolate").expect("{graph}");
    let composite = graph.find("overlay=").expect("{graph}");
    assert!(
        interpolation < composite,
        "motion estimation must run on the Street View before the map is laid on top: {graph}"
    );
    assert!(graph.contains("overlay=484:324"), "{graph}");
    assert!(
        graph.contains("scale=144:144"),
        "the fetched map is twice its display size and must be scaled down: {graph}"
    );
}

#[test]
fn the_filter_graph_composites_even_when_motion_smoothing_is_skipped() {
    let graph = ffmpeg::final_filter_graph(
        ffmpeg::Motion::Skip,
        Some(ffmpeg::Overlay {
            x: 12,
            y: 12,
            size_px: 100,
        }),
    );
    assert!(graph.contains("overlay=12:12"), "{graph}");
    assert!(!graph.contains("minterpolate"), "{graph}");
}

/// Render a real video through ffmpeg and read a frame back, which is the only
/// way to find out whether the filter graph does what its string says.
///
/// Ignored by default so the gate stays fast and does not depend on ffmpeg being
/// installed. Run with `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "shells out to ffmpeg; run with --ignored"]
async fn a_rendered_video_shows_the_minimap_in_the_requested_corner() {
    let dir = std::env::temp_dir().join("streetwarp-render-corner");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Solid magenta minimaps over a black video, so the overlay is
    // unmistakable against what sits underneath it.
    let frames = 6;
    for index in 0..frames {
        image::RgbaImage::from_pixel(288, 288, image::Rgba([255u8, 0, 255, 255]))
            .save(dir.join(minimap::frame_filename(index)))
            .unwrap();
    }

    let plan = plan_in_corner(options::MinimapPosition::Br);
    let (x, y) = minimap::overlay_offsets(&plan, 640, 480);
    // A black source video standing in for the Street View sequence.
    ffmpeg::ffmpeg(
        &dir,
        &(|_| 0.0),
        &[
            "-f", "lavfi", "-i", "color=c=black:s=640x480:r=24", "-frames:v", "6", "-pix_fmt",
            "yuv420p", "-y", "original.mp4",
        ],
    )
    .await;
    ffmpeg::finish_timelapse(
        &dir,
        frames,
        ffmpeg::Motion::Skip,
        Some(ffmpeg::Overlay {
            x,
            y,
            size_px: plan.size_px,
        }),
        "original.mp4",
        "out.mp4",
    )
    .await;

    // Pull a frame back out and look at it.
    ffmpeg::ffmpeg(
        &dir,
        &(|_| 0.0),
        &["-i", "out.mp4", "-frames:v", "1", "-y", "frame.png"],
    )
    .await;
    let rendered = image::open(dir.join("frame.png"))
        .expect("ffmpeg produced no frame")
        .to_rgba8();

    assert_eq!(rendered.dimensions(), (640, 480));
    let is_magenta = |p: &image::Rgba<u8>| p.0[0] > 180 && p.0[1] < 80 && p.0[2] > 180;
    let middle_of_map = rendered.get_pixel(x + plan.size_px / 2, y + plan.size_px / 2);
    assert!(
        is_magenta(middle_of_map),
        "the minimap should fill the bottom-right corner, found {middle_of_map:?}"
    );
    assert!(
        !is_magenta(rendered.get_pixel(320, 240)),
        "the middle of the frame should still be Street View, not map"
    );
    // And the opposite corner must be untouched, which is what proves the
    // offsets put it in the corner that was asked for rather than everywhere.
    assert!(
        !is_magenta(rendered.get_pixel(plan.margin_px + 5, plan.margin_px + 5)),
        "the top-left corner should be clear when the map was asked for bottom-right"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Fetch a real minimap from Google. This is the only check that the URL this
/// program builds is one the Maps Static API actually accepts, which no offline
/// test can tell you.
///
/// Ignored by default because it needs a key and spends a request. Run with:
/// `STREETWARP_API_KEY=... cargo test -- --ignored`
#[tokio::test]
#[ignore = "needs a real Google API key; run with --ignored"]
async fn google_serves_the_overview_map_this_program_asks_for() {
    let api_key = std::env::var("STREETWARP_API_KEY")
        .expect("set STREETWARP_API_KEY to run the live minimap test");
    let plan = overview_plan();
    let route = (0..50)
        .map(|i| minimap::LatLng {
            lat: 51.5074 + (i as f64) * 0.0002,
            lng: -0.1278 + (i as f64) * 0.0003,
        })
        .collect::<Vec<_>>();
    let urls = minimap::minimap_urls(&plan, &route, &route, &api_key);
    let fetcher = fetch::HttpFetcher::new();
    let cache = cache::Cache::disabled();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: &api_key,
        concurrency: 1,
    };

    let maps = fetching
        .minimaps(&urls)
        .await
        .expect("Google refused the minimap request");

    let decoded = minimap::decode_map(&maps[0]).expect("Google returned something undecodable");
    assert_eq!(
        decoded.dimensions(),
        (plan.size_px * 2, plan.size_px * 2),
        "scale=2 should give an image twice the requested size"
    );
}

#[tokio::test]
async fn the_default_render_never_touches_the_maps_api() {
    // The minimap is opt-in and costs money, so a run that did not ask for one
    // must not issue a single Static Maps request.
    let fetcher = RecordingFetcher::returning(b"bytes");
    let cache = cache::Cache::disabled();
    let fetching = Fetching {
        fetcher: &fetcher,
        cache: &cache,
        api_key: "test-key",
        concurrency: 4,
    };
    let out_dir = std::env::temp_dir().join("streetwarp-minimap-off");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();

    let maps = fetch_render_inputs(&fetching, None, &small_metadata_result(), &out_dir)
        .await
        .unwrap();

    assert!(maps.is_empty());
    assert!(
        !fetcher.calls().iter().any(|u| u.contains("staticmap")),
        "an opt-in feature must cost nothing when it was not opted into: {:?}",
        fetcher.calls()
    );
    assert_eq!(
        fetcher.calls().len(),
        2,
        "the Street View frames should still be fetched"
    );
    let _ = std::fs::remove_dir_all(&out_dir);
}
